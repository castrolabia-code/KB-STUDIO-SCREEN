//! Codificación H.264 con Media Foundation.
//!
//! Orden de preferencia:
//! 1. MFT de HARDWARE (NVIDIA/AMD/Intel) vía `MFTEnumEx` — asíncrono.
//!    PENDIENTE DE VALIDACIÓN EN EQUIPO REAL (los runners de CI no tienen GPU).
//! 2. "H264 Encoder MFT" de Microsoft (software, síncrono) — probado en CI.
//!
//! `KBSS_ENCODER=software` fuerza la opción 2.
//!
//! Configuración de baja latencia: perfil Baseline (máxima compatibilidad
//! con decodificadores Android económicos), CBR, sin B-frames,
//! `CODECAPI_AVLowLatencyMode`, keyframe a pedido.

use kb_proto::StreamParams;
use kb_session::{EncodedFrame, VideoEncoder};
use windows::core::{Interface, GUID};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VARIANT;

pub fn mf_startup() -> Result<(), String> {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(|e| format!("MFStartup: {e}"))
}

fn e(ctx: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |err| format!("{ctx}: {err}")
}

pub struct MfEncoder {
    mft: IMFTransform,
    api: Option<ICodecAPI>,
    events: Option<IMFMediaEventGenerator>,
    name: String,
    params: StreamParams,
    out_buf_size: u32,
    provides_samples: bool,
    seq_header: Vec<u8>,
    need_input_credits: u32,
}

unsafe impl Send for MfEncoder {}

fn h264_out_type(p: &StreamParams) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        t.SetUINT32(&MF_MT_AVG_BITRATE, p.bitrate_kbps * 1000)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((p.width as u64) << 32) | p.height as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((p.fps as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
        Ok(t)
    }
}

fn nv12_in_type(p: &StreamParams) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((p.width as u64) << 32) | p.height as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((p.fps as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        Ok(t)
    }
}

/// Encoders de hardware disponibles (nombre, activador).
fn hardware_activates() -> Vec<(String, IMFActivate)> {
    let mut v = Vec::new();
    unsafe {
        let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
        let mut arr: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        if MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            None,
            Some(&info),
            &mut arr,
            &mut n,
        )
        .is_ok()
        {
            for k in 0..n as usize {
                if let Some(a) = (*arr.add(k)).take() {
                    let mut p = windows::core::PWSTR::null();
                    let mut len = 0;
                    let name = if a.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut p, &mut len).is_ok() {
                        let s = p.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(p.0 as _));
                        s
                    } else {
                        "hardware".into()
                    };
                    v.push((name, a));
                }
            }
            if !arr.is_null() {
                CoTaskMemFree(Some(arr as _));
            }
        }
    }
    v
}

impl MfEncoder {
    pub fn new(p: &StreamParams) -> Result<Self, String> {
        let force_sw = std::env::var("KBSS_ENCODER").is_ok_and(|v| v.eq_ignore_ascii_case("software"));
        if !force_sw {
            for (name, act) in hardware_activates() {
                match Self::from_activate(&act, &name, p) {
                    Ok(enc) => return Ok(enc),
                    Err(err) => kb_session::log(&format!("encoder de hardware '{name}' no utilizable: {err}; probando el siguiente")),
                }
            }
        }
        let mft: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER) }
            .map_err(e("CoCreateInstance(MSH264EncoderMFT)"))?;
        Self::configure(mft, "Microsoft H264 Encoder MFT (software)".into(), p, false)
    }

    fn from_activate(act: &IMFActivate, name: &str, p: &StreamParams) -> Result<Self, String> {
        let mft: IMFTransform = unsafe { act.ActivateObject() }.map_err(e("ActivateObject"))?;
        // Los MFT de hardware son asíncronos y hay que desbloquearlos explícitamente.
        unsafe {
            let attrs = mft.GetAttributes().map_err(e("GetAttributes"))?;
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(e("ASYNC_UNLOCK"))?;
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
        }
        Self::configure(mft, format!("{name} (hardware)"), p, true)
    }

    fn configure(mft: IMFTransform, name: String, p: &StreamParams, is_async: bool) -> Result<Self, String> {
        let api: Option<ICodecAPI> = mft.cast().ok();
        if let Some(api) = &api {
            unsafe {
                // Fallas individuales no son fatales: no todos los encoders soportan todo.
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
                let _ = api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32));
                let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(p.bitrate_kbps * 1000));
                let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0u32));
                let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(p.fps as u32 * 10));
            }
        }
        unsafe {
            mft.SetOutputType(0, &h264_out_type(p).map_err(e("tipo H.264"))?, 0).map_err(e("SetOutputType"))?;
            mft.SetInputType(0, &nv12_in_type(p).map_err(e("tipo NV12"))?, 0).map_err(e("SetInputType"))?;
            let info = mft.GetOutputStreamInfo(0).map_err(e("GetOutputStreamInfo"))?;
            let seq_header = mft
                .GetOutputCurrentType(0)
                .ok()
                .and_then(|t| blob(&t, &MF_MT_MPEG_SEQUENCE_HEADER))
                .unwrap_or_default();
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(e("BEGIN_STREAMING"))?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(e("START_OF_STREAM"))?;
            let events = if is_async { Some(mft.cast::<IMFMediaEventGenerator>().map_err(e("eventos"))?) } else { None };
            Ok(Self {
                mft,
                api,
                events,
                name,
                params: p.clone(),
                out_buf_size: info.cbSize.max(p.width * p.height * 3 / 2),
                provides_samples: (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0,
                seq_header,
                need_input_credits: 0,
            })
        }
    }

    fn make_input(&self, nv12: &[u8], pts_us: u64) -> windows::core::Result<IMFSample> {
        unsafe {
            let buf = MFCreateMemoryBuffer(nv12.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buf.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), ptr, nv12.len());
            buf.Unlock()?;
            buf.SetCurrentLength(nv12.len() as u32)?;
            let s = MFCreateSample()?;
            s.AddBuffer(&buf)?;
            s.SetSampleTime(pts_us as i64 * 10)?;
            s.SetSampleDuration(10_000_000 / self.params.fps.max(1) as i64)?;
            Ok(s)
        }
    }

    /// Extrae todas las salidas disponibles (síncrono) o UNA (asíncrono, por evento).
    fn drain(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), String> {
        loop {
            let sample = if self.provides_samples {
                None
            } else {
                unsafe {
                    let s = MFCreateSample().map_err(e("MFCreateSample"))?;
                    let b = MFCreateMemoryBuffer(self.out_buf_size).map_err(e("buffer salida"))?;
                    s.AddBuffer(&b).map_err(e("AddBuffer"))?;
                    Some(s)
                }
            };
            let mut odb = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            }];
            let mut status = 0u32;
            let r = unsafe { self.mft.ProcessOutput(0, &mut odb, &mut status) };
            let sample = unsafe { std::mem::ManuallyDrop::take(&mut odb[0].pSample) };
            unsafe { drop(std::mem::ManuallyDrop::take(&mut odb[0].pEvents)) };
            match r {
                Ok(()) => {
                    if let Some(s) = sample {
                        out.push(self.read_output(&s)?);
                    }
                    if self.events.is_some() {
                        return Ok(());
                    }
                }
                Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => unsafe {
                    // El encoder renegoció el formato de salida: se acepta el que propone.
                    let t = self.mft.GetOutputAvailableType(0, 0).map_err(e("GetOutputAvailableType"))?;
                    self.mft.SetOutputType(0, &t, 0).map_err(e("SetOutputType (cambio)"))?;
                    if let Some(h) = blob(&t, &MF_MT_MPEG_SEQUENCE_HEADER) {
                        self.seq_header = h;
                    }
                },
                Err(err) => return Err(format!("ProcessOutput: {err}")),
            }
        }
    }

    fn read_output(&self, s: &IMFSample) -> Result<EncodedFrame, String> {
        unsafe {
            let buf = s.ConvertToContiguousBuffer().map_err(e("ConvertToContiguousBuffer"))?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut ptr, None, Some(&mut len)).map_err(e("Lock salida"))?;
            let mut data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
            buf.Unlock().map_err(e("Unlock salida"))?;
            let keyframe = s.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1 || has_nal(&data, 5);
            // Algunos encoders no repiten SPS/PPS en banda: se anteponen para
            // que un cliente que se une tarde pueda decodificar.
            if keyframe && !has_nal(&data, 7) && !self.seq_header.is_empty() {
                let mut v = self.seq_header.clone();
                v.extend_from_slice(&data);
                data = v;
            }
            Ok(EncodedFrame { data, keyframe })
        }
    }

    fn encode_async(&mut self, sample: IMFSample) -> Result<Vec<EncodedFrame>, String> {
        let ev = self.events.clone().unwrap();
        let mut out = Vec::new();
        // Esperar permiso de entrada.
        while self.need_input_credits == 0 {
            self.handle_event(&ev, &mut out)?;
        }
        unsafe { self.mft.ProcessInput(0, &sample, 0) }.map_err(e("ProcessInput"))?;
        self.need_input_credits -= 1;
        // Con baja latencia y sin B-frames, cada entrada produce una salida.
        let mut guard = 0;
        while out.is_empty() && guard < 4 {
            self.handle_event(&ev, &mut out)?;
            guard += 1;
            if self.need_input_credits >= 2 {
                break;
            }
        }
        Ok(out)
    }

    fn handle_event(&mut self, ev: &IMFMediaEventGenerator, out: &mut Vec<EncodedFrame>) -> Result<(), String> {
        let event = unsafe { ev.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) }.map_err(e("GetEvent"))?;
        let t = unsafe { event.GetType() }.map_err(e("GetType"))?;
        if t == METransformNeedInput.0 as u32 {
            self.need_input_credits += 1;
        } else if t == METransformHaveOutput.0 as u32 {
            self.drain(out)?;
        }
        Ok(())
    }
}

impl VideoEncoder for MfEncoder {
    fn encode(&mut self, nv12: &[u8], pts_us: u64, force_keyframe: bool) -> Result<Vec<EncodedFrame>, String> {
        if force_keyframe {
            if let Some(api) = &self.api {
                unsafe {
                    let _ = api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32));
                }
            }
        }
        let sample = self.make_input(nv12, pts_us).map_err(e("muestra de entrada"))?;
        if self.events.is_some() {
            return self.encode_async(sample);
        }
        unsafe { self.mft.ProcessInput(0, &sample, 0) }.map_err(e("ProcessInput"))?;
        let mut out = Vec::new();
        self.drain(&mut out)?;
        Ok(out)
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}

impl Drop for MfEncoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if let Some(ev) = &self.events {
                let _ = ev; // los eventos pendientes se descartan con el MFT
            }
        }
    }
}

fn blob(t: &IMFMediaType, key: &GUID) -> Option<Vec<u8>> {
    unsafe {
        let n = t.GetBlobSize(key).ok()?;
        let mut v = vec![0u8; n as usize];
        t.GetBlob(key, &mut v, None).ok()?;
        Some(v)
    }
}

/// ¿Contiene el buffer Annex-B un NAL del tipo dado?
pub fn has_nal(data: &[u8], nal_type: u8) -> bool {
    nal_types(data).any(|t| t == nal_type)
}

pub fn nal_types(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        while i + 3 < data.len() {
            let sc3 = data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1;
            if sc3 {
                i += 3;
                let t = data[i] & 0x1F;
                return Some(t);
            }
            i += 1;
        }
        None
    })
}

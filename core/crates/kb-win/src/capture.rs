//! Captura de un monitor con DXGI Desktop Duplication.
//!
//! * El dispositivo D3D11 se crea en el adaptador DUEÑO de la salida
//!   (necesario en notebooks con GPU híbrida).
//! * `DXGI_ERROR_ACCESS_LOST` (cambio de modo, UAC/escritorio seguro,
//!   pantalla completa exclusiva) se informa como recuperable y el hub reabre.
//! * El cursor NO viene en la imagen de Desktop Duplication: se compone
//!   aparte con `GetFramePointerShape` + posición (ver kb_session::cursor).
//! * MVP: copia a memoria de CPU (textura staging). La ruta 100 % GPU queda
//!   como optimización.

use kb_session::cursor::{composite, CursorKind, CursorShape};
use kb_session::{mono_us, CaptureError, Capturer, Frame};
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::*;

pub struct DdaCapturer {
    _device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    /// Última imagen del escritorio SIN cursor (para recomponer si sólo se mueve el mouse).
    clean: Option<Frame>,
    cursor: Option<CursorShape>,
    cursor_pos: Option<(i32, i32)>,
    pub draw_cursor: bool,
}

// Los objetos COM se usan sólo desde el hilo de captura que los posee.
unsafe impl Send for DdaCapturer {}

fn wstr(buf: &[u16]) -> String {
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn fatal(e: windows::core::Error) -> CaptureError {
    CaptureError::Fatal(e.to_string())
}

/// Busca (adaptador, salida) por nombre de dispositivo (`\\.\DISPLAYn`).
pub(crate) fn find_output(device_name: &str) -> Result<(IDXGIAdapter1, IDXGIOutput), CaptureError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(fatal)?;
    let mut i = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        let mut j = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(j) } {
            j += 1;
            if let Ok(d) = unsafe { output.GetDesc() } {
                if wstr(&d.DeviceName).eq_ignore_ascii_case(device_name) {
                    return Ok((adapter, output));
                }
            }
        }
    }
    Err(CaptureError::AccessLost(format!("monitor {device_name} no encontrado")))
}

impl DdaCapturer {
    pub fn open(device_name: &str) -> Result<Self, CaptureError> {
        let (adapter, output) = find_output(device_name)?;
        let mut device = None;
        let mut ctx = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(fatal)?;
        }
        let device = device.ok_or_else(|| CaptureError::Fatal("sin dispositivo D3D11".into()))?;
        let ctx = ctx.ok_or_else(|| CaptureError::Fatal("sin contexto D3D11".into()))?;
        let o1: IDXGIOutput1 = output.cast().map_err(fatal)?;
        // E_ACCESSDENIED aquí = escritorio seguro (UAC, bloqueo): recuperable.
        let dup = unsafe { o1.DuplicateOutput(&device) }.map_err(|e| CaptureError::AccessLost(e.to_string()))?;
        Ok(Self { _device: device, ctx, dup, staging: None, clean: None, cursor: None, cursor_pos: None, draw_cursor: true })
    }

    fn staging_for(&mut self, w: u32, h: u32, format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT) -> Result<ID3D11Texture2D, CaptureError> {
        if let Some((t, sw, sh)) = &self.staging {
            if *sw == w && *sh == h {
                return Ok(t.clone());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { self._device.CreateTexture2D(&desc, None, Some(&mut t)) }.map_err(fatal)?;
        let t = t.ok_or_else(|| CaptureError::Fatal("staging".into()))?;
        self.staging = Some((t.clone(), w, h));
        Ok(t)
    }

    fn read(&mut self, res: &IDXGIResource) -> Result<Frame, CaptureError> {
        let tex: ID3D11Texture2D = res.cast().map_err(fatal)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut desc) };
        let staging = self.staging_for(desc.Width, desc.Height, desc.Format)?;
        let capture_us = mono_us();
        unsafe {
            self.ctx.CopyResource(&staging, &tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).map_err(fatal)?;
            let (w, h) = (desc.Width as usize, desc.Height as usize);
            let row = w * 4;
            let mut data = vec![0u8; row * h];
            let src = m.pData as *const u8;
            for y in 0..h {
                std::ptr::copy_nonoverlapping(src.add(y * m.RowPitch as usize), data.as_mut_ptr().add(y * row), row);
            }
            self.ctx.Unmap(&staging, 0);
            Ok(Frame { width: desc.Width, height: desc.Height, stride: row as u32, data, capture_us })
        }
    }
}

impl DdaCapturer {
    fn read_pointer_shape(&mut self, size: u32) {
        let mut buf = vec![0u8; size as usize];
        let mut needed = 0u32;
        let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
        if unsafe { self.dup.GetFramePointerShape(size, buf.as_mut_ptr() as _, &mut needed, &mut info) }.is_err() {
            return;
        }
        let kind = match info.Type {
            t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 as u32 => CursorKind::Monochrome,
            t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 as u32 => CursorKind::MaskedColor,
            _ => CursorKind::Color,
        };
        buf.truncate(needed as usize);
        self.cursor = Some(CursorShape {
            kind,
            width: info.Width,
            height: info.Height,
            pitch: info.Pitch,
            data: buf,
            hotspot_x: info.HotSpot.x,
            hotspot_y: info.HotSpot.y,
        });
    }

    fn compose(&self) -> Option<Frame> {
        let c = self.clean.as_ref()?;
        let mut f = Frame { width: c.width, height: c.height, stride: c.stride, data: c.data.clone(), capture_us: mono_us() };
        if let (true, Some(shape), Some((x, y))) = (self.draw_cursor, &self.cursor, self.cursor_pos) {
            composite(&mut f.data, f.width, f.height, f.stride, shape, x, y);
        }
        Some(f)
    }
}

impl Capturer for DdaCapturer {
    fn next_frame(&mut self, timeout_ms: u32) -> Result<Option<Frame>, CaptureError> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe { self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST || e.code() == DXGI_ERROR_INVALID_CALL => {
                return Err(CaptureError::AccessLost(e.to_string()))
            }
            Err(e) => return Err(fatal(e)),
        }
        let mut changed = false;
        let mut err = None;
        if info.LastPresentTime != 0 {
            if let Some(r) = &res {
                match self.read(r) {
                    Ok(f) => {
                        self.clean = Some(f);
                        changed = true;
                    }
                    Err(e) => err = Some(e),
                }
            }
        }
        if info.PointerShapeBufferSize > 0 {
            self.read_pointer_shape(info.PointerShapeBufferSize);
            changed = true;
        }
        if info.LastMouseUpdateTime != 0 {
            let p = info.PointerPosition;
            let new = p.Visible.as_bool().then_some((p.Position.x, p.Position.y));
            if new != self.cursor_pos {
                self.cursor_pos = new;
                changed = true;
            }
        }
        unsafe {
            let _ = self.dup.ReleaseFrame();
        }
        if let Some(e) = err {
            return Err(e);
        }
        Ok(if changed { self.compose() } else { None })
    }
}

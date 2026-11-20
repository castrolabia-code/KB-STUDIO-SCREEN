//! KB STUDIO SCREEN — sesiones.
//!
//! * `host`: servidor que atiende hasta 3 ranuras (emparejamiento, video,
//!   entrada). No conoce Windows: usa los traits de `Platform`.
//! * `client`: lado Android (vía JNI) y herramienta de pruebas.
//! * `convert`: BGRA -> NV12 con reducción por promedio de área.

pub mod client;
pub mod convert;
pub mod host;

use kb_geometry::{PointI, Topology};
use kb_proto::{DecoderCaps, MouseButton, Orientation, QualityProfile, ScreenInfo, StreamParams};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Reloj monotónico del proceso en µs (para medir latencias).
pub fn mono_us() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

// ------------------------------------------------------------------ plataforma

/// Frame capturado en BGRA (memoria de CPU en el MVP).
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
    pub capture_us: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// Recuperable: cambio de modo, escritorio seguro (UAC), etc.
    #[error("acceso perdido: {0}")]
    AccessLost(String),
    #[error("{0}")]
    Fatal(String),
}

pub trait Capturer: Send {
    /// `Ok(None)` = no hubo cambios en `timeout_ms`.
    fn next_frame(&mut self, timeout_ms: u32) -> Result<Option<Frame>, CaptureError>;
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    /// H.264 Annex-B (con códigos de inicio).
    pub data: Vec<u8>,
    pub keyframe: bool,
}

pub trait VideoEncoder: Send {
    fn encode(&mut self, nv12: &[u8], pts_us: u64, force_keyframe: bool) -> Result<Vec<EncodedFrame>, String>;
    /// Nombre del encoder elegido (para diagnóstico).
    fn name(&self) -> String;
}

/// Evento de entrada ya resuelto a coordenadas físicas del escritorio virtual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Injected {
    Move { p: PointI },
    Button { p: PointI, button: MouseButton, down: bool },
    Wheel { p: PointI, dx: i16, dy: i16 },
}

pub trait InputSink: Send + Sync {
    fn inject(&self, ev: Injected);
}

pub trait Platform: Send + Sync + 'static {
    fn topology(&self) -> Topology;
    fn open_capture(&self, monitor_id: &str) -> Result<Box<dyn Capturer>, CaptureError>;
    fn open_encoder(&self, params: &StreamParams) -> Result<Box<dyn VideoEncoder>, String>;
    fn input(&self) -> Arc<dyn InputSink>;
}

// ------------------------------------------------------------------ parámetros

/// Elige resolución/FPS/bitrate a partir de la pantalla REAL del cliente y
/// de las capacidades de su decodificador. Nunca agranda la imagen.
pub fn choose_params(src_w: u32, src_h: u32, screen: &ScreenInfo, dec: &DecoderCaps, profile: QualityProfile) -> StreamParams {
    // La caja destino se orienta como la fuente (el cliente puede rotar la presentación).
    let (long, short) = (screen.width_px.max(screen.height_px), screen.width_px.min(screen.height_px));
    let (bw, bh) = if src_w >= src_h { (long, short) } else { (short, long) };
    let (dlong, dshort) = (dec.max_width.max(dec.max_height), dec.max_width.min(dec.max_height));
    let (mw, mh) = if src_w >= src_h { (dlong, dshort) } else { (dshort, dlong) };
    let s = 1f64
        .min(bw as f64 / src_w as f64)
        .min(bh as f64 / src_h as f64)
        .min(if mw > 0 { mw as f64 / src_w as f64 } else { 1.0 })
        .min(if mh > 0 { mh as f64 / src_h as f64 } else { 1.0 });
    let even = |v: f64| ((v.floor() as u32) & !1).max(160);
    let (w, h) = (even(src_w as f64 * s), even(src_h as f64 * s).max(120));
    let (fps, bpp) = match profile {
        QualityProfile::Mixer => (30u16, 0.10),
        QualityProfile::Lyrics => (10, 0.30),
        QualityProfile::Auto => (30, 0.12),
    };
    let fps = fps.min(screen.refresh_hz.max(1));
    let kbps = ((w as f64 * h as f64 * fps as f64 * bpp) / 1000.0).clamp(1000.0, 40_000.0) as u32;
    StreamParams { width: w, height: h, fps, bitrate_kbps: kbps, profile }
}

pub fn orientation_of(w: u32, h: u32) -> Orientation {
    if w >= h {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

// ------------------------------------------------------------------ arbitraje

/// Un único cursor de Windows: mientras una ranura tiene un arrastre activo,
/// las demás no pueden inyectar. El arrastre vence a los 5 s sin eventos
/// (por si el cliente se cae con el dedo "apoyado").
#[derive(Default)]
pub struct InputArbiter {
    owner: Mutex<Option<(u8, Instant)>>,
}

impl InputArbiter {
    const STALE: Duration = Duration::from_secs(5);

    pub fn begin(&self, slot: u8) -> bool {
        let mut o = self.owner.lock().unwrap();
        match *o {
            Some((s, t)) if s != slot && t.elapsed() < Self::STALE => false,
            _ => {
                *o = Some((slot, Instant::now()));
                true
            }
        }
    }
    pub fn continue_(&self, slot: u8) -> bool {
        let mut o = self.owner.lock().unwrap();
        match *o {
            Some((s, _)) if s == slot => {
                *o = Some((slot, Instant::now()));
                true
            }
            _ => false,
        }
    }
    pub fn end(&self, slot: u8) -> bool {
        let mut o = self.owner.lock().unwrap();
        match *o {
            Some((s, _)) if s == slot => {
                *o = None;
                true
            }
            _ => false,
        }
    }
    /// Eventos sueltos (rueda): permitidos si nadie arrastra.
    pub fn idle_or_mine(&self, slot: u8) -> bool {
        match *self.owner.lock().unwrap() {
            Some((s, t)) => s == slot || t.elapsed() >= Self::STALE,
            None => true,
        }
    }
}

// ------------------------------------------------------------------ captura compartida

pub type FrameRx = watch::Receiver<Option<Arc<Frame>>>;
type FrameTx = watch::Sender<Option<Arc<Frame>>>;
type HubMap = Arc<Mutex<HashMap<String, FrameTx>>>;

/// Una captura por monitor, compartida por todos los clientes que lo miran.
/// Semántica "último frame gana": un cliente lento nunca frena la captura.
pub struct CaptureHub {
    platform: Arc<dyn Platform>,
    map: HubMap,
}

impl CaptureHub {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self { platform, map: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn subscribe(&self, monitor_id: &str) -> FrameRx {
        let mut map = self.map.lock().unwrap();
        if let Some(tx) = map.get(monitor_id) {
            if !tx.is_closed() || tx.receiver_count() > 0 {
                return tx.subscribe();
            }
        }
        let (tx, rx) = watch::channel(None);
        map.insert(monitor_id.to_string(), tx.clone());
        let platform = self.platform.clone();
        let map2 = self.map.clone();
        let id = monitor_id.to_string();
        std::thread::Builder::new()
            .name(format!("kbss-capture-{id}"))
            .spawn(move || capture_loop(platform, id, tx, map2))
            .expect("hilo de captura");
        rx
    }
}

fn capture_loop(
    platform: Arc<dyn Platform>,
    id: String,
    tx: FrameTx,
    map: HubMap,
) {
    let mut cap: Option<Box<dyn Capturer>> = None;
    loop {
        {
            // Sin espectadores: liberar la captura (y la GPU).
            let mut m = map.lock().unwrap();
            if tx.receiver_count() == 0 {
                if m.get(&id).is_some_and(|t| t.same_channel(&tx)) {
                    m.remove(&id);
                }
                return;
            }
        }
        if cap.is_none() {
            match platform.open_capture(&id) {
                Ok(c) => cap = Some(c),
                Err(e) => {
                    log(&format!("captura {id}: no disponible ({e}); reintento en 1 s"));
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }
        match cap.as_mut().unwrap().next_frame(100) {
            Ok(Some(f)) => {
                tx.send_replace(Some(Arc::new(f)));
            }
            Ok(None) => {}
            Err(e) => {
                log(&format!("captura {id}: {e}; reabriendo"));
                cap = None;
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
}

/// Log local mínimo y redactado (nunca PIN, claves ni contenido).
pub fn log(msg: &str) {
    eprintln!("[kbss {:>10}] {msg}", mono_us() / 1000);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_proto::H264Profile;

    fn dec(w: u32, h: u32) -> DecoderCaps {
        DecoderCaps { h264_profiles: vec![H264Profile::Baseline], max_width: w, max_height: h, max_fps_at_max_res: 30, hardware: true }
    }
    fn scr(w: u32, h: u32) -> ScreenInfo {
        ScreenInfo { width_px: w, height_px: h, density_dpi: 213, orientation: orientation_of(w, h), refresh_hz: 60 }
    }

    #[test]
    fn params_fit_real_screen_and_never_upscale() {
        // Monitor 2560x1440 hacia tablet 1280x800 en vertical: se usa la caja apaisada.
        let p = choose_params(2560, 1440, &scr(800, 1280), &dec(1920, 1088), QualityProfile::Mixer);
        assert_eq!((p.width, p.height), (1280, 720));
        // S23 FE (2340x1080) mirando 1920x1080: no se agranda.
        let p = choose_params(1920, 1080, &scr(2340, 1080), &dec(3840, 2160), QualityProfile::Mixer);
        assert_eq!((p.width, p.height), (1920, 1080));
        // Decodificador limitado (tablet económica) manda sobre la pantalla.
        let p = choose_params(3840, 2160, &scr(1920, 1200), &dec(1280, 720), QualityProfile::Lyrics);
        assert_eq!((p.width, p.height), (1280, 720));
        assert_eq!(p.fps, 10);
        assert!(p.width.is_multiple_of(2) && p.height.is_multiple_of(2));
    }

    #[test]
    fn arbiter_single_cursor() {
        let a = InputArbiter::default();
        assert!(a.begin(0));
        assert!(!a.begin(1));
        assert!(!a.continue_(1));
        assert!(a.continue_(0));
        assert!(!a.idle_or_mine(1));
        assert!(a.end(0));
        assert!(a.begin(1));
    }
}

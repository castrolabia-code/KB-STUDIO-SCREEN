//! KB STUDIO SCREEN — plataforma Windows.
//!
//! Todo corre en la sesión interactiva del usuario, SIN elevar privilegios.
//! En otros sistemas este crate compila vacío.

#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod dpapi;
#[cfg(windows)]
pub mod encode;
#[cfg(windows)]
pub mod idd;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod monitors;
#[cfg(windows)]
pub mod windows_rescue;

#[cfg(windows)]
pub use platform::WinPlatform;

#[cfg(windows)]
mod platform {
    use kb_geometry::Topology;
    use kb_proto::StreamParams;
    use kb_session::{CaptureError, Capturer, InputSink, Platform, VideoEncoder};
    use std::sync::Arc;

    pub struct WinPlatform {
        input: Arc<crate::input::SendInputSink>,
        /// Driver de pantallas virtuales; se abre la primera vez que alguien pide "Extender".
        idd: std::sync::Mutex<Option<Arc<crate::idd::VirtualDisplays>>>,
    }

    impl WinPlatform {
        fn idd(&self) -> Result<Arc<crate::idd::VirtualDisplays>, String> {
            let mut g = self.idd.lock().unwrap();
            if let Some(v) = g.as_ref() {
                return Ok(v.clone());
            }
            let v = crate::idd::VirtualDisplays::open()?;
            crate::idd::wait_ready(&v, std::time::Duration::from_secs(10))?;
            *g = Some(v.clone());
            Ok(v)
        }
    }

    impl WinPlatform {
        /// Configura el proceso (DPI por monitor v2, COM, Media Foundation).
        /// Llamar una sola vez al inicio.
        pub fn init() -> Result<Self, String> {
            unsafe {
                use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
                // Sin esto, las coordenadas de DXGI y SendInput no coinciden con DPI mixto.
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            crate::encode::mf_startup()?;
            Ok(Self { input: Arc::new(crate::input::SendInputSink), idd: std::sync::Mutex::new(None) })
        }
    }

    impl Platform for WinPlatform {
        fn topology(&self) -> Topology {
            crate::monitors::topology()
        }
        fn open_capture(&self, monitor_id: &str) -> Result<Box<dyn Capturer>, CaptureError> {
            Ok(Box::new(crate::capture::DdaCapturer::open(monitor_id)?))
        }
        fn open_encoder(&self, p: &StreamParams) -> Result<Box<dyn VideoEncoder>, String> {
            Ok(Box::new(crate::encode::MfEncoder::new(p)?))
        }
        fn input(&self) -> Arc<dyn InputSink> {
            self.input.clone()
        }

        fn add_virtual_monitor(&self, slot: u8, width: u32, height: u32) -> Result<kb_geometry::MonitorDesc, String> {
            let v = self.idd()?;
            let before: Vec<String> = crate::monitors::topology().monitors.into_iter().map(|m| m.id).collect();
            v.add(slot as u32, width, height, 60)?;
            // Esperar a que Windows lo agregue al escritorio (normalmente < 2 s).
            let t = std::time::Instant::now();
            loop {
                if let Some(m) = crate::monitors::topology()
                    .monitors
                    .into_iter()
                    .find(|m| m.is_virtual && !before.contains(&m.id) && m.bounds.w as u32 == width && m.bounds.h as u32 == height)
                {
                    return Ok(m);
                }
                if t.elapsed() > std::time::Duration::from_secs(15) {
                    let _ = v.remove(slot as u32);
                    return Err("Windows no activó el monitor virtual a tiempo".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }

        fn remove_virtual_monitor(&self, slot: u8) {
            if let Ok(v) = self.idd() {
                let _ = v.remove(slot as u32);
                // Windows suele mover las ventanas solo; esto es la red de seguridad.
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let n = crate::windows_rescue::rescue_offscreen_windows(&crate::monitors::topology());
                if n > 0 {
                    kb_session::log(&format!("ventanas rescatadas tras quitar el monitor virtual: {n}"));
                }
            }
        }
    }
}

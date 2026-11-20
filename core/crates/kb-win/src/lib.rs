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
            Ok(Self { input: Arc::new(crate::input::SendInputSink) })
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
    }
}

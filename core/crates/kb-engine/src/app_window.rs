//! El panel como aplicación de escritorio: ventana propia (modo aplicación de
//! Microsoft Edge, incluido en Windows 10/11), sin barra de direcciones ni
//! pestañas; consola oculta; una sola instancia.

use std::path::{Path, PathBuf};

/// Abre el panel en su propia ventana. Si Edge no está, usa el navegador predeterminado.
pub fn open_panel(url: &str) {
    if let Some(edge) = find_edge() {
        let ok = std::process::Command::new(edge)
            .args([&format!("--app={url}"), "--window-size=1400,920", "--no-first-run"])
            .spawn()
            .is_ok();
        if ok {
            return;
        }
    }
    // La URL sólo contiene caracteres seguros (http, 127.0.0.1, token base64url).
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
}

fn find_edge() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            candidates.push(Path::new(&base).join(r"Microsoft\Edge\Application\msedge.exe"));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Archivo donde la instancia en marcha deja la dirección de su panel, para
/// que un segundo arranque (menú Inicio) reabra la ventana en vez de fallar.
/// Vive en %LOCALAPPDATA% (privado del usuario): contiene el token del panel.
pub fn url_file(dir: &Path) -> PathBuf {
    dir.join("panel.url")
}

/// Oculta la ventana de consola si es sólo nuestra (abierta por doble clic o
/// por el menú Inicio). Si se lanzó desde una terminal, no la toca.
#[cfg(windows)]
pub fn hide_own_console() {
    use windows::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        let mut ids = [0u32; 4];
        let n = GetConsoleProcessList(&mut ids);
        let hwnd = GetConsoleWindow();
        if n == 1 && !hwnd.is_invalid() {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_own_console() {}

//! Ventana propia del programa: el panel corre dentro de kbss-engine.exe con
//! WebView2 (componente de Windows), sin navegador. En la barra de tareas
//! figura "KB Studio Screen" con su ícono.
//!
//! * Cerrar la ventana la OCULTA: el motor sigue y los dispositivos siguen
//!   conectados. "Cerrar KB Studio Screen" (en el panel) apaga todo.
//! * Un segundo arranque (menú Inicio) pide a esta instancia que se muestre.
//! * Si WebView2 no estuviera disponible, se usa la ventana de Edge.

use std::path::PathBuf;
use std::sync::OnceLock;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::{Icon, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

#[derive(Debug, Clone, Copy)]
enum UiEvent {
    Show,
}

static PROXY: OnceLock<EventLoopProxy<UiEvent>> = OnceLock::new();

/// Muestra (o trae al frente) la ventana. Devuelve false si no hay ventana propia.
pub fn request_show() -> bool {
    PROXY.get().is_some_and(|p| p.send_event(UiEvent::Show).is_ok())
}

/// Ícono 64×64 dibujado en código: barras de nivel azules y una ámbar.
fn icon() -> Option<Icon> {
    const N: u32 = 64;
    let mut px = vec![0u8; (N * N * 4) as usize];
    let mut put = |x: u32, y: u32, c: [u8; 4]| {
        let i = ((y * N + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&c);
    };
    for y in 0..N {
        for x in 0..N {
            // Cuadrado redondeado oscuro.
            let (dx, dy) = ((x as i32 - 31).abs(), (y as i32 - 31).abs());
            let r = 12;
            let inside = dx <= 30 && dy <= 30 && !(dx > 30 - r && dy > 30 - r && (dx - (30 - r)).pow(2) + (dy - (30 - r)).pow(2) > r * r);
            if inside {
                put(x, y, [0x16, 0x1b, 0x23, 0xff]);
            }
        }
    }
    let bars: [(u32, u32, [u8; 4]); 4] =
        [(14, 32, [0x3d, 0x8b, 0xff, 0xff]), (24, 22, [0x3d, 0x8b, 0xff, 0xff]), (34, 14, [0xf2, 0xa5, 0x4a, 0xff]), (44, 26, [0x3d, 0x8b, 0xff, 0xff])];
    for (x0, top, c) in bars {
        for y in top..50 {
            for x in x0..x0 + 7 {
                put(x, y, c);
            }
        }
    }
    Icon::from_rgba(px, N, N).ok()
}

/// Lanza la ventana en su propio hilo. `show_now = false` (inicio con Windows)
/// la crea oculta hasta que se pida. Devuelve false si no se pudo crear.
pub fn spawn(url: String, data_dir: PathBuf, show_now: bool) -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let _ = std::thread::Builder::new().name("kbss-ventana".into()).spawn(move || {
        let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().with_any_thread(true).build();
        let window = match WindowBuilder::new()
            .with_title("KB Studio Screen")
            .with_inner_size(tao::dpi::LogicalSize::new(1400.0, 920.0))
            .with_min_inner_size(tao::dpi::LogicalSize::new(720.0, 520.0))
            .with_window_icon(icon())
            .with_visible(false)
            .build(&event_loop)
        {
            Ok(w) => w,
            Err(_) => {
                let _ = tx.send(false);
                return;
            }
        };
        // Datos de WebView2 en la carpeta del usuario (Archivos de programa es de sólo lectura).
        let mut ctx = WebContext::new(Some(data_dir.join("WebView2")));
        let webview = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_url(&url)
            .with_background_color((13, 16, 21, 255))
            // false = la página recibe los archivos arrastrados (letras PDF, fotos).
            .with_drag_drop_handler(|_| false)
            .build(&window);
        let webview = match webview {
            Ok(w) => w,
            Err(_) => {
                let _ = tx.send(false);
                return;
            }
        };
        let _ = PROXY.set(event_loop.create_proxy());
        let _ = tx.send(true);
        if show_now {
            window.set_visible(true);
            window.set_focus();
        }
        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::Wait;
            let _keep = &webview;
            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    // Se oculta: el motor y los dispositivos siguen funcionando.
                    window.set_visible(false);
                }
                Event::UserEvent(UiEvent::Show) => {
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
                }
                _ => {}
            }
        });
    });
    rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or(false)
}

/// Pide a la instancia que ya corre que muestre su ventana (POST /api/show).
pub fn ask_running_instance_to_show(panel_url: &str) -> bool {
    use std::io::{Read, Write};
    let Some(rest) = panel_url.strip_prefix("http://127.0.0.1:") else { return false };
    let Some((port, q)) = rest.split_once('/') else { return false };
    let Some(token) = q.split("t=").nth(1) else { return false };
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap_or(0))) else { return false };
    let req = format!(
        "POST /api/show HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-KBSS-Token: {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.starts_with("HTTP/1.1 200") && out.contains("\"shown\":true")
}

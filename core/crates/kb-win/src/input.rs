//! Inyección de entrada con `SendInput` (modo mouse).
//!
//! * Coordenadas absolutas sobre el escritorio virtual completo
//!   (`MOUSEEVENTF_VIRTUALDESK`), por lo que funciona con monitores a la
//!   izquierda/arriba del primario (coordenadas negativas).
//! * Cada evento lleva `dwExtraInfo = KBSS_TAG` para poder distinguir la
//!   entrada inyectada de la física.
//! * UIPI: no puede inyectar en ventanas de procesos ELEVADOS. No se eleva.

use kb_geometry::{desktop_to_sendinput_abs, Rect};
use kb_proto::MouseButton;
use kb_session::{Injected, InputSink};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};

pub const KBSS_TAG: usize = 0x4B42_5353; // "KBSS"

pub struct SendInputSink;

pub fn virtual_screen() -> Rect {
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: KBSS_TAG },
        },
    }
}

impl InputSink for SendInputSink {
    fn inject(&self, ev: Injected) {
        let vs = virtual_screen();
        let abs = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | MOUSEEVENTF_MOVE;
        let inputs: Vec<INPUT> = match ev {
            Injected::Move { p } => {
                let (x, y) = desktop_to_sendinput_abs(p, &vs);
                vec![mouse(x, y, 0, abs)]
            }
            Injected::Button { p, button, down } => {
                let (x, y) = desktop_to_sendinput_abs(p, &vs);
                let f = match (button, down) {
                    (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
                    (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
                    (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                    (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
                    (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                    (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
                };
                vec![mouse(x, y, 0, abs | f)]
            }
            Injected::Wheel { p, dx, dy } => {
                let (x, y) = desktop_to_sendinput_abs(p, &vs);
                let mut v = vec![mouse(x, y, 0, abs)];
                if dy != 0 {
                    v.push(mouse(0, 0, dy as i32, MOUSEEVENTF_WHEEL));
                }
                if dx != 0 {
                    v.push(mouse(0, 0, dx as i32, MOUSEEVENTF_HWHEEL));
                }
                v
            }
        };
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            kb_session::log("SendInput bloqueado (¿ventana elevada o escritorio seguro?)");
        }
    }
}

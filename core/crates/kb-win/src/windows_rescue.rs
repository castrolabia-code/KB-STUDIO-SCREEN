//! Rescate de ventanas que quedaron fuera de los monitores visibles (por
//! ejemplo, al desconectar un monitor virtual o físico). La decisión la toma
//! `kb_geometry::Topology::rescue_window`; acá sólo se enumeran y mueven.

use kb_geometry::{Rect, Topology};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<HWND>);
    if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).map(|o| o.is_invalid()).unwrap_or(true) {
        v.push(hwnd);
    }
    BOOL(1)
}

/// Mueve las ventanas inaccesibles al monitor primario. Devuelve cuántas movió.
pub fn rescue_offscreen_windows(topo: &Topology) -> usize {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let mut moved = 0;
    for h in hwnds {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(h, &mut r) }.is_err() {
            continue;
        }
        let w = Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);
        if w.w <= 1 || w.h <= 1 {
            continue;
        }
        if let Some(t) = topo.rescue_window(w, None) {
            let ok = unsafe { SetWindowPos(h, None, t.x, t.y, t.w, t.h, SWP_NOZORDER | SWP_NOACTIVATE) };
            if ok.is_ok() {
                moved += 1;
            }
        }
    }
    moved
}

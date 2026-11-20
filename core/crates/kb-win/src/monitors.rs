//! Topología de monitores: DXGI (posición, salida) + `GetMonitorInfoW`
//! (área de trabajo, primario) + DisplayConfig (nombre comercial del monitor,
//! p. ej. "DELL U2419H") + `GetDpiForMonitor`.

use kb_geometry::{MonitorDesc, Rect, Topology};
use std::collections::HashMap;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, RECT};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

fn wstr(buf: &[u16]) -> String {
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// `\\.\DISPLAYn` -> (nombre comercial, ¿es virtual/indirecto?).
fn friendly_names() -> HashMap<String, (String, bool)> {
    let mut out = HashMap::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return out;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) != ERROR_SUCCESS {
            return out;
        }
        for p in &paths[..np as usize] {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) == 0 && DisplayConfigGetDeviceInfo(&mut tgt.header) == 0 {
                let name = wstr(&tgt.monitorFriendlyDeviceName);
                let tech = p.targetInfo.outputTechnology;
                // Nuestro adaptador virtual se reconoce por la ruta PnP del adaptador.
                let mut ad = DISPLAYCONFIG_ADAPTER_NAME::default();
                ad.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADAPTER_NAME;
                ad.header.size = std::mem::size_of::<DISPLAYCONFIG_ADAPTER_NAME>() as u32;
                ad.header.adapterId = p.targetInfo.adapterId;
                let ours = DisplayConfigGetDeviceInfo(&mut ad.header) == 0 && wstr(&ad.adapterDevicePath).to_lowercase().contains("kbssidd");
                let is_virtual = ours
                    || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED
                    || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL;
                out.insert(wstr(&src.viewGdiDeviceName).to_uppercase(), (name, is_virtual));
            }
        }
    }
    out
}

pub fn topology() -> Topology {
    let names = friendly_names();
    let mut monitors = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Topology::default();
    };
    let mut i = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        let mut j = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(j) } {
            j += 1;
            let Ok(d) = (unsafe { output.GetDesc() }) else { continue };
            if !d.AttachedToDesktop.as_bool() {
                continue;
            }
            let id = wstr(&d.DeviceName);
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            let ok = unsafe { GetMonitorInfoW(d.Monitor, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool();
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = unsafe { GetDpiForMonitor(d.Monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            let n = monitors.len() + 1;
            let (name, is_virtual) = names
                .get(&id.to_uppercase())
                .cloned()
                .filter(|(s, _)| !s.is_empty())
                .unwrap_or_else(|| (format!("Monitor {n}"), false));
            monitors.push(MonitorDesc {
                id,
                name,
                bounds: rect(d.DesktopCoordinates),
                work_area: if ok { rect(mi.monitorInfo.rcWork) } else { rect(d.DesktopCoordinates) },
                dpi: dx,
                primary: ok && (mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
                is_virtual,
            });
        }
    }
    // Orden estable de izquierda a derecha.
    monitors.sort_by_key(|m| (m.bounds.x, m.bounds.y));
    Topology { monitors }
}

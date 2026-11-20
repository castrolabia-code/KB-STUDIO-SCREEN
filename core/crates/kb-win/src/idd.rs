//! Control del driver de pantallas virtuales `kbss-idd` (ver driver/kbss-idd).
//!
//! * `VirtualDisplays::open` + `add/remove`: NO requieren administrador (la
//!   ACL del dispositivo permite lectura/escritura a usuarios interactivos).
//! * `create_adapter` / `remove_adapter`: crean o quitan el adaptador virtual
//!   persistente. Requieren administrador: los usa el instalador UNA vez.
//! * Heartbeat: mientras el motor vive envía PING cada 2 s. Si deja de
//!   hacerlo, el driver retira los monitores a los 6 s (ver KBSS_IDD_WATCHDOG_MS).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;

/// {4c509d3e-a8a8-412d-b27b-274c19df50db} — igual que kbss_idd_ioctl.h
pub const GUID_DEVINTERFACE_KBSS_IDD: GUID = GUID::from_u128(0x4c509d3e_a8a8_412d_b27b_274c19df50db);
pub const PROTOCOL: u32 = 1;
pub const MAX_SLOTS: u32 = 3;

const fn ctl_code(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_UNKNOWN, function, METHOD_BUFFERED, FILE_ANY_ACCESS)
    (0x22 << 16) | (function << 2)
}
const IOCTL_ADD: u32 = ctl_code(0x901);
const IOCTL_REMOVE: u32 = ctl_code(0x902);
const IOCTL_PING: u32 = ctl_code(0x903);
const IOCTL_REMOVE_ALL: u32 = ctl_code(0x904);
const IOCTL_INFO: u32 = ctl_code(0x905);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverInfo {
    pub protocol: u32,
    pub active_mask: u32,
    pub adapter_ready: bool,
}

pub struct VirtualDisplays {
    handle: HANDLE,
    stop: Arc<AtomicBool>,
}

// DeviceIoControl sobre un HANDLE es seguro desde varios hilos.
unsafe impl Send for VirtualDisplays {}
unsafe impl Sync for VirtualDisplays {}

fn interface_path() -> Result<Vec<u16>, String> {
    unsafe {
        let mut len = 0u32;
        let r = CM_Get_Device_Interface_List_SizeW(&mut len, &GUID_DEVINTERFACE_KBSS_IDD, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT);
        if r != CR_SUCCESS || len <= 1 {
            return Err("driver kbss-idd no instalado o adaptador virtual no creado".into());
        }
        let mut buf = vec![0u16; len as usize];
        let r = CM_Get_Device_Interface_ListW(&GUID_DEVINTERFACE_KBSS_IDD, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT);
        if r != CR_SUCCESS {
            return Err(format!("CM_Get_Device_Interface_ListW: {r:?}"));
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        if end == 0 {
            return Err("sin interfaz kbss-idd".into());
        }
        let mut p = buf[..end].to_vec();
        p.push(0);
        Ok(p)
    }
}

impl VirtualDisplays {
    pub fn open() -> Result<Arc<Self>, String> {
        let path = interface_path()?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|e| format!("abrir kbss-idd: {e}"))?;
        let v = Arc::new(Self { handle, stop: Arc::new(AtomicBool::new(false)) });
        // Heartbeat para el watchdog del driver.
        let w = Arc::downgrade(&v);
        std::thread::Builder::new()
            .name("kbss-idd-heartbeat".into())
            .spawn(move || loop {
                let Some(v) = w.upgrade() else { return };
                if v.stop.load(Ordering::Relaxed) {
                    return;
                }
                let _ = v.ping();
                drop(v);
                std::thread::sleep(Duration::from_secs(2));
            })
            .map_err(|e| e.to_string())?;
        Ok(v)
    }

    fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32, String> {
        let mut ret = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                code,
                (!input.is_empty()).then_some(input.as_ptr() as _),
                input.len() as u32,
                (!output.is_empty()).then_some(output.as_mut_ptr() as _),
                output.len() as u32,
                Some(&mut ret),
                None,
            )
        }
        .map_err(|e| format!("IOCTL 0x{code:08X}: {e}"))?;
        Ok(ret)
    }

    pub fn info(&self) -> Result<DriverInfo, String> {
        let mut out = [0u8; 12];
        self.ioctl(IOCTL_INFO, &[], &mut out)?;
        let u = |i: usize| u32::from_le_bytes(out[i..i + 4].try_into().unwrap());
        Ok(DriverInfo { protocol: u(0), active_mask: u(4), adapter_ready: u(8) != 0 })
    }

    /// Agrega un monitor virtual en la ranura con la resolución REAL del Android.
    pub fn add(&self, slot: u32, width: u32, height: u32, refresh_hz: u32) -> Result<(), String> {
        let mut b = Vec::with_capacity(20);
        for v in [PROTOCOL, slot, width & !1, height & !1, refresh_hz.clamp(30, 120)] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        self.ioctl(IOCTL_ADD, &b, &mut []).map(|_| ())
    }

    pub fn remove(&self, slot: u32) -> Result<(), String> {
        let mut b = Vec::with_capacity(8);
        b.extend_from_slice(&PROTOCOL.to_le_bytes());
        b.extend_from_slice(&slot.to_le_bytes());
        self.ioctl(IOCTL_REMOVE, &b, &mut []).map(|_| ())
    }

    pub fn remove_all(&self) -> Result<(), String> {
        self.ioctl(IOCTL_REMOVE_ALL, &[], &mut []).map(|_| ())
    }

    pub fn ping(&self) -> Result<(), String> {
        self.ioctl(IOCTL_PING, &[], &mut []).map(|_| ())
    }

    /// Para pruebas del watchdog: deja de enviar PING sin cerrar el handle.
    pub fn stop_heartbeat(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for VirtualDisplays {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.remove_all();
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

// ------------------------------------------------------------------ adaptador (instalador)

mod swd {
    use super::*;
    use std::sync::mpsc;
    use windows::core::HRESULT;
    use windows::Win32::Devices::Enumeration::Pnp::*;

    const ENUMERATOR: windows::core::PCWSTR = windows::core::w!("KbssIdd");
    const INSTANCE: windows::core::PCWSTR = windows::core::w!("KbssIdd");

    unsafe extern "system" fn created(_h: HSWDEVICE, hr: HRESULT, ctx: *const core::ffi::c_void, _id: PCWSTR) {
        let tx = &*(ctx as *const mpsc::Sender<HRESULT>);
        let _ = tx.send(hr);
    }

    /// Crea (o abre, si ya existe) el dispositivo de software del adaptador virtual.
    pub(super) fn open_swdevice() -> Result<HSWDEVICE, String> {
        let hwids: Vec<u16> = "KbssIdd\0\0".encode_utf16().collect();
        let desc = windows::core::w!("KB STUDIO SCREEN - pantallas virtuales");
        let info = SW_DEVICE_CREATE_INFO {
            cbSize: std::mem::size_of::<SW_DEVICE_CREATE_INFO>() as u32,
            pszInstanceId: INSTANCE,
            pszzHardwareIds: PCWSTR(hwids.as_ptr()),
            pszzCompatibleIds: PCWSTR(hwids.as_ptr()),
            CapabilityFlags: (SWDeviceCapabilitiesRemovable.0 | SWDeviceCapabilitiesSilentInstall.0 | SWDeviceCapabilitiesDriverRequired.0) as u32,
            pszDeviceDescription: desc,
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel::<HRESULT>();
        let h = unsafe {
            SwDeviceCreate(ENUMERATOR, windows::core::w!("HTREE\\ROOT\\0"), &info, None, Some(created), Some(&tx as *const _ as *const _))
        }
        .map_err(|e| format!("SwDeviceCreate: {e} (¿se ejecuta como administrador?)"))?;
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(hr) if hr.is_ok() => Ok(h),
            Ok(hr) => {
                unsafe { SwDeviceClose(h) };
                Err(format!("creación del dispositivo: {hr:?}"))
            }
            Err(_) => {
                unsafe { SwDeviceClose(h) };
                Err("tiempo agotado esperando el dispositivo (¿driver instalado con pnputil?)".into())
            }
        }
    }

    pub(super) fn set_persistent(h: HSWDEVICE, persistent: bool) -> Result<(), String> {
        let l = if persistent { SWDeviceLifetimeParentPresent } else { SWDeviceLifetimeHandle };
        unsafe { SwDeviceSetLifetime(h, l) }.map_err(|e| format!("SwDeviceSetLifetime: {e}"))?;
        unsafe { SwDeviceClose(h) };
        Ok(())
    }
}

/// Crea el adaptador virtual persistente (instalador, con administrador).
pub fn create_adapter() -> Result<(), String> {
    let h = swd::open_swdevice()?;
    swd::set_persistent(h, true)
}

/// Quita el adaptador virtual (desinstalador, con administrador).
pub fn remove_adapter() -> Result<(), String> {
    let h = swd::open_swdevice()?;
    swd::set_persistent(h, false)
}

/// Espera hasta que el adaptador termine de inicializarse.
pub fn wait_ready(v: &VirtualDisplays, timeout: Duration) -> Result<(), String> {
    let t = std::time::Instant::now();
    loop {
        if v.info()?.adapter_ready {
            return Ok(());
        }
        if t.elapsed() > timeout {
            return Err("el adaptador virtual no terminó de inicializarse".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}


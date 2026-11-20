//! KB STUDIO SCREEN — sonda de Fase 0 para Windows (SOLO LECTURA).
//!
//! No instala nada, no cambia configuración, no requiere administrador, no
//! captura ni guarda imágenes de pantalla. Abre y cierra de inmediato una
//! sesión de Desktop Duplication y un dispositivo de puntero sintético para
//! comprobar que el sistema los permite.
//!
//! Salida: informe en consola + `kbss-probe-report.json` en la carpeta actual.
//! El JSON no contiene usuario, nombre de red Wi-Fi ni rutas personales.

#[cfg(not(windows))]
fn main() {
    eprintln!("kb-probe-win sólo funciona en Windows 10/11 x64.");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(windows)]
mod win {
    use serde::Serialize;
    use serde_json::{json, Value};
    use windows::core::{Interface, GUID, HSTRING, PWSTR};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, DXGI_ADAPTER_FLAG_SOFTWARE};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Controls::{CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, POINTER_FEEDBACK_NONE};
    use windows::Win32::UI::Input::Pointer::{InitializeTouchInjection, TOUCH_FEEDBACK_NONE};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, PT_TOUCH, SM_DIGITIZER, SM_MAXIMUMTOUCHES};
    use winreg::enums::*;
    use winreg::RegKey;

    #[derive(Serialize)]
    struct Status {
        ok: bool,
        detail: String,
    }
    fn st(ok: bool, detail: impl Into<String>) -> Status {
        Status { ok, detail: detail.into() }
    }

    fn wstr(buf: &[u16]) -> String {
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    fn reg_str(root: &RegKey, path: &str, name: &str) -> Option<String> {
        root.open_subkey(path).ok()?.get_value::<String, _>(name).ok()
    }
    fn reg_u32(root: &RegKey, path: &str, name: &str) -> Option<u32> {
        root.open_subkey(path).ok()?.get_value::<u32, _>(name).ok()
    }

    fn os_info() -> Value {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let cv = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let build: u32 = reg_str(&hklm, cv, "CurrentBuildNumber").and_then(|s| s.parse().ok()).unwrap_or(0);
        let ubr = reg_u32(&hklm, cv, "UBR").unwrap_or(0);
        let secure_boot = reg_u32(&hklm, r"SYSTEM\CurrentControlSet\Control\SecureBoot\State", "UEFISecureBootEnabled");
        json!({
            // ProductName dice "Windows 10" también en Windows 11: se decide por build.
            "family": if build >= 22000 { "Windows 11" } else { "Windows 10" },
            "edition": reg_str(&hklm, cv, "EditionID"),
            "display_version": reg_str(&hklm, cv, "DisplayVersion"),
            "build": build,
            "ubr": ubr,
            "arch": std::env::consts::ARCH,
            "secure_boot_enabled": secure_boot.map(|v| v == 1),
            "cpu": reg_str(&hklm, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").map(|s| s.trim().to_string()),
            "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
            "ram_gb": ram_gb(),
        })
    }

    fn ram_gb() -> f64 {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe {
            let _ = GlobalMemoryStatusEx(&mut m);
        }
        (m.ullTotalPhys as f64 / 1_073_741_824.0 * 10.0).round() / 10.0
    }

    fn vendor(id: u32) -> &'static str {
        match id {
            0x10DE => "NVIDIA",
            0x1002 | 0x1022 => "AMD",
            0x8086 => "Intel",
            0x1414 => "Microsoft (software/virtual)",
            _ => "otro",
        }
    }

    /// Adaptadores, salidas (monitores), DPI y prueba de Desktop Duplication.
    fn adapters_and_monitors() -> Value {
        let mut adapters = Vec::new();
        let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
            Ok(f) => f,
            Err(e) => return json!({ "error": format!("CreateDXGIFactory1: {e}") }),
        };
        let mut i = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
            i += 1;
            let d = match unsafe { adapter.GetDesc1() } {
                Ok(d) => d,
                Err(_) => continue,
            };
            let device = create_device(&adapter);
            let mut outputs = Vec::new();
            let mut j = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(j) } {
                j += 1;
                let Ok(od) = (unsafe { output.GetDesc() }) else { continue };
                let r = od.DesktopCoordinates;
                let (mut dx, mut dy) = (0u32, 0u32);
                let dpi = unsafe { GetDpiForMonitor(od.Monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) }.ok().map(|_| dx);
                let dup = match (&device, output.cast::<IDXGIOutput1>()) {
                    (Ok(dev), Ok(o1)) => match unsafe { o1.DuplicateOutput(dev) } {
                        Ok(_dup) => st(true, "DuplicateOutput OK (sesión cerrada de inmediato)"),
                        Err(e) => st(false, format!("DuplicateOutput: {e}")),
                    },
                    (Err(e), _) => st(false, format!("D3D11CreateDevice: {e}")),
                    (_, Err(e)) => st(false, format!("IDXGIOutput1: {e}")),
                };
                outputs.push(json!({
                    "device_name": wstr(&od.DeviceName),
                    "attached": od.AttachedToDesktop.as_bool(),
                    "x": r.left, "y": r.top, "width": r.right - r.left, "height": r.bottom - r.top,
                    "rotation_dxgi": od.Rotation.0,
                    "effective_dpi": dpi,
                    "scale_percent": dpi.map(|d| d * 100 / 96),
                    "desktop_duplication": dup,
                }));
            }
            adapters.push(json!({
                "name": wstr(&d.Description),
                "vendor": vendor(d.VendorId),
                "vendor_id": format!("0x{:04X}", d.VendorId),
                "device_id": format!("0x{:04X}", d.DeviceId),
                "dedicated_vram_mb": d.DedicatedVideoMemory / 1_048_576,
                "software": (d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
                "outputs": outputs,
            }));
        }
        json!(adapters)
    }

    fn create_device(adapter: &IDXGIAdapter1) -> windows::core::Result<ID3D11Device> {
        let mut dev = None;
        unsafe {
            D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                None,
            )?;
        }
        dev.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
    }

    /// Codificadores H.264 registrados en Media Foundation (MFTEnumEx).
    fn h264_encoders() -> Value {
        let mut out = Vec::new();
        unsafe {
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                return json!({ "error": format!("MFStartup: {e}") });
            }
            for (label, flags) in [
                ("hardware", MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER),
                ("software", MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER),
            ] {
                let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
                let mut arr: *mut Option<IMFActivate> = std::ptr::null_mut();
                let mut n = 0u32;
                if MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, None, Some(&info), &mut arr, &mut n).is_err() {
                    continue;
                }
                for k in 0..n as usize {
                    if let Some(act) = (*arr.add(k)).take() {
                        out.push(json!({
                            "kind": label,
                            "name": get_string(&act, &MFT_FRIENDLY_NAME_Attribute),
                            "hw_url": get_string(&act, &MFT_ENUM_HARDWARE_URL_Attribute),
                        }));
                    }
                }
                if !arr.is_null() {
                    CoTaskMemFree(Some(arr as _));
                }
            }
            let _ = MFShutdown();
        }
        json!(out)
    }

    unsafe fn get_string(a: &IMFActivate, key: &GUID) -> Option<String> {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        unsafe {
            a.GetAllocatedString(key, &mut p, &mut len).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as _));
            s
        }
    }

    fn capture_apis() -> Value {
        use windows::Foundation::Metadata::ApiInformation;
        use windows::Graphics::Capture::GraphicsCaptureSession;
        let cls = HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession");
        let prop = |p: &str| ApiInformation::IsPropertyPresent(&cls, &HSTRING::from(p)).unwrap_or(false);
        json!({
            "wgc_supported": GraphicsCaptureSession::IsSupported().unwrap_or(false),
            "wgc_cursor_toggle": prop("IsCursorCaptureEnabled"),
            // Sin esta propiedad, WGC dibuja un borde amarillo alrededor de lo capturado.
            "wgc_border_optional": prop("IsBorderRequired"),
            "wgc_min_update_interval": prop("MinUpdateInterval"),
        })
    }

    fn touch_injection() -> Value {
        let max_touches = unsafe { GetSystemMetrics(SM_MAXIMUMTOUCHES) };
        let digitizer = unsafe { GetSystemMetrics(SM_DIGITIZER) };
        // InitializeTouchInjection no inyecta nada: sólo reserva el canal.
        let legacy = unsafe { InitializeTouchInjection(10, TOUCH_FEEDBACK_NONE) };
        let synthetic = unsafe { CreateSyntheticPointerDevice(PT_TOUCH, 10, POINTER_FEEDBACK_NONE) };
        let synthetic_st = match synthetic {
            Ok(h) => {
                unsafe { DestroySyntheticPointerDevice(h) };
                st(true, "CreateSyntheticPointerDevice(PT_TOUCH, 10) OK")
            }
            Err(e) => st(false, format!("CreateSyntheticPointerDevice: {e}")),
        };
        json!({
            "system_max_touches": max_touches,
            "digitizer_flags": digitizer,
            "initialize_touch_injection": match legacy { Ok(_) => st(true, "OK (10 contactos)"), Err(e) => st(false, e.to_string()) },
            "synthetic_pointer_touch": synthetic_st,
            "note": "UIPI: un proceso sin elevar NO puede inyectar en ventanas elevadas. Verificar que Reason no corra como administrador.",
        })
    }

    fn file_version(path: &str) -> Option<String> {
        use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
        let h = HSTRING::from(path);
        unsafe {
            let size = GetFileVersionInfoSizeW(&h, None);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            GetFileVersionInfoW(&h, None, size, buf.as_mut_ptr() as _).ok()?;
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut l = 0u32;
            if !VerQueryValueW(buf.as_ptr() as _, &HSTRING::from("\\"), &mut p, &mut l).as_bool() || p.is_null() {
                return None;
            }
            let fi = &*(p as *const VS_FIXEDFILEINFO);
            Some(format!(
                "{}.{}.{}.{}",
                fi.dwFileVersionMS >> 16,
                fi.dwFileVersionMS & 0xFFFF,
                fi.dwFileVersionLS >> 16,
                fi.dwFileVersionLS & 0xFFFF
            ))
        }
    }

    fn indirect_display() -> Value {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let candidates = [
            format!(r"{windir}\System32\drivers\UMDF\IddCx.dll"),
            format!(r"{windir}\System32\IddCx.dll"),
            format!(r"{windir}\System32\drivers\IndirectKmd.sys"),
        ];
        let files: Vec<Value> = candidates
            .iter()
            .filter(|p| std::path::Path::new(p).exists())
            .map(|p| json!({ "file": std::path::Path::new(p).file_name().unwrap().to_string_lossy(), "version": file_version(p) }))
            .collect();
        // Controladores de pantalla virtuales ya instalados (Parsec, VDD, etc.).
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut display_drivers = Vec::new();
        if let Ok(cls) = hklm.open_subkey(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}") {
            for k in cls.enum_keys().flatten() {
                if let Ok(sub) = cls.open_subkey(&k) {
                    if let Ok(desc) = sub.get_value::<String, _>("DriverDesc") {
                        let ver: Option<String> = sub.get_value("DriverVersion").ok();
                        let prov: Option<String> = sub.get_value("ProviderName").ok();
                        display_drivers.push(json!({ "desc": desc, "provider": prov, "version": ver }));
                    }
                }
            }
        }
        json!({ "framework_files": files, "display_class_drivers": display_drivers })
    }

    fn dev_tools() -> Value {
        let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
        let kits = format!(r"{pf86}\Windows Kits\10\Include");
        let mut sdk = Vec::new();
        let mut wdk = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&kits) {
            for e in rd.flatten() {
                let v = e.file_name().to_string_lossy().to_string();
                if e.path().join("um").exists() {
                    sdk.push(v.clone());
                }
                if e.path().join("km").exists() {
                    wdk.push(v);
                }
            }
        }
        let vswhere = format!(r"{pf86}\Microsoft Visual Studio\Installer\vswhere.exe");
        let vs = std::process::Command::new(&vswhere)
            .args(["-latest", "-products", "*", "-property", "catalog_productDisplayVersion"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        let which = |cmd: &str| {
            std::process::Command::new("where").arg(cmd).output().map(|o| o.status.success()).unwrap_or(false)
        };
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        json!({
            "windows_sdk_versions": sdk,
            "wdk_versions": wdk,
            "visual_studio": vs,
            "rustc": which("rustc"), "cargo": which("cargo"), "git": which("git"),
            "java": which("java"), "adb": which("adb"),
            "android_sdk": std::env::var("ANDROID_HOME").is_ok() || std::path::Path::new(&format!(r"{local}\Android\Sdk")).exists(),
            "android_studio": std::path::Path::new(r"C:\Program Files\Android\Android Studio").exists(),
        })
    }

    fn installed_apps() -> Value {
        let mut found = Vec::new();
        for (root, path) in [
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
            (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        ] {
            let Ok(k) = RegKey::predef(root).open_subkey(path) else { continue };
            for name in k.enum_keys().flatten() {
                let Ok(s) = k.open_subkey(&name) else { continue };
                let Ok(dn) = s.get_value::<String, _>("DisplayName") else { continue };
                let l = dn.to_lowercase();
                // Sólo lo relevante para el proyecto: DAW y visores PDF.
                if ["reason", "ableton", "cubase", "fl studio", "studio one", "reaper", "acrobat", "sumatra", "foxit"]
                    .iter()
                    .any(|w| l.contains(w))
                {
                    let ver: Option<String> = s.get_value("DisplayVersion").ok();
                    found.push(json!({ "name": dn, "version": ver }));
                }
            }
        }
        json!(found)
    }

    fn network() -> Value {
        use windows::Win32::Networking::NetworkListManager::*;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        let mut nets = Vec::new();
        unsafe {
            let Ok(nlm) = CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL) else {
                return json!({ "error": "NetworkListManager no disponible" });
            };
            if let Ok(en) = nlm.GetNetworks(NLM_ENUM_NETWORK_CONNECTED) {
                loop {
                    let mut item = [None];
                    let mut fetched = 0u32;
                    if en.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                        break;
                    }
                    if let Some(n) = item[0].take() {
                        let cat = match n.GetCategory() {
                            Ok(c) if c == NLM_NETWORK_CATEGORY_PRIVATE => "Privada",
                            Ok(c) if c == NLM_NETWORK_CATEGORY_PUBLIC => "Pública",
                            Ok(c) if c == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED => "Dominio",
                            _ => "desconocida",
                        };
                        // El nombre de la red (SSID) NO se registra.
                        nets.push(json!({ "category": cat }));
                    }
                }
            }
        }
        json!({
            "connected_networks": nets,
            "note": "Con perfil 'Pública', Windows Firewall bloquea por defecto conexiones entrantes: el instalador deberá crear una regla acotada al ejecutable y a redes privadas.",
        })
    }

    pub fn run() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let report = json!({
            "tool": "kb-probe-win",
            "version": env!("CARGO_PKG_VERSION"),
            "os": os_info(),
            "adapters": adapters_and_monitors(),
            "h264_encoders": h264_encoders(),
            "capture": capture_apis(),
            "touch_injection": touch_injection(),
            "indirect_display": indirect_display(),
            "dev_tools": dev_tools(),
            "relevant_apps": installed_apps(),
            "network": network(),
        });
        let text = serde_json::to_string_pretty(&report).unwrap();
        println!("KB STUDIO SCREEN — sonda Fase 0 (solo lectura) — by Castro Labs\n");
        println!("{text}");
        match std::fs::write("kbss-probe-report.json", &text) {
            Ok(_) => println!("\nInforme guardado en kbss-probe-report.json (carpeta actual)."),
            Err(e) => eprintln!("\nNo se pudo guardar el informe: {e}"),
        }
        println!("Presioná Enter para cerrar...");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

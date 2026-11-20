//! kbss-engine — motor host de KB STUDIO SCREEN (MVP Fase 1, consola).
//!
//! * Escucha QUIC en UDP 47800 (o `KBSS_PORT`), en la LAN.
//! * Se anuncia por mDNS como `_kbss._udp.local.`.
//! * Muestra QR y PIN para emparejar; cada dispositivo nuevo requiere tu
//!   aprobación explícita (control o solo ver).
//! * Guarda identidad y dispositivos confiables en
//!   `%LOCALAPPDATA%\CastroLabs\KBStudioScreen`, cifrados con DPAPI.
//! * No requiere administrador. No transmite audio.

#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use kb_pairing::{Fingerprint, TrustStore};
use kb_proto::Permission;
use kb_session::host::{ApprovalRequest, Host, HostConfig, HostEvent, HostUi, TrustPersistence};
use kb_transport::{server_endpoint, Identity};
use std::collections::VecDeque;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

const DEFAULT_PORT: u16 = 47800;

// ------------------------------------------------------------ almacenamiento

struct Store {
    dir: PathBuf,
}

impl Store {
    fn open() -> Self {
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        #[cfg(not(windows))]
        let base = std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")).unwrap_or_else(|_| PathBuf::from("."));
        let dir = base.join("CastroLabs").join("KBStudioScreen");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    fn read_secret(&self, name: &str) -> Option<Vec<u8>> {
        let raw = std::fs::read(self.dir.join(name)).ok()?;
        #[cfg(windows)]
        return kb_win::dpapi::unprotect(&raw).ok();
        #[cfg(not(windows))]
        return Some(raw);
    }

    #[allow(clippy::needless_borrows_for_generic_args)]
    fn write_secret(&self, name: &str, data: &[u8]) {
        #[cfg(windows)]
        let data = match kb_win::dpapi::protect(data) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("No se pudo cifrar {name}: {e}");
                return;
            }
        };
        let tmp = self.dir.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, &data).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join(name));
        }
    }
}

/// identidad = host_id(16) | len_cert(u32) | cert | clave PKCS#8
fn load_identity(store: &Store) -> ([u8; 16], Identity) {
    if let Some(b) = store.read_secret("identity.bin") {
        if b.len() > 20 {
            let mut id = [0u8; 16];
            id.copy_from_slice(&b[..16]);
            let n = u32::from_be_bytes(b[16..20].try_into().unwrap()) as usize;
            if b.len() > 20 + n {
                return (id, Identity::from_der(b[20..20 + n].to_vec(), b[20 + n..].to_vec()));
            }
        }
        eprintln!("Identidad guardada ilegible: se genera una nueva (habrá que volver a emparejar).");
    }
    let ident = Identity::generate().expect("generar identidad");
    let host_id: [u8; 16] = rand::random();
    let mut b = host_id.to_vec();
    b.extend_from_slice(&(ident.cert_der.len() as u32).to_be_bytes());
    b.extend_from_slice(&ident.cert_der);
    b.extend_from_slice(ident.key_der());
    store.write_secret("identity.bin", &b);
    (host_id, ident)
}

struct FilePersist(Store);
impl TrustPersistence for FilePersist {
    fn save(&self, t: &TrustStore) {
        if let Ok(j) = serde_json::to_vec(t) {
            self.0.write_secret("trust.json", &j);
        }
    }
}

// ------------------------------------------------------------ consola

enum Ask {
    Approve(ApprovalRequest, oneshot::Sender<Option<Permission>>),
    Confirm(String, oneshot::Sender<bool>),
}
type Pending = VecDeque<Ask>;

struct ConsoleUi {
    pending: Mutex<Pending>,
}

impl HostUi for ConsoleUi {
    fn show_pin(&self, device: &str, pin: &str) {
        println!("\n  ┌──────────────────────────────────────────────┐");
        println!("  │  «{device}» quiere emparejarse.");
        println!("  │  PIN para ingresar en el dispositivo:  {pin}");
        println!("  │  (vence en 2 minutos, un solo uso)");
        println!("  └──────────────────────────────────────────────┘");
    }
    fn hide_pin(&self, _device: &str) {}
    fn approve(&self, req: ApprovalRequest) -> oneshot::Receiver<Option<Permission>> {
        let (tx, rx) = oneshot::channel();
        println!(
            "\n  ¿Aprobar «{}» ({:?}, por {}, huella {})?\n    [c] control táctil   [v] solo ver   [n] rechazar  → escribí la letra y Enter",
            req.device_name, req.kind, req.method, req.fingerprint_short
        );
        self.pending.lock().unwrap().push_back(Ask::Approve(req, tx));
        rx
    }
    fn confirm(&self, device: &str, action: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        println!("\n  «{device}» pide {action}. ¿Permitir? [s] sí  [n] no  → escribí la letra y Enter");
        self.pending.lock().unwrap().push_back(Ask::Confirm(device.to_string(), tx));
        rx
    }
    fn event(&self, ev: HostEvent) {
        match ev {
            HostEvent::Connected { slot, name, permission } => {
                println!("● Ranura {}: «{name}» conectado ({})", slot + 1, perm_es(permission))
            }
            HostEvent::Streaming { slot, monitor, width, height, fps, encoder } => {
                println!("  Ranura {}: transmitiendo {monitor} a {width}x{height} @ {fps} FPS — {encoder}", slot + 1)
            }
            HostEvent::Disconnected { slot, name } => println!("○ Ranura {}: «{name}» desconectado", slot + 1),
            HostEvent::Rejected { name, reason } => println!("✕ «{name}» rechazado: {reason}"),
        }
    }
}

fn perm_es(p: Permission) -> &'static str {
    match p {
        Permission::Control => "control táctil",
        Permission::ViewOnly => "solo ver",
    }
}

fn lan_ipv4() -> Vec<std::net::Ipv4Addr> {
    let mut v: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if !a.is_loopback() && !a.is_link_local() => Some(a),
            _ => None,
        })
        .collect();
    // Primero las direcciones privadas típicas de una LAN doméstica.
    v.sort_by_key(|a| !a.is_private());
    v
}

fn print_qr(host: &Host, ip: &str, port: u16) {
    let qr = host.issue_qr(ip.to_string(), port);
    let uri = qr.to_uri();
    match qrcode::QrCode::new(uri.as_bytes()) {
        Ok(code) => {
            let s = code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build();
            println!("\nEscaneá este QR desde la app KB STUDIO SCREEN (vale 5 minutos, un solo uso):\n{s}");
        }
        Err(e) => println!("No se pudo dibujar el QR: {e}"),
    }
    println!("Dirección manual: {ip}  puerto {port}\n");
}

fn help() {
    println!("Comandos: [q] QR nuevo  [l] dispositivos  [r N] revocar  [p N] cambiar permiso  [m] monitores  [w] rescatar ventanas  [h] ayuda  [x] salir");
    println!("Cantante: [d RUTA] enviar letra (PDF o .txt)  [n] página siguiente  [b] anterior  [a N] avance automático 0-10  [k] bloquear/desbloquear  [c] cerrar letra");
}

fn host_name() -> String {
    let raw = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "PC".into());
    let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(32).collect();
    if clean.is_empty() {
        "PC".into()
    } else {
        clean
    }
}

fn main() {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleOutputCP(65001);
    }
    println!("KB STUDIO SCREEN — motor host v{} — by Castro Labs", env!("CARGO_PKG_VERSION"));
    println!("Sin audio · sin nube · sólo red local\n");

    #[cfg(windows)]
    let platform: Arc<dyn kb_session::Platform> = match kb_win::WinPlatform::init() {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("No se pudo iniciar la plataforma Windows: {e}");
            std::process::exit(1);
        }
    };
    #[cfg(not(windows))]
    {
        eprintln!("kbss-engine sólo funciona en Windows 10/11 x64.");
        std::process::exit(2);
    }

    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("driver") {
            std::process::exit(driver_cmd(&args[1..]));
        }
        run(platform);
    }
}

/// Subcomandos de mantenimiento del driver de pantallas virtuales.
/// `create-adapter` y `remove-adapter` requieren administrador (los usa el instalador).
#[cfg(windows)]
fn driver_cmd(args: &[String]) -> i32 {
    use kb_win::idd;
    let n = |i: usize, d: u32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let r: Result<String, String> = match args.first().map(String::as_str) {
        Some("create-adapter") => idd::create_adapter().map(|_| "Adaptador virtual creado.".into()),
        Some("remove-adapter") => idd::remove_adapter().map(|_| "Adaptador virtual quitado.".into()),
        Some("info") => idd::VirtualDisplays::open().and_then(|v| v.info()).map(|i| format!("{i:?}")),
        Some("add") => idd::VirtualDisplays::open().and_then(|v| {
            idd::wait_ready(&v, std::time::Duration::from_secs(10))?;
            v.add(n(3, 0), n(1, 1920), n(2, 1080), 60)?;
            println!("Monitor agregado. Enter para quitarlo…");
            let _ = std::io::stdin().read_line(&mut String::new());
            Ok("Monitor quitado.".into())
        }),
        _ => Err("uso: kbss-engine driver create-adapter | remove-adapter | info | add ANCHO ALTO [RANURA]".into()),
    };
    match r {
        Ok(m) => {
            println!("{m}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(windows)]
fn run(platform: Arc<dyn kb_session::Platform>) {
    let store = Store::open();
    let (host_id, ident) = load_identity(&store);
    let trust: TrustStore = store.read_secret("trust.json").and_then(|j| serde_json::from_slice(&j).ok()).unwrap_or_default();
    let port: u16 = std::env::var("KBSS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let name = host_name();
    let host_fp: Fingerprint = ident.fingerprint();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build().expect("runtime");
    let _g = rt.enter();
    let endpoint = match server_endpoint(&ident, std::net::SocketAddr::from(([0, 0, 0, 0], port))) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("No se pudo abrir el puerto UDP {port}: {e}\n¿Hay otra copia de kbss-engine abierta?");
            std::process::exit(1);
        }
    };
    let ui = Arc::new(ConsoleUi { pending: Mutex::new(VecDeque::new()) });
    let host = Host::new(
        HostConfig { host_id, host_name: name.clone(), extend_grace: kb_session::host::EXTEND_GRACE },
        host_fp,
        platform.clone(),
        ui.clone(),
        Arc::new(FilePersist(Store::open())),
        trust,
    );

    // mDNS: para que la app encuentre la PC sin escribir la IP.
    let ips = lan_ipv4();
    let _mdns = mdns_sd::ServiceDaemon::new().ok().and_then(|d| {
        let fp = B64.encode(host_fp.0);
        let props = [("v", "1"), ("fp", fp.as_str()), ("name", name.as_str())];
        let info = mdns_sd::ServiceInfo::new("_kbss._udp.local.", &name, &format!("{name}.local."), "", port, &props[..])
            .ok()?
            .enable_addr_auto();
        d.register(info).ok()?;
        Some(d)
    });
    if _mdns.is_none() {
        println!("Aviso: no se pudo anunciar por mDNS; usá el QR o la IP manual.");
    }

    println!("PC: {name}   Huella del host: {}", host_fp.short());
    println!("Monitores:");
    for (i, m) in platform.topology().monitors.iter().enumerate() {
        println!("  {}. {} — {}x{} en ({}, {}) {} dpi{}", i + 1, m.name, m.bounds.w, m.bounds.h, m.bounds.x, m.bounds.y, m.dpi, if m.primary { " [principal]" } else { "" });
    }
    println!("\nSi Windows pregunta por el Firewall, permití el acceso SOLO en «Redes privadas».");
    match ips.first() {
        Some(ip) => print_qr(&host, &ip.to_string(), port),
        None => println!("No se encontró ninguna dirección de red local. ¿Está conectada la PC al Wi-Fi o al router?"),
    }
    help();

    let h = host.clone();
    rt.spawn(async move { h.serve(endpoint).await });

    // Consola: las respuestas a aprobaciones tienen prioridad sobre los comandos.
    let mut locked = false;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let l = line.trim().to_lowercase();
        let ask = ui.pending.lock().unwrap().pop_front();
        match ask {
            Some(Ask::Approve(req, tx)) => {
                let d = match l.as_str() {
                    "c" => Some(Permission::Control),
                    "v" => Some(Permission::ViewOnly),
                    _ => None,
                };
                match d {
                    Some(p) => println!("  «{}» aprobado: {}", req.device_name, perm_es(p)),
                    None => println!("  «{}» rechazado.", req.device_name),
                }
                let _ = tx.send(d);
                continue;
            }
            Some(Ask::Confirm(device, tx)) => {
                let ok = l == "s" || l == "si" || l == "sí";
                println!("  «{device}»: {}", if ok { "permitido" } else { "denegado" });
                let _ = tx.send(ok);
                continue;
            }
            None => {}
        }
        // Comando "d": la ruta conserva mayúsculas y espacios.
        if l.starts_with("d ") {
            let path = line.trim()[2..].trim().trim_matches('"');
            match std::fs::read(path) {
                Ok(bytes) => {
                    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let kind = if bytes.starts_with(b"%PDF") { kb_proto::DocKind::Pdf } else { kb_proto::DocKind::Text };
                    match host.send_document(&name, kind, &bytes) {
                        Ok(0) => println!("  No hay dispositivos conectados."),
                        Ok(n) => println!("  «{name}» enviado a {n} dispositivo(s) ({} KB).", bytes.len() / 1024),
                        Err(e) => println!("  {e}"),
                    }
                }
                Err(e) => println!("  No se pudo leer el archivo: {e}"),
            }
            continue;
        }
        let mut parts = l.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let arg: Option<usize> = parts.next().and_then(|n| n.parse().ok());
        let devices = host.trusted();
        match cmd {
            "q" => match ips.first() {
                Some(ip) => print_qr(&host, &ip.to_string(), port),
                None => println!("Sin dirección de red local."),
            },
            "l" => {
                if devices.is_empty() {
                    println!("No hay dispositivos emparejados.");
                }
                for (i, (_, d)) in devices.iter().enumerate() {
                    println!("  {}. {} — {}", i + 1, d.name, perm_es(d.permission));
                }
            }
            "r" | "p" => match arg.and_then(|n| devices.get(n.wrapping_sub(1))) {
                Some((k, d)) => {
                    let fp = TrustStore::fingerprint_from_key(k).unwrap();
                    if cmd == "r" {
                        host.revoke(&fp);
                        println!("  «{}» revocado: se cortó su sesión y deberá emparejarse de nuevo.", d.name);
                    } else {
                        let np = if d.permission == Permission::Control { Permission::ViewOnly } else { Permission::Control };
                        host.set_permission(&fp, np);
                        println!("  «{}» ahora: {}", d.name, perm_es(np));
                    }
                }
                None => println!("  Número inválido. Usá [l] para ver la lista."),
            },
            "m" => {
                for (i, m) in platform.topology().monitors.iter().enumerate() {
                    println!("  {}. {} — {}x{} en ({}, {})", i + 1, m.name, m.bounds.w, m.bounds.h, m.bounds.x, m.bounds.y);
                }
            }
            "w" => {
                let n = kb_win::windows_rescue::rescue_offscreen_windows(&platform.topology());
                println!("  Ventanas reubicadas: {n}");
            }
            "n" | "b" | "a" | "k" | "c" => {
                use kb_proto::PrompterAction as P;
                let action = match cmd {
                    "n" => P::NextPage,
                    "b" => P::PrevPage,
                    "a" => P::AutoScroll(arg.unwrap_or(0).min(10) as u8),
                    "k" => {
                        locked = !locked;
                        P::Lock(locked)
                    }
                    _ => P::Close,
                };
                let n = host.prompter(action);
                println!("  Teleprompter: {action:?} → {n} dispositivo(s){}", if cmd == "k" { if locked { " (bloqueado)" } else { " (desbloqueado)" } } else { "" });
            }
            "x" => break,
            "" => {}
            _ => help(),
        }
    }
    println!("Cerrando…");
}

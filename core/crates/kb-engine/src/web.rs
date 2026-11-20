//! Panel de control local (http://127.0.0.1:PUERTO). Sólo escucha en la
//! propia PC y exige un token aleatorio. Protecciones:
//! * Encabezado `Host` debe ser 127.0.0.1/localhost (evita DNS rebinding).
//! * La API exige el encabezado `X-KBSS-Token` (un sitio web ajeno no puede
//!   agregarlo sin un preflight CORS que nunca se aprueba) y `Origin` propio.
//! * Límites de tamaño en todos los cuerpos.

use crate::profiles::{Profiles, NAMES};
use crate::ui::{perm_es, Ui};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use kb_pairing::TrustStore;
use kb_proto::{DocKind, Permission, PrompterAction, QualityProfile};
use kb_session::host::Host;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server};

const PAGE: &str = include_str!("panel.html");
const MAX_SMALL: usize = 8 * 1024;
const MAX_DOC: usize = 64 * 1024 * 1024;
const MAX_BG: usize = 12 * 1024 * 1024;

pub struct Panel {
    pub host: Host,
    pub ui: Arc<Ui>,
    pub platform: Arc<dyn kb_session::Platform>,
    pub host_name: String,
    pub fp_short: String,
    pub lan_ip: Option<String>,
    pub quic_port: u16,
    pub token: String,
    pub port: u16,
    pub locked: AtomicBool,
    pub profiles: std::sync::Mutex<Profiles>,
    pub profiles_path: std::path::PathBuf,
    /// Foto de fondo elegida por el usuario (queda sólo en esta PC).
    pub bg_path: std::path::PathBuf,
}

pub fn new_token() -> String {
    let b: [u8; 24] = rand::random();
    B64.encode(b)
}

fn header(r: &Request, name: &str) -> Option<String> {
    r.headers().iter().find(|h| h.field.to_string().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

fn json_resp(v: Value, code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(v.to_string().into_bytes())
        .with_status_code(code)
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

impl Panel {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    /// Bloquea el hilo atendiendo pedidos.
    pub fn serve(self: Arc<Self>) -> Result<(), String> {
        let server = Server::http(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        for req in server.incoming_requests() {
            let p = self.clone();
            // Pedidos cortos: se atienden en el mismo hilo, salvo subidas de documentos.
            if req.url().starts_with("/api/doc") {
                std::thread::spawn(move || p.handle(req));
            } else {
                p.handle(req);
            }
        }
        Ok(())
    }

    fn host_ok(&self, r: &Request) -> bool {
        let h = header(r, "Host").unwrap_or_default();
        h == format!("127.0.0.1:{}", self.port) || h == format!("localhost:{}", self.port)
    }

    fn api_ok(&self, r: &Request) -> bool {
        let tok_ok = header(r, "X-KBSS-Token").is_some_and(|t| constant_eq(t.as_bytes(), self.token.as_bytes()));
        let origin_ok = header(r, "Origin")
            .is_none_or(|o| o == format!("http://127.0.0.1:{}", self.port) || o == format!("http://localhost:{}", self.port));
        tok_ok && origin_ok
    }

    fn handle(&self, mut r: Request) {
        if !self.host_ok(&r) {
            let _ = r.respond(Response::from_string("host no permitido").with_status_code(403));
            return;
        }
        let url = r.url().to_string();
        let path = url.split('?').next().unwrap_or("").to_string();
        if path == "/" && *r.method() == Method::Get {
            let q = url.split_once('?').map(|x| x.1).unwrap_or("");
            let ok = q.split('&').any(|kv| kv.strip_prefix("t=").is_some_and(|t| constant_eq(t.as_bytes(), self.token.as_bytes())));
            let resp = if ok {
                Response::from_string(PAGE)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap())
                    .with_header(Header::from_bytes(&b"Content-Security-Policy"[..], &b"default-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src 'self' data: blob:; font-src data:"[..]).unwrap())
                    .with_header(Header::from_bytes(&b"X-Frame-Options"[..], &b"DENY"[..]).unwrap())
            } else {
                Response::from_string("Abrí el panel desde el enlace que muestra kbss-engine.").with_status_code(403)
            };
            let _ = r.respond(resp);
            return;
        }
        if !path.starts_with("/api/") || !self.api_ok(&r) {
            let _ = r.respond(Response::from_string("prohibido").with_status_code(403));
            return;
        }
        let limit = match path.as_str() {
            "/api/doc" => MAX_DOC,
            "/api/background" => MAX_BG,
            _ => MAX_SMALL,
        };
        if r.body_length().unwrap_or(0) > limit {
            let _ = r.respond(json_resp(json!({"error": "demasiado grande"}), 413));
            return;
        }
        let mut body = Vec::new();
        if r.as_reader().take(limit as u64 + 1).read_to_end(&mut body).is_err() || body.len() > limit {
            let _ = r.respond(json_resp(json!({"error": "cuerpo inválido"}), 400));
            return;
        }
        let resp = match (r.method(), path.as_str()) {
            (Method::Get, "/api/state") => json_resp(self.state(), 200),
            (Method::Get, "/api/qr.svg") => self.qr(),
            (Method::Post, "/api/answer") => {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let ok = self.ui.answer(v["id"].as_u64(), v["answer"].as_str().unwrap_or("n"));
                json_resp(json!({"ok": ok}), 200)
            }
            (Method::Post, "/api/device") => {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                json_resp(self.device(v["key"].as_str().unwrap_or(""), v["action"].as_str().unwrap_or("")), 200)
            }
            (Method::Post, "/api/doc") => {
                let name = url
                    .split_once("name=")
                    .map(|x| percent_decode(x.1.split('&').next().unwrap_or("")))
                    .unwrap_or_else(|| "documento".into());
                let kind = if body.starts_with(b"%PDF") { DocKind::Pdf } else { DocKind::Text };
                match self.host.send_document(&name, kind, &body) {
                    Ok(n) => json_resp(json!({"ok": true, "sent": n}), 200),
                    Err(e) => json_resp(json!({"error": e}), 400),
                }
            }
            (Method::Post, "/api/prompter") => {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let action = match v["action"].as_str().unwrap_or("") {
                    "next" => Some(PrompterAction::NextPage),
                    "prev" => Some(PrompterAction::PrevPage),
                    "auto" => Some(PrompterAction::AutoScroll(v["value"].as_u64().unwrap_or(0).min(10) as u8)),
                    "lock" => {
                        let l = !self.locked.load(Ordering::Relaxed);
                        self.locked.store(l, Ordering::Relaxed);
                        Some(PrompterAction::Lock(l))
                    }
                    "close" => Some(PrompterAction::Close),
                    _ => None,
                };
                match action {
                    Some(a) => json_resp(json!({"ok": true, "sent": self.host.prompter(a)}), 200),
                    None => json_resp(json!({"error": "acción desconocida"}), 400),
                }
            }
            (Method::Get, "/api/background") => match std::fs::read(&self.bg_path) {
                Ok(b) => {
                    let ct = image_type(&b);
                    Response::from_data(b)
                        .with_header(Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap())
                        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
                },
                Err(_) => Response::from_data(Vec::new()).with_status_code(204),
            },
            (Method::Post, "/api/background") => {
                if body.is_empty() {
                    let _ = std::fs::remove_file(&self.bg_path);
                    json_resp(json!({"ok": true, "removed": true}), 200)
                } else if image_type(&body) == "application/octet-stream" {
                    json_resp(json!({"error": "formato no admitido (JPG, PNG o WebP)"}), 400)
                } else {
                    match std::fs::write(&self.bg_path, &body) {
                        Ok(_) => json_resp(json!({"ok": true}), 200),
                        Err(e) => json_resp(json!({"error": e.to_string()}), 500),
                    }
                }
            }
            (Method::Post, "/api/show") => {
                // Otra copia del programa pide mostrar la ventana.
                #[cfg(windows)]
                let (shown, err) = (crate::gui::request_show(), crate::gui::last_error());
                #[cfg(not(windows))]
                let (shown, err) = (false, String::new());
                json_resp(json!({"ok": true, "shown": shown, "error": err}), 200)
            }
            (Method::Post, "/api/quit") => {
                // Cerrar el programa desde el panel: responde y sale enseguida.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    std::process::exit(0);
                });
                json_resp(json!({"ok": true}), 200)
            }
            (Method::Post, "/api/slot") => {
                // Cambio en vivo de un dispositivo desde el panel.
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let fp = v["key"].as_str().and_then(TrustStore::fingerprint_from_key);
                let mon = v["monitor_id"].as_str().map(str::to_string);
                let q = v["quality"].as_str().and_then(quality_from);
                match fp {
                    Some(fp) => json_resp(json!({"ok": self.host.apply_device(&fp, mon, q, None)}), 200),
                    None => json_resp(json!({"error": "dispositivo inválido"}), 400),
                }
            }
            (Method::Post, "/api/profile") => {
                let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                let name = v["name"].as_str().unwrap_or("");
                if !NAMES.contains(&name) {
                    json_resp(json!({"error": "perfil desconocido"}), 400)
                } else {
                    let topo = self.platform.topology();
                    let mut p = self.profiles.lock().unwrap();
                    match v["action"].as_str() {
                        Some("save") => {
                            let n = p.snapshot(name, &self.host, &topo);
                            p.save(&self.profiles_path);
                            self.ui.note(format!("  Perfil «{name}» guardado ({n} dispositivo/s)."));
                            json_resp(json!({"ok": true, "devices": n}), 200)
                        }
                        Some("apply") => {
                            let n = p.apply(name, &self.host, &topo);
                            self.ui.note(format!("  Perfil «{name}» aplicado a {n} dispositivo/s."));
                            json_resp(json!({"ok": true, "devices": n}), 200)
                        }
                        _ => json_resp(json!({"error": "acción desconocida"}), 400),
                    }
                }
            }
            #[cfg(windows)]
            (Method::Post, "/api/rescue") => {
                let n = kb_win::windows_rescue::rescue_offscreen_windows(&self.platform.topology());
                json_resp(json!({"ok": true, "moved": n}), 200)
            }
            _ => json_resp(json!({"error": "no encontrado"}), 404),
        };
        let _ = r.respond(resp);
    }

    fn state(&self) -> Value {
        let slots: Vec<Value> = self.ui.slots.lock().unwrap().iter().map(|s| json!(s)).collect();
        let devices: Vec<Value> = self
            .host
            .trusted()
            .into_iter()
            .map(|(k, d)| json!({"key": k, "name": d.name, "permission": perm_es(d.permission), "control": d.permission == Permission::Control,
                "quality": d.profile.map(quality_key), "monitor_id": d.last_monitor}))
            .collect();
        let monitors: Vec<Value> = self
            .platform
            .topology()
            .monitors
            .iter()
            .map(|m| json!({"id": m.id, "name": m.name, "w": m.bounds.w, "h": m.bounds.h, "x": m.bounds.x, "y": m.bounds.y, "primary": m.primary, "virtual": m.is_virtual}))
            .collect();
        json!({
            "host": self.host_name, "fp": self.fp_short, "ip": self.lan_ip, "port": self.quic_port,
            "slots": slots, "pending": self.ui.pending_view(),
            "pin": self.ui.pin_view().map(|(d, p)| json!({"device": d, "pin": p})),
            "devices": devices, "monitors": monitors, "locked": self.locked.load(Ordering::Relaxed),
            "log": self.ui.log_lines(),
            "profiles": self.profiles_view(),
        })
    }

    fn profiles_view(&self) -> Vec<Value> {
        let p = self.profiles.lock().unwrap();
        NAMES
            .iter()
            .map(|n| {
                let devs: Vec<Value> = p
                    .items
                    .get(*n)
                    .map(|m| {
                        m.values()
                            .map(|d| json!({"name": d.name, "monitor": d.monitor_name, "quality": d.profile.map(quality_key), "permission": perm_es(d.permission)}))
                            .collect()
                    })
                    .unwrap_or_default();
                json!({"name": n, "devices": devs})
            })
            .collect()
    }

    fn qr(&self) -> Response<std::io::Cursor<Vec<u8>>> {
        let Some(ip) = &self.lan_ip else {
            return Response::from_data(Vec::new()).with_status_code(404);
        };
        let uri = self.host.issue_qr(ip.clone(), self.quic_port).to_uri();
        let svg = qrcode::QrCode::new(uri.as_bytes())
            .map(|c| c.render::<qrcode::render::svg::Color>().min_dimensions(260, 260).quiet_zone(true).build())
            .unwrap_or_default();
        Response::from_data(svg.into_bytes())
            .with_header(Header::from_bytes(&b"Content-Type"[..], &b"image/svg+xml"[..]).unwrap())
            .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
    }

    fn device(&self, key: &str, action: &str) -> Value {
        let Some(fp) = TrustStore::fingerprint_from_key(key) else { return json!({"error": "dispositivo inválido"}) };
        let dev = self.host.trusted().into_iter().find(|(k, _)| k == key).map(|(_, d)| d);
        let Some(dev) = dev else { return json!({"error": "no encontrado"}) };
        match action {
            "revoke" => json!({"ok": self.host.revoke(&fp)}),
            "toggle" => {
                let np = if dev.permission == Permission::Control { Permission::ViewOnly } else { Permission::Control };
                json!({"ok": self.host.set_permission(&fp, np)})
            }
            _ => json!({"error": "acción desconocida"}),
        }
    }
}

/// Tipo de imagen por su firma (sólo JPG, PNG, WebP).
fn image_type(b: &[u8]) -> &'static str {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if b.starts_with(b"\x89PNG") {
        "image/png"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

fn quality_key(q: QualityProfile) -> &'static str {
    match q {
        QualityProfile::Mixer => "mixer",
        QualityProfile::Lyrics => "lyrics",
        QualityProfile::Auto => "auto",
    }
}

fn quality_from(s: &str) -> Option<QualityProfile> {
    match s {
        "mixer" => Some(QualityProfile::Mixer),
        "lyrics" => Some(QualityProfile::Lyrics),
        "auto" => Some(QualityProfile::Auto),
        _ => None,
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percent_decoding_and_constant_eq() {
        assert_eq!(percent_decode("Tema%201.pdf"), "Tema 1.pdf");
        assert_eq!(percent_decode("Canci%C3%B3n.txt"), "Canción.txt");
        assert_eq!(percent_decode("roto%2"), "roto%2");
        assert!(constant_eq(b"abc", b"abc"));
        assert!(!constant_eq(b"abc", b"abd"));
        assert!(!constant_eq(b"abc", b"ab"));
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use kb_geometry::{MonitorDesc, Rect, Topology};
    use kb_session::host::{HostConfig, TrustPersistence};
    use kb_session::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    struct P;
    impl Platform for P {
        fn topology(&self) -> Topology {
            let b = Rect::new(0, 0, 2560, 1440);
            Topology { monitors: vec![MonitorDesc { id: "M".into(), name: "DELL U2719D".into(), bounds: b, work_area: b, dpi: 96, primary: true, is_virtual: false }] }
        }
        fn open_capture(&self, _: &str) -> Result<Box<dyn Capturer>, CaptureError> {
            Err(CaptureError::Fatal("test".into()))
        }
        fn open_encoder(&self, _: &kb_proto::StreamParams) -> Result<Box<dyn VideoEncoder>, String> {
            Err("test".into())
        }
        fn input(&self) -> Arc<dyn InputSink> {
            struct N;
            impl InputSink for N {
                fn inject(&self, _: Injected) {}
            }
            Arc::new(N)
        }
    }
    struct NoP;
    impl TrustPersistence for NoP {
        fn save(&self, _: &TrustStore) {}
    }

    pub(crate) fn start(port: u16) -> Arc<Panel> {
        let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().unwrap()));
        let _g = rt.enter();
        let id = kb_transport::Identity::generate().unwrap();
        let ui = Arc::new(Ui::new());
        let platform: Arc<dyn Platform> = Arc::new(P);
        let host = Host::new(
            HostConfig { host_id: [0; 16], host_name: "ESTUDIO".into(), extend_grace: std::time::Duration::from_secs(1) },
            id.fingerprint(),
            platform.clone(),
            ui.clone(),
            Arc::new(NoP),
            TrustStore::default(),
        );
        let p = Arc::new(Panel {
            host,
            ui,
            platform,
            host_name: "ESTUDIO".into(),
            fp_short: id.fingerprint().short(),
            lan_ip: Some("192.168.0.10".into()),
            quic_port: 47800,
            token: new_token(),
            port,
            locked: AtomicBool::new(false),
            profiles: std::sync::Mutex::new(Profiles::default()),
            profiles_path: std::env::temp_dir().join(format!("kbss_profiles_test_{port}.json")),
            bg_path: std::env::var("KBSS_TEST_BG").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join(format!("kbss_bg_test_{port}.bin"))),
        });
        let p2 = p.clone();
        std::thread::spawn(move || p2.serve().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(200));
        p
    }

    fn req(port: u16, raw: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn panel_requires_token_local_host_and_own_origin() {
        let port = 47911;
        let p = start(port);
        let t = &p.token;
        let ok_host = format!("Host: 127.0.0.1:{port}");
        // Página sin token / con token.
        assert!(req(port, &format!("GET / HTTP/1.1\r\n{ok_host}\r\nConnection: close\r\n\r\n")).starts_with("HTTP/1.1 403"));
        let page = req(port, &format!("GET /?t={t} HTTP/1.1\r\n{ok_host}\r\nConnection: close\r\n\r\n"));
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("KB Studio Screen"));
        // DNS rebinding: Host ajeno, aun con token.
        assert!(req(port, &format!("GET /api/state HTTP/1.1\r\nHost: evil.example:{port}\r\nX-KBSS-Token: {t}\r\nConnection: close\r\n\r\n")).starts_with("HTTP/1.1 403"));
        // API sin token.
        assert!(req(port, &format!("GET /api/state HTTP/1.1\r\n{ok_host}\r\nConnection: close\r\n\r\n")).starts_with("HTTP/1.1 403"));
        // API desde otro origen (sitio web malicioso), aun con token.
        assert!(req(port, &format!("POST /api/prompter HTTP/1.1\r\n{ok_host}\r\nX-KBSS-Token: {t}\r\nOrigin: https://evil.example\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}")).starts_with("HTTP/1.1 403"));
        // Correcto.
        let st = req(port, &format!("GET /api/state HTTP/1.1\r\n{ok_host}\r\nX-KBSS-Token: {t}\r\nConnection: close\r\n\r\n"));
        assert!(st.starts_with("HTTP/1.1 200"), "{st}");
        assert!(st.contains("\"host\":\"ESTUDIO\"") && st.contains("DELL U2719D"));
        let qr = req(port, &format!("GET /api/qr.svg HTTP/1.1\r\n{ok_host}\r\nX-KBSS-Token: {t}\r\nConnection: close\r\n\r\n"));
        assert!(qr.starts_with("HTTP/1.1 200") && qr.contains("<svg"));
        // Cuerpo demasiado grande en una ruta chica.
        let big = "x".repeat(MAX_SMALL + 10);
        let r = req(port, &format!("POST /api/answer HTTP/1.1\r\n{ok_host}\r\nX-KBSS-Token: {t}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{big}", big.len()));
        assert!(r.starts_with("HTTP/1.1 413"), "{r}");
    }

    /// Mantiene el panel abierto con datos de ejemplo (para capturas de pantalla).
    /// `KBSS_PANEL_DEMO=1 cargo test -p kb-engine panel_demo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn panel_demo() {
        let p = start(47912);
        {
            let mut s = p.ui.slots.lock().unwrap();
            s[0] = Some(crate::ui::SlotView { key: String::new(), name: "Galaxy S23 FE".into(), permission: "control táctil".into(), monitor: "M".into(), width: 2340, height: 1080, fps: 30, encoder: "NVIDIA NVENC H.264 (hardware)".into() });
            s[1] = Some(crate::ui::SlotView { key: String::new(), name: "Tablet EXO".into(), permission: "solo ver".into(), monitor: "M".into(), width: 1280, height: 800, fps: 10, encoder: "NVIDIA NVENC H.264 (hardware)".into() });
        }
        p.profiles.lock().unwrap().items.insert(
            "Grabación vocal".into(),
            [("a".to_string(), crate::profiles::DeviceSetup { name: "Galaxy S23 FE".into(), monitor_id: Some("M".into()), monitor_name: Some("DELL U2719D".into()), profile: Some(kb_proto::QualityProfile::Mixer), permission: Permission::Control }),
             ("b".to_string(), crate::profiles::DeviceSetup { name: "Tablet EXO".into(), monitor_id: None, monitor_name: None, profile: Some(kb_proto::QualityProfile::Lyrics), permission: Permission::ViewOnly })]
            .into_iter().collect(),
        );
        use kb_session::host::HostUi;
        p.ui.show_pin("Teléfono del bajista", "482913");
        let _rx = p.ui.approve(kb_session::host::ApprovalRequest { device_name: "Teléfono del bajista".into(), kind: kb_proto::DeviceKind::Phone, fingerprint_short: "3F-A1-09-C2".into(), method: "PIN" });
        println!("PANEL_URL {}", p.url());
        std::fs::write(std::env::var("KBSS_PANEL_URL_FILE").unwrap_or("/tmp/kbss_panel_url".into()), p.url()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(40));
    }
}

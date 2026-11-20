//! Host de DEMOSTRACIÓN para pruebas extremo a extremo con la app Android
//! (emulador en CI). No captura la pantalla: transmite un H.264 real
//! pregrabado y registra las entradas recibidas en lugar de inyectarlas.
//! Aprueba automáticamente con permiso de control. NO es un producto: sólo
//! existe para probar el cliente sin una PC con Windows.
//!
//!   cargo run -p kb-session --example demo_host -- <archivo.h264> <puerto> <host-para-QR> <salida-qr.txt>

use kb_geometry::{MonitorDesc, Rect, Topology};
use kb_pairing::TrustStore;
use kb_proto::{Permission, StreamParams};
use kb_session::host::{ApprovalRequest, Host, HostConfig, HostEvent, HostUi, TrustPersistence};
use kb_session::*;
use kb_transport::{server_endpoint, Identity};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

/// Divide un Annex-B en unidades de acceso usando los delimitadores (NAL 9).
fn access_units(data: &[u8]) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 4 < data.len() {
        let sc4 = data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1;
        if sc4 && data[i + 4] & 0x1F == 9 {
            starts.push(i);
        }
        i += 1;
    }
    starts.push(data.len());
    starts.windows(2).map(|w| data[w[0]..w[1]].to_vec()).filter(|u| !u.is_empty()).collect()
}

fn is_key(au: &[u8]) -> bool {
    au.windows(4).any(|w| w[0] == 0 && w[1] == 0 && w[2] == 1 && w[3] & 0x1F == 5)
}

struct DemoPlatform {
    aus: Arc<Vec<Vec<u8>>>,
}

struct Tick;
impl Capturer for Tick {
    fn next_frame(&mut self, _t: u32) -> Result<Option<Frame>, CaptureError> {
        std::thread::sleep(Duration::from_millis(33));
        // Frame ficticio: el encoder de demo ignora su contenido.
        Ok(Some(Frame { width: 1280, height: 720, stride: 1280 * 4, data: vec![0; 1280 * 720 * 4], capture_us: mono_us() }))
    }
}

struct Replay {
    aus: Arc<Vec<Vec<u8>>>,
    pos: usize,
}
impl VideoEncoder for Replay {
    fn encode(&mut self, _nv12: &[u8], _pts: u64, force_key: bool) -> Result<Vec<EncodedFrame>, String> {
        if force_key || self.pos >= self.aus.len() {
            self.pos = 0; // el archivo empieza con un IDR (SPS/PPS repetidos)
        }
        let au = self.aus[self.pos].clone();
        self.pos += 1;
        Ok(vec![EncodedFrame { keyframe: is_key(&au), data: au }])
    }
    fn name(&self) -> String {
        "demo (H.264 pregrabado)".into()
    }
}

struct LogInput;
impl InputSink for LogInput {
    fn inject(&self, ev: Injected) {
        println!("INJECT {ev:?}");
    }
}

impl Platform for DemoPlatform {
    fn topology(&self) -> Topology {
        let b = Rect::new(0, 0, 1280, 720);
        Topology { monitors: vec![MonitorDesc { id: "DEMO".into(), name: "Demo 720p".into(), bounds: b, work_area: b, dpi: 96, primary: true, is_virtual: false }] }
    }
    fn open_capture(&self, _id: &str) -> Result<Box<dyn Capturer>, CaptureError> {
        Ok(Box::new(Tick))
    }
    fn open_encoder(&self, p: &StreamParams) -> Result<Box<dyn VideoEncoder>, String> {
        println!("STREAM {}x{} @ {} fps", p.width, p.height, p.fps);
        Ok(Box::new(Replay { aus: self.aus.clone(), pos: 0 }))
    }
    fn input(&self) -> Arc<dyn InputSink> {
        Arc::new(LogInput)
    }
}

struct AutoUi;
impl HostUi for AutoUi {
    fn show_pin(&self, d: &str, _pin: &str) {
        println!("PIN mostrado para {d}");
    }
    fn hide_pin(&self, _d: &str) {}
    fn approve(&self, r: ApprovalRequest) -> oneshot::Receiver<Option<Permission>> {
        println!("APROBADO {} por {}", r.device_name, r.method);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Some(Permission::Control));
        rx
    }
    fn confirm(&self, _d: &str, _a: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(false);
        rx
    }
    fn event(&self, e: HostEvent) {
        println!("EVENT {e:?}");
    }
}

struct NoPersist;
impl TrustPersistence for NoPersist {
    fn save(&self, _t: &TrustStore) {}
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let file = a.get(1).expect("archivo .h264");
    let port: u16 = a.get(2).and_then(|p| p.parse().ok()).unwrap_or(47800);
    let qr_host = a.get(3).cloned().unwrap_or_else(|| "127.0.0.1".into());
    let qr_out = a.get(4).cloned().unwrap_or_else(|| "qr.txt".into());
    let aus = access_units(&std::fs::read(file).expect("leer h264"));
    assert!(!aus.is_empty() && is_key(&aus[0]), "el archivo debe empezar con un IDR");
    println!("Demo: {} unidades de acceso cargadas", aus.len());

    let id = Identity::generate().unwrap();
    let ep = server_endpoint(&id, std::net::SocketAddr::from(([0, 0, 0, 0], port))).unwrap();
    let host = Host::new(
        HostConfig { host_id: [7; 16], host_name: "DemoHost".into(), extend_grace: Duration::from_secs(5) },
        id.fingerprint(),
        Arc::new(DemoPlatform { aus: Arc::new(aus) }),
        Arc::new(AutoUi),
        Arc::new(NoPersist),
        TrustStore::default(),
    );
    let qr = host.issue_qr(qr_host, port);
    std::fs::write(&qr_out, qr.to_uri()).unwrap();
    println!("QR escrito en {qr_out}");
    host.serve(ep).await;
}

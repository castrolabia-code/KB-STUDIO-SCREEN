//! Pruebas extremo a extremo: host y cliente reales sobre QUIC (loopback),
//! con una plataforma falsa (captura sintética, encoder falso, entrada registrada).

use kb_geometry::{MonitorDesc, NormPoint, PointI, Rect, Topology};
use kb_pairing::TrustStore;
use kb_proto::*;
use kb_session::client::{Auth, Client, ClientError, ClientInfo, Stage};
use kb_session::host::{ApprovalRequest, Host, HostConfig, HostEvent, HostUi, TrustPersistence};
use kb_session::*;
use kb_transport::{server_endpoint, Identity};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

struct FakePlatform {
    injected: Arc<Mutex<Vec<Injected>>>,
    virtuals: Arc<Mutex<Vec<(u8, MonitorDesc)>>>,
}
struct FakeCap(u32);
impl Capturer for FakeCap {
    fn next_frame(&mut self, _t: u32) -> Result<Option<Frame>, CaptureError> {
        std::thread::sleep(Duration::from_millis(20));
        self.0 += 1;
        let (w, h) = (640u32, 360u32);
        Ok(Some(Frame { width: w, height: h, stride: w * 4, data: vec![(self.0 % 255) as u8; (w * h * 4) as usize], capture_us: mono_us() }))
    }
}
struct FakeEnc;
impl VideoEncoder for FakeEnc {
    fn encode(&mut self, nv12: &[u8], _pts: u64, key: bool) -> Result<Vec<EncodedFrame>, String> {
        let mut d = vec![0, 0, 0, 1, if key { 0x65 } else { 0x41 }];
        d.extend_from_slice(&nv12[..64]);
        Ok(vec![EncodedFrame { data: d, keyframe: key }])
    }
    fn name(&self) -> String {
        "fake".into()
    }
}
struct Rec(Arc<Mutex<Vec<Injected>>>);
impl InputSink for Rec {
    fn inject(&self, ev: Injected) {
        self.0.lock().unwrap().push(ev);
    }
}
impl Platform for FakePlatform {
    fn topology(&self) -> Topology {
        let m = |id: &str, b: Rect, primary| MonitorDesc { id: id.into(), name: id.into(), bounds: b, work_area: b, dpi: 96, primary, is_virtual: false };
        let mut v = vec![m("LEFT", Rect::new(-1920, 0, 1920, 1080), false), m("MAIN", Rect::new(0, 0, 2560, 1440), true)];
        v.extend(self.virtuals.lock().unwrap().iter().map(|(_, d)| d.clone()));
        Topology { monitors: v }
    }
    fn add_virtual_monitor(&self, slot: u8, w: u32, h: u32) -> Result<MonitorDesc, String> {
        let b = Rect::new(2560 + slot as i32 * 3000, 0, w as i32, h as i32);
        let d = MonitorDesc { id: format!("VIRT{slot}"), name: "KBSS Pantalla".into(), bounds: b, work_area: b, dpi: 96, primary: false, is_virtual: true };
        self.virtuals.lock().unwrap().push((slot, d.clone()));
        Ok(d)
    }
    fn remove_virtual_monitor(&self, slot: u8) {
        self.virtuals.lock().unwrap().retain(|(s, _)| *s != slot);
    }
    fn open_capture(&self, _id: &str) -> Result<Box<dyn Capturer>, CaptureError> {
        Ok(Box::new(FakeCap(0)))
    }
    fn open_encoder(&self, _p: &StreamParams) -> Result<Box<dyn VideoEncoder>, String> {
        Ok(Box::new(FakeEnc))
    }
    fn input(&self) -> Arc<dyn InputSink> {
        Arc::new(Rec(self.injected.clone()))
    }
}

/// UI de prueba: aprueba con el permiso configurado y publica el PIN mostrado.
struct TestUi {
    approve_with: Mutex<Option<Permission>>,
    pin: Mutex<Option<String>>,
}
impl HostUi for TestUi {
    fn show_pin(&self, _d: &str, pin: &str) {
        *self.pin.lock().unwrap() = Some(pin.to_string());
    }
    fn hide_pin(&self, _d: &str) {}
    fn confirm(&self, _d: &str, _a: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(true);
        rx
    }
    fn approve(&self, _r: ApprovalRequest) -> oneshot::Receiver<Option<Permission>> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(*self.approve_with.lock().unwrap());
        rx
    }
    fn event(&self, _e: HostEvent) {}
}
struct NoPersist;
impl TrustPersistence for NoPersist {
    fn save(&self, _t: &TrustStore) {}
}

struct Rig {
    host: Host,
    addr: std::net::SocketAddr,
    ui: Arc<TestUi>,
    injected: Arc<Mutex<Vec<Injected>>>,
    virtuals: Arc<Mutex<Vec<(u8, MonitorDesc)>>>,
    host_fp: kb_pairing::Fingerprint,
}

fn rig(approve: Option<Permission>) -> Rig {
    let id = Identity::generate().unwrap();
    let host_fp = id.fingerprint();
    let ep = server_endpoint(&id, "127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = ep.local_addr().unwrap();
    let injected = Arc::new(Mutex::new(Vec::new()));
    let virtuals = Arc::new(Mutex::new(Vec::new()));
    let ui = Arc::new(TestUi { approve_with: Mutex::new(approve), pin: Mutex::new(None) });
    let host = Host::new(
        HostConfig { host_id: [1; 16], host_name: "PC Estudio".into(), extend_grace: Duration::from_millis(800) },
        host_fp,
        Arc::new(FakePlatform { injected: injected.clone(), virtuals: virtuals.clone() }),
        ui.clone(),
        Arc::new(NoPersist),
        TrustStore::default(),
    );
    let h = host.clone();
    tokio::spawn(async move { h.serve(ep).await });
    Rig { host, addr, ui, injected, virtuals, host_fp }
}

fn info(name: &str) -> ClientInfo {
    ClientInfo {
        device_id: [9; 16],
        device_name: name.into(),
        kind: DeviceKind::Tablet,
        screen: ScreenInfo { width_px: 1280, height_px: 800, density_dpi: 213, orientation: Orientation::Landscape, refresh_hz: 60 },
        decoder: DecoderCaps { h264_profiles: vec![H264Profile::Baseline], max_width: 1920, max_height: 1088, max_fps_at_max_res: 30, hardware: true },
    }
}

async fn pair_by_pin(r: &Rig, dev: &Identity, name: &str) -> Result<(Client, kb_session::client::SessionInfo), ClientError> {
    let (c, stage) = Client::connect(r.addr, dev, info(name), Auth::Pin).await?;
    assert_eq!(stage, Stage::NeedPin);
    c.request_pin().await?;
    let pin = r.ui.pin.lock().unwrap().clone().expect("el host mostró un PIN");
    let s = c.submit_pin(&pin).await?;
    Ok((c, s))
}

fn touch(phase: PointerPhase, x: u16, y: u16) -> Message {
    Message::Input(InputEvent::Pointer(PointerEvent { pointer_id: 0, phase, pos: NormPoint { x, y }, t_us: 0 }))
}

async fn wait_injected(r: &Rig, n: usize) -> Vec<Injected> {
    for _ in 0..100 {
        if r.injected.lock().unwrap().len() >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    r.injected.lock().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn pin_pairing_video_and_fader_drag_end_to_end() {
    let r = rig(Some(Permission::Control));
    let dev = Identity::generate().unwrap();
    let (c, s) = pair_by_pin(&r, &dev, "Tablet EXO").await.unwrap();
    assert_eq!(s.permission, Permission::Control);
    assert_eq!(s.mode, Mode::Mirror { monitor_id: "MAIN".into() });
    // 2560x1440 en pantalla 1280x800 -> 1280x720, perfil Mixer.
    assert_eq!((s.params.width, s.params.height, s.params.profile), (1280, 720, QualityProfile::Mixer));
    assert_eq!(c.host_fp, r.host_fp);
    assert_eq!(c.monitors().len(), 2);

    // Primer frame recibido: debe ser keyframe.
    let (h, d) = tokio::time::timeout(Duration::from_secs(5), c.next_frame()).await.unwrap().unwrap();
    assert!(h.keyframe && d[4] == 0x65);
    assert_eq!((h.width, h.height), (1280, 720));

    // Arrastre sostenido de un fader: abajo -> mover -> arriba.
    c.send(&touch(PointerPhase::Down, 32768, 49151)).await.unwrap();
    c.send(&touch(PointerPhase::Move, 32768, 16384)).await.unwrap();
    c.send(&touch(PointerPhase::Up, 32768, 16384)).await.unwrap();
    let ev = wait_injected(&r, 5).await;
    assert_eq!(
        ev,
        vec![
            Injected::Move { p: PointI { x: 1280, y: 1079 } },
            Injected::Button { p: PointI { x: 1280, y: 1079 }, button: MouseButton::Left, down: true },
            Injected::Move { p: PointI { x: 1280, y: 360 } },
            Injected::Move { p: PointI { x: 1280, y: 360 } },
            Injected::Button { p: PointI { x: 1280, y: 360 }, button: MouseButton::Left, down: false },
        ]
    );

    // Cambio de origen al monitor izquierdo (coordenadas negativas).
    c.send(&Message::ModeChangeRequest { mode: Mode::Mirror { monitor_id: "LEFT".into() } }).await.unwrap();
    loop {
        match c.recv_control().await.unwrap() {
            Message::ModeChangeResult { accepted, .. } => assert!(accepted),
            Message::SessionConfig { mode, .. } => {
                assert_eq!(mode, Mode::Mirror { monitor_id: "LEFT".into() });
                break;
            }
            _ => {}
        }
    }
    r.injected.lock().unwrap().clear();
    c.send(&touch(PointerPhase::Down, 0, 0)).await.unwrap();
    c.send(&touch(PointerPhase::Up, 0, 0)).await.unwrap();
    let ev = wait_injected(&r, 4).await;
    assert_eq!(ev[0], Injected::Move { p: PointI { x: -1920, y: 0 } });

    // Reconexión con huella fijada: sin volver a emparejar.
    c.close();
    let (c2, stage) = Client::connect(r.addr, &dev, info("Tablet EXO"), Auth::Trusted(r.host_fp)).await.unwrap();
    assert!(matches!(stage, Stage::Ready(_)));

    // Revocación: corta la sesión y obliga a emparejar de nuevo.
    let key = r.host.trusted()[0].0.clone();
    assert!(r.host.revoke(&TrustStore::fingerprint_from_key(&key).unwrap()));
    let res = tokio::time::timeout(Duration::from_secs(5), c2.recv_control()).await.unwrap();
    assert!(res.is_err(), "la sesión revocada debe cerrarse");
    let (_c3, stage) = Client::connect(r.addr, &dev, info("Tablet EXO"), Auth::Trusted(r.host_fp)).await.unwrap();
    assert_eq!(stage, Stage::NeedPin);
}

#[tokio::test(flavor = "multi_thread")]
async fn view_only_singer_cannot_touch_and_gets_lyrics_profile() {
    let r = rig(Some(Permission::ViewOnly));
    let dev = Identity::generate().unwrap();
    let (c, s) = pair_by_pin(&r, &dev, "Cantante").await.unwrap();
    assert_eq!(s.permission, Permission::ViewOnly);
    assert_eq!(s.params.profile, QualityProfile::Lyrics);
    c.send(&touch(PointerPhase::Down, 100, 100)).await.unwrap();
    c.send(&touch(PointerPhase::Up, 100, 100)).await.unwrap();
    c.send(&Message::ModeChangeRequest { mode: Mode::Mirror { monitor_id: "LEFT".into() } }).await.unwrap();
    loop {
        if let Message::ModeChangeResult { accepted, reason } = c.recv_control().await.unwrap() {
            assert!(!accepted);
            assert_eq!(reason, Some(ErrorCode::PermissionDenied));
            break;
        }
    }
    assert!(r.injected.lock().unwrap().is_empty(), "solo lectura no debe inyectar nada");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_pin_and_host_rejection_are_refused() {
    let r = rig(Some(Permission::Control));
    let dev = Identity::generate().unwrap();
    let (c, _) = Client::connect(r.addr, &dev, info("Intruso"), Auth::Pin).await.unwrap();
    c.request_pin().await.unwrap();
    let real = r.ui.pin.lock().unwrap().clone().unwrap();
    let wrong = if real == "000000" { "111111" } else { "000000" };
    assert!(c.submit_pin(wrong).await.is_err());
    assert!(r.host.trusted().is_empty());

    // El usuario del host rechaza la aprobación aunque el PIN sea correcto.
    *r.ui.approve_with.lock().unwrap() = None;
    let (c, _) = Client::connect(r.addr, &dev, info("Intruso"), Auth::Pin).await.unwrap();
    c.request_pin().await.unwrap();
    let pin = r.ui.pin.lock().unwrap().clone().unwrap();
    assert!(matches!(c.submit_pin(&pin).await, Err(ClientError::Rejected)));
    assert!(r.host.trusted().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn qr_pairing_and_single_use_token() {
    let r = rig(Some(Permission::Control));
    let qr = r.host.issue_qr("127.0.0.1".into(), r.addr.port());
    let qr = kb_pairing::QrPayload::parse(&qr.to_uri()).unwrap();
    let dev = Identity::generate().unwrap();
    let (_c, stage) = Client::connect(r.addr, &dev, info("S23 FE"), Auth::Qr(qr.clone())).await.unwrap();
    assert!(matches!(stage, Stage::Ready(_)));
    assert!(r.ui.pin.lock().unwrap().is_none(), "con QR no se muestra PIN");
    // El mismo QR no sirve para otro dispositivo.
    let other = Identity::generate().unwrap();
    let res = Client::connect(r.addr, &other, info("Otro"), Auth::Qr(qr)).await;
    assert!(matches!(res, Err(ClientError::Host(ErrorCode::PairingRejected))));
}

#[tokio::test(flavor = "multi_thread")]
async fn three_slots_then_full_and_single_cursor_arbitration() {
    let r = rig(Some(Permission::Control));
    let mut clients = Vec::new();
    for i in 0..3 {
        let dev = Identity::generate().unwrap();
        let (c, s) = pair_by_pin(&r, &dev, &format!("Dispositivo {i}")).await.unwrap();
        assert_eq!(s.slot, i);
        clients.push(c);
    }
    let dev = Identity::generate().unwrap();
    // Cuarto dispositivo: se queda sin ranura.
    let res = pair_by_pin(&r, &dev, "Cuarto").await;
    assert!(matches!(res, Err(ClientError::Host(ErrorCode::SlotsFull))), "la cuarta conexión debe rechazarse: {:?}", res.as_ref().map(|r| r.1.clone()));

    // Arbitraje: mientras el 0 arrastra, el 1 no puede.
    clients[0].send(&touch(PointerPhase::Down, 10, 10)).await.unwrap();
    assert_eq!(wait_injected(&r, 2).await.len(), 2);
    clients[1].send(&touch(PointerPhase::Down, 60000, 60000)).await.unwrap();
    clients[1].send(&touch(PointerPhase::Move, 60000, 50000)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.injected.lock().unwrap().len(), 2, "el segundo cliente no debe mover el cursor durante el arrastre del primero");
    clients[0].send(&touch(PointerPhase::Up, 10, 10)).await.unwrap();
    assert_eq!(wait_injected(&r, 4).await.len(), 4);
    // Liberado el cursor, el segundo ya puede.
    clients[1].send(&touch(PointerPhase::Down, 60000, 60000)).await.unwrap();
    assert_eq!(wait_injected(&r, 6).await.len(), 6);
}

async fn wait_mode(c: &Client) -> (bool, Option<Mode>) {
    let mut accepted = None;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), c.recv_control()).await.unwrap().unwrap() {
            Message::ModeChangeResult { accepted: a, .. } => {
                accepted = Some(a);
                if !a {
                    return (false, None);
                }
            }
            Message::SessionConfig { mode, .. } if accepted == Some(true) => return (true, Some(mode)),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn extend_creates_real_resolution_monitor_survives_brief_drop_and_cleans_up() {
    let r = rig(Some(Permission::Control));
    let dev = Identity::generate().unwrap();
    let (c, _) = pair_by_pin(&r, &dev, "Tablet EXO").await.unwrap();
    c.send(&Message::ModeChangeRequest { mode: Mode::Extend { position: ExtendPosition::RightOf } }).await.unwrap();
    let (ok, mode) = wait_mode(&c).await;
    assert!(ok);
    assert!(matches!(mode, Some(Mode::Extend { .. })));
    // Monitor virtual con la resolución real de la tablet (1280x800), no 1080p fija.
    let v = r.virtuals.lock().unwrap().clone();
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].1.bounds.w, v[0].1.bounds.h), (1280, 800));
    // El tacto cae en el monitor virtual.
    c.send(&touch(PointerPhase::Down, 0, 0)).await.unwrap();
    c.send(&touch(PointerPhase::Up, 0, 0)).await.unwrap();
    let ev = wait_injected(&r, 4).await;
    assert_eq!(ev[0], Injected::Move { p: PointI { x: v[0].1.bounds.x, y: 0 } });

    // Corte breve: el monitor sigue y al volver se retoma Extender.
    c.close();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.virtuals.lock().unwrap().len(), 1, "se conserva durante la gracia");
    let (c2, stage) = Client::connect(r.addr, &dev, info("Tablet EXO"), Auth::Trusted(r.host_fp)).await.unwrap();
    match stage {
        Stage::Ready(s) => assert!(matches!(s.mode, Mode::Extend { .. }), "retoma el monitor virtual"),
        _ => panic!(),
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(r.virtuals.lock().unwrap().len(), 1, "la sesión retomada es dueña del monitor");

    // Volver a duplicar retira el monitor virtual.
    c2.send(&Message::ModeChangeRequest { mode: Mode::Mirror { monitor_id: "MAIN".into() } }).await.unwrap();
    assert!(wait_mode(&c2).await.0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.virtuals.lock().unwrap().is_empty());

    // Extender y desaparecer: tras la gracia, se retira solo.
    c2.send(&Message::ModeChangeRequest { mode: Mode::Extend { position: ExtendPosition::RightOf } }).await.unwrap();
    assert!(wait_mode(&c2).await.0);
    c2.close();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(r.virtuals.lock().unwrap().is_empty(), "sin reconexión el monitor virtual se retira");
}

#[tokio::test(flavor = "multi_thread")]
async fn view_only_cannot_create_virtual_monitors() {
    let r = rig(Some(Permission::ViewOnly));
    let dev = Identity::generate().unwrap();
    let (c, _) = pair_by_pin(&r, &dev, "Cantante").await.unwrap();
    c.send(&Message::ModeChangeRequest { mode: Mode::Extend { position: ExtendPosition::RightOf } }).await.unwrap();
    assert!(!wait_mode(&c).await.0);
    assert!(r.virtuals.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_and_monitor_are_remembered_per_device() {
    let r = rig(Some(Permission::Control));
    let dev = Identity::generate().unwrap();
    let (c, s) = pair_by_pin(&r, &dev, "S23 FE").await.unwrap();
    assert_eq!(s.params.profile, QualityProfile::Mixer);
    // Elegir "Letras" desde el teléfono: el host reconfigura el stream.
    c.send(&Message::SetProfile { profile: QualityProfile::Lyrics }).await.unwrap();
    loop {
        if let Message::SessionConfig { params, .. } = c.recv_control().await.unwrap() {
            assert_eq!(params.profile, QualityProfile::Lyrics);
            assert_eq!(params.fps, 10);
            break;
        }
    }
    c.send(&Message::ModeChangeRequest { mode: Mode::Mirror { monitor_id: "LEFT".into() } }).await.unwrap();
    assert!(wait_mode(&c).await.0);
    c.close();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Al volver: mismo monitor y mismo perfil, sin tocar nada.
    let (_c2, stage) = Client::connect(r.addr, &dev, info("S23 FE"), Auth::Trusted(r.host_fp)).await.unwrap();
    match stage {
        Stage::Ready(s) => {
            assert_eq!(s.mode, Mode::Mirror { monitor_id: "LEFT".into() });
            assert_eq!(s.params.profile, QualityProfile::Lyrics);
        }
        _ => panic!(),
    }
}

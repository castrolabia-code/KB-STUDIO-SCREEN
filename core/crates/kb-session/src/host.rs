//! Servidor del host. Una tarea por conexión; hasta `MAX_SLOTS` sesiones.

use crate::{convert, log, now_ms, CaptureHub, Injected, InputArbiter, Platform};
use kb_geometry::{norm_to_desktop, MonitorDesc, Rect, Rotation};
use kb_pairing::{
    Fingerprint, HostPinExchange, PairError, PinCode, QrPayload, QrTokens, RateLimiter, ResumeTickets, TrustStore, TrustedDevice,
    PIN_TTL_MS,
};
use kb_proto::*;
use kb_transport::{authorize_inbound, peer_fingerprint, read_message, send_frame, write_message, TransportError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub device_name: String,
    pub kind: DeviceKind,
    /// Forma corta de la huella del dispositivo (verificación visual opcional).
    pub fingerprint_short: String,
    pub method: &'static str,
}

#[derive(Debug, Clone)]
pub enum HostEvent {
    Connected { slot: u8, name: String, permission: Permission },
    Streaming { slot: u8, monitor: String, width: u32, height: u32, fps: u16, encoder: String },
    Disconnected { slot: u8, name: String },
    Rejected { name: String, reason: String },
}

/// Interfaz con el usuario del host (consola en el MVP; Tauri después).
pub trait HostUi: Send + Sync + 'static {
    fn show_pin(&self, device: &str, pin: &str);
    fn hide_pin(&self, device: &str);
    /// Debe responder `Some(permiso)` sólo si el usuario aprobó explícitamente.
    fn approve(&self, req: ApprovalRequest) -> oneshot::Receiver<Option<Permission>>;
    /// Confirmación para operaciones sensibles pedidas desde Android
    /// (p. ej. crear un monitor virtual). `true` sólo si el usuario aceptó.
    fn confirm(&self, device: &str, action: &str) -> oneshot::Receiver<bool>;
    fn event(&self, ev: HostEvent);
}

pub trait TrustPersistence: Send + Sync + 'static {
    fn save(&self, t: &TrustStore);
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("tiempo de espera agotado")]
    Timeout,
    #[error("emparejamiento: {0}")]
    Pairing(#[from] PairError),
    #[error("rechazado: {0}")]
    Rejected(&'static str),
    #[error("conexión: {0}")]
    Conn(#[from] quinn::ConnectionError),
}

impl From<tokio::time::error::Elapsed> for SessionError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        SessionError::Timeout
    }
}

pub struct HostConfig {
    pub host_id: [u8; 16],
    pub host_name: String,
    /// Cuánto se conserva un monitor virtual tras una desconexión (normalmente `EXTEND_GRACE`).
    pub extend_grace: Duration,
}

struct Inner {
    cfg: HostConfig,
    host_fp: Fingerprint,
    platform: Arc<dyn Platform>,
    ui: Arc<dyn HostUi>,
    persist: Arc<dyn TrustPersistence>,
    trust: Mutex<TrustStore>,
    pending_pins: Mutex<HashMap<Fingerprint, PinCode>>,
    qr: Mutex<QrTokens>,
    tickets: ResumeTickets,
    limiter: Mutex<RateLimiter>,
    /// (dispositivo, generación de sesión) por ranura.
    slots: Mutex<[Option<(Fingerprint, u64)>; MAX_SLOTS as usize]>,
    next_gen: std::sync::atomic::AtomicU64,
    hub: CaptureHub,
    arbiter: InputArbiter,
    conns: Mutex<HashMap<Fingerprint, (u64, quinn::Connection)>>,
    /// Monitores virtuales vivos por dispositivo: (ranura, monitor, generación de sesión).
    extended: Mutex<HashMap<Fingerprint, (u8, MonitorDesc, u64)>>,
}

/// Tiempo que se conserva un monitor virtual tras una desconexión, esperando
/// que el mismo dispositivo vuelva (corte breve de Wi-Fi).
pub const EXTEND_GRACE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

impl Host {
    pub fn new(
        cfg: HostConfig,
        host_fp: Fingerprint,
        platform: Arc<dyn Platform>,
        ui: Arc<dyn HostUi>,
        persist: Arc<dyn TrustPersistence>,
        trust: TrustStore,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                host_fp,
                hub: CaptureHub::new(platform.clone()),
                platform,
                ui,
                persist,
                trust: Mutex::new(trust),
                pending_pins: Mutex::new(HashMap::new()),
                qr: Mutex::new(QrTokens::default()),
                tickets: ResumeTickets::new(),
                // 10 conexiones de ráfaga, luego 20 por minuto por IP.
                limiter: Mutex::new(RateLimiter::new(10, 20)),
                slots: Mutex::new([None; MAX_SLOTS as usize]),
                next_gen: std::sync::atomic::AtomicU64::new(1),
                arbiter: InputArbiter::default(),
                conns: Mutex::new(HashMap::new()),
                extended: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Genera un QR de un solo uso para emparejar.
    pub fn issue_qr(&self, host: String, port: u16) -> QrPayload {
        let p = QrPayload::new(host, port, self.inner.host_fp, now_ms());
        self.inner.qr.lock().unwrap().issue(&p);
        p
    }

    pub fn trusted(&self) -> Vec<(String, TrustedDevice)> {
        self.inner.trust.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Revoca un dispositivo: lo borra de la lista y corta su sesión ya.
    pub fn revoke(&self, fp: &Fingerprint) -> bool {
        let removed = {
            let mut t = self.inner.trust.lock().unwrap();
            let r = t.revoke(fp);
            self.inner.persist.save(&t);
            r
        };
        if let Some((_, c)) = self.inner.conns.lock().unwrap().remove(fp) {
            c.close(1u32.into(), b"revoked");
        }
        removed
    }

    pub fn set_permission(&self, fp: &Fingerprint, p: Permission) -> bool {
        let mut t = self.inner.trust.lock().unwrap();
        let ok = t.set_permission(fp, p);
        self.inner.persist.save(&t);
        ok
    }

    /// Bucle de aceptación. No retorna mientras el endpoint esté abierto.
    pub async fn serve(&self, endpoint: quinn::Endpoint) {
        while let Some(incoming) = endpoint.accept().await {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(inner, incoming).await {
                    log(&format!("sesión terminada: {e}"));
                }
            });
        }
    }
}

struct SlotGuard {
    inner: Arc<Inner>,
    slot: u8,
    peer: Fingerprint,
    gen: u64,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        // Sólo libera si la ranura sigue siendo de ESTA sesión (no de una
        // reconexión posterior del mismo dispositivo).
        let mut s = self.inner.slots.lock().unwrap();
        if s[self.slot as usize] == Some((self.peer, self.gen)) {
            s[self.slot as usize] = None;
        }
        let mut c = self.inner.conns.lock().unwrap();
        if c.get(&self.peer).is_some_and(|(g, _)| *g == self.gen) {
            c.remove(&self.peer);
        }
    }
}

async fn send(tx: &mut quinn::SendStream, m: Message) -> Result<(), SessionError> {
    Ok(write_message(tx, &m).await?)
}

/// Envía un error final y espera (máx. 2 s) a que el cliente lo reciba y
/// cierre, para que vea el motivo real y no un "conexión perdida".
async fn reject(conn: &quinn::Connection, tx: &mut quinn::SendStream, code: ErrorCode) {
    let _ = write_message(tx, &Message::Error { code }).await;
    let _ = tx.finish();
    let _ = timeout(Duration::from_secs(2), conn.closed()).await;
}

async fn handle(inner: Arc<Inner>, incoming: quinn::Incoming) -> Result<(), SessionError> {
    let ip = incoming.remote_address().ip().to_string();
    if !inner.limiter.lock().unwrap().allow(&ip, now_ms()) {
        incoming.refuse();
        return Err(SessionError::Rejected("rate limit"));
    }
    let conn = timeout(Duration::from_secs(10), incoming).await??;
    let peer = peer_fingerprint(&conn)?;
    let (mut tx, mut rx) = timeout(Duration::from_secs(10), conn.accept_bi()).await??;
    let hello = timeout(Duration::from_secs(10), read_message(&mut rx)).await??;
    let Message::ClientHello { device_name, kind, screen, decoder, .. } = hello else {
        reject(&conn, &mut tx, ErrorCode::Internal).await;
        return Err(SessionError::Rejected("se esperaba ClientHello"));
    };
    send(&mut tx, Message::HostHello { host_id: inner.cfg.host_id, host_name: inner.cfg.host_name.clone() }).await?;

    // Sin ranura disponible no tiene sentido pedir aprobación al usuario.
    let has_room = inner.slots.lock().unwrap().iter().any(|x| x.is_none_or(|(p, _)| p == peer));
    if !has_room {
        reject(&conn, &mut tx, ErrorCode::SlotsFull).await;
        return Err(SessionError::Rejected("sin ranuras libres"));
    }
    let known = inner.trust.lock().unwrap().get(&peer).cloned();
    if known.is_none() {
        send(&mut tx, Message::Error { code: ErrorCode::NotPaired }).await?;
        if let Err(e) = pair(&inner, &mut tx, &mut rx, peer, &device_name, kind).await {
            inner.ui.event(HostEvent::Rejected { name: device_name.clone(), reason: e.to_string() });
            // Dar tiempo a que el mensaje de error llegue antes de cerrar.
            let _ = tx.finish();
            let _ = timeout(Duration::from_secs(2), conn.closed()).await;
            return Err(e);
        }
    } else {
        inner.trust.lock().unwrap().touch(&peer, now_ms());
    }

    // Ranura libre (o la misma si el dispositivo se reconecta).
    let gen = inner.next_gen.fetch_add(1, Ordering::Relaxed);
    let slot = {
        let mut s = inner.slots.lock().unwrap();
        let i = s
            .iter()
            .position(|x| x.is_some_and(|(p, _)| p == peer))
            .or_else(|| s.iter().position(|x| x.is_none()));
        if let Some(i) = i {
            s[i] = Some((peer, gen));
        }
        i
    };
    let Some(slot) = slot else {
        reject(&conn, &mut tx, ErrorCode::SlotsFull).await;
        return Err(SessionError::Rejected("sin ranuras libres"));
    };
    let slot = slot as u8;
    let _guard = SlotGuard { inner: inner.clone(), slot, peer, gen };
    // Sesión vieja colgada del mismo dispositivo (p. ej. Wi-Fi caído): se reemplaza.
    if let Some((_, old)) = inner.conns.lock().unwrap().insert(peer, (gen, conn.clone())) {
        old.close(2u32.into(), b"replaced");
    }
    run_session(&inner, conn, tx, rx, peer, slot, gen, device_name, screen, decoder).await
}

fn pair_error_code(e: &PairError) -> ErrorCode {
    match e {
        PairError::Expired => ErrorCode::PairingExpired,
        PairError::TooManyAttempts => ErrorCode::TooManyAttempts,
        _ => ErrorCode::PairingRejected,
    }
}

async fn pair(
    inner: &Arc<Inner>,
    tx: &mut quinn::SendStream,
    rx: &mut quinn::RecvStream,
    peer: Fingerprint,
    name: &str,
    kind: DeviceKind,
) -> Result<Permission, SessionError> {
    let mut host_mac: Option<[u8; 32]> = None;
    let method;
    loop {
        let m = timeout(Duration::from_millis(PIN_TTL_MS + 10_000), read_message(rx)).await??;
        match m {
            Message::PairQrToken { token } => {
                let r = inner.qr.lock().unwrap().redeem(&token, now_ms());
                if let Err(e) = r {
                    send(tx, Message::Error { code: pair_error_code(&e) }).await?;
                    return Err(e.into());
                }
                method = "QR";
                break;
            }
            Message::PairPinRequest => {
                let pin = {
                    let mut pins = inner.pending_pins.lock().unwrap();
                    let now = now_ms();
                    let fresh = pins.get(&peer).is_none_or(|p| p.expires_at_ms() <= now);
                    if fresh {
                        pins.insert(peer, PinCode::generate(now));
                    }
                    pins[&peer].display().to_string()
                };
                inner.ui.show_pin(name, &pin);
                send(tx, Message::PairPinReady { ttl_s: (PIN_TTL_MS / 1000) as u16 }).await?;
            }
            Message::PairStart { spake_msg } => {
                let ex = {
                    let mut pins = inner.pending_pins.lock().unwrap();
                    let pin = pins.get_mut(&peer).ok_or(PairError::NotActive);
                    pin.and_then(|p| HostPinExchange::respond(p, now_ms(), &spake_msg, inner.host_fp, peer))
                };
                let (ex, reply) = match ex {
                    Ok(v) => v,
                    Err(e) => {
                        send(tx, Message::Error { code: pair_error_code(&e) }).await?;
                        return Err(e.into());
                    }
                };
                send(tx, Message::PairReply { spake_msg: reply }).await?;
                let Message::PairConfirm { mac } = timeout(Duration::from_secs(30), read_message(rx)).await?? else {
                    return Err(SessionError::Rejected("se esperaba PairConfirm"));
                };
                let v = {
                    let mut pins = inner.pending_pins.lock().unwrap();
                    match pins.get_mut(&peer) {
                        Some(p) => ex.verify(p, &mac),
                        None => Err(PairError::NotActive),
                    }
                };
                match v {
                    Ok(hm) => {
                        inner.pending_pins.lock().unwrap().remove(&peer);
                        inner.ui.hide_pin(name);
                        host_mac = Some(hm);
                        method = "PIN";
                        break;
                    }
                    Err(e) => {
                        send(tx, Message::Error { code: pair_error_code(&e) }).await?;
                        return Err(e.into());
                    }
                }
            }
            Message::Ping { t_us } => send(tx, Message::Pong { t_us }).await?,
            Message::Bye => return Err(SessionError::Rejected("el cliente canceló")),
            _ => return Err(SessionError::Rejected("mensaje fuera de orden durante el emparejamiento")),
        }
    }

    let req = ApprovalRequest { device_name: name.to_string(), kind, fingerprint_short: peer.short(), method };
    let decision = timeout(Duration::from_secs(120), inner.ui.approve(req)).await.ok().and_then(|r| r.ok()).flatten();
    let Some(permission) = decision else {
        send(tx, Message::PairResult { approved: false, permission: Permission::ViewOnly }).await?;
        return Err(SessionError::Rejected("el usuario del host no aprobó"));
    };
    if let Some(mac) = host_mac {
        send(tx, Message::PairConfirm { mac }).await?;
    }
    send(tx, Message::PairResult { approved: true, permission }).await?;
    let mut t = inner.trust.lock().unwrap();
    t.add(peer, TrustedDevice { name: name.to_string(), permission, paired_at_ms: now_ms(), last_seen_ms: now_ms() });
    inner.persist.save(&t);
    Ok(permission)
}

struct VideoHandle {
    stop: Arc<AtomicBool>,
    keyreq: Arc<AtomicBool>,
}

impl Drop for VideoHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn start_video(inner: &Arc<Inner>, conn: &quinn::Connection, monitor: &MonitorDesc, params: StreamParams, slot: u8) -> VideoHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let keyreq = Arc::new(AtomicBool::new(false));
    let mut rx = inner.hub.subscribe(&monitor.id);
    // Cola corta: si la red no da abasto se descarta y se pide keyframe.
    let (ftx, mut frx) = mpsc::channel::<(VideoChunkHeader, Vec<u8>)>(3);
    let c = conn.clone();
    tokio::spawn(async move {
        while let Some((h, d)) = frx.recv().await {
            if send_frame(&c, &h, &d).await.is_err() {
                break;
            }
        }
    });
    let rt = tokio::runtime::Handle::current();
    let (platform, ui, s2, k2, mid) = (inner.platform.clone(), inner.ui.clone(), stop.clone(), keyreq.clone(), monitor.id.clone());
    std::thread::Builder::new()
        .name(format!("kbss-encode-{slot}"))
        .spawn(move || {
            let mut enc = match platform.open_encoder(&params) {
                Ok(e) => e,
                Err(e) => {
                    log(&format!("ranura {slot}: no se pudo abrir el encoder: {e}"));
                    return;
                }
            };
            ui.event(HostEvent::Streaming {
                slot,
                monitor: mid,
                width: params.width,
                height: params.height,
                fps: params.fps,
                encoder: enc.name(),
            });
            let interval = Duration::from_micros(1_000_000 / params.fps.max(1) as u64);
            let mut last = Instant::now() - interval;
            let mut last_key = Instant::now();
            let mut need_key = true;
            let mut seq = 0u32;
            while !s2.load(Ordering::Relaxed) {
                let changed = rt.block_on(async { timeout(Duration::from_millis(500), rx.changed()).await });
                match changed {
                    Ok(Err(_)) => break,
                    Err(_) if !(need_key || k2.load(Ordering::Relaxed)) => continue,
                    _ => {}
                }
                let since = last.elapsed();
                if since < interval {
                    std::thread::sleep(interval - since);
                }
                let Some(frame) = rx.borrow_and_update().clone() else { continue };
                let force = need_key || k2.swap(false, Ordering::Relaxed) || last_key.elapsed() > Duration::from_secs(10);
                let nv12 = convert::bgra_to_nv12(&frame.data, frame.width, frame.height, frame.stride, params.width, params.height);
                match enc.encode(&nv12, frame.capture_us, force) {
                    Ok(out) => {
                        for e in out {
                            let h = VideoChunkHeader {
                                seq,
                                capture_us: frame.capture_us,
                                keyframe: e.keyframe,
                                width: params.width as u16,
                                height: params.height as u16,
                                len: e.data.len() as u32,
                            };
                            seq = seq.wrapping_add(1);
                            if e.keyframe {
                                need_key = false;
                                last_key = Instant::now();
                            }
                            if ftx.try_send((h, e.data)).is_err() {
                                // Frame descartado: el siguiente debe ser keyframe.
                                need_key = true;
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!("ranura {slot}: error de codificación: {e}"));
                        break;
                    }
                }
                last = Instant::now();
            }
        })
        .expect("hilo de codificación");
    VideoHandle { stop, keyreq }
}

fn default_profile(p: Permission) -> QualityProfile {
    // El cantante (solo lectura) quiere nitidez; el que controla, latencia.
    match p {
        Permission::Control => QualityProfile::Mixer,
        Permission::ViewOnly => QualityProfile::Lyrics,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    inner: &Arc<Inner>,
    conn: quinn::Connection,
    mut tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    peer: Fingerprint,
    slot: u8,
    gen: u64,
    name: String,
    mut screen: ScreenInfo,
    decoder: DecoderCaps,
) -> Result<(), SessionError> {
    let topo = inner.platform.topology();
    let summaries: Vec<MonitorSummary> = topo
        .monitors
        .iter()
        .map(|m| MonitorSummary { id: m.id.clone(), name: m.name.clone(), bounds: m.bounds, is_virtual: m.is_virtual, primary: m.primary })
        .collect();
    send(&mut tx, Message::MonitorList { monitors: summaries }).await?;
    let Some(mut monitor) = topo.primary().cloned() else {
        send(&mut tx, Message::Error { code: ErrorCode::Internal }).await?;
        return Err(SessionError::Rejected("no hay monitores"));
    };
    // Reconexión tras un corte: si este dispositivo tenía un monitor virtual, se retoma.
    let mut mode = Mode::Mirror { monitor_id: monitor.id.clone() };
    {
        let mut ext = inner.extended.lock().unwrap();
        if let Some((s0, m, g)) = ext.get_mut(&peer) {
            if *s0 == slot && topo.by_id(&m.id).is_some() {
                *g = gen;
                monitor = m.clone();
                mode = Mode::Extend { position: ExtendPosition::RightOf };
            }
        }
    }
    let permission = inner.trust.lock().unwrap().get(&peer).map(|d| d.permission).unwrap_or(Permission::ViewOnly);
    inner.ui.event(HostEvent::Connected { slot, name: name.clone(), permission });

    let profile = default_profile(permission);
    let mut params = crate::choose_params(monitor.bounds.w as u32, monitor.bounds.h as u32, &screen, &decoder, profile);
    let mut video = start_video(inner, &conn, &monitor, params.clone(), slot);
    let ticket = inner.tickets.issue(&peer, slot, now_ms());
    send(
        &mut tx,
        Message::SessionConfig {
            slot,
            mode: mode.clone(),
            permission,
            params: params.clone(),
            present_rotation: Rotation::R0,
            resume_ticket: ticket.clone(),
        },
    )
    .await?;

    let sink = inner.platform.input();
    let mut pressed = false;
    let result = loop {
        let msg = match read_message(&mut rx).await {
            Ok(m) => m,
            Err(e) => break Err(e.into()),
        };
        let auth = authorize_inbound(&msg, &inner.trust.lock().unwrap(), &peer);
        if let Err(code) = auth {
            if code == ErrorCode::NotPaired {
                let _ = send(&mut tx, Message::Error { code }).await;
                break Err(SessionError::Rejected("revocado"));
            }
            continue; // Solo lectura: la entrada se descarta en silencio.
        }
        match msg {
            Message::Input(ev) => {
                inject(&inner.arbiter, &*sink, slot, &monitor.bounds, ev, &mut pressed);
            }
            Message::RequestKeyframe => video.keyreq.store(true, Ordering::Relaxed),
            Message::ViewportChanged { screen: s } => {
                screen = s;
                let perm = inner.trust.lock().unwrap().get(&peer).map(|d| d.permission).unwrap_or(Permission::ViewOnly);
                params = crate::choose_params(monitor.bounds.w as u32, monitor.bounds.h as u32, &screen, &decoder, params.profile);
                drop(video);
                video = start_video(inner, &conn, &monitor, params.clone(), slot);
                send(
                    &mut tx,
                    Message::SessionConfig {
                        slot,
                        mode: mode.clone(),
                        permission: perm,
                        params: params.clone(),
                        present_rotation: Rotation::R0,
                        resume_ticket: ticket.clone(),
                    },
                )
                .await?;
            }
            Message::ModeChangeRequest { mode: requested } => {
                let can = inner.trust.lock().unwrap().may_control(&peer);
                let mut reply: Option<ErrorCode> = None;
                match requested {
                    _ if !can => reply = Some(ErrorCode::PermissionDenied),
                    Mode::Mirror { monitor_id } => match inner.platform.topology().by_id(&monitor_id).cloned() {
                        Some(m) => {
                            // Volver a duplicar: el monitor virtual propio se retira.
                            release_virtual(inner, &peer);
                            monitor = m;
                            mode = Mode::Mirror { monitor_id };
                        }
                        None => reply = Some(ErrorCode::ModeChangeRejected),
                    },
                    Mode::Extend { position } => {
                        if matches!(mode, Mode::Extend { .. }) {
                            // Ya extendido: nada que hacer.
                        } else if !timeout(Duration::from_secs(60), inner.ui.confirm(&name, "extender el escritorio con un monitor virtual"))
                            .await
                            .ok()
                            .and_then(|r| r.ok())
                            .unwrap_or(false)
                        {
                            reply = Some(ErrorCode::ModeChangeRejected);
                        } else {
                            // Monitor virtual con la resolución REAL del dispositivo (apaisado).
                            let (w, h) = (screen.width_px.max(screen.height_px) & !1, screen.width_px.min(screen.height_px) & !1);
                            let p2 = inner.platform.clone();
                            let r = tokio::task::spawn_blocking(move || p2.add_virtual_monitor(slot, w, h)).await;
                            match r {
                                Ok(Ok(m)) => {
                                    inner.extended.lock().unwrap().insert(peer, (slot, m.clone(), gen));
                                    monitor = m;
                                    mode = Mode::Extend { position };
                                }
                                Ok(Err(e)) => {
                                    log(&format!("ranura {slot}: no se pudo crear el monitor virtual: {e}"));
                                    reply = Some(ErrorCode::VirtualDisplayUnavailable);
                                }
                                Err(_) => reply = Some(ErrorCode::VirtualDisplayUnavailable),
                            }
                        }
                    }
                }
                if reply.is_none() {
                    params = crate::choose_params(monitor.bounds.w as u32, monitor.bounds.h as u32, &screen, &decoder, params.profile);
                    drop(video);
                    video = start_video(inner, &conn, &monitor, params.clone(), slot);
                }
                send(&mut tx, Message::ModeChangeResult { accepted: reply.is_none(), reason: reply }).await?;
                if reply.is_none() {
                    send(
                        &mut tx,
                        Message::SessionConfig {
                            slot,
                            mode: mode.clone(),
                            permission: if can { Permission::Control } else { Permission::ViewOnly },
                            params: params.clone(),
                            present_rotation: Rotation::R0,
                            resume_ticket: ticket.clone(),
                        },
                    )
                    .await?;
                }
            }
            Message::Ping { t_us } => send(&mut tx, Message::Pong { t_us }).await?,
            Message::Bye => break Ok(()),
            _ => {}
        }
    };
    // Nunca dejar el botón del mouse apretado si el cliente se cae a mitad de un arrastre.
    if pressed && inner.arbiter.end(slot) {
        sink.inject(Injected::Button { p: kb_geometry::PointI { x: 0, y: 0 }, button: MouseButton::Left, down: false });
    }
    drop(video);
    inner.ui.event(HostEvent::Disconnected { slot, name });
    // Monitor virtual: se conserva EXTEND_GRACE por si el dispositivo vuelve.
    if inner.extended.lock().unwrap().get(&peer).is_some_and(|(_, _, g)| *g == gen) {
        let inner2 = inner.clone();
        tokio::spawn(async move {
            tokio::time::sleep(inner2.cfg.extend_grace).await;
            let still_ours = inner2.extended.lock().unwrap().get(&peer).is_some_and(|(_, _, g)| *g == gen);
            if still_ours {
                release_virtual(&inner2, &peer);
            }
        });
    }
    result
}

/// Quita el monitor virtual de un dispositivo (si tiene) en un hilo aparte.
fn release_virtual(inner: &Arc<Inner>, peer: &Fingerprint) {
    if let Some((slot, _, _)) = inner.extended.lock().unwrap().remove(peer) {
        let p = inner.platform.clone();
        std::thread::spawn(move || p.remove_virtual_monitor(slot));
    }
}

/// Modo mouse: el primer dedo es el puntero; arrastre sostenido = botón apretado.
fn inject(arb: &InputArbiter, sink: &dyn crate::InputSink, slot: u8, bounds: &Rect, ev: InputEvent, pressed: &mut bool) {
    let map = |p| norm_to_desktop(p, bounds, Rotation::R0);
    match ev {
        InputEvent::Pointer(p) if p.pointer_id == 0 => {
            let pt = map(p.pos);
            match p.phase {
                PointerPhase::Down => {
                    if arb.begin(slot) {
                        sink.inject(Injected::Move { p: pt });
                        sink.inject(Injected::Button { p: pt, button: MouseButton::Left, down: true });
                        *pressed = true;
                    }
                }
                PointerPhase::Move => {
                    if arb.continue_(slot) {
                        sink.inject(Injected::Move { p: pt });
                    }
                }
                PointerPhase::Up => {
                    if arb.end(slot) {
                        sink.inject(Injected::Move { p: pt });
                        sink.inject(Injected::Button { p: pt, button: MouseButton::Left, down: false });
                        *pressed = false;
                    }
                }
                PointerPhase::Cancel => {
                    if arb.end(slot) {
                        sink.inject(Injected::Button { p: pt, button: MouseButton::Left, down: false });
                        *pressed = false;
                    }
                }
            }
        }
        InputEvent::Pointer(_) => {}
        InputEvent::Wheel { pos, dx, dy } => {
            if arb.idle_or_mine(slot) {
                sink.inject(Injected::Wheel { p: map(pos), dx, dy });
            }
        }
        InputEvent::MouseButton { pos, button, down } => {
            if arb.idle_or_mine(slot) {
                let pt = map(pos);
                sink.inject(Injected::Move { p: pt });
                sink.inject(Injected::Button { p: pt, button, down });
            }
        }
    }
}

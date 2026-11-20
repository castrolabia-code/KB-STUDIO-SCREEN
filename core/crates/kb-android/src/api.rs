//! API del cliente Android en Rust puro (probable en Linux). `lib.rs` la
//! expone por JNI.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use kb_geometry::{ClientViewport, LocalZoom, PointF, Rotation};
use kb_pairing::{Fingerprint, QrPayload};
use kb_proto::*;
use kb_session::client::{Auth, Client, ClientError, ClientInfo, Stage};
use kb_transport::Identity;
use serde_json::json;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime"))
}

pub struct ConnectArgs {
    pub host: String,
    pub port: u16,
    pub mode: i32,
    pub pinned_fp: String,
    pub qr_uri: String,
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
    pub device_name: String,
    pub is_tablet: bool,
    pub screen_w: u32,
    pub screen_h: u32,
    pub dpi: u32,
    pub refresh: u16,
    pub dec_max_w: u32,
    pub dec_max_h: u32,
}

/// Códigos estables que Kotlin traduce (ES/EN).
pub fn err_code(e: &ClientError) -> String {
    let code = match e {
        ClientError::Host(c) => match c {
            ErrorCode::SlotsFull => "slots_full",
            ErrorCode::PairingRejected => "pin_wrong",
            ErrorCode::PairingExpired => "pin_expired",
            ErrorCode::TooManyAttempts => "too_many",
            ErrorCode::NotPaired => "not_paired",
            ErrorCode::VersionMismatch => "version",
            ErrorCode::RateLimited => "rate_limited",
            _ => "host_error",
        },
        ClientError::Rejected => "rejected",
        ClientError::Mitm => "mitm",
        ClientError::Protocol => "protocol",
        ClientError::Pairing(_) => "pin_wrong",
        ClientError::Transport(t) => match t {
            kb_transport::TransportError::Connect(_) | kb_transport::TransportError::Connection(_) => "unreachable",
            kb_transport::TransportError::Proto(ProtoError::IncompatibleVersion { .. }) => "version",
            _ => "io",
        },
    };
    format!("{code}|{e}")
}

/// (id, nombre, tipo, bytes, recibidos)
type PartialDoc = (u32, String, DocKind, Vec<u8>, usize);

pub struct Session {
    client: Arc<Client>,
    stage: Mutex<Stage>,
    viewport: Mutex<ClientViewport>,
    permission: Mutex<Permission>,
    dragging: AtomicBool,
    out: mpsc::UnboundedSender<Message>,
    closed: Arc<AtomicBool>,
    /// Documento en recepción: (id, nombre, tipo, bytes, recibidos).
    doc_rx: Mutex<Option<PartialDoc>>,
    /// Documento completo, listo para que Kotlin lo tome (una sola vez).
    doc_ready: Mutex<Option<Vec<u8>>>,
}

fn stage_json(s: &Stage) -> String {
    match s {
        Stage::NeedPin => json!({"stage": "need_pin"}).to_string(),
        Stage::Ready(i) => session_json(i.slot, &i.mode, i.permission, &i.params, "ready"),
    }
}

fn session_json(slot: u8, mode: &Mode, permission: Permission, p: &StreamParams, kind: &str) -> String {
    let monitor = match mode {
        Mode::Mirror { monitor_id } => monitor_id.clone(),
        Mode::Extend { .. } => "extend".into(),
    };
    json!({
        "stage": kind, "type": "session", "slot": slot, "monitor": monitor,
        "permission": if permission == Permission::Control { "control" } else { "view" },
        "width": p.width, "height": p.height, "fps": p.fps, "kbps": p.bitrate_kbps,
        "profile": format!("{:?}", p.profile).to_lowercase(),
    })
    .to_string()
}

pub fn connect(a: ConnectArgs) -> Result<Session, String> {
    let id = Identity::from_der(a.cert, a.key);
    let (host, port, auth) = match a.mode {
        0 => {
            let fp: [u8; 32] = B64
                .decode(&a.pinned_fp)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| "protocol|huella guardada inválida".to_string())?;
            (a.host, a.port, Auth::Trusted(Fingerprint(fp)))
        }
        1 => {
            let q = QrPayload::parse(&a.qr_uri).map_err(|e| format!("qr_invalid|{e}"))?;
            (q.host.clone(), q.port, Auth::Qr(q))
        }
        _ => (a.host, a.port, Auth::Pin),
    };
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut i| i.next())
        .ok_or_else(|| "unreachable|dirección inválida".to_string())?;
    let (long, short) = (a.screen_w.max(a.screen_h), a.screen_w.min(a.screen_h));
    let info = ClientInfo {
        device_id: {
            let d = Fingerprint::of_der(&id.cert_der).0;
            d[..16].try_into().unwrap()
        },
        device_name: a.device_name.chars().filter(|c| !c.is_control()).take(MAX_NAME_LEN).collect::<String>(),
        kind: if a.is_tablet { DeviceKind::Tablet } else { DeviceKind::Phone },
        screen: ScreenInfo {
            width_px: long,
            height_px: short,
            density_dpi: a.dpi,
            orientation: Orientation::Landscape,
            refresh_hz: a.refresh.max(30),
        },
        decoder: DecoderCaps {
            h264_profiles: vec![H264Profile::Baseline, H264Profile::Main],
            max_width: a.dec_max_w.max(640),
            max_height: a.dec_max_h.max(480),
            max_fps_at_max_res: 30,
            hardware: true,
        },
    };
    let (client, stage) = rt()
        .block_on(async { tokio::time::timeout(Duration::from_secs(12), Client::connect(addr, &id, info, auth)).await })
        .map_err(|_| "unreachable|tiempo de espera agotado".to_string())?
        .map_err(|e| err_code(&e))?;
    let client = Arc::new(client);
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let c2 = client.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let cl2 = closed.clone();
    rt().spawn(async move {
        while let Some(m) = rx.recv().await {
            if c2.send(&m).await.is_err() {
                cl2.store(true, Ordering::Relaxed);
                break;
            }
        }
    });
    let permission = match &stage {
        Stage::Ready(i) => i.permission,
        Stage::NeedPin => Permission::ViewOnly,
    };
    Ok(Session {
        client,
        stage: Mutex::new(stage),
        viewport: Mutex::new(ClientViewport {
            view_w: 1.0,
            view_h: 1.0,
            frame_w: 1,
            frame_h: 1,
            present_rotation: Rotation::R0,
            zoom: LocalZoom::default(),
        }),
        permission: Mutex::new(permission),
        dragging: AtomicBool::new(false),
        out: tx,
        closed,
        doc_rx: Mutex::new(None),
        doc_ready: Mutex::new(None),
    })
}

impl Session {
    pub fn stage_json(&self) -> String {
        stage_json(&self.stage.lock().unwrap())
    }

    pub fn host_json(&self) -> String {
        json!({"name": self.client.host_name, "fp": B64.encode(self.client.host_fp.0), "short": self.client.host_fp.short()}).to_string()
    }

    pub fn request_pin(&self) -> Result<u16, String> {
        rt().block_on(self.client.request_pin()).map_err(|e| err_code(&e))
    }

    pub fn submit_pin(&self, pin: &str) -> Result<String, String> {
        let pin: String = pin.chars().filter(|c| c.is_ascii_digit()).collect();
        let info = rt()
            .block_on(async { tokio::time::timeout(Duration::from_secs(150), self.client.submit_pin(&pin)).await })
            .map_err(|_| "rejected|sin respuesta del host".to_string())?
            .map_err(|e| err_code(&e))?;
        *self.permission.lock().unwrap() = info.permission;
        let st = Stage::Ready(info);
        let j = stage_json(&st);
        *self.stage.lock().unwrap() = st;
        Ok(j)
    }

    pub fn next_frame(&self) -> Option<Vec<u8>> {
        let (h, d) = rt().block_on(self.client.next_frame()).ok()?;
        let mut v = Vec::with_capacity(VideoChunkHeader::SIZE + d.len());
        v.extend_from_slice(&h.to_bytes());
        v.extend_from_slice(&d);
        Some(v)
    }

    pub fn next_control(&self) -> String {
        match rt().block_on(self.client.recv_control()) {
            Ok(Message::SessionConfig { slot, mode, permission, params, .. }) => {
                *self.permission.lock().unwrap() = permission;
                session_json(slot, &mode, permission, &params, "ready")
            }
            Ok(Message::MonitorList { .. }) => json!({"type": "monitors"}).to_string(),
            Ok(Message::ModeChangeResult { accepted, reason }) => {
                json!({"type": "mode_result", "accepted": accepted, "reason": reason.map(|r| format!("{r:?}"))}).to_string()
            }
            Ok(Message::Error { code }) => json!({"type": "error", "code": format!("{code:?}")}).to_string(),
            Ok(Message::Pong { t_us }) => json!({"type": "pong", "t_us": t_us}).to_string(),
            Ok(Message::DocChunk { doc_id, name, kind, total, offset, data }) => self.doc_chunk(doc_id, name, kind, total, offset, data),
            Ok(Message::Prompter { action }) => {
                let j = match action {
                    PrompterAction::NextPage => json!({"action": "next"}),
                    PrompterAction::PrevPage => json!({"action": "prev"}),
                    PrompterAction::GoToPage(p) => json!({"action": "goto", "page": p}),
                    PrompterAction::AutoScroll(v) => json!({"action": "auto", "speed": v}),
                    PrompterAction::Lock(l) => json!({"action": "lock", "locked": l}),
                    PrompterAction::Close => json!({"action": "close"}),
                };
                let mut j = j;
                j["type"] = "prompter".into();
                j.to_string()
            }
            Ok(_) => json!({"type": "other"}).to_string(),
            Err(e) => json!({"type": "closed", "reason": err_code(&e)}).to_string(),
        }
    }

    fn doc_chunk(&self, id: u32, name: String, kind: DocKind, total: u32, offset: u32, data: Vec<u8>) -> String {
        let mut g = self.doc_rx.lock().unwrap();
        // Documento nuevo (o distinto): se descarta el anterior a medio recibir.
        if g.as_ref().is_none_or(|(i, ..)| *i != id) {
            *g = Some((id, name.clone(), kind, vec![0u8; total as usize], 0));
        }
        let (_, _, _, buf, got) = g.as_mut().unwrap();
        let (o, e) = (offset as usize, offset as usize + data.len());
        if e <= buf.len() {
            buf[o..e].copy_from_slice(&data);
            *got += data.len();
        }
        if *got >= buf.len() {
            let (_, name, kind, buf, _) = g.take().unwrap();
            let size = buf.len();
            *self.doc_ready.lock().unwrap() = Some(buf);
            let k = if kind == DocKind::Pdf { "pdf" } else { "text" };
            json!({"type": "doc_ready", "name": name, "kind": k, "size": size}).to_string()
        } else {
            json!({"type": "doc_progress", "pct": (*got * 100 / buf.len().max(1))}).to_string()
        }
    }

    /// Entrega el documento completo (y lo olvida: no queda copia en el núcleo).
    pub fn take_document(&self) -> Option<Vec<u8>> {
        self.doc_ready.lock().unwrap().take()
    }

    pub fn monitors_json(&self) -> String {
        let v: Vec<_> = self
            .client
            .monitors()
            .into_iter()
            .map(|m| json!({"id": m.id, "name": m.name, "w": m.bounds.w, "h": m.bounds.h, "primary": m.primary, "virtual": m.is_virtual}))
            .collect();
        serde_json::Value::Array(v).to_string()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_viewport(&self, vw: f32, vh: f32, fw: u32, fh: u32, zoom: f32, ox: f32, oy: f32) {
        let mut v = self.viewport.lock().unwrap();
        v.view_w = vw.max(1.0);
        v.view_h = vh.max(1.0);
        v.frame_w = fw.max(1);
        v.frame_h = fh.max(1);
        v.zoom = LocalZoom { scale: zoom.clamp(LocalZoom::MIN_SCALE, LocalZoom::MAX_SCALE), offset_x: ox, offset_y: oy };
    }

    pub fn send(&self, m: Message) {
        let _ = self.out.send(m);
    }

    /// Modo mouse: el primer dedo arrastra. Devuelve si el toque se envió.
    pub fn touch(&self, phase: i32, x: f32, y: f32) -> bool {
        if *self.permission.lock().unwrap() != Permission::Control || self.closed.load(Ordering::Relaxed) {
            return false;
        }
        let v = *self.viewport.lock().unwrap();
        let p = PointF { x, y };
        let (phase, pos) = match phase {
            0 => match v.view_to_norm(p) {
                // Toque en las barras negras: no se envía.
                None => return false,
                Some(n) => {
                    self.dragging.store(true, Ordering::Relaxed);
                    (PointerPhase::Down, n)
                }
            },
            1 if self.dragging.load(Ordering::Relaxed) => (PointerPhase::Move, v.view_to_norm_clamped(p)),
            2 | 3 if self.dragging.swap(false, Ordering::Relaxed) => {
                (if phase == 2 { PointerPhase::Up } else { PointerPhase::Cancel }, v.view_to_norm_clamped(p))
            }
            _ => return false,
        };
        self.send(Message::Input(InputEvent::Pointer(PointerEvent { pointer_id: 0, phase, pos, t_us: kb_session::mono_us() })));
        true
    }

    pub fn wheel(&self, x: f32, y: f32, dy: i16) {
        if *self.permission.lock().unwrap() != Permission::Control {
            return;
        }
        let v = *self.viewport.lock().unwrap();
        if let Some(pos) = v.view_to_norm(PointF { x, y }) {
            self.send(Message::Input(InputEvent::Wheel { pos, dx: 0, dy }));
        }
    }

    pub fn close(&self) {
        self.send(Message::Bye);
        let c = self.client.clone();
        rt().spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            c.close();
        });
    }
}

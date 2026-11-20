//! Cliente (Android vía JNI y pruebas). Todo es async; la capa JNI lo
//! ejecuta sobre un runtime tokio propio.

use kb_pairing::{ClientPinPairing, Fingerprint, QrPayload};
use kb_proto::*;
use kb_transport::{client_config, connect, peer_fingerprint, read_message, recv_frame, write_message, Identity, TransportError};
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("el host respondió con error: {0:?}")]
    Host(ErrorCode),
    #[error("el host rechazó el emparejamiento")]
    Rejected,
    #[error("la confirmación del host no coincide: posible intermediario en la red")]
    Mitm,
    #[error("respuesta inesperada del host")]
    Protocol,
    #[error("emparejamiento: {0}")]
    Pairing(#[from] kb_pairing::PairError),
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub device_id: [u8; 16],
    pub device_name: String,
    pub kind: DeviceKind,
    pub screen: ScreenInfo,
    pub decoder: DecoderCaps,
}

/// Cómo autenticar al host en esta conexión.
#[derive(Debug, Clone)]
pub enum Auth {
    /// Host ya emparejado: huella fijada.
    Trusted(Fingerprint),
    /// QR escaneado: huella fijada + token de un solo uso.
    Qr(QrPayload),
    /// Primer emparejamiento por PIN (la huella se autentica con SPAKE2).
    Pin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub slot: u8,
    pub mode: Mode,
    pub permission: Permission,
    pub params: StreamParams,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Ready(SessionInfo),
    /// El host necesita emparejar: llamar `request_pin` y luego `submit_pin`.
    NeedPin,
}

pub struct Client {
    _endpoint: quinn::Endpoint,
    pub conn: quinn::Connection,
    tx: Mutex<quinn::SendStream>,
    rx: Mutex<quinn::RecvStream>,
    pub host_fp: Fingerprint,
    pub host_name: String,
    my_fp: Fingerprint,
    monitors: StdMutex<Vec<MonitorSummary>>,
    last_seq: StdMutex<Option<u32>>,
}

impl Client {
    pub async fn connect(addr: SocketAddr, id: &Identity, info: ClientInfo, auth: Auth) -> Result<(Client, Stage), ClientError> {
        let bind: SocketAddr = if addr.is_ipv6() { "[::]:0".parse().unwrap() } else { "0.0.0.0:0".parse().unwrap() };
        let endpoint = quinn::Endpoint::client(bind).map_err(TransportError::from)?;
        let pinned = match &auth {
            Auth::Trusted(fp) => Some(*fp),
            Auth::Qr(q) => Some(q.host_fp),
            Auth::Pin => None,
        };
        let conn = connect(&endpoint, client_config(id, pinned)?, addr).await?;
        let host_fp = peer_fingerprint(&conn)?;
        let (mut tx, mut rx) = conn.open_bi().await.map_err(TransportError::from)?;
        write_message(
            &mut tx,
            &Message::ClientHello {
                device_id: info.device_id,
                device_name: info.device_name,
                kind: info.kind,
                screen: info.screen,
                decoder: info.decoder,
            },
        )
        .await?;
        let host_name = match read_message(&mut rx).await? {
            Message::HostHello { host_name, .. } => host_name,
            Message::Error { code } => return Err(ClientError::Host(code)),
            _ => return Err(ClientError::Protocol),
        };
        let qr = matches!(auth, Auth::Qr(_));
        if let Auth::Qr(q) = &auth {
            write_message(&mut tx, &Message::PairQrToken { token: q.token }).await?;
        }
        let c = Client {
            _endpoint: endpoint,
            conn,
            tx: Mutex::new(tx),
            rx: Mutex::new(rx),
            host_fp,
            host_name,
            my_fp: id.fingerprint(),
            monitors: StdMutex::new(Vec::new()),
            last_seq: StdMutex::new(None),
        };
        let stage = c.read_stage(qr).await?;
        Ok((c, stage))
    }

    async fn read_stage(&self, skip_not_paired: bool) -> Result<Stage, ClientError> {
        let mut rx = self.rx.lock().await;
        loop {
            match read_message(&mut *rx).await? {
                Message::Error { code: ErrorCode::NotPaired } if skip_not_paired => continue,
                Message::Error { code: ErrorCode::NotPaired } => return Ok(Stage::NeedPin),
                Message::Error { code } => return Err(ClientError::Host(code)),
                Message::PairResult { approved: false, .. } => return Err(ClientError::Rejected),
                Message::PairResult { approved: true, .. } => continue,
                Message::MonitorList { monitors } => *self.monitors.lock().unwrap() = monitors,
                Message::SessionConfig { slot, mode, permission, params, .. } => {
                    return Ok(Stage::Ready(SessionInfo { slot, mode, permission, params }));
                }
                _ => continue,
            }
        }
    }

    /// Pide al host que muestre un PIN. Devuelve su vigencia en segundos.
    pub async fn request_pin(&self) -> Result<u16, ClientError> {
        self.send(&Message::PairPinRequest).await?;
        let mut rx = self.rx.lock().await;
        loop {
            match read_message(&mut *rx).await? {
                Message::PairPinReady { ttl_s } => return Ok(ttl_s),
                Message::Error { code } => return Err(ClientError::Host(code)),
                _ => continue,
            }
        }
    }

    /// Envía el PIN tipeado. Si es correcto y el usuario del host aprueba,
    /// devuelve la sesión. Ante PIN incorrecto el host cierra la conexión:
    /// hay que reconectar con `Auth::Pin` (el mismo PIN sigue vigente).
    pub async fn submit_pin(&self, pin: &str) -> Result<SessionInfo, ClientError> {
        let (p, msg) = ClientPinPairing::start(pin, self.host_fp, self.my_fp);
        self.send(&Message::PairStart { spake_msg: msg }).await?;
        let reply = { read_message(&mut *self.rx.lock().await).await? };
        let spake = match reply {
            Message::PairReply { spake_msg } => spake_msg,
            Message::Error { code } => return Err(ClientError::Host(code)),
            _ => return Err(ClientError::Protocol),
        };
        let out = p.finish(&spake)?;
        self.send(&Message::PairConfirm { mac: out.client_confirm }).await?;
        loop {
            let m = { read_message(&mut *self.rx.lock().await).await? };
            match m {
                Message::PairConfirm { mac } => {
                    out.verify_host_confirm(&mac).map_err(|_| ClientError::Mitm)?;
                    break;
                }
                Message::PairResult { approved: false, .. } => return Err(ClientError::Rejected),
                Message::Error { code } => return Err(ClientError::Host(code)),
                _ => continue,
            }
        }
        match self.read_stage(false).await? {
            Stage::Ready(s) => Ok(s),
            Stage::NeedPin => Err(ClientError::Protocol),
        }
    }

    pub fn monitors(&self) -> Vec<MonitorSummary> {
        self.monitors.lock().unwrap().clone()
    }

    pub async fn send(&self, m: &Message) -> Result<(), ClientError> {
        Ok(write_message(&mut *self.tx.lock().await, m).await?)
    }

    pub async fn recv_control(&self) -> Result<Message, ClientError> {
        let m = read_message(&mut *self.rx.lock().await).await?;
        if let Message::MonitorList { monitors } = &m {
            *self.monitors.lock().unwrap() = monitors.clone();
        }
        Ok(m)
    }

    /// Siguiente frame de video. Descarta frames atrasados (seq menor).
    pub async fn next_frame(&self) -> Result<(VideoChunkHeader, Vec<u8>), ClientError> {
        loop {
            let (h, d) = recv_frame(&self.conn).await?;
            let mut last = self.last_seq.lock().unwrap();
            let stale = last.is_some_and(|l| (h.seq.wrapping_sub(l) as i32) <= 0);
            if stale {
                continue;
            }
            *last = Some(h.seq);
            return Ok((h, d));
        }
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"bye");
    }
}

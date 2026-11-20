//! KB STUDIO SCREEN — protocolo de control.
//!
//! * Codificación: `postcard` (serde, binario compacto, MIT/Apache-2.0).
//! * Framing: `u32` big-endian de longitud + `Envelope`. Límite duro de
//!   tamaño por mensaje de control (`MAX_CONTROL_FRAME`) para que un peer
//!   malicioso no pueda forzar reservas de memoria grandes.
//! * Versionado: `PROTO_MAJOR` incompatible => se rechaza la sesión con un
//!   error explícito. `PROTO_MINOR` mayor del peer => se ignoran campos
//!   desconocidos sólo en mensajes marcados como extensibles (ver `Hello`).
//! * El video NO viaja por este canal: viaja por streams QUIC
//!   unidireccionales dedicados (ver `kb-transport`), con `VideoChunkHeader`.
//! * No existe ningún mensaje de audio. Es deliberado.

use kb_geometry::{NormPoint, Rect, Rotation};
use serde::{Deserialize, Serialize};

pub const PROTO_MAJOR: u16 = 1;
pub const PROTO_MINOR: u16 = 2;
pub const ALPN: &[u8] = b"kbss/1";

/// Tamaño máximo de un mensaje de control serializado.
pub const MAX_CONTROL_FRAME: usize = 16 * 1024;
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_POINTERS: u8 = 10;
pub const MAX_SLOTS: u8 = 3;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProtoError {
    #[error("mensaje demasiado grande: {0} bytes")]
    TooLarge(usize),
    #[error("mensaje truncado")]
    Truncated,
    #[error("error de decodificación")]
    Decode,
    #[error("versión de protocolo incompatible: peer {peer_major}.{peer_minor}, local {PROTO_MAJOR}.{PROTO_MINOR}")]
    IncompatibleVersion { peer_major: u16, peer_minor: u16 },
    #[error("campo inválido: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    Phone,
    Tablet,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    Landscape,
    Portrait,
}

/// Pantalla real del dispositivo (negociada, nunca asumida).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub width_px: u32,
    pub height_px: u32,
    pub density_dpi: u32,
    pub orientation: Orientation,
    pub refresh_hz: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum H264Profile {
    Baseline,
    Main,
    High,
}

/// Capacidades del decodificador Android (MediaCodecInfo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecoderCaps {
    pub h264_profiles: Vec<H264Profile>,
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps_at_max_res: u16,
    pub hardware: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    ViewOnly,
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityProfile {
    /// Prioriza latencia (mezcla en el DAW).
    Mixer,
    /// Prioriza nitidez con pocos FPS (letras/PDF).
    Lyrics,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Duplica un monitor existente (físico o virtual).
    Mirror { monitor_id: String },
    /// Monitor virtual IDD asignado a esta ranura.
    Extend { position: ExtendPosition },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtendPosition {
    LeftOf,
    RightOf,
    Above,
    Below,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamParams {
    pub width: u32,
    pub height: u32,
    pub fps: u16,
    pub bitrate_kbps: u32,
    pub profile: QualityProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerEvent {
    /// Id estable mientras el dedo está apoyado (0..MAX_POINTERS).
    pub pointer_id: u8,
    pub phase: PointerPhase,
    pub pos: NormPoint,
    /// Reloj monotónico del cliente, microsegundos.
    pub t_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    /// Tacto (se inyecta como puntero táctil o, en modo ratón, como mouse).
    Pointer(PointerEvent),
    /// Rueda: unidades de 1/120 de "notch" (WHEEL_DELTA).
    Wheel { pos: NormPoint, dx: i16, dy: i16 },
    /// Modo ratón de respaldo explícito.
    MouseButton { pos: NormPoint, button: MouseButton, down: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    VersionMismatch,
    NotPaired,
    PairingRejected,
    PairingExpired,
    TooManyAttempts,
    PermissionDenied,
    SlotsFull,
    ModeChangeRejected,
    VirtualDisplayUnavailable,
    RateLimited,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    // --- handshake ---
    ClientHello {
        device_id: [u8; 16],
        device_name: String,
        kind: DeviceKind,
        screen: ScreenInfo,
        decoder: DecoderCaps,
    },
    HostHello {
        host_id: [u8; 16],
        host_name: String,
    },
    // --- emparejamiento por PIN (SPAKE2, ver kb-pairing) ---
    /// Cliente -> host: "mostrá un PIN en la PC". El host lo muestra y responde `PairPinReady`.
    PairPinRequest,
    PairPinReady { ttl_s: u16 },
    PairStart { spake_msg: Vec<u8> },
    PairReply { spake_msg: Vec<u8> },
    PairConfirm { mac: [u8; 32] },
    /// Enviado por el host cuando el usuario aprobó/rechazó en pantalla.
    PairResult { approved: bool, permission: Permission },
    /// Emparejamiento por QR: token de un solo uso leído del QR.
    PairQrToken { token: [u8; 16] },

    // --- sesión ---
    /// Host -> cliente: configuración efectiva de la ranura.
    SessionConfig {
        slot: u8,
        mode: Mode,
        permission: Permission,
        params: StreamParams,
        /// Rotación que el cliente debe aplicar al presentar.
        present_rotation: Rotation,
        /// Ficha de reanudación (HMAC, corta duración) para reconectar tras
        /// una caída de Wi-Fi sin repetir el emparejamiento.
        resume_ticket: Vec<u8>,
    },
    /// Cliente -> host: pide cambiar de modo. El host puede pedir
    /// confirmación al usuario y SIEMPRE decide.
    ModeChangeRequest { mode: Mode },
    ModeChangeResult { accepted: bool, reason: Option<ErrorCode> },
    /// Cliente -> host: cambio de orientación/tamaño de la vista.
    ViewportChanged { screen: ScreenInfo },
    RequestKeyframe,
    Input(InputEvent),
    /// Host -> cliente: lista de monitores para elegir origen (sólo nombre y tamaño).
    MonitorList { monitors: Vec<MonitorSummary> },
    Identify { show: bool },
    Ping { t_us: u64 },
    Pong { t_us: u64 },
    Stats(ClientStats),
    Error { code: ErrorCode },
    Bye,
    /// Cliente -> host: cambiar el perfil de calidad (Mezcla / Letras / Automático).
    /// Agregado al final del enum: compatible con clientes 1.0 que no lo envían.
    SetProfile { profile: QualityProfile },
    /// Host -> cliente: fragmento de un documento para el visor del cantante.
    /// Viaja cifrado por el canal de control; nunca pasa por la nube.
    DocChunk { doc_id: u32, name: String, kind: DocKind, total: u32, offset: u32, data: Vec<u8> },
    /// Host -> cliente: control del teleprompter.
    Prompter { action: PrompterAction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocKind {
    Pdf,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrompterAction {
    NextPage,
    PrevPage,
    GoToPage(u16),
    /// Velocidad de avance automático, 0 = detenido, 1..=10.
    AutoScroll(u8),
    /// true = el cantante no puede tocar los controles.
    Lock(bool),
    Close,
}

/// Tamaño máximo de un fragmento de documento (cabe en MAX_CONTROL_FRAME).
pub const DOC_CHUNK: usize = 12 * 1024;
/// Tamaño máximo de un documento.
pub const MAX_DOC: u32 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorSummary {
    pub id: String,
    pub name: String,
    pub bounds: Rect,
    pub is_virtual: bool,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStats {
    pub frames_decoded: u32,
    pub frames_dropped: u32,
    pub decode_ms_p50: u16,
    pub decode_ms_p95: u16,
}

/// Cabecera de cada frame de video (precede a los NAL units Annex-B en el
/// stream unidireccional dedicado).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoChunkHeader {
    pub seq: u32,
    /// Tiempo de captura en el reloj del host, µs.
    pub capture_us: u64,
    pub keyframe: bool,
    pub width: u16,
    pub height: u16,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub major: u16,
    pub minor: u16,
    pub msg: Message,
}

impl Message {
    /// Validación semántica tras decodificar. Todo mensaje recibido de la
    /// red pasa por aquí antes de llegar a la lógica.
    pub fn validate(&self) -> Result<(), ProtoError> {
        use Message::*;
        match self {
            ClientHello { device_name, screen, decoder, .. } => {
                check_name(device_name)?;
                if screen.width_px == 0
                    || screen.height_px == 0
                    || screen.width_px > 16384
                    || screen.height_px > 16384
                {
                    return Err(ProtoError::Invalid("screen"));
                }
                if decoder.h264_profiles.len() > 8 {
                    return Err(ProtoError::Invalid("decoder"));
                }
            }
            HostHello { host_name, .. } => check_name(host_name)?,
            PairStart { spake_msg } | PairReply { spake_msg } => {
                if spake_msg.len() != 33 {
                    return Err(ProtoError::Invalid("spake_msg"));
                }
            }
            SessionConfig { slot, params, resume_ticket, .. } => {
                if *slot >= MAX_SLOTS {
                    return Err(ProtoError::Invalid("slot"));
                }
                check_params(params)?;
                if resume_ticket.len() > 256 {
                    return Err(ProtoError::Invalid("resume_ticket"));
                }
            }
            ModeChangeRequest { mode } => check_mode(mode)?,
            DocChunk { name, total, offset, data, .. } => {
                check_name(name)?;
                if data.len() > DOC_CHUNK || *total == 0 || *total > MAX_DOC || offset.checked_add(data.len() as u32).is_none_or(|e| e > *total) {
                    return Err(ProtoError::Invalid("doc_chunk"));
                }
            }
            Prompter { action: PrompterAction::AutoScroll(v) } if *v > 10 => return Err(ProtoError::Invalid("speed")),
            Input(InputEvent::Pointer(p)) if p.pointer_id >= MAX_POINTERS => {
                return Err(ProtoError::Invalid("pointer_id"));
            }
            MonitorList { monitors } => {
                if monitors.len() > 16 {
                    return Err(ProtoError::Invalid("monitors"));
                }
                for m in monitors {
                    check_name(&m.name)?;
                    check_id(&m.id)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// ¿Este mensaje intenta actuar sobre Windows? (para aplicar permisos).
    pub fn is_input(&self) -> bool {
        matches!(self, Message::Input(_))
    }
}

fn check_name(s: &str) -> Result<(), ProtoError> {
    if s.is_empty() || s.len() > MAX_NAME_LEN || s.chars().any(|c| c.is_control()) {
        Err(ProtoError::Invalid("name"))
    } else {
        Ok(())
    }
}

fn check_id(s: &str) -> Result<(), ProtoError> {
    if s.is_empty() || s.len() > 256 || s.chars().any(|c| c.is_control()) {
        Err(ProtoError::Invalid("id"))
    } else {
        Ok(())
    }
}

fn check_mode(m: &Mode) -> Result<(), ProtoError> {
    match m {
        Mode::Mirror { monitor_id } => check_id(monitor_id),
        Mode::Extend { .. } => Ok(()),
    }
}

fn check_params(p: &StreamParams) -> Result<(), ProtoError> {
    if p.width < 160 || p.height < 120 || p.width > 7680 || p.height > 4320 {
        return Err(ProtoError::Invalid("resolution"));
    }
    if !p.width.is_multiple_of(2) || !p.height.is_multiple_of(2) {
        return Err(ProtoError::Invalid("resolution_odd"));
    }
    if !(1..=120).contains(&p.fps) {
        return Err(ProtoError::Invalid("fps"));
    }
    if !(100..=200_000).contains(&p.bitrate_kbps) {
        return Err(ProtoError::Invalid("bitrate"));
    }
    Ok(())
}

/// Serializa con prefijo de longitud.
pub fn encode(msg: &Message) -> Result<Vec<u8>, ProtoError> {
    let env = Envelope { major: PROTO_MAJOR, minor: PROTO_MINOR, msg: msg.clone() };
    let body = postcard::to_allocvec(&env).map_err(|_| ProtoError::Decode)?;
    if body.len() > MAX_CONTROL_FRAME {
        return Err(ProtoError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Lee la longitud declarada y la valida ANTES de reservar memoria.
pub fn frame_len(prefix: [u8; 4]) -> Result<usize, ProtoError> {
    let n = u32::from_be_bytes(prefix) as usize;
    if n > MAX_CONTROL_FRAME {
        Err(ProtoError::TooLarge(n))
    } else if n == 0 {
        Err(ProtoError::Truncated)
    } else {
        Ok(n)
    }
}

/// Decodifica un cuerpo (sin prefijo), verifica versión y valida.
pub fn decode_body(body: &[u8]) -> Result<Message, ProtoError> {
    if body.len() > MAX_CONTROL_FRAME {
        return Err(ProtoError::TooLarge(body.len()));
    }
    // La versión va primero: se lee aunque el resto no sea decodificable,
    // para dar un error de versión útil en vez de "basura".
    let (major, rest): (u16, &[u8]) = postcard::take_from_bytes(body).map_err(|_| ProtoError::Decode)?;
    let (minor, _): (u16, &[u8]) = postcard::take_from_bytes(rest).map_err(|_| ProtoError::Decode)?;
    if major != PROTO_MAJOR {
        return Err(ProtoError::IncompatibleVersion { peer_major: major, peer_minor: minor });
    }
    let env: Envelope = postcard::from_bytes(body).map_err(|_| ProtoError::Decode)?;
    env.msg.validate()?;
    Ok(env.msg)
}

/// Decodifica un buffer con prefijo completo (para tests y transporte en memoria).
pub fn decode(buf: &[u8]) -> Result<Message, ProtoError> {
    if buf.len() < 4 {
        return Err(ProtoError::Truncated);
    }
    let n = frame_len(buf[..4].try_into().unwrap())?;
    let body = buf.get(4..4 + n).ok_or(ProtoError::Truncated)?;
    decode_body(body)
}

/// Encoder de cabecera de video (tamaño fijo para lectura sencilla en Android).
impl VideoChunkHeader {
    pub const SIZE: usize = 4 + 8 + 1 + 2 + 2 + 4;
    pub const MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.seq.to_be_bytes());
        b[4..12].copy_from_slice(&self.capture_us.to_be_bytes());
        b[12] = self.keyframe as u8;
        b[13..15].copy_from_slice(&self.width.to_be_bytes());
        b[15..17].copy_from_slice(&self.height.to_be_bytes());
        b[17..21].copy_from_slice(&self.len.to_be_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Result<Self, ProtoError> {
        let h = Self {
            seq: u32::from_be_bytes(b[0..4].try_into().unwrap()),
            capture_us: u64::from_be_bytes(b[4..12].try_into().unwrap()),
            keyframe: match b[12] {
                0 => false,
                1 => true,
                _ => return Err(ProtoError::Invalid("keyframe")),
            },
            width: u16::from_be_bytes(b[13..15].try_into().unwrap()),
            height: u16::from_be_bytes(b[15..17].try_into().unwrap()),
            len: u32::from_be_bytes(b[17..21].try_into().unwrap()),
        };
        if h.len == 0 || h.len > Self::MAX_FRAME_LEN {
            return Err(ProtoError::Invalid("len"));
        }
        Ok(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Message {
        Message::ClientHello {
            device_id: [7; 16],
            device_name: "Tablet cantante".into(),
            kind: DeviceKind::Tablet,
            screen: ScreenInfo {
                width_px: 1920,
                height_px: 1200,
                density_dpi: 240,
                orientation: Orientation::Landscape,
                refresh_hz: 60,
            },
            decoder: DecoderCaps {
                h264_profiles: vec![H264Profile::Baseline, H264Profile::High],
                max_width: 3840,
                max_height: 2160,
                max_fps_at_max_res: 30,
                hardware: true,
            },
        }
    }

    #[test]
    fn roundtrip_all_main_messages() {
        let msgs = vec![
            hello(),
            Message::Input(InputEvent::Pointer(PointerEvent {
                pointer_id: 0,
                phase: PointerPhase::Move,
                pos: NormPoint { x: 100, y: 65535 },
                t_us: 123,
            })),
            Message::ModeChangeRequest { mode: Mode::Extend { position: ExtendPosition::RightOf } },
            Message::SessionConfig {
                slot: 2,
                mode: Mode::Mirror { monitor_id: r"\\?\DISPLAY#ABC".into() },
                permission: Permission::ViewOnly,
                params: StreamParams {
                    width: 1280,
                    height: 800,
                    fps: 15,
                    bitrate_kbps: 4000,
                    profile: QualityProfile::Lyrics,
                },
                present_rotation: Rotation::R0,
                resume_ticket: vec![1; 64],
            },
            Message::Bye,
        ];
        for m in msgs {
            let b = encode(&m).unwrap();
            assert_eq!(decode(&b).unwrap(), m);
        }
    }

    #[test]
    fn rejects_oversized_length_before_allocating() {
        let mut b = encode(&Message::Bye).unwrap();
        b[..4].copy_from_slice(&(10_000_000u32).to_be_bytes());
        assert_eq!(decode(&b), Err(ProtoError::TooLarge(10_000_000)));
    }

    #[test]
    fn rejects_major_version_mismatch() {
        let env = Envelope { major: 2, minor: 0, msg: Message::Bye };
        let body = postcard::to_allocvec(&env).unwrap();
        assert_eq!(
            decode_body(&body),
            Err(ProtoError::IncompatibleVersion { peer_major: 2, peer_minor: 0 })
        );
    }

    #[test]
    fn rejects_invalid_fields() {
        let bad_name = Message::HostHello { host_id: [0; 16], host_name: "a\u{0007}b".into() };
        assert!(decode(&encode(&bad_name).unwrap()).is_err());
        let bad_ptr = Message::Input(InputEvent::Pointer(PointerEvent {
            pointer_id: 200,
            phase: PointerPhase::Down,
            pos: NormPoint { x: 0, y: 0 },
            t_us: 0,
        }));
        assert_eq!(decode(&encode(&bad_ptr).unwrap()), Err(ProtoError::Invalid("pointer_id")));
    }

    #[test]
    fn garbage_never_panics() {
        // Fuzz barato y determinista: ningún buffer debe provocar pánico.
        let mut x: u64 = 0x9E3779B97F4A7C15;
        for len in 0..300usize {
            let mut v = Vec::with_capacity(len);
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.push(x as u8);
            }
            let _ = decode(&v);
            let _ = decode_body(&v);
        }
    }

    #[test]
    fn video_header_roundtrip_and_limits() {
        let h = VideoChunkHeader { seq: 9, capture_us: 42, keyframe: true, width: 1920, height: 1080, len: 5000 };
        assert_eq!(VideoChunkHeader::from_bytes(&h.to_bytes()).unwrap(), h);
        let mut b = h.to_bytes();
        b[17..21].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(VideoChunkHeader::from_bytes(&b).is_err());
    }
}

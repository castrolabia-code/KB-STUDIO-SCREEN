//! KB STUDIO SCREEN — emparejamiento y autorización.
//!
//! Modelo de amenazas (LAN): un atacante en la misma Wi-Fi puede ver y
//! manipular tráfico, conectarse al host e intentar adivinar el PIN.
//!
//! Diseño:
//! 1. Cada extremo tiene un par de claves y un certificado autofirmado
//!    persistente. La conexión es QUIC/TLS 1.3 con autenticación MUTUA.
//! 2. Durante el emparejamiento todavía no se conoce el certificado del otro
//!    extremo, así que se acepta cualquiera y se autentica después:
//!    * **QR**: el QR (mostrado por el host) contiene la huella SHA-256 del
//!      certificado del host + token aleatorio de 128 bits de un solo uso.
//!      El cliente fija (pin) la huella antes del handshake.
//!    * **PIN de 6 dígitos**: SPAKE2 (PAKE) derivando una clave compartida
//!      del PIN, y confirmación HMAC que incluye las huellas de AMBOS
//!      certificados (channel binding). Un MITM tendría certificados
//!      distintos => la confirmación falla. Un atacante sólo obtiene UN
//!      intento en línea por sesión SPAKE2; no hay ataque offline al PIN.
//! 3. Además el usuario del host APRUEBA explícitamente el dispositivo
//!    (nombre visible) y elige permiso: solo ver / control.
//! 4. Reconexión: mTLS con huellas fijadas en ambos lados + ficha de
//!    reanudación HMAC de corta duración para recuperar la ranura.
//! 5. Revocación: borrar la huella de la lista de confianza cierra la sesión
//!    y rechaza reconexiones.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use kb_proto::{Permission, QualityProfile};
use rand::{rngs::OsRng, Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::collections::HashMap;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const PIN_TTL_MS: u64 = 120_000;
pub const QR_TTL_MS: u64 = 300_000;
pub const PIN_MAX_ATTEMPTS: u8 = 3;
pub const RESUME_TTL_MS: u64 = 10 * 60_000;

const ID_HOST: &[u8] = b"kbss-host";
const ID_CLIENT: &[u8] = b"kbss-client";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairError {
    #[error("código vencido")]
    Expired,
    #[error("código ya utilizado o inexistente")]
    NotActive,
    #[error("demasiados intentos")]
    TooManyAttempts,
    #[error("confirmación inválida (PIN incorrecto o intermediario)")]
    BadConfirmation,
    #[error("mensaje SPAKE2 inválido")]
    BadSpake,
    #[error("QR inválido: {0}")]
    BadQr(&'static str),
    #[error("ficha inválida")]
    BadTicket,
}

/// Huella SHA-256 de un certificado DER.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of_der(cert_der: &[u8]) -> Self {
        Self(Sha256::digest(cert_der).into())
    }
    /// Forma corta legible para mostrar en ambas pantallas (verificación visual opcional).
    pub fn short(&self) -> String {
        self.0[..4].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("-")
    }
}

// ---------------------------------------------------------------- PIN

/// PIN de un solo uso generado por el host.
#[derive(Debug)]
pub struct PinCode {
    digits: String,
    expires_at_ms: u64,
    attempts: u8,
    used: bool,
}

impl PinCode {
    pub fn generate(now_ms: u64) -> Self {
        let n: u32 = OsRng.gen_range(0..1_000_000);
        Self { digits: format!("{n:06}"), expires_at_ms: now_ms + PIN_TTL_MS, attempts: 0, used: false }
    }
    /// Sólo para mostrar en la UI del host. Nunca loguear.
    pub fn display(&self) -> &str {
        &self.digits
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    /// Reserva un intento; falla si venció, se usó o se agotaron los intentos.
    fn take_attempt(&mut self, now_ms: u64) -> Result<(), PairError> {
        if self.used {
            return Err(PairError::NotActive);
        }
        if now_ms >= self.expires_at_ms {
            return Err(PairError::Expired);
        }
        if self.attempts >= PIN_MAX_ATTEMPTS {
            return Err(PairError::TooManyAttempts);
        }
        self.attempts += 1;
        Ok(())
    }
}

fn transcript_mac(
    key: &[u8],
    role: &[u8],
    host_fp: &Fingerprint,
    client_fp: &Fingerprint,
    msg_client: &[u8],
    msg_host: &[u8],
) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac acepta cualquier clave");
    m.update(b"kbss-confirm-v1");
    m.update(role);
    m.update(&host_fp.0);
    m.update(&client_fp.0);
    m.update(&(msg_client.len() as u32).to_be_bytes());
    m.update(msg_client);
    m.update(&(msg_host.len() as u32).to_be_bytes());
    m.update(msg_host);
    m.finalize().into_bytes().into()
}

/// Lado cliente (Android) del emparejamiento por PIN.
pub struct ClientPinPairing {
    state: Option<Spake2<Ed25519Group>>,
    msg_client: Vec<u8>,
    host_fp: Fingerprint,
    client_fp: Fingerprint,
}

/// Resultado del lado cliente tras recibir la respuesta del host.
pub struct ClientPinOutcome {
    /// MAC a enviar al host (`PairConfirm`).
    pub client_confirm: [u8; 32],
    /// MAC que el host debe devolver; se verifica con `verify_host_confirm`.
    expected_host_confirm: [u8; 32],
}

impl ClientPinOutcome {
    pub fn verify_host_confirm(&self, mac: &[u8; 32]) -> Result<(), PairError> {
        if bool::from(self.expected_host_confirm.ct_eq(mac)) {
            Ok(())
        } else {
            Err(PairError::BadConfirmation)
        }
    }
}

impl ClientPinPairing {
    /// `host_fp`/`client_fp`: huellas de los certificados de ESTA conexión TLS.
    pub fn start(pin: &str, host_fp: Fingerprint, client_fp: Fingerprint) -> (Self, Vec<u8>) {
        let (state, msg) =
            Spake2::<Ed25519Group>::start_a(&Password::new(pin.as_bytes()), &Identity::new(ID_CLIENT), &Identity::new(ID_HOST));
        (Self { state: Some(state), msg_client: msg.clone(), host_fp, client_fp }, msg)
    }

    pub fn finish(mut self, msg_host: &[u8]) -> Result<ClientPinOutcome, PairError> {
        let key = self.state.take().unwrap().finish(msg_host).map_err(|_| PairError::BadSpake)?;
        let client_confirm =
            transcript_mac(&key, b"client", &self.host_fp, &self.client_fp, &self.msg_client, msg_host);
        let expected_host_confirm =
            transcript_mac(&key, b"host", &self.host_fp, &self.client_fp, &self.msg_client, msg_host);
        Ok(ClientPinOutcome { client_confirm, expected_host_confirm })
    }
}

/// Lado host: responde a un `PairStart`.
pub struct HostPinExchange {
    expected_client_confirm: [u8; 32],
    host_confirm: [u8; 32],
}

impl HostPinExchange {
    /// Consume un intento del PIN y devuelve el mensaje SPAKE2 del host.
    pub fn respond(
        pin: &mut PinCode,
        now_ms: u64,
        msg_client: &[u8],
        host_fp: Fingerprint,
        client_fp: Fingerprint,
    ) -> Result<(Self, Vec<u8>), PairError> {
        pin.take_attempt(now_ms)?;
        let (state, msg_host) = Spake2::<Ed25519Group>::start_b(
            &Password::new(pin.digits.as_bytes()),
            &Identity::new(ID_CLIENT),
            &Identity::new(ID_HOST),
        );
        let key = state.finish(msg_client).map_err(|_| PairError::BadSpake)?;
        Ok((
            Self {
                expected_client_confirm: transcript_mac(&key, b"client", &host_fp, &client_fp, msg_client, &msg_host),
                host_confirm: transcript_mac(&key, b"host", &host_fp, &client_fp, msg_client, &msg_host),
            },
            msg_host,
        ))
    }

    /// Verifica la confirmación del cliente. Si es correcta, el PIN queda
    /// consumido (un solo uso) y se devuelve la MAC del host para enviarla
    /// DESPUÉS de que el usuario apruebe en pantalla.
    pub fn verify(&self, pin: &mut PinCode, client_mac: &[u8; 32]) -> Result<[u8; 32], PairError> {
        if bool::from(self.expected_client_confirm.ct_eq(client_mac)) {
            pin.used = true;
            Ok(self.host_confirm)
        } else {
            Err(PairError::BadConfirmation)
        }
    }
}

// ---------------------------------------------------------------- QR

/// Contenido del QR mostrado por el host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPayload {
    pub host: String,
    pub port: u16,
    pub host_fp: Fingerprint,
    pub token: [u8; 16],
    pub expires_at_ms: u64,
}

impl QrPayload {
    pub fn new(host: String, port: u16, host_fp: Fingerprint, now_ms: u64) -> Self {
        let mut token = [0u8; 16];
        OsRng.fill_bytes(&mut token);
        Self { host, port, host_fp, token, expires_at_ms: now_ms + QR_TTL_MS }
    }

    pub fn to_uri(&self) -> String {
        format!(
            "kbss://pair?v=1&h={}&p={}&fp={}&t={}&e={}",
            self.host,
            self.port,
            B64.encode(self.host_fp.0),
            B64.encode(self.token),
            self.expires_at_ms
        )
    }

    pub fn parse(uri: &str) -> Result<Self, PairError> {
        let q = uri.strip_prefix("kbss://pair?").ok_or(PairError::BadQr("esquema"))?;
        if q.len() > 512 {
            return Err(PairError::BadQr("largo"));
        }
        let mut kv: HashMap<&str, &str> = HashMap::new();
        for part in q.split('&') {
            let (k, v) = part.split_once('=').ok_or(PairError::BadQr("formato"))?;
            if kv.insert(k, v).is_some() {
                return Err(PairError::BadQr("clave duplicada"));
            }
        }
        if kv.get("v") != Some(&"1") {
            return Err(PairError::BadQr("versión"));
        }
        let host = *kv.get("h").ok_or(PairError::BadQr("host"))?;
        // Sólo IPv4/IPv6 literales o nombres .local: nada de URLs arbitrarias.
        let host_ok = host.parse::<std::net::IpAddr>().is_ok()
            || (host.ends_with(".local")
                && host.len() <= 253
                && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
        if !host_ok {
            return Err(PairError::BadQr("host"));
        }
        let port: u16 = kv.get("p").and_then(|p| p.parse().ok()).filter(|p| *p != 0).ok_or(PairError::BadQr("puerto"))?;
        let fp: [u8; 32] = kv
            .get("fp")
            .and_then(|s| B64.decode(s).ok())
            .and_then(|v| v.try_into().ok())
            .ok_or(PairError::BadQr("huella"))?;
        let token: [u8; 16] = kv
            .get("t")
            .and_then(|s| B64.decode(s).ok())
            .and_then(|v| v.try_into().ok())
            .ok_or(PairError::BadQr("token"))?;
        let expires_at_ms = kv.get("e").and_then(|e| e.parse().ok()).ok_or(PairError::BadQr("vencimiento"))?;
        Ok(Self { host: host.to_string(), port, host_fp: Fingerprint(fp), token, expires_at_ms })
    }
}

/// Registro host-side de tokens QR vigentes (un solo uso).
#[derive(Default)]
pub struct QrTokens {
    active: Vec<([u8; 16], u64)>,
}

impl QrTokens {
    pub fn issue(&mut self, p: &QrPayload) {
        self.active.push((p.token, p.expires_at_ms));
    }
    pub fn redeem(&mut self, token: &[u8; 16], now_ms: u64) -> Result<(), PairError> {
        self.active.retain(|(_, e)| *e > now_ms);
        let idx = self
            .active
            .iter()
            .position(|(t, _)| bool::from(t.ct_eq(token)))
            .ok_or(PairError::NotActive)?;
        self.active.swap_remove(idx);
        Ok(())
    }
}

// ---------------------------------------------------------------- confianza

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub name: String,
    pub permission: Permission,
    pub paired_at_ms: u64,
    pub last_seen_ms: u64,
    /// Preferencias recordadas para la próxima conexión.
    #[serde(default)]
    pub last_monitor: Option<String>,
    #[serde(default)]
    pub profile: Option<QualityProfile>,
}

/// Lista de dispositivos confiables del host. La persistencia en disco debe
/// cifrarse con DPAPI (Windows) — responsabilidad del host, no de este crate.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    devices: HashMap<String, TrustedDevice>,
}

impl TrustStore {
    fn key(fp: &Fingerprint) -> String {
        B64.encode(fp.0)
    }
    pub fn add(&mut self, fp: Fingerprint, dev: TrustedDevice) {
        self.devices.insert(Self::key(&fp), dev);
    }
    pub fn get(&self, fp: &Fingerprint) -> Option<&TrustedDevice> {
        self.devices.get(&Self::key(fp))
    }
    pub fn touch(&mut self, fp: &Fingerprint, now_ms: u64) {
        if let Some(d) = self.devices.get_mut(&Self::key(fp)) {
            d.last_seen_ms = now_ms;
        }
    }
    pub fn set_permission(&mut self, fp: &Fingerprint, p: Permission) -> bool {
        self.devices.get_mut(&Self::key(fp)).map(|d| d.permission = p).is_some()
    }
    /// Actualiza las preferencias recordadas del dispositivo.
    pub fn update(&mut self, fp: &Fingerprint, f: impl FnOnce(&mut TrustedDevice)) -> bool {
        self.devices.get_mut(&Self::key(fp)).map(f).is_some()
    }
    pub fn revoke(&mut self, fp: &Fingerprint) -> bool {
        self.devices.remove(&Self::key(fp)).is_some()
    }
    /// Recorre (huella en base64url, dispositivo).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TrustedDevice)> {
        self.devices.iter()
    }
    pub fn fingerprint_from_key(key: &str) -> Option<Fingerprint> {
        B64.decode(key).ok()?.try_into().ok().map(Fingerprint)
    }
    pub fn key_of(fp: &Fingerprint) -> String {
        Self::key(fp)
    }
    pub fn len(&self) -> usize {
        self.devices.len()
    }
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
    /// ¿Puede este certificado inyectar entrada? Fuente única de verdad
    /// para el filtro de entrada del host.
    pub fn may_control(&self, fp: &Fingerprint) -> bool {
        matches!(self.get(fp), Some(TrustedDevice { permission: Permission::Control, .. }))
    }
}

// ---------------------------------------------------------------- reanudación

/// Ficha de reanudación: `slot | expires | HMAC(host_secret, fp|slot|expires)`.
pub struct ResumeTickets {
    secret: [u8; 32],
}

impl ResumeTickets {
    pub fn new() -> Self {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        Self { secret }
    }

    fn mac(&self, fp: &Fingerprint, slot: u8, exp: u64) -> [u8; 32] {
        let mut m = <HmacSha256 as Mac>::new_from_slice(&self.secret).unwrap();
        m.update(b"kbss-resume-v1");
        m.update(&fp.0);
        m.update(&[slot]);
        m.update(&exp.to_be_bytes());
        m.finalize().into_bytes().into()
    }

    pub fn issue(&self, fp: &Fingerprint, slot: u8, now_ms: u64) -> Vec<u8> {
        let exp = now_ms + RESUME_TTL_MS;
        let mut v = Vec::with_capacity(41);
        v.push(slot);
        v.extend_from_slice(&exp.to_be_bytes());
        v.extend_from_slice(&self.mac(fp, slot, exp));
        v
    }

    /// Devuelve la ranura si la ficha es válida PARA ESTE certificado.
    pub fn verify(&self, fp: &Fingerprint, ticket: &[u8], now_ms: u64) -> Result<u8, PairError> {
        if ticket.len() != 41 {
            return Err(PairError::BadTicket);
        }
        let slot = ticket[0];
        let exp = u64::from_be_bytes(ticket[1..9].try_into().unwrap());
        let ok: bool = self.mac(fp, slot, exp).ct_eq(&ticket[9..]).into();
        if !ok || now_ms >= exp {
            return Err(PairError::BadTicket);
        }
        Ok(slot)
    }
}

impl Default for ResumeTickets {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- rate limit

/// Token bucket por clave (p. ej. IP de origen).
pub struct RateLimiter {
    capacity: f64,
    refill_per_ms: f64,
    buckets: HashMap<String, (f64, u64)>,
    max_keys: usize,
}

impl RateLimiter {
    pub fn new(capacity: u32, per_minute: u32) -> Self {
        Self {
            capacity: capacity as f64,
            refill_per_ms: per_minute as f64 / 60_000.0,
            buckets: HashMap::new(),
            max_keys: 1024,
        }
    }

    pub fn allow(&mut self, key: &str, now_ms: u64) -> bool {
        if self.buckets.len() >= self.max_keys && !self.buckets.contains_key(key) {
            // Bajo inundación, se descartan buckets llenos (inactivos) y si
            // aun así no hay lugar se rechaza: fallar cerrado.
            let cap = self.capacity;
            self.buckets.retain(|_, (t, _)| *t < cap);
            if self.buckets.len() >= self.max_keys {
                return false;
            }
        }
        let (tokens, last) = self.buckets.entry(key.to_string()).or_insert((self.capacity, now_ms));
        *tokens = (*tokens + (now_ms.saturating_sub(*last)) as f64 * self.refill_per_ms).min(self.capacity);
        *last = now_ms;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HFP: Fingerprint = Fingerprint([1; 32]);
    const CFP: Fingerprint = Fingerprint([2; 32]);

    fn run(pin_host: &mut PinCode, pin_typed: &str, now: u64, host_fp_seen_by_client: Fingerprint) -> Result<(), PairError> {
        let (client, m_c) = ClientPinPairing::start(pin_typed, host_fp_seen_by_client, CFP);
        let (host, m_h) = HostPinExchange::respond(pin_host, now, &m_c, HFP, CFP)?;
        let out = client.finish(&m_h)?;
        let host_mac = host.verify(pin_host, &out.client_confirm)?;
        out.verify_host_confirm(&host_mac)
    }

    #[test]
    fn correct_pin_pairs_and_is_single_use() {
        let mut pin = PinCode::generate(0);
        let typed = pin.display().to_string();
        assert_eq!(typed.len(), 6);
        run(&mut pin, &typed, 10, HFP).unwrap();
        assert_eq!(run(&mut pin, &typed, 20, HFP), Err(PairError::NotActive));
    }

    #[test]
    fn wrong_pin_fails_and_attempts_are_limited() {
        let mut pin = PinCode::generate(0);
        let wrong = if pin.display() == "000000" { "000001" } else { "000000" };
        for _ in 0..PIN_MAX_ATTEMPTS {
            assert_eq!(run(&mut pin, wrong, 1, HFP), Err(PairError::BadConfirmation));
        }
        let right = pin.display().to_string();
        assert_eq!(run(&mut pin, &right, 1, HFP), Err(PairError::TooManyAttempts));
    }

    #[test]
    fn expired_pin_rejected() {
        let mut pin = PinCode::generate(0);
        let right = pin.display().to_string();
        assert_eq!(run(&mut pin, &right, PIN_TTL_MS, HFP), Err(PairError::Expired));
    }

    #[test]
    fn mitm_with_different_certificate_is_detected_even_with_right_pin() {
        // El cliente ve el certificado del atacante, el host el suyo propio.
        let mut pin = PinCode::generate(0);
        let right = pin.display().to_string();
        assert_eq!(run(&mut pin, &right, 1, Fingerprint([9; 32])), Err(PairError::BadConfirmation));
    }

    #[test]
    fn qr_roundtrip_and_rejects_tampering() {
        let p = QrPayload::new("192.168.1.20".into(), 47800, HFP, 1000);
        let uri = p.to_uri();
        assert_eq!(QrPayload::parse(&uri).unwrap(), p);
        assert!(QrPayload::parse(&uri.replace("v=1", "v=2")).is_err());
        assert!(QrPayload::parse(&uri.replace("192.168.1.20", "evil.com")).is_err());
        assert!(QrPayload::parse(&format!("{uri}&h=10.0.0.1")).is_err());
        assert!(QrPayload::parse("https://pair?v=1").is_err());
        let mut tokens = QrTokens::default();
        tokens.issue(&p);
        assert_eq!(tokens.redeem(&[0; 16], 2000), Err(PairError::NotActive));
        tokens.redeem(&p.token, 2000).unwrap();
        assert_eq!(tokens.redeem(&p.token, 2000), Err(PairError::NotActive));
        let p2 = QrPayload::new("studio-pc.local".into(), 47800, HFP, 0);
        tokens.issue(&p2);
        assert_eq!(tokens.redeem(&p2.token, QR_TTL_MS + 1), Err(PairError::NotActive));
    }

    #[test]
    fn trust_store_permissions_and_revocation() {
        let mut ts = TrustStore::default();
        ts.add(CFP, TrustedDevice { name: "Tablet".into(), permission: Permission::ViewOnly, paired_at_ms: 0, last_seen_ms: 0, last_monitor: None, profile: None });
        assert!(!ts.may_control(&CFP));
        assert!(ts.set_permission(&CFP, Permission::Control));
        assert!(ts.may_control(&CFP));
        assert!(!ts.may_control(&HFP));
        assert!(ts.revoke(&CFP));
        assert!(!ts.may_control(&CFP));
    }

    #[test]
    fn resume_ticket_bound_to_certificate_and_expires() {
        let rt = ResumeTickets::new();
        let t = rt.issue(&CFP, 1, 0);
        assert_eq!(rt.verify(&CFP, &t, 5), Ok(1));
        assert_eq!(rt.verify(&HFP, &t, 5), Err(PairError::BadTicket));
        assert_eq!(rt.verify(&CFP, &t, RESUME_TTL_MS), Err(PairError::BadTicket));
        let mut forged = t.clone();
        forged[0] = 2;
        assert_eq!(rt.verify(&CFP, &forged, 5), Err(PairError::BadTicket));
    }

    #[test]
    fn rate_limiter_blocks_bursts_and_refills() {
        let mut rl = RateLimiter::new(3, 6);
        assert!(rl.allow("10.0.0.9", 0));
        assert!(rl.allow("10.0.0.9", 0));
        assert!(rl.allow("10.0.0.9", 0));
        assert!(!rl.allow("10.0.0.9", 0));
        assert!(rl.allow("10.0.0.8", 0));
        assert!(rl.allow("10.0.0.9", 10_000)); // 6/min => 1 cada 10 s
    }
}

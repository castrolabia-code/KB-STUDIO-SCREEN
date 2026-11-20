//! KB STUDIO SCREEN — transporte.
//!
//! Dos capas:
//!
//! 1. **Abstracta** (`read_message` / `write_message` / `read_video` /
//!    `write_video`): funciones sobre cualquier `AsyncRead`/`AsyncWrite`.
//!    Un transporte USB futuro (p. ej. `adb reverse` = socket TCP local, o
//!    Android Open Accessory) sólo tiene que producir un flujo de bytes y
//!    reutiliza exactamente el mismo framing y validación. USB **no está
//!    implementado** en esta versión.
//! 2. **QUIC** (`quinn`, TLS 1.3 vía `rustls`): canal de control = stream
//!    bidireccional; video = un stream unidireccional POR FRAME, para que un
//!    frame atrasado pueda descartarse (`reset`) sin bloquear a los
//!    siguientes (evita el head-of-line blocking de TCP sobre Wi-Fi).
//!
//! Autenticación: TLS mutuo con certificados autofirmados; ambos lados
//! verifican la huella SHA-256 del certificado del otro contra la fijada
//! (pinning). En modo "emparejamiento" el host acepta cualquier certificado
//! de cliente y la autorización se decide en la capa de aplicación (PIN/QR
//! más aprobación). Ningún mensaje de entrada se acepta de un certificado no
//! confiable (ver `kb_pairing::TrustStore::may_control`).

use kb_pairing::Fingerprint;
use kb_proto::{Message, ProtoError, VideoChunkHeader};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub use quinn;

/// Nombre SNI fijo: la identidad real es la huella, no el nombre.
pub const SNI: &str = "kbss-host";

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("protocolo: {0}")]
    Proto(#[from] ProtoError),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("conexión: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("conexión: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("certificado: {0}")]
    Cert(String),
    #[error("lectura: {0}")]
    Read(#[from] quinn::ReadExactError),
    #[error("escritura: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("el peer no presentó certificado")]
    NoPeerCert,
}

// ------------------------------------------------------------ capa abstracta

pub async fn write_message<W: AsyncWrite + Unpin>(w: &mut W, m: &Message) -> Result<(), TransportError> {
    let buf = kb_proto::encode(m)?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_message<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message, TransportError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = kb_proto::frame_len(len)?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(kb_proto::decode_body(&body)?)
}

pub async fn write_video<W: AsyncWrite + Unpin>(w: &mut W, h: &VideoChunkHeader, data: &[u8]) -> Result<(), TransportError> {
    debug_assert_eq!(h.len as usize, data.len());
    w.write_all(&h.to_bytes()).await?;
    w.write_all(data).await?;
    Ok(())
}

pub async fn read_video<R: AsyncRead + Unpin>(r: &mut R) -> Result<(VideoChunkHeader, Vec<u8>), TransportError> {
    let mut hb = [0u8; VideoChunkHeader::SIZE];
    r.read_exact(&mut hb).await?;
    let h = VideoChunkHeader::from_bytes(&hb)?;
    let mut data = vec![0u8; h.len as usize];
    r.read_exact(&mut data).await?;
    Ok((h, data))
}

// ------------------------------------------------------------ identidad

/// Certificado + clave persistentes de un extremo.
pub struct Identity {
    pub cert_der: CertificateDer<'static>,
    key_der: Vec<u8>,
}

impl Identity {
    pub fn generate() -> Result<Self, TransportError> {
        let ck = rcgen::generate_simple_self_signed(vec![SNI.to_string()])
            .map_err(|e| TransportError::Cert(e.to_string()))?;
        Ok(Self { cert_der: ck.cert.der().clone(), key_der: ck.key_pair.serialize_der() })
    }
    /// Reconstruye desde almacenamiento (en Windows: cifrado con DPAPI;
    /// en Android: clave envuelta con Android Keystore).
    pub fn from_der(cert_der: Vec<u8>, pkcs8_key_der: Vec<u8>) -> Self {
        Self { cert_der: CertificateDer::from(cert_der), key_der: pkcs8_key_der }
    }
    pub fn key_der(&self) -> &[u8] {
        &self.key_der
    }
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_der(&self.cert_der)
    }
    fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

// ------------------------------------------------------------ verificadores

#[derive(Debug)]
struct PinnedServer {
    expected: Fingerprint,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Fingerprint::of_der(end_entity) == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Sólo para el PRIMER emparejamiento por PIN (sin QR): se acepta el
/// certificado del host y la autenticación la da SPAKE2 con channel binding.
#[derive(Debug)]
struct AcceptAnyServerForPairing {
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for AcceptAnyServerForPairing {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// El host exige certificado de cliente (prueba de posesión de la clave) y
/// lo entrega a la aplicación, que decide según la lista de confianza.
#[derive(Debug)]
struct RequireClientCert {
    algs: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for RequireClientCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn client_auth_mandatory(&self) -> bool {
        true
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

// ------------------------------------------------------------ QUIC

fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(Duration::from_secs(2)));
    t.max_idle_timeout(Some(Duration::from_secs(8).try_into().unwrap()));
    // Un frame por stream: suficientes streams concurrentes para 3 clientes
    // a 60 FPS con algunos frames en vuelo.
    t.max_concurrent_uni_streams(64u32.into());
    t
}

pub fn server_endpoint(id: &Identity, bind: SocketAddr) -> Result<quinn::Endpoint, TransportError> {
    let p = provider();
    let algs = p.signature_verification_algorithms;
    let mut tls = rustls::ServerConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(RequireClientCert { algs }))
        .with_single_cert(vec![id.cert_der.clone()], id.key())?;
    tls.alpn_protocols = vec![kb_proto::ALPN.to_vec()];
    let qtls = quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(|e| TransportError::Cert(e.to_string()))?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(qtls));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(quinn::Endpoint::server(cfg, bind)?)
}

/// `pinned_host`: `Some(huella)` para reconexión o QR; `None` sólo para
/// emparejamiento por PIN.
pub fn client_config(id: &Identity, pinned_host: Option<Fingerprint>) -> Result<quinn::ClientConfig, TransportError> {
    let p = provider();
    let algs = p.signature_verification_algorithms;
    let verifier: Arc<dyn ServerCertVerifier> = match pinned_host {
        Some(fp) => Arc::new(PinnedServer { expected: fp, algs }),
        None => Arc::new(AcceptAnyServerForPairing { algs }),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![id.cert_der.clone()], id.key())?;
    tls.alpn_protocols = vec![kb_proto::ALPN.to_vec()];
    let qtls = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| TransportError::Cert(e.to_string()))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(qtls));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

pub async fn connect(endpoint: &quinn::Endpoint, cfg: quinn::ClientConfig, addr: SocketAddr) -> Result<quinn::Connection, TransportError> {
    Ok(endpoint.connect_with(cfg, addr, SNI)?.await?)
}

/// Huella del certificado que presentó el otro extremo en esta conexión.
pub fn peer_fingerprint(conn: &quinn::Connection) -> Result<Fingerprint, TransportError> {
    let any = conn.peer_identity().ok_or(TransportError::NoPeerCert)?;
    let certs = any.downcast::<Vec<CertificateDer<'static>>>().map_err(|_| TransportError::NoPeerCert)?;
    certs.first().map(|c| Fingerprint::of_der(c)).ok_or(TransportError::NoPeerCert)
}

/// Envía un frame en su propio stream unidireccional. Devuelve el stream
/// para que el llamador pueda hacer `reset` si llega un frame más nuevo
/// antes de que este termine (política "último frame gana").
pub async fn send_frame(conn: &quinn::Connection, h: &VideoChunkHeader, data: &[u8]) -> Result<(), TransportError> {
    let mut s = conn.open_uni().await?;
    // Keyframes con prioridad mayor que P-frames.
    let _ = s.set_priority(if h.keyframe { 1 } else { 0 });
    write_video(&mut s, h, data).await?;
    let _ = s.finish();
    Ok(())
}

pub async fn recv_frame(conn: &quinn::Connection) -> Result<(VideoChunkHeader, Vec<u8>), TransportError> {
    let mut s = conn.accept_uni().await?;
    read_video(&mut s).await
}

/// Filtro de entrada del host: punto ÚNICO por donde pasan los mensajes del
/// cliente antes de tocar Windows.
pub fn authorize_inbound(msg: &Message, trust: &kb_pairing::TrustStore, peer: &Fingerprint) -> Result<(), kb_proto::ErrorCode> {
    let paired = trust.get(peer).is_some();
    let pairing_msg = matches!(
        msg,
        Message::ClientHello { .. }
            | Message::PairStart { .. }
            | Message::PairConfirm { .. }
            | Message::PairQrToken { .. }
            | Message::Ping { .. }
            | Message::Bye
    );
    if !paired && !pairing_msg {
        return Err(kb_proto::ErrorCode::NotPaired);
    }
    if msg.is_input() && !trust.may_control(peer) {
        return Err(kb_proto::ErrorCode::PermissionDenied);
    }
    Ok(())
}

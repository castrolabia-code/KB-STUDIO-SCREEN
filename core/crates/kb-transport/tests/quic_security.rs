//! Pruebas de integración sobre QUIC real (loopback UDP).

use kb_geometry::NormPoint;
use kb_pairing::{ClientPinPairing, HostPinExchange, PinCode, TrustStore, TrustedDevice};
use kb_proto::{ErrorCode, InputEvent, Message, Permission, PointerEvent, PointerPhase, VideoChunkHeader};
use kb_transport::*;
use std::net::SocketAddr;

fn localhost() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn client_ep() -> quinn::Endpoint {
    quinn::Endpoint::client(localhost()).unwrap()
}

#[tokio::test]
async fn pinned_mutual_tls_control_and_video_roundtrip() {
    let host = Identity::generate().unwrap();
    let dev = Identity::generate().unwrap();
    let server = server_endpoint(&host, localhost()).unwrap();
    let addr = server.local_addr().unwrap();
    let dev_fp = dev.fingerprint();

    let srv = tokio::spawn(async move {
        let conn = server.accept().await.unwrap().await.unwrap();
        assert_eq!(peer_fingerprint(&conn).unwrap(), dev_fp, "el host ve el certificado real del cliente");
        let (mut tx, mut rx) = conn.accept_bi().await.unwrap();
        let m = read_message(&mut rx).await.unwrap();
        assert_eq!(m, Message::Ping { t_us: 7 });
        write_message(&mut tx, &Message::Pong { t_us: 7 }).await.unwrap();
        let data = vec![0xAB; 50_000];
        let h = VideoChunkHeader { seq: 1, capture_us: 1, keyframe: true, width: 1280, height: 720, len: data.len() as u32 };
        send_frame(&conn, &h, &data).await.unwrap();
        conn.closed().await;
    });

    let ep = client_ep();
    let conn = connect(&ep, client_config(&dev, Some(host.fingerprint())).unwrap(), addr).await.unwrap();
    assert_eq!(peer_fingerprint(&conn).unwrap(), host.fingerprint());
    let (mut tx, mut rx) = conn.open_bi().await.unwrap();
    write_message(&mut tx, &Message::Ping { t_us: 7 }).await.unwrap();
    assert_eq!(read_message(&mut rx).await.unwrap(), Message::Pong { t_us: 7 });
    let (h, data) = recv_frame(&conn).await.unwrap();
    assert!(h.keyframe && data.len() == 50_000 && data.iter().all(|b| *b == 0xAB));
    conn.close(0u32.into(), b"bye");
    srv.await.unwrap();
}

#[tokio::test]
async fn client_rejects_host_with_unexpected_fingerprint() {
    let real_host = Identity::generate().unwrap();
    let impostor = Identity::generate().unwrap();
    let dev = Identity::generate().unwrap();
    let server = server_endpoint(&impostor, localhost()).unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        if let Some(inc) = server.accept().await {
            let _ = inc.await;
        }
    });
    let ep = client_ep();
    let r = connect(&ep, client_config(&dev, Some(real_host.fingerprint())).unwrap(), addr).await;
    assert!(r.is_err(), "un host con otro certificado debe ser rechazado");
}

#[tokio::test]
async fn pin_pairing_over_real_tls_channel_binding() {
    // Primer emparejamiento: el cliente todavía no conoce la huella del host.
    let host = Identity::generate().unwrap();
    let dev = Identity::generate().unwrap();
    let server = server_endpoint(&host, localhost()).unwrap();
    let addr = server.local_addr().unwrap();
    let mut pin = PinCode::generate(0);
    let typed = pin.display().to_string();
    let host_fp = host.fingerprint();

    let srv = tokio::spawn(async move {
        let conn = server.accept().await.unwrap().await.unwrap();
        let client_fp = peer_fingerprint(&conn).unwrap();
        let (mut tx, mut rx) = conn.accept_bi().await.unwrap();
        let Message::PairStart { spake_msg } = read_message(&mut rx).await.unwrap() else { panic!() };
        let (ex, reply) = HostPinExchange::respond(&mut pin, 1, &spake_msg, host_fp, client_fp).unwrap();
        write_message(&mut tx, &Message::PairReply { spake_msg: reply }).await.unwrap();
        let Message::PairConfirm { mac } = read_message(&mut rx).await.unwrap() else { panic!() };
        let host_mac = ex.verify(&mut pin, &mac).unwrap();
        // (Aquí la UI del host pide aprobación al usuario.)
        write_message(&mut tx, &Message::PairConfirm { mac: host_mac }).await.unwrap();
        write_message(&mut tx, &Message::PairResult { approved: true, permission: Permission::ViewOnly }).await.unwrap();
        conn.closed().await;
        client_fp
    });

    let ep = client_ep();
    let conn = connect(&ep, client_config(&dev, None).unwrap(), addr).await.unwrap();
    let seen_host_fp = peer_fingerprint(&conn).unwrap();
    let (mut tx, mut rx) = conn.open_bi().await.unwrap();
    let (p, msg) = ClientPinPairing::start(&typed, seen_host_fp, dev.fingerprint());
    write_message(&mut tx, &Message::PairStart { spake_msg: msg }).await.unwrap();
    let Message::PairReply { spake_msg } = read_message(&mut rx).await.unwrap() else { panic!() };
    let out = p.finish(&spake_msg).unwrap();
    write_message(&mut tx, &Message::PairConfirm { mac: out.client_confirm }).await.unwrap();
    let Message::PairConfirm { mac } = read_message(&mut rx).await.unwrap() else { panic!() };
    out.verify_host_confirm(&mac).unwrap();
    assert_eq!(
        read_message(&mut rx).await.unwrap(),
        Message::PairResult { approved: true, permission: Permission::ViewOnly }
    );
    // A partir de acá el cliente fija la huella del host para reconectar.
    assert_eq!(seen_host_fp, host.fingerprint());
    conn.close(0u32.into(), b"paired");
    assert_eq!(srv.await.unwrap(), dev.fingerprint());
}

#[test]
fn view_only_and_unpaired_devices_cannot_inject_input() {
    let viewer = kb_pairing::Fingerprint([1; 32]);
    let controller = kb_pairing::Fingerprint([2; 32]);
    let stranger = kb_pairing::Fingerprint([3; 32]);
    let mut ts = TrustStore::default();
    ts.add(viewer, TrustedDevice { name: "Cantante".into(), permission: Permission::ViewOnly, paired_at_ms: 0, last_seen_ms: 0, last_monitor: None, profile: None });
    ts.add(controller, TrustedDevice { name: "Mixer".into(), permission: Permission::Control, paired_at_ms: 0, last_seen_ms: 0, last_monitor: None, profile: None });
    let touch = Message::Input(InputEvent::Pointer(PointerEvent {
        pointer_id: 0,
        phase: PointerPhase::Down,
        pos: NormPoint { x: 1, y: 1 },
        t_us: 0,
    }));
    assert_eq!(authorize_inbound(&touch, &ts, &viewer), Err(ErrorCode::PermissionDenied));
    assert_eq!(authorize_inbound(&touch, &ts, &stranger), Err(ErrorCode::NotPaired));
    assert_eq!(authorize_inbound(&Message::RequestKeyframe, &ts, &stranger), Err(ErrorCode::NotPaired));
    assert_eq!(authorize_inbound(&touch, &ts, &controller), Ok(()));
    assert_eq!(authorize_inbound(&Message::RequestKeyframe, &ts, &viewer), Ok(()));
    ts.revoke(&controller);
    assert_eq!(authorize_inbound(&touch, &ts, &controller), Err(ErrorCode::NotPaired));
}

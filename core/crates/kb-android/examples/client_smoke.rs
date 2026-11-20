//! Prueba de humo de la API que usa la app por JNI, contra `demo_host`.
//!   cargo run -p kb-android --example client_smoke -- <qr.txt>
use kbcore::api::{connect, ConnectArgs};

fn main() {
    let qr = std::fs::read_to_string(std::env::args().nth(1).expect("qr.txt")).unwrap();
    let id = kb_transport::Identity::generate().unwrap();
    let s = connect(ConnectArgs {
        host: String::new(), port: 0, mode: 1, pinned_fp: String::new(), qr_uri: qr.trim().into(),
        cert: id.cert_der.to_vec(), key: id.key_der().to_vec(), device_name: "Tablet EXO (smoke)".into(),
        is_tablet: true, screen_w: 1280, screen_h: 800, dpi: 213, refresh: 60, dec_max_w: 1920, dec_max_h: 1080,
    }).expect("connect");
    println!("STAGE {}", s.stage_json());
    println!("HOST {}", s.host_json());
    let mut keys = 0;
    for _ in 0..20 {
        let f = s.next_frame().expect("frame");
        let h = kb_proto::VideoChunkHeader::from_bytes(f[..21].try_into().unwrap()).unwrap();
        if h.keyframe { keys += 1; }
    }
    println!("FRAMES 20 recibidos, keyframes={keys}");
    s.set_viewport(1280.0, 800.0, 1280, 720, 1.0, 0.0, 0.0);
    assert!(!s.touch(0, 640.0, 10.0), "toque en la barra negra no se envía");
    assert!(s.touch(0, 640.0, 400.0));
    assert!(s.touch(1, 640.0, 200.0));
    assert!(s.touch(2, 640.0, 200.0));
    std::thread::sleep(std::time::Duration::from_millis(300));
    s.close();
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("SMOKE OK");
}

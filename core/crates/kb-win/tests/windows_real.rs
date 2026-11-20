//! Pruebas sobre Windows REAL (se ejecutan en el runner de CI y en tu PC).
//! En otros sistemas este archivo no compila nada.
#![cfg(windows)]

use kb_geometry::PointI;
use kb_proto::{DecoderCaps, H264Profile, Orientation, QualityProfile, ScreenInfo};
use kb_session::{choose_params, convert, Injected, Platform};
use kb_win::encode::nal_types;
use kb_win::WinPlatform;
use std::time::{Duration, Instant};

#[test]
fn capture_encode_and_inject_on_real_windows() {
    let p = WinPlatform::init().expect("init");
    let topo = p.topology();
    println!("TOPOLOGIA {:?}", topo.monitors.iter().map(|m| (&m.id, &m.name, m.bounds, m.dpi, m.primary)).collect::<Vec<_>>());
    let primary = topo.primary().expect("al menos un monitor").clone();

    // --- captura ---
    let mut cap = p.open_capture(&primary.id).expect("DuplicateOutput");
    let t0 = Instant::now();
    let frame = loop {
        if let Some(f) = cap.next_frame(500).expect("AcquireNextFrame") {
            break f;
        }
        assert!(t0.elapsed() < Duration::from_secs(5), "sin frames en 5 s");
    };
    println!("CAPTURA {}x{} en {:?}", frame.width, frame.height, t0.elapsed());
    assert_eq!((frame.width as i32, frame.height as i32), (primary.bounds.w, primary.bounds.h));

    // --- codificación ---
    let screen = ScreenInfo { width_px: 1280, height_px: 800, density_dpi: 213, orientation: Orientation::Landscape, refresh_hz: 60 };
    let dec = DecoderCaps { h264_profiles: vec![H264Profile::Baseline], max_width: 1920, max_height: 1080, max_fps_at_max_res: 30, hardware: true };
    let params = choose_params(frame.width, frame.height, &screen, &dec, QualityProfile::Mixer);
    let mut enc = p.open_encoder(&params).expect("encoder");
    println!("ENCODER {} {}x{} {} fps {} kbps", enc.name(), params.width, params.height, params.fps, params.bitrate_kbps);
    let mut stream = Vec::new();
    let mut conv_ms = Vec::new();
    let mut enc_ms = Vec::new();
    let mut first_key = None;
    for i in 0..30u64 {
        let t = Instant::now();
        let nv12 = convert::bgra_to_nv12(&frame.data, frame.width, frame.height, frame.stride, params.width, params.height);
        conv_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let out = enc.encode(&nv12, i * 33_333, i == 0).expect("encode");
        enc_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        for o in out {
            if o.keyframe && first_key.is_none() {
                first_key = Some(o.data.clone());
            }
            stream.extend_from_slice(&o.data);
        }
    }
    let key = first_key.expect("se esperaba al menos un keyframe");
    let types: Vec<u8> = nal_types(&key).collect();
    println!("NAL_KEYFRAME {types:?}");
    assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "keyframe sin SPS/PPS/IDR: {types:?}");
    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
    println!(
        "MEDICION conversion_ms prom={:.2} max={:.2} | encode_ms prom={:.2} max={:.2} | bytes={} (30 frames)",
        avg(&conv_ms),
        max(&conv_ms),
        avg(&enc_ms),
        max(&enc_ms),
        stream.len()
    );
    // Volcado del stream para verificarlo con un decodificador independiente (ffmpeg) fuera de Windows.
    if std::env::var("KBSS_DUMP_H264").is_ok() {
        use std::fmt::Write;
        let mut hex = String::with_capacity(stream.len() * 2);
        for b in &stream {
            let _ = write!(hex, "{b:02x}");
        }
        println!("H264_BEGIN");
        for chunk in hex.as_bytes().chunks(4000) {
            println!("{}", std::str::from_utf8(chunk).unwrap());
        }
        println!("H264_END");
    }

    // --- entrada: el cursor debe caer en el píxel exacto ---
    let sink = p.input();
    let target = PointI { x: primary.bounds.x + primary.bounds.w / 3, y: primary.bounds.y + primary.bounds.h / 4 };
    sink.inject(Injected::Move { p: target });
    std::thread::sleep(Duration::from_millis(100));
    let mut pt = windows::Win32::Foundation::POINT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) }.expect("GetCursorPos");
    println!("CURSOR objetivo={target:?} real=({}, {})", pt.x, pt.y);
    assert!((pt.x - target.x).abs() <= 1 && (pt.y - target.y).abs() <= 1, "SendInput no llegó al píxel esperado");
}

#[test]
fn dpapi_roundtrip() {
    let secret = b"clave privada de prueba";
    let enc = kb_win::dpapi::protect(secret).unwrap();
    assert_ne!(&enc[..], &secret[..]);
    assert_eq!(kb_win::dpapi::unprotect(&enc).unwrap(), secret);
}

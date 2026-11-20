//! PoC medible del transporte de video: N clientes QUIC reciben frames
//! sintéticos de tamaño fijo a F FPS y se mide la latencia de entrega
//! (mismo reloj, mismo equipo).
//!
//! Esto mide el COSTO PROPIO del stack (QUIC + TLS + framing), no el de la
//! Wi-Fi. Para la red real se usa la misma herramienta entre PC y Android
//! (Fase 1) o entre dos PCs. Uso:
//!
//!   cargo run --release -p kb-transport --example loopback_bench -- <clientes> <fps> <kb_por_frame> <segundos>

use kb_proto::VideoChunkHeader;
use kb_transport::*;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let clients: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let fps: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let kb: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);
    let secs: u64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(5);

    let host = Identity::generate().unwrap();
    let server = server_endpoint(&host, "127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = server.local_addr().unwrap();
    let t0 = Instant::now();

    // Host: por cada cliente, envía frames a ritmo fijo.
    let total = fps * secs;
    tokio::spawn(async move {
        for _ in 0..clients {
            let conn = server.accept().await.unwrap().await.unwrap();
            tokio::spawn(async move {
                let data = vec![0x5Au8; kb * 1024];
                let mut tick = tokio::time::interval(Duration::from_micros(1_000_000 / fps));
                for seq in 0..total as u32 {
                    tick.tick().await;
                    let h = VideoChunkHeader {
                        seq,
                        capture_us: t0.elapsed().as_micros() as u64,
                        keyframe: seq % (fps as u32 * 2) == 0,
                        width: 1920,
                        height: 1080,
                        len: data.len() as u32,
                    };
                    let c = conn.clone();
                    let d = data.clone();
                    tokio::spawn(async move {
                        let _ = send_frame(&c, &h, &d).await;
                    });
                }
                conn.closed().await;
            });
        }
    });

    let mut tasks = Vec::new();
    for _ in 0..clients {
        let cfg = client_config(&Identity::generate().unwrap(), Some(host.fingerprint())).unwrap();
        tasks.push(tokio::spawn(async move {
            let ep = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
            let conn = connect(&ep, cfg, addr).await.unwrap();
            let mut lat = Vec::with_capacity(total as usize);
            while lat.len() < total as usize {
                match tokio::time::timeout(Duration::from_secs(3), recv_frame(&conn)).await {
                    Ok(Ok((h, _))) => lat.push(t0.elapsed().as_micros() as u64 - h.capture_us),
                    _ => break,
                }
            }
            conn.close(0u32.into(), b"done");
            lat
        }));
    }

    println!("KB STUDIO SCREEN — loopback_bench");
    println!("clientes={clients} fps={fps} frame={kb}KiB duración={secs}s  (≈{:.1} Mbit/s por cliente)", (kb * 1024 * 8) as f64 * fps as f64 / 1e6);
    for (i, t) in tasks.into_iter().enumerate() {
        let mut l = t.await.unwrap();
        let recv = l.len();
        l.sort_unstable();
        let p = |q: f64| l.get(((recv as f64 * q) as usize).min(recv.saturating_sub(1))).copied().unwrap_or(0) as f64 / 1000.0;
        println!(
            "cliente {i}: recibidos {recv}/{total}  latencia ms p50={:.2} p95={:.2} p99={:.2} max={:.2}",
            p(0.50), p(0.95), p(0.99), p(1.0)
        );
    }
}

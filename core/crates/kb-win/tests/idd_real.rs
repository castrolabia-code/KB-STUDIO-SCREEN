//! Prueba del driver kbss-idd en Windows REAL. Requiere el driver instalado
//! y el adaptador creado (`kbss-engine driver create-adapter`). Sólo corre si
//! `KBSS_IDD_TEST=1` (job de CI "driver"); en cualquier otro caso se omite.
#![cfg(windows)]

use kb_session::Platform;
use kb_win::idd::{wait_ready, VirtualDisplays};
use kb_win::WinPlatform;
use std::time::{Duration, Instant};

fn wait_for<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            println!("OK {what} en {:?}", t.elapsed());
            return v;
        }
        assert!(t.elapsed() < Duration::from_secs(secs), "tiempo agotado: {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn virtual_monitors_appear_are_capturable_and_disappear() {
    if std::env::var("KBSS_IDD_TEST").is_err() {
        println!("omitida (KBSS_IDD_TEST no definido)");
        return;
    }
    let p = WinPlatform::init().unwrap();
    let before = p.topology();
    println!("ANTES {:?}", before.monitors.iter().map(|m| (&m.id, &m.name, m.bounds, m.is_virtual)).collect::<Vec<_>>());

    let v = VirtualDisplays::open().expect("abrir el driver SIN administrador vía interfaz");
    wait_ready(&v, Duration::from_secs(15)).expect("adaptador listo");
    let info = v.info().unwrap();
    println!("DRIVER {info:?}");
    assert_eq!(info.protocol, 1);

    // --- Tablet EXO 10" típica: 1280x800 ---
    v.add(0, 1280, 800, 60).expect("agregar monitor ranura 0");
    let tablet = wait_for("monitor virtual 1280x800 visible para Windows", 20, || {
        p.topology().monitors.into_iter().find(|m| m.is_virtual && m.bounds.w == 1280 && m.bounds.h == 800)
    });
    println!("VIRTUAL {:?} '{}' en {:?}", tablet.id, tablet.name, tablet.bounds);
    assert_eq!(tablet.name, "KBSS Pantalla", "nombre del EDID");

    // Se captura igual que un monitor físico.
    let mut cap = p.open_capture(&tablet.id).expect("Desktop Duplication sobre el monitor virtual");
    let f = wait_for("primer frame del monitor virtual", 10, || cap.next_frame(500).ok().flatten());
    assert_eq!((f.width, f.height), (1280, 800));
    drop(cap);

    // --- S23 FE en horizontal: 2340x1080 (resolución no estándar) ---
    v.add(1, 2340, 1080, 60).expect("agregar monitor ranura 1");
    wait_for("segundo monitor 2340x1080", 20, || p.topology().monitors.into_iter().find(|m| m.is_virtual && m.bounds.w == 2340));
    assert_eq!(v.info().unwrap().active_mask, 0b11);

    // Entradas inválidas rechazadas por el driver.
    assert!(v.add(0, 1280, 800, 60).is_err(), "ranura ocupada");
    assert!(v.add(5, 1280, 800, 60).is_err(), "ranura fuera de rango");
    assert!(v.add(2, 100, 100, 60).is_err(), "resolución inválida");

    // --- Quitar uno: Windows vuelve a la topología sin él ---
    v.remove(0).unwrap();
    wait_for("monitor ranura 0 retirado", 20, || {
        (!p.topology().monitors.iter().any(|m| m.is_virtual && m.bounds.w == 1280 && m.bounds.h == 800)).then_some(())
    });

    // --- Watchdog: el motor "se cuelga" (sin PING) => el driver retira todo ---
    v.stop_heartbeat();
    wait_for("watchdog retiró los monitores", 20, || (!p.topology().monitors.iter().any(|m| m.is_virtual)).then_some(()));
    assert_eq!(v.info().unwrap().active_mask, 0);

    let after = p.topology();
    assert_eq!(after.monitors.len(), before.monitors.len(), "la topología original se restauró");
}

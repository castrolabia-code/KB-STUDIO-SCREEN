# 05 — Estado de la Fase 0

Fecha: 2026-11-20 · Entorno de esta sesión: contenedor Linux x86_64 (4 vCPU, 15 GB RAM), Rust 1.97.0, sin Windows, sin GPU, sin monitores, sin Android, sin acceso a la PC del estudio.

## Tareas

| ID | Tarea | Estado | Evidencia |
|---|---|---|---|
| F0-01 | Repositorio y workspace Rust | **IMPLEMENTADO · PROBADO** | `cargo test`: 25/25; `cargo clippy --all-targets`: 0 advertencias |
| F0-02 | Protocolo v1 (`kb-proto`) | **IMPLEMENTADO · PROBADO** | 6 pruebas: ida y vuelta, longitud excesiva rechazada antes de reservar memoria, versión incompatible, campos inválidos, 300 buffers aleatorios sin pánico, cabecera de video |
| F0-03 | Emparejamiento y autorización (`kb-pairing`) | **IMPLEMENTADO · PROBADO** | 8 pruebas: PIN de un solo uso, límite de intentos, vencimiento, **MITM con PIN correcto detectado**, QR alterado rechazado, revocación, ficha ligada al certificado, rate limit |
| F0-04 | Transporte QUIC (`kb-transport`) | **IMPLEMENTADO · PROBADO (loopback)** · Wi-Fi PENDIENTE | 4 pruebas de integración sobre UDP real: mTLS con huella fijada, **host impostor rechazado**, emparejamiento por PIN sobre TLS real, solo lectura y no emparejado no pueden inyectar |
| F0-05 | Geometría (`kb-geometry`) | **IMPLEMENTADO · PROBADO (unitario)** · equipo real PENDIENTE | 7 pruebas: letterbox 16:9 en 16:10, toque en barra descartado, monitor con coordenadas negativas, rotación, zoom local, `SendInput` absoluto en 3 monitores, rescate de ventana |
| F0-06 | Sonda Windows (`kb-probe-win`) | **IMPLEMENTADO** (compila para x86_64-pc-windows-gnu, 0 advertencias) · **PENDIENTE DE VALIDACIÓN EN EQUIPO REAL** | No se pudo ejecutar: no hay Windows en esta sesión |
| F0-07 | Factibilidad del driver IddCx (versión por build, firma con Secure Boot) | PENDIENTE · **BLOQUEADO por aprobación (B-03)** | Investigación documental en 01; falta la prueba en VM |
| F0-08 | Inyección de entrada en Reason 14 (mouse vs táctil, UIPI) | PENDIENTE | Requiere la PC con Reason |
| F0-09 | Toolchain Android y sonda de dispositivo (MediaCodec, resolución, densidad) | PENDIENTE | No hay Android SDK ni dispositivos en esta sesión; se puede compilar la APK acá en la Fase 1 |
| F0-10 | Benchmark de captura + codificación en tu GPU | PENDIENTE | Requiere la PC |
| F0-11 | Medición de transporte sobre tu Wi-Fi | PENDIENTE | Requiere PC + Android |

## Mediciones realizadas

`loopback_bench` (release, mismo equipo, **sin Wi-Fi**: mide sólo el costo propio de QUIC + TLS + framing):

| Clientes | FPS | Frame | Recibidos | p50 | p95 | p99 | máx |
|---|---|---|---|---|---|---|---|
| 1 | 60 | 40 KiB (~20 Mbit/s) | 300/300 | 0,24 ms | 0,35 ms | 0,69 ms | 1,49 ms |
| 3 | 60 | 40 KiB c/u | 900/900 | 0,21–0,23 ms | 0,32–0,34 ms | 0,49–0,71 ms | 0,77–5,07 ms |
| 3 | 30 | 120 KiB c/u (~30 Mbit/s) | 450/450 | 0,44–0,49 ms | 0,67–0,84 ms | 1,67–2,59 ms | 2,75–3,44 ms |

**Interpretación:** el stack de transporte no es el cuello de botella (< 1 ms en p95). La latencia real la van a definir la Wi-Fi, la codificación y la decodificación, que todavía no se midieron. **Estos números no son una promesa de rendimiento.**

## Reproducir

```bash
cd core && cargo test
```

```bash
cd core && cargo run --release -p kb-transport --example loopback_bench -- 3 60 40 5
```

## Lo que necesito de vos para cerrar la Fase 0
1. **Ejecutar la sonda en la PC del estudio** (`dist/kb-probe-win.exe`, SHA-256 `4b86b11c2577914b8e4f3b4711fb8c7f294fd1d74b6cccde4dbe0eb178da601a`) con Reason abierto como lo usás normalmente, y pasarme `kbss-probe-report.json`. Es de solo lectura: no instala nada, no pide administrador, no guarda imágenes, no registra usuario ni nombre de la red. Como no está firmada, Windows SmartScreen va a advertir ("Más información" → "Ejecutar de todas formas"). Si preferís no ejecutar un binario compilado por otro, la podés compilar vos mismo con `cargo run --release -p kb-probe-win`.
2. **Modelos de los Android** de prueba.
3. **Decisión sobre el driver** (Fase 2): ¿aprobás que escriba el código del driver IddCx y lo pruebe primero en una VM? No se instala nada en tu PC sin una segunda aprobación.
4. **Repositorio remoto**: este repositorio existe sólo en un contenedor temporal. Para no perder el trabajo hace falta un repositorio de GitHub donde subirlo.

## Próxima tarea
Con el informe de la sonda: cerrar F0-07/F0-08/F0-10 sobre datos reales, fijar los objetivos de rendimiento iniciales y empezar la Fase 1 (`kb-win-capture` + `kb-engine` + APK con descubrimiento, emparejamiento y video).

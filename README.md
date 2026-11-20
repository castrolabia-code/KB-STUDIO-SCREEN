# KB STUDIO SCREEN
**by Castro Labs**

Convierte hasta tres dispositivos Android (tablets o teléfonos, en cualquier combinación) en pantallas de una PC con Windows 10/11 x64. Cada dispositivo puede **duplicar** un monitor existente o **extender** el escritorio como monitor real, y se controla con el tacto sobre la interfaz original de Windows (por ejemplo, el mixer de Reason 14). Funciona en red local, sin Internet, sin cuentas y sin suscripciones. No transmite audio.

> **Estado: Fases 1 (Duplicar) y 2 (Extender) implementadas y probadas en Windows real (CI); PENDIENTE la validación en tu estudio** con Reason 14, la tablet EXO y el S23 FE. Seguí [docs/GUIA-PRUEBA.md](docs/GUIA-PRUEBA.md). Estado detallado: [docs/06-estado-fases-1-2.md](docs/06-estado-fases-1-2.md).

## Documentación

| Documento | Contenido |
|---|---|
| [00-comprension.md](docs/00-comprension.md) | Resumen de lo que se va a construir y lo que no |
| [01-factibilidad.md](docs/01-factibilidad.md) | Matriz de factibilidad, bloqueos, riesgos y mitigaciones |
| [02-arquitectura.md](docs/02-arquitectura.md) | Arquitectura, procesos, árbol del proyecto, decisiones (ADR) |
| [03-dependencias-licencias.md](docs/03-dependencias-licencias.md) | Dependencias, licencias, costos e instalación del entorno |
| [04-plan-fases.md](docs/04-plan-fases.md) | Plan por fases con criterios de salida |
| [05-estado-fase0.md](docs/05-estado-fase0.md) | Estado de la Fase 0 |
| [06-estado-fases-1-2.md](docs/06-estado-fases-1-2.md) | Estado de las Fases 1 y 2: IMPLEMENTADO / PROBADO / PENDIENTE / BLOQUEADO |
| [GUIA-PRUEBA.md](docs/GUIA-PRUEBA.md) | Guía paso a paso para probar en tu estudio |

## Estructura

```
core/                      Workspace Rust (mismo núcleo en la PC y en Android)
  crates/kb-geometry       Mapeo de toques: letterbox, zoom local, rotación, multi-monitor, rescate de ventanas
  crates/kb-proto          Protocolo v1 versionado, framing con límites, validación
  crates/kb-pairing        PIN de un solo uso (SPAKE2 + channel binding), QR, confianza, fichas, rate limit
  crates/kb-transport      Transporte abstracto (USB a futuro) + QUIC/TLS 1.3 mutuo con huella fijada
  crates/kb-session        Sesiones host/cliente: emparejamiento, 3 ranuras, video, entrada, Extender
  crates/kb-win            Windows: Desktop Duplication, H.264 Media Foundation, SendInput, DPAPI, driver
  crates/kb-engine         kbss-engine.exe (host, consola en el MVP)
  crates/kb-android        libkbcore.so (JNI) para la app
  crates/kb-probe-win      Sonda de diagnóstico (solo lectura)
android/                   App Kotlin + Jetpack Compose (Android 10+)
driver/kbss-idd/           Driver IddCx de pantallas virtuales (C++, WDK por NuGet)
docs/                      Documentación, evidencias y guía de prueba
.github/workflows/ci.yml   Linux + Windows real + APK + driver instalado en Windows de CI
```

## Compilar y probar (Linux, macOS o Windows con Rust ≥ 1.80)

```bash
cd core && cargo test
```

```bash
cd core && cargo run --release -p kb-transport --example loopback_bench -- 3 60 40 5
```

## Sonda de Windows (Fase 0)

Compilarla en la propia PC (requiere Rust con el toolchain MSVC):

```bash
cd core && cargo run --release -p kb-probe-win
```

O compilarla en Linux para Windows:

```bash
rustup target add x86_64-pc-windows-gnu && sudo apt-get install -y mingw-w64 && cd core && cargo build --release -p kb-probe-win --target x86_64-pc-windows-gnu
```

La sonda es de solo lectura: no instala nada, no requiere administrador y no guarda imágenes. Genera `kbss-probe-report.json`.

## Licencia

Código propietario de Castro Labs (pendiente de definir licencia comercial). Las dependencias de terceros y sus licencias están en [docs/03-dependencias-licencias.md](docs/03-dependencias-licencias.md).

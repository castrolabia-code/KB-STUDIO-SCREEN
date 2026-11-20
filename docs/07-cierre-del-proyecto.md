# 07 — Cierre del proyecto (versión 0.7.0)

**KB STUDIO SCREEN — by Castro Labs** · 2026-11-20

Este documento contrasta el producto entregado con los **criterios de aceptación** del pedido original (sección 10). Leyenda: **CUMPLIDO** (probado, con evidencia) · **CUMPLIDO EN CI** (probado automáticamente en Windows real / emulador Android) · **VALIDADO POR EL USUARIO** (probado en el estudio) · **PENDIENTE EXTERNO** (requiere hardware, dinero o trámites fuera de este desarrollo).

## Entregables

| Entregable | Dónde |
|---|---|
| Instalador de Windows x64 (con regla de firewall acotada, accesos, desinstalación limpia) | `descargas/KBStudioScreen-Setup.exe` |
| App Android 10+ (firmada con clave de prueba) | `descargas/KBStudioScreen.apk` |
| Motor portable y diagnóstico | `descargas/kbss-engine.exe`, `descargas/kb-probe-win.exe` |
| Driver de pantallas virtuales (firma de prueba) | `driver/kbss-idd/` (se compila e instala en CI) |
| Manual de uso | `docs/MANUAL.md` (también dentro del instalador) |
| Código fuente, pruebas, CI | este repositorio; `.github/workflows/ci.yml` |

## Criterios de aceptación

| # | Criterio | Estado | Evidencia |
|---|---|---|---|
| 1 | Windows 10 y 11 x64: compilación, instalación, reinicio si el driver lo exige, desinstalación y restauración | **CUMPLIDO EN CI** (Windows 11 24H2 / Server 2025). Windows 10: compatible por diseño (IddCx 1.4, APIs desde 1809), **sin prueba en un equipo con Windows 10** | Job `windows`: instalación, motor escuchando, desinstalación limpia. Job `driver`: instala/retira el driver sin reinicio |
| 2 | APK instalable en Android 10+ y en dispositivos de dos tamaños | **CUMPLIDO EN CI** (emulador Android 10) + **VALIDADO POR EL USUARIO** (teléfono Samsung y tablet EXO) | `docs/evidencia/android-e2e-emulador-api29.txt`, `app-inicio-emulador.png` |
| 3 | Un Android duplicando un monitor, imagen legible, fader con arrastre continuo | **VALIDADO POR EL USUARIO** con Reason 14 ("funciona perfecto") + CI | e2e `pin_pairing_video_and_fader_drag_end_to_end`; `captura-h264-runner-win11.png` |
| 4 | Un Android como monitor extendido reconocido por Windows, ventana arrastrable, tacto mapeado | **CUMPLIDO EN CI** (monitor virtual reconocido, capturado y retirado; mapeo probado). En la PC del usuario: **PENDIENTE EXTERNO** (firma del driver) | `driver-kbss-idd-runner-ci.txt`; e2e `extend_…` |
| 5 | Dos y hasta tres dispositivos, mediciones de FPS/latencia/CPU/RAM/GPU | Dos dispositivos **VALIDADOS POR EL USUARIO**; tres **CUMPLIDO EN CI** (lógica, ranuras, arbitraje). Mediciones con la GPU del estudio: **PENDIENTE EXTERNO** (el informe de `kb-probe-win` nunca se recibió) | e2e `three_slots…`; `loopback_bench` |
| 6 | Solo lectura no puede enviar entradas; cliente no autorizado rechazado | **CUMPLIDO** | e2e `view_only_…`, `wrong_pin_…`, `quic_security` (MITM, huella ajena) |
| 7 | PDF visible en la tablet del cantante mientras Reason sigue usable | **VALIDADO POR EL USUARIO** + CI (el emulador abre y dibuja el PDF) | e2e `singer_receives_document…`; `EndToEndTest` |
| 8 | Desconectar un monitor virtual no cierra Reason ni deja ventanas inaccesibles | **CUMPLIDO EN CI** (retiro, watchdog, topología restaurada, rescate de ventanas); con Reason: **PENDIENTE EXTERNO** (firma del driver) | `driver-kbss-idd-runner-ci.txt` |
| 9 | Rendimiento: objetivos medibles tras benchmark | Transporte medido (p95 < 1 ms en loopback); captura 7,5 ms y codificación 3 ms por frame en el runner de CI (software, 1024×768). Con tu GPU: **PENDIENTE EXTERNO** | `05-estado-fase0.md`, `06-estado-fases-1-2.md` |
| 10 | Sin audio, sin Internet, sin suscripciones | **CUMPLIDO** (no existe ningún mensaje de audio en el protocolo; todo es local) | `kb-proto` |
| 11 | Comandos, botones y pantallas funcionales; pruebas automatizadas; checklist manual | **CUMPLIDO**: 45 pruebas Rust + 3 sobre Windows real + 2 en emulador + ciclo del instalador; guía manual | `docs/GUIA-PRUEBA.md` |

## Lo que queda fuera de este desarrollo (y por qué)

Estas tareas no se pueden resolver con código: dependen de **dinero, trámites o hardware** que sólo vos podés aportar.

1. **Firma digital** del programa y del driver.
   - Sin firma, Windows muestra "Windows protegió su PC" (se acepta con "Ejecutar de todas formas").
   - El modo **Extender** no se puede instalar en una PC con Secure Boot.
   - Solución: certificado **EV** (≈ USD 250–600/año) y firma de **Microsoft (atestación)** del driver. Con eso, el instalador puede incluir el driver y Extender funciona en cualquier PC.
2. **Medición con la GPU del estudio**: ejecutar *Diagnóstico del equipo* (menú Inicio) y analizar el informe para afinar bitrates y FPS por placa de video (hoy se usa el encoder por hardware si existe, con respaldo por software).
3. **Prueba con tres dispositivos físicos a la vez** y con **Windows 10** real.
4. **Publicación de la APK**: para venderla fuera de la tienda, registro de Castro Labs como desarrollador en Google (USD 25) y clave de firma propia (hoy usa la clave de prueba).
5. **Derechos de la foto de fondo**: confirmar la licencia de uso comercial de `design/estudio_foto.jpg` (si no, usar la ilustración propia `design/estudio_kbss.svg`).
6. **Verificación del INF del driver** (`infverif`) con el WDK completo instalado (el paquete NuGet no lo incluye).

## Mapa del código

| Módulo | Función |
|---|---|
| `core/crates/kb-proto` | Protocolo versionado y validado |
| `core/crates/kb-pairing` | Emparejamiento PIN/QR (SPAKE2), confianza, fichas, límites |
| `core/crates/kb-transport` | QUIC/TLS 1.3 mutuo con huella fijada; capa abstracta (USB a futuro) |
| `core/crates/kb-geometry` | Mapeo exacto de toques, rotación, zoom, multi-monitor, rescate de ventanas |
| `core/crates/kb-session` | Sesiones host/cliente, 3 ranuras, video, Extender, perfiles en vivo, modo cantante |
| `core/crates/kb-win` | Captura DXGI, H.264 Media Foundation, SendInput, cursor, DPAPI, driver |
| `core/crates/kb-engine` | `kbss-engine.exe`: motor + panel de control local + perfiles |
| `core/crates/kb-android` | Núcleo Rust de la app (JNI) |
| `android/` | App Kotlin + Jetpack Compose |
| `driver/kbss-idd` | Driver IddCx de pantallas virtuales (MS-PL) |
| `installer/` | Instalador NSIS |

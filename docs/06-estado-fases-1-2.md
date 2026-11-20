# 06 — Estado de las Fases 1 (Duplicar) y 2 (Extender)

Fecha: 2026-11-20 · Repositorio: `castrolabia-code/KB-STUDIO-SCREEN` · CI: GitHub Actions (Ubuntu + Windows Server 2025 / build 26100, "Test Mode", sin GPU).

**Resumen honesto:** el sistema completo está implementado y la mayor parte está probada automáticamente: captura, codificación, entrada y el driver de monitores virtuales **sobre Windows real**, y la app **en un emulador Android 10**. Pero la app **todavía no se ejecutó en un dispositivo físico** (EXO, S23 FE) y nada se probó aún con Reason 14, tu Wi-Fi ni tus equipos. El producto **no está terminado**: faltan las pruebas de aceptación en tu estudio ([GUIA-PRUEBA.md](GUIA-PRUEBA.md)).

## Leyenda
**PROBADO-CI** = prueba automatizada en verde en GitHub Actions (Windows real o Linux) · **PROBADO-LOCAL** = prueba en esta sesión (Linux) · **PENDIENTE** = falta validar en equipo real · **NO IMPLEMENTADO** = trabajo de fases siguientes.

## Fase 1 — Duplicar

| Función | Estado | Evidencia |
|---|---|---|
| Protocolo v1, validación, límites | PROBADO-LOCAL + CI | 6 pruebas `kb-proto` |
| Emparejamiento QR / PIN (SPAKE2 + channel binding), aprobación en el host, revocación | PROBADO-LOCAL + CI | 8 pruebas `kb-pairing` + 4 de transporte + e2e |
| Sesión completa host↔cliente sobre QUIC real (loopback): PIN, video, arrastre de fader, cambio de monitor, reconexión con huella, revocación | PROBADO-LOCAL + CI | `kb-session/tests/e2e.rs` |
| Solo lectura no inyecta ni cambia de monitor | PROBADO-LOCAL + CI | e2e `view_only_singer_…` |
| PIN incorrecto y rechazo del usuario del host | PROBADO-LOCAL + CI | e2e `wrong_pin_and_host_rejection…` |
| 3 ranuras, la 4.ª rechazada con mensaje claro; un solo cursor (arbitraje) | PROBADO-LOCAL + CI | e2e `three_slots…` (encontró y corrigió un bug: el rechazo cerraba la conexión antes de enviar el motivo) |
| Captura Desktop Duplication del monitor real | **PROBADO-CI (Windows real)** | 1024×768 en 7,5 ms |
| BGRA→NV12 con promedio de área | PROBADO-CI | 2,98 ms/frame (1024×768, CPU del runner) |
| H.264 por software (Media Foundation) | **PROBADO-CI** | 2,95 ms/frame promedio; stream decodificado sin errores por ffmpeg (Constrained Baseline); imagen: [evidencia/captura-h264-runner-win11.png](evidencia/captura-h264-runner-win11.png) |
| H.264 por hardware (NVIDIA/AMD/Intel) | **PENDIENTE** | Implementado con respaldo automático a software; el runner no tiene GPU |
| `SendInput` al píxel exacto | **PROBADO-CI** | objetivo (341,192) = cursor real (341,192) |
| Topología con nombres reales de monitores | PROBADO-CI (1 monitor) · **PENDIENTE** con tus 3+ monitores y DPI mixto | |
| DPAPI para identidad y lista de confianza | PROBADO-CI | `dpapi_roundtrip` |
| mDNS (anuncio del host) | Implementado · **PENDIENTE** en tu red | |
| `kbss-engine.exe` (consola: QR, PIN, aprobación, lista, revocar, permisos, rescatar ventanas) | Compila (MSVC, CI) · **PENDIENTE** de uso real | |
| App Android en emulador Android 10 (API 29): carga del núcleo por JNI, emparejamiento QR sobre QUIC, decodificación MediaCodec a Surface (≥ 30 frames recibidos, ≥ 10 decodificados), toques con letterbox y arrastre recibidos por el host, arranque de la pantalla principal | **PROBADO-CI (emulador)** | [evidencia/android-e2e-emulador-api29.txt](evidencia/android-e2e-emulador-api29.txt) |
| API del cliente (la que usa JNI) contra el host de demostración | PROBADO-LOCAL | `client_smoke`: QR, 20 frames, toque en barra descartado, y=400 → y=360 correcto |
| App en el teléfono (S23 FE): conexión, visor y control de Reason | **VALIDADO EN EQUIPO REAL** (usuario) | |
| App en la tablet EXO 10"; PIN; solo lectura; reconexión; métricas de FPS/latencia | **PENDIENTE** | Pruebas B3–B11 de la guía |
| Fader de Reason 14 con arrastre continuo | **VALIDADO EN EQUIPO REAL** (2026-11-20, usuario) | Teléfono Samsung → PC del estudio: pantalla visible y faders de Reason controlados "perfecto" |
| Latencia y FPS sobre tu Wi-Fi, 720p/1080p, 15/30/60 FPS | **PENDIENTE** | Prueba B11 |
| Cursor del mouse dibujado en la imagen | **NO IMPLEMENTADO** | Desktop Duplication no incluye el cursor; falta componerlo |
| UI gráfica del host (Tauri) | **NO IMPLEMENTADO** | El MVP usa consola |

## Fase 2 — Extender

| Función | Estado | Evidencia |
|---|---|---|
| Driver IddCx `kbss-idd` compila (WDK NuGet 10.0.28000, /W4 con advertencias como error) | **PROBADO-CI** | |
| ApiValidator (driver universal) | **PROBADO-CI** | |
| InfVerif | **PENDIENTE** | El WDK por NuGet no lo trae ejecutable; correr `infverif /u` con el WDK completo |
| Instalación (`pnputil`), adaptador `SWD\KbssIdd\KbssIdd` | **PROBADO-CI** | firma de PRUEBA, máquina en Test Mode |
| Monitor virtual con la resolución real (1280×800) reconocido por Windows | **PROBADO-CI** | visible en 252 ms como "KBSS Pantalla", a la derecha del principal |
| Captura del monitor virtual | **PROBADO-CI** | primer frame en 8 ms |
| Resolución no estándar (2340×1080, S23 FE) | **PROBADO-CI** | |
| Validación de entradas del driver (ranura ocupada/fuera de rango, tamaño inválido) | **PROBADO-CI** | |
| Retiro de monitor y restauración de la topología | **PROBADO-CI** | 16 ms |
| Watchdog: motor colgado ⇒ el driver retira los monitores | **PROBADO-CI** | 6,3 s |
| Desinstalación limpia | **PROBADO-CI** | evidencia: [evidencia/driver-kbss-idd-runner-ci.txt](evidencia/driver-kbss-idd-runner-ci.txt) |
| Control sin administrador (ACL del dispositivo para usuarios interactivos) | Implementado · **PENDIENTE** de verificar con un usuario estándar (el runner es administrador) | |
| Extender desde Android con confirmación del host; gracia de 30 s ante cortes; retiro al volver a Duplicar | PROBADO-LOCAL + CI (lógica, plataforma simulada) | e2e `extend_…` |
| Ventana arrastrada desde un monitor físico al Android y tocada | **PENDIENTE** | Prueba C2 |
| Reason no se cierra ni queda inaccesible al quitar el monitor | **PENDIENTE** con Reason; el rescate de ventanas está implementado | Prueba C3 |
| Posición configurable (izquierda/arriba/abajo) | **NO IMPLEMENTADO** | Windows lo ubica a la derecha; se puede mover desde Configuración → Pantalla |
| **Instalar en TU PC con Secure Boot** | **BLOQUEADO** por la firma | Ver abajo |

### El bloqueo de la firma del driver
El driver está firmado con un certificado de **prueba** y sólo se probó en una máquina en *Test Mode*. Para tu PC de estudio hay tres caminos (detalle en [GUIA-PRUEBA.md](GUIA-PRUEBA.md), sección C):
1. **Test Mode** (requiere desactivar Secure Boot): no recomendado en la PC de producción.
2. **Certificado propio de confianza con Secure Boot activo:** hipótesis sin validar (tarea F2-01).
3. **Firma por atestación de Microsoft** con certificado EV: el camino del producto comercial, con costo anual.

**Duplicar no necesita el driver**, así que podés probar toda la Fase 1 ya.

## Fase 4 — Modo cantante (adelantado)

| Función | Estado | Evidencia |
|---|---|---|
| Envío de PDF / texto desde la PC al dispositivo en solo lectura (cifrado, LAN, fragmentos de 12 KB, máx. 64 MB) | PROBADO-LOCAL + CI | e2e `singer_receives_document…`: llega íntegro sólo al cantante |
| Recepción y apertura del PDF con `PdfRenderer` en Android 10 | PROBADO-CI (emulador) | `EndToEndTest`: 2 páginas, texto dibujado |
| Teleprompter: páginas, avance automático 0–10, contraste (3 temas, PDF invertido), tamaño de letra, bloqueo desde la PC, cierre con borrado del archivo | Implementado · **PENDIENTE** en la tablet EXO | Guía, sección D |
| Comandos de consola `d`, `n`, `b`, `a N`, `k`, `c` | Implementado · **PENDIENTE** en tu PC | |

## Números de las pruebas
- Rust: 37 pruebas en Linux (unitarias + integración + e2e), más 3 pruebas sobre Windows real (captura/codificación/entrada, DPAPI, driver). Clippy sin advertencias en Linux y Windows.
- CI: 5 jobs (Linux, Windows, APK, driver en Windows, app en emulador Android 10), todos en verde.

## Próximas tareas
1. **Tus pruebas** con la [guía](GUIA-PRUEBA.md), sobre todo B5 (fader de Reason) y B11 (rendimiento).
3. Dibujar el cursor en la imagen y ruta de codificación 100 % GPU.
4. Fase 3: perfiles (Grabación vocal / Mezcla / Mastering), UI del host, posición del monitor virtual.
5. F2-01: validar la instalación del driver con Secure Boot activo usando un certificado propio.

# 01 — Factibilidad, bloqueos y riesgos

Leyenda de evidencia: **[P]** probado en esta sesión · **[D]** documentación oficial / conocimiento técnico, pendiente de prueba en equipo real · **[H]** hipótesis que requiere prueba específica.

## 1. Matriz de factibilidad

| # | Capacidad | Tecnología elegida | Factible | Evidencia | Prueba que la valida | Estado |
|---|---|---|---|---|---|---|
| F-01 | Enumerar monitores (nombre, resolución, posición, DPI, orientación) | DXGI `EnumOutputs` + `GetDpiForMonitor` (proceso Per-Monitor DPI Aware v2) | Sí | [D] | `kb-probe-win` → `adapters[].outputs[]` | Sonda lista, sin ejecutar |
| F-02 | Capturar un monitor completo con baja latencia | **DXGI Desktop Duplication** (Win8+): textura GPU, rectángulos modificados, sin borde | Sí | [D] | `kb-probe-win` → `desktop_duplication.ok`; benchmark F1 | Sonda lista, sin ejecutar |
| F-03 | Capturar una ventana individual | Windows Graphics Capture (1803+) | Parcial | [D] En Win10 WGC dibuja un **borde amarillo** obligatorio; sólo Win11 permite quitarlo (`IsBorderRequired`) | `capture.wgc_border_optional` | No se necesita para el MVP (ver nota 1) |
| F-04 | Monitor virtual real reconocido por Windows | **Driver IddCx (UMDF)** basado en el `IddSampleDriver` de Microsoft (MIT) | Sí, con costos de firma | [D] | Fase 2: instalar en VM Hyper-V con Secure Boot → monitor aparece en Configuración > Pantalla | **Requiere tu aprobación** |
| F-05 | Firma del driver para uso personal | Certificado autofirmado en *Trusted Root* + *Trusted Publishers* de la máquina | Probable | [H] Es el método que usaron controladores IDD de código abierto; hay que confirmarlo con Secure Boot activo en Win10 y Win11 | Prueba F2-01 en VM | PENDIENTE |
| F-06 | Firma del driver para distribución comercial | Firma de atestación de Microsoft (Partner Center), requiere certificado **EV** | Sí, con costo | [D] | Trámite, no técnico | Fase 5 |
| F-07 | Codificación H.264 por hardware (NVIDIA/AMD/Intel) | Media Foundation: MFT de hardware vía `MFTEnumEx`; respaldo con el MFT de software de Microsoft | Sí | [D] | `kb-probe-win` → `h264_encoders` | Sonda lista, sin ejecutar |
| F-08 | Decodificación con baja latencia en Android 10+ | `MediaCodec` → `SurfaceView`; `KEY_LOW_LATENCY` desde Android 11 | Sí | [D] | Fase 1 en dispositivo real | PENDIENTE |
| F-09 | Toque → clic/arrastre en apps de escritorio | `SendInput` mouse absoluto (`MOUSEEVENTF_VIRTUALDESK`) | Sí | [D] + geometría [P] | Fase 1: fader de Reason con arrastre continuo | PENDIENTE |
| F-10 | Multitáctil nativo | `InjectSyntheticPointerInput` (`PT_TOUCH`, Win10 1809+) | Sí a nivel API; **depende de la app** | [D] | `touch_injection.synthetic_pointer_touch`; prueba con app táctil y con Reason | PENDIENTE (riesgo R-04) |
| F-11 | Transporte seguro de baja latencia | **QUIC** (`quinn`) + TLS 1.3 mutuo con huellas fijadas; 1 stream por frame | Sí | **[P]** loopback: 3 clientes × 60 FPS × 40 KiB, p95 ≈ 0,33 ms, 0 pérdidas | `loopback_bench`; Wi-Fi en Fase 1 | Probado en loopback |
| F-12 | Emparejamiento PIN/QR resistente a MITM | SPAKE2 + confirmación HMAC con huellas de ambos certificados | Sí | **[P]** prueba de MITM con PIN correcto → rechazado | `cargo test` | PROBADO |
| F-13 | Solo lectura no puede inyectar entrada | Filtro único `authorize_inbound` en el host | Sí | **[P]** | `cargo test` | PROBADO (lógica) |
| F-14 | Mapeo exacto de coordenadas | `kb-geometry` (letterbox, rotación, zoom local, coordenadas negativas) | Sí | **[P]** unitario | Fase 1 con monitores reales y DPI mixto | Probado unitario |
| F-15 | Descubrimiento LAN | mDNS/DNS-SD: `DnsServiceRegister` (Win10 1809+) en el host, `NsdManager` en Android; QR o IP manual como respaldo | Sí | [D] `NsdManager` tiene fallos conocidos en algunos fabricantes → el respaldo es obligatorio | Fase 1 | PENDIENTE |
| F-16 | Reubicar ventanas que quedan fuera de pantalla | `EnumWindows` + `SetWindowPos` en la sesión del usuario; lógica en `Topology::rescue_window` | Sí | **[P]** lógica | Fase 2: desconectar el monitor virtual con Reason abierto | Probado unitario |
| F-17 | PDF/letras para el cantante | (a) mover el visor PDF de Windows al monitor extendido; (b) teleprompter propio renderizado en Android con `PdfRenderer` (incluido en Android) | Sí | [D] | Fase 4 | PENDIENTE |
| F-18 | Uso sin administrador | Motor en la sesión interactiva del usuario; el driver expone una interfaz con ACL para usuarios interactivos | Sí | [D] | Fase 2 | PENDIENTE |

**Nota 1.** El caso "mostrar cualquier ventana en la tablet del cantante" se resuelve en modo Extender moviendo esa ventana al monitor virtual, que se captura completo con Desktop Duplication (sin borde). La captura de ventana individual con WGC queda como función opcional para Windows 11.

## 2. Bloqueos actuales

| ID | Bloqueo | Qué lo destraba |
|---|---|---|
| B-01 | Esta sesión no tiene acceso a tu PC (no hay equipo conectado; contenedor Linux sin GPU ni monitores) | Ejecutar `kb-probe-win.exe` en tu PC y compartir el JSON, o conectar tu PC a la sesión con Remote Control desde la app de escritorio de Claude |
| B-02 | Sin dispositivos Android reales | Indicar modelos (tablets y celular) para la prueba de Fase 1; instalar la APK por ADB |
| B-03 | Driver IddCx: **no se escribe ni se instala sin tu aprobación explícita** (lo pediste así) | Tu aprobación para (1) escribir el código del driver y (2) probarlo primero en una VM Hyper-V |
| B-04 | WDK y Visual Studio: el driver sólo compila con el toolchain MSVC + WDK en Windows | Instalarlos en tu PC (pasos en 03) o en un runner de CI Windows |

## 3. Riesgos y mitigaciones

| ID | Riesgo | Prob. | Impacto | Mitigación |
|---|---|---|---|---|
| R-01 | **Firma del driver**: sin firma de Microsoft, otros usuarios no pueden instalarlo con Secure Boot | Alta | Alta (comercial) | Uso personal: certificado autofirmado (validar F-05). Comercial: certificado EV + atestación. Presupuestar en Fase 5. Alternativa temporal para pruebas: un IDD de código abierto con licencia MIT ya firmado, sólo en entornos de prueba. |
| R-02 | Desktop Duplication sobre monitores IDD o en notebooks con GPU híbrida falla (`DXGI_ERROR_UNSUPPORTED`) | Media | Alta | Crear el dispositivo D3D11 en el adaptador dueño de la salida (la sonda ya lo hace así). Plan B: el driver IDD entrega directamente los frames de su *swapchain* por memoria compartida. |
| R-03 | **Texto fino de color borroso** por el submuestreo de croma 4:2:0 de H.264 (la interfaz del DAW tiene texto chico de colores) | Alta | Media | Perfil "Letras": más bitrate y resolución nativa, pocos FPS. Medir legibilidad en Fase 1. Más adelante: HEVC o 4:4:4 si el decodificador del dispositivo lo admite (no se asume). |
| R-04 | **Reason 14 probablemente procesa sólo mouse**: el tacto se convierte en mouse, hay un único cursor y no se pueden mover dos faders a la vez | Alta | Media | Modo por defecto para DAW: **mouse con arrastre sostenido** (determinista, sin el retardo de detección de gestos). Modo "táctil nativo" opcional para apps multitáctiles. No se promete multi-fader. Validar con Reason real. |
| R-05 | Conflicto de cursor: el productor usa el mouse físico mientras un Android inyecta | Media | Media | Arbitraje: mientras dura un arrastre táctil se bloquea la inyección de otros clientes; los clientes de solo lectura nunca mueven el cursor. |
| R-06 | **UIPI**: un proceso sin elevar no puede inyectar en ventanas elevadas | Media | Media | Diagnóstico que detecta Reason corriendo como administrador y lo avisa. No se eleva el motor. (`uiAccess` firmado queda como opción futura.) |
| R-07 | Wi-Fi: interferencia en 2,4 GHz, aislamiento de clientes (redes de invitados), PC por cable y tablet en otra subred | Alta | Alta | Asistente de diagnóstico (alcance, RTT, pérdida, subred); recomendar 5 GHz; adaptación por cliente (bitrate → FPS → resolución). |
| R-08 | Firewall: perfil de red "Pública" bloquea conexiones entrantes | Alta | Alta | El instalador crea una regla acotada al ejecutable del motor, UDP, sólo perfiles Privado/Dominio. Nunca desactivar el firewall global. El asistente detecta el perfil (la sonda ya lo lee). |
| R-09 | Límite de sesiones simultáneas de NVENC en GeForce | Media | Media | Codificar una vez por combinación (origen, resolución, FPS) y compartir entre clientes iguales; respaldo por software para el tercer cliente, con FPS reducidos. Medir con el GPU real. |
| R-10 | CPU/GPU compitiendo con el DAW en una PC de 8 GB | Media | Alta | Codificación por hardware, prioridad de proceso normal, límite de FPS, escalado en GPU, presupuesto de memoria medido en Fase 1. |
| R-11 | Windows 10 22H2 está fuera de soporte desde octubre de 2025 (sólo actualizaciones extendidas, ESU) | Cierta | Media | Se mantiene compatibilidad con Win10 22H2 como objetivo, priorizando Win11. Confirmar la versión de IddCx en cada build con la sonda. |
| R-12 | Android: `NsdManager` poco confiable; restricciones nuevas de acceso a la red local en versiones recientes | Media | Media | QR con IP + puerto y entrada manual como respaldo obligatorio. Revisar los permisos de red local al fijar el `targetSdk`. |
| R-13 | Distribución de la APK fuera de Google Play: Google está desplegando la verificación obligatoria de desarrolladores para apps instaladas por fuera de la tienda (2026–2027 según el país) | Media | Media (comercial) | Uso personal: instalación por ADB. Comercial: registrar a Castro Labs como desarrollador. Verificar el calendario para Argentina antes de la Fase 5. |
| R-14 | SmartScreen advierte sobre ejecutables sin firmar | Cierta | Baja (personal) / Alta (comercial) | Firma de código (certificado OV/EV o servicio de firma en la nube) en Fase 5. Verificar disponibilidad para Argentina. |
| R-15 | Patentes de H.264 | Baja | Media (comercial) | No se incluye ningún códec propio: se usan los codificadores del sistema operativo (Media Foundation) y los decodificadores del dispositivo (MediaCodec). Consultar con un abogado antes de vender. |

## 4. Decisiones fundamentadas (resumen; detalle en 02)
1. **Captura**: Desktop Duplication para monitores (Win10 y Win11, sin borde); WGC sólo para ventanas en Win11.
2. **Monitor virtual**: driver IddCx mínimo (crea y quita monitores, publica modos/EDID). El driver **no** captura; el motor captura el monitor virtual igual que uno físico. Así el driver es chico y el riesgo de inestabilidad baja.
3. **Transporte**: QUIC con protocolo propio. Se descartó WebRTC (ver ADR-03).
4. **Entrada**: mouse por defecto para DAW; táctil nativo opcional.
5. **Núcleo compartido en Rust** para host y Android (una sola implementación del protocolo y la seguridad).

# 02 — Arquitectura

## 1. Procesos y límites de privilegio

```
┌──────────────────────────── PC Windows (sesión interactiva del usuario) ────────────────────────────┐
│                                                                                                   │
│  kbss-ui.exe  (Tauri 2 / WebView2)          kbss-engine.exe  (Rust, integridad media, SIN admin)  │
│  ─ panel de 3 ranuras, perfiles,     ◄────► ─ monitores/topología      ─ captura DXGI DD          │
│    aprobación de emparejamiento     tubería ─ codificación MF H.264    ─ inyección de entrada     │
│  ─ diagnóstico, ajustes             con nombre ─ QUIC/TLS (kb-transport) ─ confianza (DPAPI)      │
│                                     (ACL: usuario actual)       │                                   │
│                                                                  │ IOCTL (interfaz de dispositivo,  │
│                                                                  ▼ ACL: usuarios interactivos)      │
│                                              KBSS IDD driver (UMDF/IddCx, corre en WUDFHost)        │
│                                              ─ sólo agrega/quita monitores virtuales y modos        │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
          ▲ instalación única con administrador: driver + regla de firewall (kbss-setup.exe)
          │
          │ LAN Wi-Fi · QUIC/UDP · TLS 1.3 mutuo con huellas fijadas
          ▼
┌──────────────────────── Android 10+ ────────────────────────┐
│ App Kotlin + Jetpack Compose                                │
│ ─ descubrimiento (NsdManager), QR (CameraX + ZXing), ajustes│
│ ─ MediaCodec → SurfaceView (decodificación por hardware)    │
│ ─ gestos: navegación local vs. entrada para Windows         │
│ libkbcore.so (Rust vía JNI): kb-proto, kb-pairing,          │
│   kb-transport, kb-geometry — MISMO código que el host      │
└─────────────────────────────────────────────────────────────┘
```

**Por qué el motor no es un servicio de Windows.** Desktop Duplication y `SendInput` funcionan sólo en la sesión interactiva del usuario; un servicio corre en la sesión 0 y no ve el escritorio. El motor corre como proceso del usuario, sin elevar. Lo único privilegiado (instalar el driver y crear la regla de firewall) ocurre **una sola vez** en el instalador, con aviso previo.

**Por qué la UI va en un proceso aparte.** Si la UI se cierra o falla, los streams siguen. El motor queda chico y auditable.

## 2. Flujo de un frame
1. `IDXGIOutputDuplication::AcquireNextFrame` → textura BGRA en GPU (sin copiar a CPU).
2. Conversión BGRA → NV12 y escalado en GPU (D3D11 Video Processor).
3. MFT H.264 por hardware: modo baja latencia, CBR, sin B-frames, IDR periódico y a pedido.
4. Se codifica **una vez por combinación (origen, resolución, FPS)**; los clientes con los mismos parámetros comparten el resultado.
5. `send_frame`: un stream QUIC unidireccional por frame. Si llega un frame nuevo y el anterior no terminó de enviarse, el viejo se descarta (perfil Mixer) y el cliente pide un keyframe.
6. Android: `MediaCodec` → `SurfaceView`, sin copias intermedias.

## 3. Flujo de un toque
`MotionEvent` → `ClientViewport::view_to_norm` (deshace el zoom local, el letterbox y la rotación) → `PointerEvent{NormPoint}` → host `authorize_inbound` (¿emparejado? ¿tiene permiso de control?) → `norm_to_desktop` **contra el monitor que el host asignó a esa ranura** → `SendInput` o `InjectSyntheticPointerInput`.
El cliente nunca envía coordenadas absolutas: no puede tocar un monitor que no está viendo.

## 4. Árbol del proyecto (objetivo; ✅ = existe hoy)

```
kb-studio-screen/
├─ core/                         Workspace Rust                               ✅
│  ├─ crates/kb-geometry         geometría y rescate de ventanas              ✅ probado
│  ├─ crates/kb-proto            protocolo v1                                 ✅ probado
│  ├─ crates/kb-pairing          emparejamiento y autorización                ✅ probado
│  ├─ crates/kb-transport        transporte abstracto + QUIC                  ✅ probado (loopback)
│  ├─ crates/kb-probe-win        sonda Fase 0                                 ✅ compila, sin ejecutar
│  ├─ crates/kb-win-capture      DXGI DD + MF encoder                         Fase 1
│  ├─ crates/kb-win-input        SendInput / InjectSyntheticPointerInput      Fase 1
│  ├─ crates/kb-win-display      topología, IOCTL al driver, rescate          Fase 1-2
│  ├─ crates/kb-engine           binario kbss-engine                          Fase 1
│  └─ crates/kb-android-jni      libkbcore.so                                 Fase 1
├─ host-ui/                      Tauri 2 (kbss-ui)                            Fase 1
├─ driver/kbss-idd/              UMDF IddCx (C++, WDK) — REQUIERE APROBACIÓN  Fase 2
├─ installer/                    WiX (MSI) o Inno Setup                       Fase 2/5
├─ android/                      Gradle, Kotlin, Compose                      Fase 1
├─ tools/                        scripts de diagnóstico y medición
└─ docs/                                                                      ✅
```

## 5. Protocolo v1 (implementado en `kb-proto`)
- Sobre: `{major, minor, msg}` en `postcard`, con prefijo `u32` de longitud y **límite de 16 KiB** validado antes de reservar memoria.
- `major` distinto → error explícito `IncompatibleVersion` (no se intenta interpretar).
- Todo mensaje entrante pasa por `Message::validate` (largos, rangos, caracteres de control).
- Video: cabecera fija de 21 bytes (`VideoChunkHeader`) + NAL Annex-B, límite de 8 MiB por frame.
- No existe ningún mensaje de audio.
- Mensajes: `ClientHello` (pantalla real y capacidades del decodificador, **negociadas, nunca supuestas**), `HostHello`, `PairStart/Reply/Confirm/Result`, `PairQrToken`, `SessionConfig` (ranura, modo, permiso, parámetros, ficha de reanudación), `ModeChangeRequest/Result` (el host siempre decide), `ViewportChanged`, `RequestKeyframe`, `Input`, `MonitorList`, `Identify`, `Ping/Pong`, `Stats`, `Error`, `Bye`.

## 6. Seguridad (implementada en `kb-pairing` y `kb-transport`)
| Elemento | Implementación |
|---|---|
| Identidad | Certificado autofirmado ECDSA P-256 por extremo (`rcgen`); clave protegida con DPAPI (Windows) y Android Keystore (clave de envoltura) |
| Canal | QUIC + TLS 1.3 únicamente, autenticación mutua obligatoria |
| Primer emparejamiento por QR | El QR trae la huella del host + un token de 128 bits de un solo uso que vence a los 5 min |
| Primer emparejamiento por PIN | PIN de 6 dígitos, 120 s, máx. 3 intentos, de un solo uso; SPAKE2 + HMAC sobre las huellas de **ambos** certificados (un MITM queda detectado aunque tenga el PIN) |
| Aprobación | Siempre explícita en el host, mostrando el nombre del dispositivo y el permiso elegido |
| Reconexión | Huellas fijadas en ambos lados + ficha HMAC de 10 min ligada al certificado |
| Autorización | `authorize_inbound`: no emparejado ⇒ sólo mensajes de emparejamiento; solo lectura ⇒ nunca `Input` |
| Revocación | Borrar la huella cierra la sesión y bloquea reconexiones |
| Abuso | Rate limit por IP (token bucket, falla cerrado bajo inundación), límite de 3 ranuras |
| Logs | Sin PIN, tokens, claves, letras, imágenes ni SSID |

## 7. Decisiones (ADR)

**ADR-01 — Rust para el núcleo y el host; Kotlin para Android.** Rust da seguridad de memoria en el código que parsea datos de la red, tiene acceso completo a Win32/WinRT a través de `windows-rs` (oficial de Microsoft) y compila a Android (NDK). C++ queda sólo para el driver, porque el WDK y las plantillas de IddCx son C/C++.

**ADR-02 — Núcleo compartido por JNI en Android.** Una sola implementación del protocolo, la criptografía y la geometría, probada una vez. Costo: integrar `cargo-ndk` en Gradle y una capa JNI fina. Alternativa descartada: reimplementar en Kotlin (dos implementaciones de la criptografía, divergencias).

**ADR-03 — QUIC propio en lugar de WebRTC.**
| Criterio | WebRTC (libwebrtc) | TLS/TCP | **QUIC (quinn)** |
|---|---|---|---|
| Latencia con pérdidas Wi-Fi | buena, pero con jitter buffer de videollamada | mala (bloqueo de cabeza de línea) | buena (un stream por frame, descarte) |
| Tamaño y mantenimiento | enorme, build complejo; Google ya no publica el AAR oficial de Android | mínimo | moderado, Rust puro |
| NAT/ICE/STUN | incluido, innecesario en LAN | — | — |
| Cifrado | DTLS-SRTP | TLS 1.3 | TLS 1.3 integrado |
| Mismo código en ambos extremos | no | sí | **sí** |
| Migración de conexión (roaming Wi-Fi) | ICE restart | no | sí |
Evidencia actual: loopback medido (ver 05). Pendiente: medición sobre Wi-Fi real antes de cerrar la decisión en Fase 1. Si QUIC rindiera mal sobre Wi-Fi real, la capa abstracta permite cambiar el transporte sin tocar el protocolo.

**ADR-04 — Desktop Duplication como captura principal.** Funciona en Win10 y Win11 sin borde amarillo y entrega rectángulos modificados (sirve para ahorrar trabajo en el perfil Letras). WGC sólo para ventanas individuales en Win11.

**ADR-05 — Codificadores del sistema (Media Foundation).** Detecta los encoders reales de NVIDIA, AMD e Intel sin incluir SDKs de fabricantes, y evita distribuir códecs con licencias GPL o con regalías.

**ADR-06 — Driver IddCx mínimo.** El driver sólo agrega y quita monitores; no procesa frames. Menos código privilegiado y menos riesgo de dejar Windows sin imagen.

**ADR-07 — UI con Tauri 2 (WebView2).** Diseño (grafito, glassmorphism moderado, Inter) y accesibilidad con lectores de pantalla maduras, i18n simple, backend Rust directo; MIT/Apache; WebView2 ya viene en Win10/11. Presupuesto: ≤ 150 MB de RAM, medido en Fase 1; si se excede, la alternativa es Slint.

**ADR-08 — Entrada para DAW en modo mouse.** Reason procesa, previsiblemente, sólo mouse. La conversión táctil→mouse de Windows añade detección de gestos (mantener presionado = clic derecho) que rompe arrastres finos. El modo mouse con arrastre sostenido es determinista. El táctil nativo queda como opción.

**ADR-09 — Teleprompter renderizado en Android.** Para letras/PDF propios: `PdfRenderer` de Android y texto nativo dan nitidez perfecta (sin compresión de video) y casi no usan ancho de banda. El documento se copia al almacenamiento privado de la app sólo durante la sesión y se borra al terminar. Sin nube.

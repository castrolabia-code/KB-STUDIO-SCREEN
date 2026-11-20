# 03 — Dependencias, licencias, costos e instalación

## 1. Dependencias en uso hoy (verificadas en `Cargo.lock`)

| Componente | Uso | Licencia | ¿Apta para producto comercial cerrado? |
|---|---|---|---|
| `serde`, `postcard` | serialización del protocolo | MIT / Apache-2.0 | Sí (incluir aviso) |
| `thiserror` | errores | MIT / Apache-2.0 | Sí |
| `sha2`, `hmac` (RustCrypto) | huellas, MAC | MIT / Apache-2.0 | Sí |
| `spake2` (RustCrypto) | PAKE para el PIN | MIT / Apache-2.0 | Sí |
| `subtle` | comparaciones en tiempo constante | BSD-3-Clause | Sí (incluir aviso) |
| `rand`, `base64` | aleatoriedad, QR | MIT / Apache-2.0 | Sí |
| `quinn` | QUIC | MIT / Apache-2.0 | Sí |
| `rustls` | TLS 1.3 | Apache-2.0 / ISC / MIT | Sí |
| `ring` | primitivas criptográficas | Apache-2.0 e ISC (más avisos heredados de BoringSSL/OpenSSL) | Sí (incluir avisos) |
| `rcgen` | certificados autofirmados | MIT / Apache-2.0 | Sí |
| `tokio` | async | MIT | Sí |
| `windows` (windows-rs, Microsoft) | Win32/WinRT | MIT / Apache-2.0 | Sí |
| `winreg` | registro (sonda) | MIT | Sí |
| `serde_json` | informe de la sonda | MIT / Apache-2.0 | Sí |

**Verificación realizada (`cargo metadata`, 92 dependencias transitivas):** 62 `MIT OR Apache-2.0`, 12 `MIT`, 2 `BSD-3-Clause`, 2 `ISC`, 1 `Apache-2.0 AND ISC` (ring), y el resto variantes permisivas (Zlib, Unlicense, Unicode-3.0, BSD-2). **Ninguna copyleft.**

Antes de cada entrega se generará el listado completo de dependencias transitivas con `cargo-about` o `cargo-deny`, con bloqueo automático de licencias copyleft (GPL/AGPL/LGPL) en el binario distribuido.

## 2. Dependencias previstas

| Componente | Uso | Licencia | Notas |
|---|---|---|---|
| `mdns-sd` | anuncio mDNS del host | MIT / Apache-2.0 | En uso desde Fase 1 |
| `qrcode` | QR en la consola del host | MIT / Apache-2.0 | En uso desde Fase 1 |
| `if-addrs` | direcciones de red local | MIT / BSD-3-Clause | En uso desde Fase 1 |
| `jni` | puente Rust ↔ Kotlin | MIT / Apache-2.0 | En uso desde Fase 1 |
| `zxing-android-embedded` 4.3.0 | lector de QR en Android | Apache-2.0 | En uso desde Fase 1 |
| AndroidX / Jetpack Compose (BOM 2025.06.00) | UI Android | Apache-2.0 | En uso desde Fase 1 |
| WDK/SDK NuGet 10.0.28000.2526 | compilar el driver | EULA de Microsoft (herramienta de build, no se redistribuye) | En uso desde Fase 2 |
| Tauri 2 | UI del host | MIT / Apache-2.0 | |
| WebView2 Runtime (Microsoft) | motor de la UI | Redistribuible gratuito | Preinstalado en Win10/11 actuales |
| Microsoft Windows-driver-samples (`IddSampleDriver`) | base del driver `driver/kbss-idd` | **MS-PL** (corrección: en la Fase 0 figuraba MIT por error) | Permite uso comercial y distribuir binarios bajo licencia propia; el código fuente derivado conserva el aviso MS-PL (`driver/kbss-idd/LICENSE-MS-PL.txt`) |
| WDK + Visual Studio Build Tools/Community | compilar el driver | EULA gratuita de Microsoft | Para uso comercial con equipos grandes, VS Community tiene límites: usar Build Tools o licencia Professional |
| Media Foundation, DXGI, D3D11 | captura y codificación | Parte de Windows | Sin regalías de códec propias |
| Kotlin, AndroidX, Jetpack Compose, CameraX | app Android | Apache-2.0 | |
| ZXing (núcleo) | lectura del QR | Apache-2.0 | Sin depender de Google Play Services |
| `jni` crate, `cargo-ndk` | puente Rust ↔ Kotlin | MIT / Apache-2.0 | |
| Android `PdfRenderer` | PDF en el teleprompter | Parte de Android | |
| PDFium (opcional, en el host) | miniaturas de PDF | BSD-3 / Apache-2.0 | Sólo si hace falta |
| Inter (tipografía, `third_party/fonts/inter`) | panel y app | SIL OFL 1.1 | En uso desde 0.6.0; incrustada en el panel y en la APK. Licencia incluida |
| WiX Toolset (instalador) | MSI | MS-RL | Sólo herramienta de build; no se distribuye código de WiX en el producto |

## 3. Costos posibles (todos opcionales para uso personal)

| Concepto | Necesario para | Costo aproximado (verificar al momento de comprar) |
|---|---|---|
| Certificado de firma de código EV | firmar el driver por atestación de Microsoft y evitar SmartScreen | ~USD 250–600 por año |
| Registro en Microsoft Partner Center (hardware) | atestación del driver | Gratuito, pero exige certificado EV |
| Servicio de firma en la nube de Microsoft (Trusted Signing / Artifact Signing) | firmar el .exe/.msi | ~USD 10 por mes; **verificar si está disponible para Argentina** |
| Cuenta de desarrollador de Google Play | publicar en Play o cumplir con la verificación de desarrolladores | USD 25, pago único |
| Asesoría legal (patentes H.264, términos de uso) | venta | Variable |

**Uso personal: costo cero.** Driver con certificado autofirmado (validar F-05), APK instalada por ADB, ejecutable sin firmar (aceptando el aviso de SmartScreen).

## 4. Instalación del entorno de desarrollo en tu PC (cuando llegue cada fase)

No se instala nada sin tu confirmación. Lista para que decidas:

**Fase 1 (sin cambios de sistema):**
1. Rust (rustup, toolchain `stable-x86_64-pc-windows-msvc`) y Visual Studio 2022 Build Tools con "Desarrollo de escritorio con C++" y Windows SDK.
2. Android Studio (incluye JDK y Android SDK) o sólo las command-line tools + `adb`.
3. Activar "Depuración USB" en las tablets/teléfonos de prueba.

**Fase 2 (driver; requiere tu aprobación explícita):**
4. WDK que coincida con la versión del Windows SDK instalada.
5. Hyper-V (Windows Pro/Enterprise) o VirtualBox con una VM de Windows 11 y otra de Windows 10 con Secure Boot, para probar el driver **antes** de tocar tu PC de estudio.
6. Instalación del certificado de prueba en la VM; en tu PC, sólo después de validar en la VM.

## 5. Reproducir lo hecho en esta sesión

```bash
cd core && cargo test
```

```bash
cd core && cargo build --release -p kb-probe-win --target x86_64-pc-windows-gnu
```

Toolchain usada: Rust 1.97.0, target `x86_64-pc-windows-gnu`, MinGW-w64 GCC 13.2 (Ubuntu).

## Estética
La estética "Studio" (0.6.0) está inspirada en una sala de control de un estudio de grabación. La ilustración de fábrica (`design/estudio_kbss.svg`) es un dibujo original de Castro Labs. El usuario puede elegir una foto propia como fondo; esa foto queda sólo en su equipo y no se distribuye.

# 04 — Plan por fases y criterios de salida

Cada fase cierra sólo cuando se cumplen **todos** sus criterios con evidencia (logs, mediciones, capturas de la UI propia; nunca contenido del usuario). Lo que no se pueda probar por falta de hardware queda como PENDIENTE DE VALIDACIÓN EN EQUIPO REAL y bloquea el cierre.

## Fase 0 — Investigación y riesgos
**Tareas:** F0-01 a F0-11 (ver 05).
**Criterios de salida:**
- [x] Matriz de factibilidad, riesgos y decisiones documentadas.
- [x] Protocolo v1, emparejamiento y transporte con pruebas automatizadas que pasan.
- [x] Benchmark de transporte reproducible (loopback).
- [ ] Informe de `kb-probe-win` de tu PC: monitores, DPI, GPU, encoders H.264, Desktop Duplication, inyección táctil, IddCx, perfil de red.
- [ ] Confirmar si Reason 14 está instalado y si corre sin elevar.
- [ ] Modelos de los dispositivos Android de prueba (tamaño, resolución, versión).
- [ ] Tu decisión sobre el driver (aprobación o no para la Fase 2).

## Fase 1 — MVP de duplicación (1 dispositivo)
**Entregables:** `kbss-engine.exe` + `kbss-ui.exe` (sin instalador, portables) y APK debug.
**Criterios de salida:**
1. La APK se instala en Android 10+ y descubre la PC (mDNS) o la encuentra por QR/IP.
2. Emparejamiento por QR y por PIN con aprobación en el host; un dispositivo no aprobado es rechazado (prueba registrada).
3. Se elige un monitor de origen entre los ≥ 3 físicos y se ve en el Android con la relación de aspecto correcta.
4. Un fader de Reason 14 se mueve con **arrastre continuo** del dedo (si Reason no está disponible: prueba con una app accesible de Windows + checklist manual pendiente para Reason).
5. Un dispositivo en solo lectura intenta tocar y no pasa nada (log del rechazo).
6. Wi-Fi cortado 10 s → reconexión automática sin repetir el emparejamiento.
7. Métricas registradas: latencia toque→cambio en pantalla (cámara a 240 FPS o marca de tiempo), FPS, CPU, GPU, RAM del motor, a 720p y 1080p, a 15/30/60 FPS según soporte.
8. Reason sigue usable y sin cortes de audio durante 30 min de transmisión (prueba manual registrada).

## Fase 2 — Extensión real (driver IddCx) — *requiere tu aprobación*
**Criterios de salida:**
1. Driver instalado primero en una VM Win11 y otra Win10 con Secure Boot; después en tu PC.
2. Windows muestra el monitor virtual en Configuración > Pantalla; se puede ordenar respecto a los físicos; resolución y escala aplicadas.
3. Se arrastra una ventana desde un monitor físico al Android; el tacto acierta en la posición correcta (prueba en las 4 esquinas y en el centro).
4. Desconectar el Android o quitar el monitor virtual con Reason abierto: Reason no se cierra y ninguna ventana queda inaccesible (rescate verificado).
5. Desinstalación limpia: el driver y el certificado se eliminan y la topología original se restaura. Documentar si hace falta reiniciar.

## Fase 3 — Multipantalla (2 y luego 3 dispositivos)
**Criterios de salida:**
1. Dos dispositivos, después tres: mezcla de duplicar y extender, con un origen distinto por dispositivo.
2. Permisos independientes: un dispositivo controla y otro sólo ve, al mismo tiempo.
3. Perfiles Grabación vocal, Mezcla, Mastering y Personalizado que se guardan y se restauran.
4. Cambio de modo desde Windows y desde Android con confirmación; si falla, se restaura el estado anterior.
5. Mediciones con 1, 2 y 3 clientes; los objetivos se fijan después del benchmark de la Fase 1, no antes. Si no hay 3 dispositivos físicos, la prueba queda PENDIENTE.

## Fase 4 — Cantante
**Criterios de salida:**
1. PDF visible en la tablet del cantante mientras Reason sigue usable en el monitor principal.
2. Teleprompter: tamaño de letra, contraste, avance manual y automático, velocidad, pausa, páginas y bloqueo de interacción.
3. El documento no sale de la LAN y se borra del dispositivo al cerrar la sesión (verificado inspeccionando el almacenamiento de la app).
4. La sesión se restaura después de reiniciar el host.

## Fase 5 — Producto
**Criterios de salida:**
Instalador x64 (MSI) con desinstalación limpia, APK firmada para pruebas, interfaz en español e inglés, revisión de accesibilidad (contraste, tamaños táctiles ≥ 48 dp, lector de pantalla), asistente de diagnóstico (firewall, aislamiento de Wi-Fi, subred), actualizaciones opcionales firmadas, manual de uso, suite de regresión, listado de licencias de terceros y documentación comercial (costos de firma, distribución).

## Futuro (fuera de esta versión)
USB (ADB reverse o AOA sobre la misma capa abstracta de transporte), audio, acceso remoto fuera de la LAN.

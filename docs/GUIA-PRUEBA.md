# Guía de prueba en tu estudio — Fase 1 (Duplicar) y Fase 2 (Extender)

Equipos previstos: PC con Windows 10/11 x64 y 3+ monitores · tablet EXO 10" (4 GB) · Samsung S23 FE.
Tiempo estimado: 20–30 minutos. Marcá cada casilla y anotá lo que veas: eso es la evidencia de las pruebas.

> Nada de esta guía instala drivers ni cambia la configuración de Windows, **salvo la sección C**, que es opcional y está explicada aparte.

## Qué descargar

En GitHub → repositorio **KB-STUDIO-SCREEN** → pestaña **Actions** → la ejecución más reciente en verde → sección **Artifacts**:

| Artefacto | Contenido | Para |
|---|---|---|
| `windows-binaries` | `kbss-engine.exe`, `kb-probe-win.exe` | PC |
| `kb-studio-screen-apk` | `app-release.apk` (firmada con clave de prueba) | tablet y celular |
| `kbss-idd-driver-test-signed` | driver de pantallas virtuales (firma de PRUEBA) | sólo sección C |

Windows SmartScreen va a advertir porque los ejecutables todavía no están firmados: **Más información → Ejecutar de todas formas**.

## Panel de control (desde la versión 0.3.0)

Al abrir `kbss-engine.exe` se abre solo un **panel en tu navegador** (dirección `http://127.0.0.1:47801/...`). Desde ahí podés hacer todo lo de la consola con botones: aprobar dispositivos (PIN grande), ver las 3 ranuras, mostrar el QR, dar o quitar control, revocar, y el modo cantante (arrastrás el PDF al panel). La ventana negra tiene que seguir abierta: es el motor. Si cerrás la pestaña, el enlace sigue impreso en la consola.

El panel sólo funciona en tu propia PC: no es accesible desde la red ni desde otros sitios web.

## A. Preparación (una sola vez)

- [ ] **A1.** La PC y los dos Android en la **misma red Wi-Fi** (ideal 5 GHz). Si la PC va por cable al mismo router, también sirve. Evitá redes de "invitados": suelen aislar dispositivos.
- [ ] **A2.** En cada Android: Ajustes → Seguridad → permitir instalar apps desde el navegador/archivos, e instalar `app-release.apk`.
- [ ] **A3.** En la PC: copiar `kbss-engine.exe` a una carpeta propia (por ejemplo `C:\KBStudioScreen`). **No hace falta ejecutarlo como administrador** (no lo hagas).
- [ ] **A4.** Ejecutar `kb-probe-win.exe` y guardar `kbss-probe-report.json` (sirve para ajustar el rendimiento a tu GPU).

## B. Duplicar (no requiere driver)

- [ ] **B1.** Abrí `kbss-engine.exe`. Debe listar tus monitores con sus **nombres reales** y mostrar un **QR**.
  - Si Windows pregunta por el Firewall: marcá **sólo "Redes privadas"** y aceptá.
  - Si tu red figura como "Pública" en Windows, cambiala a "Privada" (Configuración → Red → tu Wi-Fi → Perfil de red: Privada).
- [ ] **B2. Tablet EXO — emparejar por QR:** abrí la app → **Escanear QR** → apuntá al QR de la consola. En la PC aparece «¿Aprobar…?» → escribí **c** (control) y Enter.
  - Esperado: la tablet muestra tu monitor principal a pantalla completa.
- [ ] **B3. S23 FE — emparejar por PIN:** abrí la app → la PC debería aparecer sola en "PCs en tu red" → tocála → en la consola aparece un **PIN de 6 dígitos** → escribilo en el celular → en la PC escribí **v** (solo ver).
- [ ] **B4. Solo lectura:** en el S23 FE tocá y arrastrá sobre la imagen. Esperado: **no pasa nada en Windows**.
- [ ] **B5. Fader de Reason (la prueba clave):** abrí Reason 14 en el monitor que ve la tablet. Con un dedo, arrastrá un fader del mixer hacia arriba y abajo **sin levantar el dedo**.
  - Esperado: el fader sigue al dedo de forma continua; al soltar, queda donde lo dejaste.
  - Anotá: ¿se siente el retardo? (sí/no/poco), ¿se "suelta" el fader?
- [ ] **B6. Rueda:** con **dos dedos** deslizá vertical sobre una lista o perilla de Reason → debe actuar como la rueda del mouse.
- [ ] **B7. Zoom local:** pellizcá con dos dedos → la imagen se agranda **sólo en la tablet** (Windows no cambia). Botón "Restablecer zoom" en la barra superior.
- [ ] **B8. Cambiar de monitor:** barra superior (tocá la pestañita arriba si está oculta) → ícono de monitor → elegí otro de tus 3 monitores.
- [ ] **B9. Reconexión:** apagá el Wi-Fi de la tablet 10 s y volvé a encenderlo. Esperado: "Reconectando…" y vuelve sola, sin PIN.
- [ ] **B10. Revocar:** en la consola escribí `l` (lista) y `r N` para el S23 FE. Esperado: el celular se desconecta y, para volver, pide emparejar otra vez.
- [ ] **B11. Rendimiento:** en la barra superior se ve `resolución · fps`. Anotá los valores en la tablet y el celular, y si Reason o el audio sufren algún corte (**no debería**).

## C. Extender (opcional — requiere el driver; leé antes)

El driver de pantallas virtuales está **probado en Windows real** (máquina de CI de GitHub), pero por ahora está firmado sólo con un certificado de **prueba**. Windows sólo lo carga si:

- **Opción 1 (recomendada para probar):** una **máquina virtual** o una PC de pruebas con *modo de prueba* activado (`bcdedit /set testsigning on`, requiere **Secure Boot desactivado** y reiniciar; aparece la marca "Modo de prueba" en el escritorio). **No lo recomiendo en tu PC de estudio.**
- **Opción 2 (pendiente de validar):** instalar un certificado autofirmado propio de Castro Labs como de confianza en tu PC, con Secure Boot activo. Hay que probarlo primero (tarea F2-01).
- **Opción 3 (producto):** firma de Microsoft por atestación, con certificado EV (costo anual). Es lo que permite instalarlo en cualquier PC sin tocar nada.

Si decidís probarlo en una PC de prueba con modo de prueba activo (como administrador, una sola vez):

```bash
pnputil /add-driver kbss_idd.inf /install
```

```bash
kbss-engine.exe driver create-adapter
```

Luego, abrí `kbss-engine.exe` normal (sin administrador) y en la tablet: barra superior → ícono de monitor → **Extender escritorio (pantalla nueva)** → confirmá con **s** en la consola.

- [ ] **C1.** En Windows aparece un monitor nuevo "KBSS Pantalla" con la resolución de la tablet (Configuración → Pantalla).
- [ ] **C2.** Arrastrá una ventana desde un monitor físico hasta la tablet y tocala con el dedo.
- [ ] **C3.** Cerrá la app de la tablet: a los 30 s el monitor virtual desaparece y la ventana vuelve a un monitor físico.
- [ ] **C4.** Cerrá `kbss-engine.exe` con un monitor virtual activo: a los ~6 s el driver lo retira solo.

Para desinstalar: `kbss-engine.exe driver remove-adapter` y `pnputil /delete-driver oemNN.inf /uninstall` (el número `oemNN` lo muestra `pnputil /enum-drivers`).

## D. Modo cantante (letras en la tablet)

- [ ] **D1.** Emparejá la tablet del cantante en **solo ver** (en la PC respondé **v**).
- [ ] **D2.** En la consola de la PC escribí `d ` seguido de la ruta del archivo, por ejemplo:
  `d C:\Letras\Tema 1.pdf` (también sirve un `.txt`). Esperado: la letra aparece a pantalla completa en la tablet.
- [ ] **D3.** En la tablet: botones de abajo → página siguiente/anterior, avance automático (▶ y velocidad −/+), contraste, tamaño de letra (en `.txt`).
- [ ] **D4.** Desde la PC: `n` página siguiente, `b` anterior, `a 3` avance automático a velocidad 3 (`a 0` lo detiene).
- [ ] **D5.** Bloqueo: escribí `k` en la PC → desaparecen los botones de la tablet (el cantante no puede tocar nada). `k` otra vez para desbloquear.
- [ ] **D6.** `c` cierra la letra en la tablet (y se borra de la tablet).
- [ ] **D7.** Mientras tanto, Reason sigue usable desde el teléfono o el mouse.

Si no hay ningún dispositivo en "solo ver", la letra se envía a todos los conectados.

## Qué me tenés que mandar

1. `kbss-probe-report.json`.
2. Las casillas marcadas y tus notas (sobre todo B5 y B11).
3. Si algo falla: lo que dice la consola de `kbss-engine` (no contiene PIN ni datos privados más allá de los nombres de los dispositivos).

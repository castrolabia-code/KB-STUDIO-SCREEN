# KB STUDIO SCREEN — Manual de uso
**by Castro Labs** · versión 0.7

KB STUDIO SCREEN convierte hasta **tres teléfonos o tablets Android** en pantallas de tu PC con Windows. Desde ellos podés **ver** cualquier monitor y **controlar con el dedo** el programa que tengas abierto (por ejemplo, los faders del mixer de Reason). También sirve para mostrarle **la letra al cantante** en una tablet.

Funciona sólo en tu red local: sin Internet, sin cuentas y sin suscripciones. **No transmite audio.**

---

## 1. Instalación

### En la PC (Windows 10 u 11, 64 bits)
1. Ejecutá **KBStudioScreen-Setup.exe**. Windows va a pedir permiso de administrador **sólo para instalar** (para crear la regla del firewall).
2. Elegí si querés acceso directo en el escritorio y si querés que arranque con Windows.
3. Terminado: abrí **KB Studio Screen** desde el menú Inicio. **No hace falta ejecutarlo como administrador** (y es mejor no hacerlo).

> Si Windows muestra "Windows protegió su PC", tocá **Más información → Ejecutar de todas formas**. Aparece porque el programa todavía no tiene firma digital comercial.

### En el teléfono o la tablet (Android 10 o más nuevo)
1. Descargá **KBStudioScreen.apk** en el dispositivo.
2. Abrilo e instalalo. Si Android lo pide, permití instalar apps de esa fuente.

---

## 2. Primer uso

1. **La PC y los dispositivos tienen que estar en la misma red Wi-Fi** (mejor 5 GHz). Si la PC va por cable al mismo router, también sirve. Evitá las redes de "invitados": suelen aislar los equipos entre sí.
2. Abrí **KB Studio Screen** en la PC. Se abren dos cosas:
   - una **ventana negra**: es el motor. **Tiene que quedar abierta** (podés minimizarla);
   - el **panel de control** en tu navegador.
3. En el dispositivo abrí la app y emparejalo de una de estas dos formas:
   - **QR:** en el panel tocá **Mostrar QR nuevo** y en la app **Escanear QR**.
   - **PIN:** en la app tocá tu PC en la lista "PCs en tu red"; en el panel aparece un **PIN de 6 dígitos** que escribís en el dispositivo.
4. En el panel aparece la pregunta **¿Aprobar…?**:
   - **Control táctil:** el dispositivo puede tocar y mover cosas en Windows (para el productor o técnico).
   - **Solo ver:** sólo mira, nunca toca nada (ideal para el cantante).
   - **Rechazar.**

Listo. La próxima vez se conecta solo, sin PIN ni QR.

---

## 3. Usar el dispositivo

| Gesto | Qué hace |
|---|---|
| **Un dedo**: tocar | Clic |
| **Un dedo**: tocar y arrastrar sin soltar | Arrastrar (mover un fader, una perilla, una ventana) |
| **Dos dedos**: deslizar arriba/abajo | Rueda del mouse |
| **Dos dedos**: pellizcar | Zoom **sólo en el dispositivo** (Windows no cambia) |

**Barra superior** (si está oculta, tocá la rayita de arriba):
- **✕** desconectar · **Control táctil / Solo ver** (indicador)
- **Monitor:** elegir qué monitor de la PC ver.
- **Bloquear tacto:** el dispositivo deja de tocar Windows (para navegar con zoom sin riesgo).
- **Calidad:** *Mezcla* (respuesta rápida), *Letras* (máxima nitidez) o *Automática*.
- **Restablecer zoom** y **Refrescar imagen**.

Si se corta el Wi-Fi, la app muestra "Reconectando…" y vuelve sola.

---

## 4. Panel de control (en la PC)

- **Pantallas Android:** las 3 ranuras, con LED verde cuando transmiten. En cada una podés cambiar **monitor de origen** y **calidad** sin tocar el dispositivo.
- **Perfiles de estudio:** *Grabación vocal, Mezcla, Mastering, Personalizado.* Dejá todo como lo querés y tocá **Guardar actual**; otro día, **Aplicar** lo deja igual con un clic.
- **Dispositivos de confianza:** dar o quitar control, o **Revocar** (el dispositivo queda desconectado y tiene que volver a emparejarse).
- **Monitores de Windows** y **Rescatar ventanas fuera de pantalla** (por si una ventana quedó en un monitor que ya no está).
- **Apariencia:** cambiar la foto de fondo del panel.

---

## 5. Modo cantante (letras)

1. Emparejá la tablet del cantante en **Solo ver**.
2. En el panel, **arrastrá el PDF o el .txt** con la letra al recuadro de *Modo cantante*.
3. La letra aparece a pantalla completa en la tablet. Desde el panel:
   - **◀ Página / Página ▶**
   - **Avance − / +**: avance automático (0 = detenido)
   - **Bloquear tablet**: desaparecen los botones; el cantante no puede tocar nada por error.
   - **Cerrar letra**: se cierra **y se borra** de la tablet.

En la tablet (si no está bloqueada) también hay botones para páginas, avance, contraste y tamaño de letra.

---

## 6. Problemas frecuentes

| Problema | Solución |
|---|---|
| La app no encuentra la PC | ¿Está abierta la ventana negra? ¿Mismo Wi-Fi? Probá con el **QR** o **Conectar por IP** (la dirección figura en el panel). |
| "No se pudo llegar a la PC" | En Windows: Configuración → Red → tu red → **Perfil de red: Privada**. La red "Pública" bloquea las conexiones. |
| La imagen va lenta o con saltos | Usá Wi-Fi de 5 GHz, acercate al router, o elegí calidad **Mezcla**. |
| El toque no hace nada | Fijate que el dispositivo tenga **Control táctil** y que **Bloquear tacto** esté apagado. Si el programa de destino corre **como administrador**, Windows no permite controlarlo: abrilo normal. |
| "La PC ya tiene 3 dispositivos" | Desconectá uno o revocá alguno en el panel. |
| Una ventana quedó fuera de la pantalla | Panel → **Rescatar ventanas fuera de pantalla**. |
| La pantalla de inicio de sesión o de UAC no se ve | Es normal: Windows protege esas pantallas y no permite capturarlas. |

---

## 7. Privacidad y seguridad
- Todo viaja **cifrado** y sólo por tu red local. Nada pasa por Internet.
- Cada dispositivo necesita tu **aprobación**; podés **revocarlo** cuando quieras.
- Los PIN y los QR sirven **una sola vez** y vencen en minutos.
- No se envían datos a Castro Labs ni a terceros. No hay telemetría.

## 8. Desinstalar
Menú Inicio → **KB Studio Screen → Desinstalar** (o Configuración → Aplicaciones). Al final te pregunta si también querés borrar los dispositivos emparejados y las preferencias.

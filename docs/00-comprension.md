# 00 — Resumen de comprensión

## Qué es
Un **host para Windows 10/11 x64** y un **cliente Android 10+** que convierten hasta **3 dispositivos Android** (cualquier mezcla de tablets de ~10" y teléfonos) en pantallas de la PC, cada una configurable por separado:

- **Duplicar**: muestra un monitor existente (físico o virtual). Varios Android pueden ver el mismo monitor.
- **Extender**: Windows ve un **monitor nuevo real** (driver Indirect Display / IddCx), con su posición en la disposición, resolución y DPI. Se arrastran ventanas hacia él como a cualquier monitor.

El tacto del Android controla la **interfaz original** de Windows (mixer de Reason 14, plugins VST, etc.). No se construye un mixer alternativo ni hay que mapear faders por MIDI.

## Casos de uso prioritarios
1. **Productor**: tablet con el mixer de Reason, arrastrando faders con el dedo, con baja latencia.
2. **Cantante**: tablet en **solo lectura** con letras/PDF (ventana de Windows o teleprompter propio) mientras el productor trabaja en Reason en la PC, sin que el cantante pueda tocar nada por accidente.

## Reglas que condicionan todo el diseño
| Regla | Consecuencia técnica |
|---|---|
| La estabilidad del DAW y del audio va antes que el streaming | El motor corre con prioridad normal, no toca dispositivos de audio y se degrada (menos FPS o resolución) antes de competir con el DAW. **No hay audio** en esta versión. |
| Sin nube, sin cuentas, sin Internet | Descubrimiento mDNS en la LAN, emparejamiento local, sin telemetría. |
| Seguridad | TLS 1.3 mutuo, PIN/QR de un solo uso con aprobación en el host, permisos por dispositivo (solo ver / control), revocación inmediata. |
| Mínimos privilegios | Sólo el instalador (una vez) requiere administrador: driver y regla de firewall. El uso diario corre sin elevar. |
| Tres dispositivos es una capacidad, no una promesa de 3×1080p60 | Adaptación por dispositivo y objetivos medidos sobre el hardware real. |
| Nada simulado | Cada función se marca IMPLEMENTADO / PROBADO / PENDIENTE / BLOQUEADO, con evidencia. |

## Qué NO incluye esta versión
- Audio (ni captura ni dispositivos virtuales de audio).
- USB (se deja la interfaz de transporte preparada; no está implementado).
- Acceso por Internet. El servicio nunca se expone fuera de la LAN por defecto.
- Captura del escritorio seguro (UAC, Ctrl+Alt+Supr, pantalla de inicio de sesión). Windows lo impide y no se intentará eludirlo; el cliente mostrará "pantalla protegida".
- Movimiento simultáneo de varios faders si la aplicación sólo procesa mouse (ver riesgo R-04).

## Entorno de esta sesión
Esta sesión corre en un **contenedor Linux en la nube**: no tiene Windows, GPU, monitores, Reason ni dispositivos Android, y tu PC no está conectada. Por eso:
- Lo independiente de la plataforma (protocolo, seguridad, transporte, geometría) se implementó y **se probó acá**.
- Lo que depende de Windows o Android se preparó para ejecutarse en tu equipo y queda **PENDIENTE DE VALIDACIÓN EN EQUIPO REAL**.

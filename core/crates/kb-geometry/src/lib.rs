//! KB STUDIO SCREEN — geometría.
//!
//! Cadena de transformación de un toque:
//!
//! ```text
//! [Android] píxel en la View
//!     -> deshacer zoom/paneo LOCAL del visor (no se envía a Windows)
//!     -> deshacer letterbox (fit "contain")       => None si cae en las barras
//!     -> deshacer rotación de presentación local
//!     => NormPoint (0..=65535 en ambos ejes, relativo al FRAME transmitido)
//! ----------------------------- red -----------------------------
//! [Host] NormPoint
//!     -> aplicar rotación de contenido de la captura (p. ej. DXGI entrega
//!        el framebuffer sin rotar en monitores en vertical)
//!     -> escalar al rectángulo del monitor ASIGNADO (píxeles físicos)
//!     => punto en el escritorio virtual (coordenadas físicas, DPI-aware v2)
//! ```
//!
//! Decisión de seguridad: el cliente nunca envía coordenadas absolutas de
//! escritorio; envía coordenadas normalizadas del frame y el host las
//! resuelve contra el monitor que ÉL asignó a esa sesión. Así un cliente no
//! puede inyectar entrada en un monitor que no está viendo.

use serde::{Deserialize, Serialize};

/// Máximo valor de coordenada normalizada (inclusive).
pub const NORM_MAX: u16 = u16::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn contains(&self, p: PointI) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
    pub fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.w as i64 * self.h as i64
        }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointI {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Punto normalizado relativo al frame transmitido. 0 = borde izquierdo/superior,
/// 65535 = borde derecho/inferior (centro del último píxel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormPoint {
    pub x: u16,
    pub y: u16,
}

/// Rotación en sentido horario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    /// ¿Intercambia ancho y alto?
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::R90 | Rotation::R270)
    }
    pub fn inverse(self) -> Rotation {
        match self {
            Rotation::R0 => Rotation::R0,
            Rotation::R90 => Rotation::R270,
            Rotation::R180 => Rotation::R180,
            Rotation::R270 => Rotation::R90,
        }
    }
    /// Rota un punto en espacio unitario [0,1]^2 en sentido horario.
    pub fn apply_unit(self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Rotation::R0 => (x, y),
            Rotation::R90 => (1.0 - y, x),
            Rotation::R180 => (1.0 - x, 1.0 - y),
            Rotation::R270 => (y, 1.0 - x),
        }
    }
}

/// Rectángulo donde se dibuja un contenido `src_w x src_h` dentro de una
/// vista `view_w x view_h` manteniendo relación de aspecto (letterbox/pillarbox).
pub fn fit_contain(view_w: f32, view_h: f32, src_w: u32, src_h: u32) -> RectF {
    if view_w <= 0.0 || view_h <= 0.0 || src_w == 0 || src_h == 0 {
        return RectF { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    }
    let scale = (view_w / src_w as f32).min(view_h / src_h as f32);
    let w = src_w as f32 * scale;
    let h = src_h as f32 * scale;
    RectF { x: (view_w - w) / 2.0, y: (view_h - h) / 2.0, w, h }
}

/// Zoom/paneo LOCAL del visor Android (pellizco). No altera Windows.
/// `view = content * scale + offset`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalZoom {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for LocalZoom {
    fn default() -> Self {
        Self { scale: 1.0, offset_x: 0.0, offset_y: 0.0 }
    }
}

impl LocalZoom {
    pub const MIN_SCALE: f32 = 1.0;
    pub const MAX_SCALE: f32 = 6.0;

    pub fn to_unzoomed(&self, p: PointF) -> PointF {
        PointF { x: (p.x - self.offset_x) / self.scale, y: (p.y - self.offset_y) / self.scale }
    }

    /// Aplica un gesto de pellizco con foco `focus` (en coordenadas de vista)
    /// manteniendo el punto bajo los dedos fijo, y limita el paneo para que
    /// el contenido no se salga de la vista.
    pub fn pinch(&mut self, factor: f32, focus: PointF, view_w: f32, view_h: f32) {
        let new_scale = (self.scale * factor).clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        let k = new_scale / self.scale;
        self.offset_x = focus.x - (focus.x - self.offset_x) * k;
        self.offset_y = focus.y - (focus.y - self.offset_y) * k;
        self.scale = new_scale;
        self.clamp(view_w, view_h);
    }

    pub fn pan(&mut self, dx: f32, dy: f32, view_w: f32, view_h: f32) {
        self.offset_x += dx;
        self.offset_y += dy;
        self.clamp(view_w, view_h);
    }

    fn clamp(&mut self, view_w: f32, view_h: f32) {
        let min_x = view_w - view_w * self.scale;
        let min_y = view_h - view_h * self.scale;
        self.offset_x = self.offset_x.clamp(min_x, 0.0);
        self.offset_y = self.offset_y.clamp(min_y, 0.0);
    }
}

/// Estado de presentación del cliente necesario para mapear un toque.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClientViewport {
    pub view_w: f32,
    pub view_h: f32,
    /// Tamaño del frame decodificado (tal como llega por la red).
    pub frame_w: u32,
    pub frame_h: u32,
    /// Rotación con que el cliente DIBUJA el frame (p. ej. usuario giró la
    /// tablet y elige no renegociar resolución).
    pub present_rotation: Rotation,
    pub zoom: LocalZoom,
}

impl ClientViewport {
    /// Dimensiones del frame tal como se ve en pantalla (tras rotar).
    pub fn presented_size(&self) -> (u32, u32) {
        if self.present_rotation.swaps_axes() {
            (self.frame_h, self.frame_w)
        } else {
            (self.frame_w, self.frame_h)
        }
    }

    pub fn content_rect(&self) -> RectF {
        let (w, h) = self.presented_size();
        fit_contain(self.view_w, self.view_h, w, h)
    }

    /// Píxel de la vista Android -> punto normalizado del frame.
    /// Devuelve `None` si el toque cae en las barras negras (no se envía).
    pub fn view_to_norm(&self, p: PointF) -> Option<NormPoint> {
        let p = self.zoom.to_unzoomed(p);
        let r = self.content_rect();
        if r.w <= 0.0 || r.h <= 0.0 {
            return None;
        }
        let u = (p.x - r.x) / r.w;
        let v = (p.y - r.y) / r.h;
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        // De espacio presentado a espacio del frame: deshacer rotación.
        let (fu, fv) = self.present_rotation.inverse().apply_unit(u as f64, v as f64);
        Some(NormPoint { x: unit_to_norm(fu), y: unit_to_norm(fv) })
    }

    /// Igual que `view_to_norm` pero, si el punto cae fuera, lo pega al
    /// borde. Se usa DURANTE un arrastre ya iniciado (un fader no debe
    /// "soltarse" si el dedo pasa sobre la barra negra).
    pub fn view_to_norm_clamped(&self, p: PointF) -> NormPoint {
        let p = self.zoom.to_unzoomed(p);
        let r = self.content_rect();
        if r.w <= 0.0 || r.h <= 0.0 {
            return NormPoint { x: 0, y: 0 };
        }
        let u = ((p.x - r.x) / r.w).clamp(0.0, 1.0);
        let v = ((p.y - r.y) / r.h).clamp(0.0, 1.0);
        let (fu, fv) = self.present_rotation.inverse().apply_unit(u as f64, v as f64);
        NormPoint { x: unit_to_norm(fu), y: unit_to_norm(fv) }
    }
}

fn unit_to_norm(u: f64) -> u16 {
    (u.clamp(0.0, 1.0) * NORM_MAX as f64).round() as u16
}

fn norm_to_unit(n: u16) -> f64 {
    n as f64 / NORM_MAX as f64
}

/// Descripción de un monitor Windows en coordenadas físicas del escritorio virtual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorDesc {
    /// Identificador estable (p. ej. device path de la interfaz del monitor).
    pub id: String,
    pub name: String,
    pub bounds: Rect,
    pub work_area: Rect,
    pub dpi: u32,
    pub primary: bool,
    pub is_virtual: bool,
}

/// Normalizado del frame -> píxel del escritorio virtual dentro del monitor
/// asignado. `capture_rotation` es la rotación que hay que aplicar al
/// contenido del frame para que coincida con la orientación del monitor
/// (R0 con Windows Graphics Capture; puede no ser R0 con DXGI Desktop
/// Duplication en monitores girados, ver `DXGI_OUTDUPL_DESC::Rotation`).
pub fn norm_to_desktop(n: NormPoint, monitor: &Rect, capture_rotation: Rotation) -> PointI {
    let (u, v) = capture_rotation.apply_unit(norm_to_unit(n.x), norm_to_unit(n.y));
    // Mapea [0,1] al CENTRO del primer..último píxel, y garantiza quedar dentro.
    let px = monitor.x as f64 + u * (monitor.w - 1).max(0) as f64;
    let py = monitor.y as f64 + v * (monitor.h - 1).max(0) as f64;
    PointI {
        x: (px.round() as i32).clamp(monitor.x, monitor.right() - 1),
        y: (py.round() as i32).clamp(monitor.y, monitor.bottom() - 1),
    }
}

/// Coordenadas absolutas para `SendInput` con
/// `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK` (0..=65535 sobre el
/// escritorio virtual). PENDIENTE DE VALIDACIÓN EN EQUIPO REAL: verificar
/// redondeo en topologías con coordenadas negativas y escalas DPI mixtas.
pub fn desktop_to_sendinput_abs(p: PointI, virtual_bounds: &Rect) -> (i32, i32) {
    let fx = (p.x - virtual_bounds.x) as f64 / (virtual_bounds.w - 1).max(1) as f64;
    let fy = (p.y - virtual_bounds.y) as f64 / (virtual_bounds.h - 1).max(1) as f64;
    (
        (fx.clamp(0.0, 1.0) * 65535.0).round() as i32,
        (fy.clamp(0.0, 1.0) * 65535.0).round() as i32,
    )
}

/// Topología de monitores visibles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    pub monitors: Vec<MonitorDesc>,
}

impl Topology {
    pub fn virtual_bounds(&self) -> Option<Rect> {
        self.monitors.iter().map(|m| m.bounds).reduce(|a, b| a.union(&b))
    }
    pub fn primary(&self) -> Option<&MonitorDesc> {
        self.monitors.iter().find(|m| m.primary).or(self.monitors.first())
    }
    pub fn by_id(&self, id: &str) -> Option<&MonitorDesc> {
        self.monitors.iter().find(|m| m.id == id)
    }
    pub fn monitor_at(&self, p: PointI) -> Option<&MonitorDesc> {
        self.monitors.iter().find(|m| m.bounds.contains(p))
    }

    /// Decide si una ventana quedó inaccesible y, de ser así, a dónde
    /// moverla. Criterio: la "zona de agarre" (franja superior de la ventana,
    /// donde suele estar la barra de título) debe tener al menos
    /// `min_grab_px` de ancho visible en algún monitor. Si no, se reubica en
    /// el área de trabajo del monitor `fallback` (o el primario),
    /// conservando el tamaño si entra o recortándolo si no.
    pub fn rescue_window(&self, window: Rect, fallback: Option<&str>) -> Option<Rect> {
        const GRAB_H: i32 = 32;
        const MIN_GRAB_PX: i32 = 64;
        let grab = Rect::new(window.x, window.y, window.w, GRAB_H.min(window.h.max(1)));
        let reachable = self
            .monitors
            .iter()
            .filter_map(|m| m.work_area.intersection(&grab))
            .any(|i| i.w >= MIN_GRAB_PX.min(window.w.max(1)) && i.h >= 8.min(grab.h));
        if reachable {
            return None;
        }
        let target = fallback.and_then(|id| self.by_id(id)).or(self.primary())?;
        let wa = target.work_area;
        let w = window.w.min(wa.w);
        let h = window.h.min(wa.h);
        Some(Rect::new(wa.x + (wa.w - w) / 2, wa.y + (wa.h - h) / 2, w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(id: &str, b: Rect, primary: bool) -> MonitorDesc {
        MonitorDesc {
            id: id.into(),
            name: id.into(),
            bounds: b,
            work_area: Rect::new(b.x, b.y, b.w, b.h - 40),
            dpi: 96,
            primary,
            is_virtual: false,
        }
    }

    #[test]
    fn fit_letterbox_16_9_in_16_10_tablet() {
        // Tablet 1920x1200 mostrando 1920x1080: barras arriba/abajo de 60 px.
        let r = fit_contain(1920.0, 1200.0, 1920, 1080);
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 60.0, 1920.0, 1080.0));
    }

    #[test]
    fn touch_in_letterbox_bar_is_dropped() {
        let vp = ClientViewport {
            view_w: 1920.0,
            view_h: 1200.0,
            frame_w: 1920,
            frame_h: 1080,
            present_rotation: Rotation::R0,
            zoom: LocalZoom::default(),
        };
        assert_eq!(vp.view_to_norm(PointF { x: 500.0, y: 30.0 }), None);
        let n = vp.view_to_norm(PointF { x: 0.0, y: 60.0 }).unwrap();
        assert_eq!(n, NormPoint { x: 0, y: 0 });
        let n = vp.view_to_norm(PointF { x: 1920.0, y: 1140.0 }).unwrap();
        assert_eq!(n, NormPoint { x: NORM_MAX, y: NORM_MAX });
        // Arrastre que sale a la barra se pega al borde.
        assert_eq!(vp.view_to_norm_clamped(PointF { x: 960.0, y: 5.0 }).y, 0);
    }

    #[test]
    fn full_chain_hits_exact_pixel_on_secondary_monitor() {
        // Monitor a la izquierda del primario: coordenadas negativas.
        let m = Rect::new(-2560, 0, 2560, 1440);
        let vp = ClientViewport {
            view_w: 2000.0,
            view_h: 1200.0,
            frame_w: 1280,
            frame_h: 720,
            present_rotation: Rotation::R0,
            zoom: LocalZoom::default(),
        };
        // fit: 2000x1125 en y=37.5. Centro de la vista -> centro del monitor.
        let n = vp.view_to_norm(PointF { x: 1000.0, y: 600.0 }).unwrap();
        let p = norm_to_desktop(n, &m, Rotation::R0);
        assert!((p.x - (-1280)).abs() <= 1 && (p.y - 720).abs() <= 1, "{p:?}");
        // Esquinas siempre dentro del monitor asignado.
        let br = norm_to_desktop(NormPoint { x: NORM_MAX, y: NORM_MAX }, &m, Rotation::R0);
        assert_eq!(br, PointI { x: -1, y: 1439 });
        let tl = norm_to_desktop(NormPoint { x: 0, y: 0 }, &m, Rotation::R0);
        assert_eq!(tl, PointI { x: -2560, y: 0 });
    }

    #[test]
    fn rotation_roundtrip_and_presentation() {
        for r in [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270] {
            let (x, y) = r.apply_unit(0.2, 0.7);
            let (bx, by) = r.inverse().apply_unit(x, y);
            assert!((bx - 0.2).abs() < 1e-12 && (by - 0.7).abs() < 1e-12);
        }
        // Frame horizontal 1920x1080 presentado girado 90° en un teléfono vertical 1080x1920.
        let vp = ClientViewport {
            view_w: 1080.0,
            view_h: 1920.0,
            frame_w: 1920,
            frame_h: 1080,
            present_rotation: Rotation::R90,
            zoom: LocalZoom::default(),
        };
        // Esquina superior DERECHA de la pantalla = esquina superior IZQUIERDA del frame.
        let n = vp.view_to_norm(PointF { x: 1080.0, y: 0.0 }).unwrap();
        assert_eq!(n, NormPoint { x: 0, y: 0 });
    }

    #[test]
    fn local_zoom_is_undone_before_mapping() {
        let mut vp = ClientViewport {
            view_w: 1000.0,
            view_h: 1000.0,
            frame_w: 1000,
            frame_h: 1000,
            present_rotation: Rotation::R0,
            zoom: LocalZoom::default(),
        };
        vp.zoom.pinch(2.0, PointF { x: 500.0, y: 500.0 }, 1000.0, 1000.0);
        // Con zoom 2x centrado, el centro sigue siendo el centro...
        let c = vp.view_to_norm(PointF { x: 500.0, y: 500.0 }).unwrap();
        assert!((c.x as i32 - 32768).abs() <= 40);
        // ...y la esquina de la vista corresponde a 1/4 del frame.
        let q = vp.view_to_norm(PointF { x: 0.0, y: 0.0 }).unwrap();
        assert!((q.x as i32 - 16384).abs() <= 40, "{q:?}");
        // El paneo no deja salir el contenido.
        vp.zoom.pan(10_000.0, 0.0, 1000.0, 1000.0);
        assert_eq!(vp.zoom.offset_x, 0.0);
    }

    #[test]
    fn sendinput_abs_spans_virtual_desktop() {
        let t = Topology {
            monitors: vec![
                mon("L", Rect::new(-1920, 0, 1920, 1080), false),
                mon("P", Rect::new(0, 0, 2560, 1440), true),
                mon("R", Rect::new(2560, -200, 1080, 1920), false),
            ],
        };
        let vb = t.virtual_bounds().unwrap();
        assert_eq!(vb, Rect::new(-1920, -200, 1920 + 2560 + 1080, 1920));
        assert_eq!(desktop_to_sendinput_abs(PointI { x: -1920, y: -200 }, &vb), (0, 0));
        assert_eq!(
            desktop_to_sendinput_abs(PointI { x: vb.right() - 1, y: vb.bottom() - 1 }, &vb),
            (65535, 65535)
        );
    }

    #[test]
    fn window_on_removed_virtual_monitor_is_rescued() {
        let t = Topology {
            monitors: vec![mon("P", Rect::new(0, 0, 1920, 1080), true)],
        };
        // Ventana que estaba en un monitor virtual a la derecha (ya desconectado).
        let lost = Rect::new(2000, 100, 1200, 800);
        let r = t.rescue_window(lost, None).unwrap();
        assert_eq!(r, Rect::new(360, 120, 1200, 800));
        // Ventana mayormente visible: no se toca.
        assert_eq!(t.rescue_window(Rect::new(1500, 100, 1200, 800), None), None);
        // Ventana con barra de título por encima del borde superior: se rescata.
        assert!(t.rescue_window(Rect::new(100, -500, 800, 600), None).is_some());
        // Ventana gigante se recorta al área de trabajo.
        let big = t.rescue_window(Rect::new(5000, 0, 4000, 3000), None).unwrap();
        assert_eq!(big, Rect::new(0, 0, 1920, 1040));
    }
}

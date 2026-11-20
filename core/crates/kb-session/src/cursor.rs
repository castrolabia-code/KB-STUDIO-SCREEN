//! Composición del cursor del mouse sobre el frame capturado.
//!
//! Desktop Duplication entrega la imagen del escritorio SIN el cursor; la
//! forma y la posición llegan aparte. Acá se dibuja sobre la copia BGRA,
//! respetando los tres formatos de Windows (DXGI_OUTDUPL_POINTER_SHAPE_TYPE).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    /// 1 bpp: AND (alto/2 filas) seguido de XOR (alto/2 filas).
    Monochrome,
    /// BGRA con alfa.
    Color,
    /// BGRA: alfa 0x00 = reemplazar con el color; 0xFF = XOR con la pantalla.
    MaskedColor,
}

#[derive(Debug, Clone)]
pub struct CursorShape {
    pub kind: CursorKind,
    pub width: u32,
    /// Para `Monochrome`, el alto TOTAL (AND + XOR), como lo entrega Windows.
    pub height: u32,
    pub pitch: u32,
    pub data: Vec<u8>,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
}

impl CursorShape {
    fn visible_height(&self) -> u32 {
        match self.kind {
            CursorKind::Monochrome => self.height / 2,
            _ => self.height,
        }
    }
}

/// Dibuja el cursor con su punto activo en (x, y) — coordenadas del frame.
pub fn composite(frame: &mut [u8], fw: u32, fh: u32, stride: u32, shape: &CursorShape, x: i32, y: i32) {
    let (ox, oy) = (x - shape.hotspot_x, y - shape.hotspot_y);
    let h = shape.visible_height();
    for cy in 0..h as i32 {
        let py = oy + cy;
        if py < 0 || py >= fh as i32 {
            continue;
        }
        for cx in 0..shape.width as i32 {
            let px = ox + cx;
            if px < 0 || px >= fw as i32 {
                continue;
            }
            let o = py as usize * stride as usize + px as usize * 4;
            let Some(dst) = frame.get_mut(o..o + 4) else { continue };
            match shape.kind {
                CursorKind::Color => {
                    let s = cy as usize * shape.pitch as usize + cx as usize * 4;
                    let Some(src) = shape.data.get(s..s + 4) else { continue };
                    let a = src[3] as u32;
                    for k in 0..3 {
                        // Windows entrega el color premultiplicado por alfa.
                        dst[k] = (src[k] as u32 + dst[k] as u32 * (255 - a) / 255).min(255) as u8;
                    }
                }
                CursorKind::MaskedColor => {
                    let s = cy as usize * shape.pitch as usize + cx as usize * 4;
                    let Some(src) = shape.data.get(s..s + 4) else { continue };
                    if src[3] == 0xFF {
                        for k in 0..3 {
                            dst[k] ^= src[k];
                        }
                    } else {
                        dst[..3].copy_from_slice(&src[..3]);
                    }
                }
                CursorKind::Monochrome => {
                    let byte = cx as usize / 8;
                    let bit = 0x80u8 >> (cx % 8);
                    let and_row = cy as usize * shape.pitch as usize;
                    let xor_row = (cy as usize + h as usize) * shape.pitch as usize;
                    let (Some(a), Some(xr)) = (shape.data.get(and_row + byte), shape.data.get(xor_row + byte)) else { continue };
                    let and = a & bit != 0;
                    let xor = xr & bit != 0;
                    for v in dst.iter_mut().take(3) {
                        *v = match (and, xor) {
                            (false, false) => 0x00,
                            (false, true) => 0xFF,
                            (true, false) => *v,
                            (true, true) => !*v,
                        };
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: u32, h: u32) -> Vec<u8> {
        vec![0x80; (w * h * 4) as usize]
    }

    #[test]
    fn color_cursor_respects_hotspot_alpha_and_clipping() {
        let mut f = gray(10, 10);
        // Cursor 2x2: píxel (0,0) blanco opaco, el resto transparente.
        let mut d = vec![0u8; 16];
        d[..4].copy_from_slice(&[255, 255, 255, 255]);
        let s = CursorShape { kind: CursorKind::Color, width: 2, height: 2, pitch: 8, data: d, hotspot_x: 1, hotspot_y: 1 };
        composite(&mut f, 10, 10, 40, &s, 5, 5);
        assert_eq!(&f[(4 * 40 + 4 * 4)..(4 * 40 + 4 * 4) + 3], &[255, 255, 255]); // (4,4) por el hotspot
        assert_eq!(f[5 * 40 + 5 * 4], 0x80); // transparente: sin cambios
        // Fuera de la imagen no rompe.
        composite(&mut f, 10, 10, 40, &s, 0, 0);
        composite(&mut f, 10, 10, 40, &s, 50, -3);
    }

    #[test]
    fn monochrome_and_masked_semantics() {
        // 8x2 monocromo (AND fila 0, XOR fila 1) => 1 fila visible.
        // bit0: AND=0 XOR=0 -> negro; bit1: AND=0 XOR=1 -> blanco; bit2: AND=1 XOR=0 -> igual; bit3: AND=1 XOR=1 -> invertido
        let data = vec![0b0011_0000, 0b0101_0000];
        let s = CursorShape { kind: CursorKind::Monochrome, width: 8, height: 2, pitch: 1, data, hotspot_x: 0, hotspot_y: 0 };
        let mut f = gray(8, 1);
        composite(&mut f, 8, 1, 32, &s, 0, 0);
        assert_eq!(f[0], 0x00);
        assert_eq!(f[4], 0xFF);
        assert_eq!(f[8], 0x80);
        assert_eq!(f[12], 0x7F);
        // Color enmascarado: alfa 0 reemplaza, alfa FF hace XOR.
        let s = CursorShape { kind: CursorKind::MaskedColor, width: 2, height: 1, pitch: 8, data: vec![1, 2, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF], hotspot_x: 0, hotspot_y: 0 };
        let mut f = gray(2, 1);
        composite(&mut f, 2, 1, 8, &s, 0, 0);
        assert_eq!(&f[..3], &[1, 2, 3]);
        assert_eq!(&f[4..7], &[0x7F, 0x7F, 0x7F]);
    }
}

//! Conversión BGRA (Desktop Duplication) -> NV12 (entrada de H.264) con
//! reducción de escala por PROMEDIO DE ÁREA. El promedio de área conserva
//! mejor el texto fino de la interfaz del DAW que el vecino más cercano.
//!
//! Espacio de color: BT.601 rango limitado (el que asumen por defecto la
//! mayoría de los decodificadores Android si el stream no trae VUI).
//!
//! Esta es la ruta por CPU (MVP). La ruta por GPU (D3D11 Video Processor)
//! queda para la optimización de Fase 1/3.

/// Precalcula, para cada píxel destino, el rango [inicio, fin) de píxeles
/// fuente que cubre (al menos 1).
fn spans(src: u32, dst: u32) -> Vec<(u32, u32)> {
    (0..dst)
        .map(|i| {
            let a = (i as u64 * src as u64 / dst as u64) as u32;
            let b = (((i + 1) as u64 * src as u64).div_ceil(dst as u64) as u32).clamp(a + 1, src);
            (a, b)
        })
        .collect()
}

/// `src`: BGRA con `stride` bytes por fila. `dw`/`dh` deben ser pares.
/// Devuelve NV12: plano Y (dw*dh) + plano UV intercalado (dw*dh/2).
pub fn bgra_to_nv12(src: &[u8], sw: u32, sh: u32, stride: u32, dw: u32, dh: u32) -> Vec<u8> {
    assert!(dw.is_multiple_of(2) && dh.is_multiple_of(2) && dw > 0 && dh > 0);
    assert!(src.len() >= (stride as usize) * (sh as usize - 1) + sw as usize * 4);
    let (dwu, dhu) = (dw as usize, dh as usize);
    let mut out = vec![0u8; dwu * dhu * 3 / 2];
    // RGB promediado por píxel destino (para calcular Y y luego UV por bloques 2x2).
    let mut rgb = vec![[0i32; 3]; dwu * 2];
    let xs = spans(sw, dw);
    let ys = spans(sh, dh);
    let same = sw == dw && sh == dh;
    for row_pair in 0..dhu / 2 {
        for k in 0..2 {
            let dy = row_pair * 2 + k;
            let (y0, y1) = ys[dy];
            for (dx, &(x0, x1)) in xs.iter().enumerate() {
                let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
                if same {
                    let o = dy * stride as usize + dx * 4;
                    b = src[o] as u32;
                    g = src[o + 1] as u32;
                    r = src[o + 2] as u32;
                    n = 1;
                } else {
                    for sy in y0..y1 {
                        let row = sy as usize * stride as usize;
                        for sx in x0..x1 {
                            let o = row + sx as usize * 4;
                            b += src[o] as u32;
                            g += src[o + 1] as u32;
                            r += src[o + 2] as u32;
                            n += 1;
                        }
                    }
                }
                let (r, g, b) = ((r / n) as i32, (g / n) as i32, (b / n) as i32);
                rgb[k * dwu + dx] = [r, g, b];
                // BT.601 limitado, punto fijo 8 bits.
                let yv = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
                out[dy * dwu + dx] = yv.clamp(16, 235) as u8;
            }
        }
        let uv_row = dwu * dhu + row_pair * dwu;
        for cx in 0..dwu / 2 {
            let mut s = [0i32; 3];
            for k in 0..2 {
                for j in 0..2 {
                    let p = rgb[k * dwu + cx * 2 + j];
                    s[0] += p[0];
                    s[1] += p[1];
                    s[2] += p[2];
                }
            }
            let (r, g, b) = (s[0] / 4, s[1] / 4, s[2] / 4);
            let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            out[uv_row + cx * 2] = u.clamp(16, 240) as u8;
            out[uv_row + cx * 2 + 1] = v.clamp(16, 240) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn white_black_and_red_map_to_bt601_limited() {
        let w = bgra_to_nv12(&solid(4, 4, [255, 255, 255, 255]), 4, 4, 16, 4, 4);
        assert!(w[..16].iter().all(|&y| y == 235));
        assert!(w[16..].iter().all(|&c| c == 128));
        let k = bgra_to_nv12(&solid(4, 4, [0, 0, 0, 255]), 4, 4, 16, 4, 4);
        assert!(k[..16].iter().all(|&y| y == 16));
        let red = bgra_to_nv12(&solid(4, 4, [0, 0, 255, 255]), 4, 4, 16, 4, 4);
        assert_eq!(red[0], 82); // Y de rojo puro en BT.601 limitado
        assert_eq!(red[17], 240); // V máximo
    }

    #[test]
    fn downscale_averages_area_and_respects_stride() {
        // 4x2 fuente: mitad izquierda negra, derecha blanca, con 8 bytes de relleno por fila.
        let stride = 4 * 4 + 8;
        let mut src = vec![0u8; stride * 2];
        for y in 0..2 {
            for x in 2..4 {
                let o = y * stride + x * 4;
                src[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let o = bgra_to_nv12(&src, 4, 2, stride as u32, 2, 2);
        assert_eq!(o[0], 16);
        assert_eq!(o[1], 235);
        // Reducción 3->2 no múltiplo: no debe entrar en pánico y debe cubrir todo.
        let s = solid(3, 3, [10, 20, 30, 255]);
        let o = bgra_to_nv12(&s, 3, 3, 12, 2, 2);
        assert!(o[..4].iter().all(|&y| y == o[0]));
    }

    #[test]
    fn spans_cover_source_without_gaps() {
        for (s, d) in [(2560u32, 1280u32), (1920, 1280), (1366, 800), (100, 98)] {
            let sp = spans(s, d);
            assert_eq!(sp[0].0, 0);
            assert_eq!(sp[d as usize - 1].1, s);
            for w in sp.windows(2) {
                assert!(w[1].0 <= w[0].1, "hueco entre {:?} y {:?}", w[0], w[1]);
            }
        }
    }
}

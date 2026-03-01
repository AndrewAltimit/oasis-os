//! YUV420p to RGBA conversion using BT.601 coefficients.

/// Convert a YUV420 planar image to RGBA.
///
/// `y`, `u`, `v` are the three planes.  `stride_y` / `stride_uv` are the byte
/// strides of each plane.  Output is `width * height * 4` bytes (RGBA).
pub fn yuv420_to_rgba(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    width: u32,
    height: u32,
    stride_y: usize,
    stride_uv: usize,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w * h * 4];

    for row in 0..h {
        let y_row = row * stride_y;
        let uv_row = (row / 2) * stride_uv;
        let dst_row = row * w * 4;

        for col in 0..w {
            let y_val = y[y_row + col] as i32;
            let u_val = u[uv_row + col / 2] as i32 - 128;
            let v_val = v[uv_row + col / 2] as i32 - 128;

            // BT.601 conversion
            let r = y_val + ((351 * v_val) >> 8);
            let g = y_val - ((179 * v_val + 86 * u_val) >> 8);
            let b = y_val + ((443 * u_val) >> 8);

            let dst = dst_row + col * 4;
            rgba[dst] = r.clamp(0, 255) as u8;
            rgba[dst + 1] = g.clamp(0, 255) as u8;
            rgba[dst + 2] = b.clamp(0, 255) as u8;
            rgba[dst + 3] = 255;
        }
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame() {
        // Y=0, U=128, V=128 → black (R=0, G=0, B=0)
        let y = vec![0u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let rgba = yuv420_to_rgba(&y, &u, &v, 2, 2, 2, 1);
        assert_eq!(rgba.len(), 16);
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel, &[0, 0, 0, 255]);
        }
    }

    #[test]
    fn white_frame() {
        // Y=255, U=128, V=128 → white (R=255, G=255, B=255)
        let y = vec![255u8; 4];
        let u = vec![128u8; 1];
        let v = vec![128u8; 1];
        let rgba = yuv420_to_rgba(&y, &u, &v, 2, 2, 2, 1);
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel, &[255, 255, 255, 255]);
        }
    }
}

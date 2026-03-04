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

    #[test]
    fn red_frame() {
        // Pure red in BT.601: Y=82, U=90, V=240
        let y = vec![82u8; 4];
        let u = vec![90u8; 1];
        let v = vec![240u8; 1];
        let rgba = yuv420_to_rgba(&y, &u, &v, 2, 2, 2, 1);
        for pixel in rgba.chunks(4) {
            // Approximate: BT.601 integer math won't be pixel-perfect.
            assert!(pixel[0] > 200, "R should be high, got {}", pixel[0]);
            assert!(pixel[1] < 50, "G should be low, got {}", pixel[1]);
            assert!(pixel[2] < 50, "B should be low, got {}", pixel[2]);
            assert_eq!(pixel[3], 255);
        }
    }

    #[test]
    fn output_length_correct() {
        let w: u32 = 8;
        let h: u32 = 6;
        let y = vec![128u8; (w * h) as usize];
        let u = vec![128u8; ((w / 2) * (h / 2)) as usize];
        let v = vec![128u8; ((w / 2) * (h / 2)) as usize];
        let rgba = yuv420_to_rgba(&y, &u, &v, w, h, w as usize, (w / 2) as usize);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }

    #[test]
    fn alpha_always_255() {
        // Random-ish Y/U/V values; alpha must always be 255.
        let y = vec![50, 100, 150, 200];
        let u = vec![60];
        let v = vec![200];
        let rgba = yuv420_to_rgba(&y, &u, &v, 2, 2, 2, 1);
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel[3], 255);
        }
    }

    #[test]
    fn stride_larger_than_width() {
        // Stride can be larger than width (e.g. 512 stride for 480 width).
        let stride_y: usize = 8;
        let stride_uv: usize = 4;
        let w: u32 = 4;
        let h: u32 = 4;
        let y = vec![128u8; stride_y * h as usize];
        let u = vec![128u8; stride_uv * (h as usize / 2)];
        let v = vec![128u8; stride_uv * (h as usize / 2)];
        let rgba = yuv420_to_rgba(&y, &u, &v, w, h, stride_y, stride_uv);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Mid-gray: Y=128, U=0, V=0 → R=G=B=128
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel[0], 128);
            assert_eq!(pixel[1], 128);
            assert_eq!(pixel[2], 128);
        }
    }
}

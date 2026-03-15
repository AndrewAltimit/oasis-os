//! YUV420p to RGBA conversion using BT.601 coefficients.

/// Convert a YUV420 planar image to RGBA.
///
/// `y`, `u`, `v` are the three planes.  `stride_y` / `stride_uv` are the byte
/// strides of each plane.  Output is `width * height * 4` bytes (RGBA).
///
/// Processes pixel pairs sharing the same chroma sample (4:2:0 subsampling)
/// to halve UV lookups.  Uses unchecked indexing within bounds-verified rows
/// to eliminate per-pixel bounds checks in the inner loop.
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

    // Number of full pixel pairs per row (rounded down).
    let pairs = w / 2;
    let has_odd = w & 1 != 0;

    for row in 0..h {
        let y_row = row * stride_y;
        let uv_row = (row / 2) * stride_uv;
        let dst_row = row * w * 4;

        // SAFETY: We verify that all indices accessed in the inner loop are
        // within bounds.  For the Y plane: y_row + pairs*2 (+ 1 if odd) <= y.len()
        // because stride_y >= w.  For U/V: uv_row + pairs (<= (w+1)/2) <= u/v.len()
        // because stride_uv >= ceil(w/2).  For dst: dst_row + w*4 <= rgba.len()
        // by construction (rgba has exactly w*h*4 bytes).
        debug_assert!(y_row + w <= y.len());
        debug_assert!(uv_row + w.div_ceil(2) <= u.len());
        debug_assert!(uv_row + w.div_ceil(2) <= v.len());
        debug_assert!(dst_row + w * 4 <= rgba.len());

        unsafe {
            for p in 0..pairs {
                let col = p * 2;
                let uv_col = p;

                // Shared chroma for the pixel pair.
                let u_val = *u.get_unchecked(uv_row + uv_col) as i32 - 128;
                let v_val = *v.get_unchecked(uv_row + uv_col) as i32 - 128;

                // Pre-compute chroma contributions (shared by both pixels).
                let cr = (351 * v_val) >> 8;
                let cg = (179 * v_val + 86 * u_val) >> 8;
                let cb = (443 * u_val) >> 8;

                // Left pixel.
                let y0 = *y.get_unchecked(y_row + col) as i32;
                let d0 = dst_row + col * 4;
                *rgba.get_unchecked_mut(d0) = (y0 + cr).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d0 + 1) = (y0 - cg).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d0 + 2) = (y0 + cb).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d0 + 3) = 255;

                // Right pixel.
                let y1 = *y.get_unchecked(y_row + col + 1) as i32;
                let d1 = d0 + 4;
                *rgba.get_unchecked_mut(d1) = (y1 + cr).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d1 + 1) = (y1 - cg).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d1 + 2) = (y1 + cb).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d1 + 3) = 255;
            }

            // Handle odd trailing pixel (reuses last chroma column).
            if has_odd {
                let col = pairs * 2;
                let u_val = *u.get_unchecked(uv_row + pairs) as i32 - 128;
                let v_val = *v.get_unchecked(uv_row + pairs) as i32 - 128;
                let y_val = *y.get_unchecked(y_row + col) as i32;
                let d = dst_row + col * 4;
                *rgba.get_unchecked_mut(d) = (y_val + ((351 * v_val) >> 8)).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d + 1) =
                    (y_val - ((179 * v_val + 86 * u_val) >> 8)).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d + 2) = (y_val + ((443 * u_val) >> 8)).clamp(0, 255) as u8;
                *rgba.get_unchecked_mut(d + 3) = 255;
            }
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
    fn odd_width() {
        // 3x2 image: pairs + remainder pixel per row.
        let w: u32 = 3;
        let h: u32 = 2;
        let y = vec![128u8; (w * h) as usize];
        let u = vec![128u8; ((w.div_ceil(2)) * ((h + 1) / 2)) as usize];
        let v = vec![128u8; ((w.div_ceil(2)) * ((h + 1) / 2)) as usize];
        let rgba = yuv420_to_rgba(&y, &u, &v, w, h, w as usize, (w.div_ceil(2)) as usize);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        for pixel in rgba.chunks(4) {
            assert_eq!(pixel[0], 128);
            assert_eq!(pixel[1], 128);
            assert_eq!(pixel[2], 128);
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

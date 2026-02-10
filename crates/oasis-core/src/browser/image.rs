//! Image decode dispatch and scaling for the browser.

use crate::backend::Color;

/// Decoded image data (RGBA pixels).
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data, 4 bytes per pixel.
    pub pixels: Vec<u8>,
}

/// Image format detected from content type or magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Bmp,
    Gif,
    Unknown,
}

/// Detect image format from the first few bytes (magic numbers).
pub fn detect_format(data: &[u8]) -> ImageFormat {
    if data.len() < 4 {
        return ImageFormat::Unknown;
    }

    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ImageFormat::Jpeg
    } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        ImageFormat::Png
    } else if data.starts_with(b"BM") {
        ImageFormat::Bmp
    } else if data.starts_with(b"GIF8") {
        ImageFormat::Gif
    } else {
        ImageFormat::Unknown
    }
}

/// Decode an image from raw bytes.
///
/// Returns the decoded RGBA pixel data with dimensions.
///
/// For v1.0, this provides a basic BMP decoder. JPEG and PNG require
/// external crate support (handled by the backend or crate features).
pub fn decode_image(data: &[u8]) -> Option<DecodedImage> {
    match detect_format(data) {
        ImageFormat::Bmp => decode_bmp(data),
        ImageFormat::Png => None,  // Requires `png` crate
        ImageFormat::Jpeg => None, // Requires `jpeg-decoder` crate
        ImageFormat::Gif => None,  // Requires `gif` crate
        ImageFormat::Unknown => None,
    }
}

/// Decode a BMP image (uncompressed 24-bit or 32-bit).
fn decode_bmp(data: &[u8]) -> Option<DecodedImage> {
    if data.len() < 54 {
        return None;
    }
    if &data[0..2] != b"BM" {
        return None;
    }

    let pixel_offset = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
    let width = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
    let height = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    let compression = u32::from_le_bytes([data[30], data[31], data[32], data[33]]);

    if width <= 0 || height == 0 {
        return None;
    }
    if compression != 0 {
        return None; // Only uncompressed
    }
    if bpp != 24 && bpp != 32 {
        return None;
    }

    let w = width as u32;
    let abs_h = height.unsigned_abs();
    let bottom_up = height > 0;
    let bytes_per_pixel = (bpp / 8) as usize;
    // Row size padded to 4-byte boundary.
    let row_size = (w as usize * bytes_per_pixel).div_ceil(4) * 4;

    let mut pixels = vec![0u8; (w * abs_h * 4) as usize];

    for row in 0..abs_h {
        let src_row = if bottom_up { abs_h - 1 - row } else { row };
        let src_offset = pixel_offset + src_row as usize * row_size;

        for col in 0..w {
            let src = src_offset + col as usize * bytes_per_pixel;
            let dst = (row * w + col) as usize * 4;

            if src + bytes_per_pixel > data.len() {
                return None;
            }
            if dst + 4 > pixels.len() {
                return None;
            }

            // BMP stores BGR(A).
            pixels[dst] = data[src + 2]; // R
            pixels[dst + 1] = data[src + 1]; // G
            pixels[dst + 2] = data[src]; // B
            pixels[dst + 3] = if bpp == 32 { data[src + 3] } else { 255 };
        }
    }

    Some(DecodedImage {
        width: w,
        height: abs_h,
        pixels,
    })
}

/// Scale an image to fit within max dimensions, preserving aspect ratio.
pub fn scale_to_fit(image: &DecodedImage, max_width: u32, max_height: u32) -> DecodedImage {
    if image.width <= max_width && image.height <= max_height {
        return image.clone();
    }

    let scale_x = max_width as f32 / image.width as f32;
    let scale_y = max_height as f32 / image.height as f32;
    let scale = scale_x.min(scale_y);

    let new_w = (image.width as f32 * scale) as u32;
    let new_h = (image.height as f32 * scale) as u32;

    bilinear_scale(image, new_w.max(1), new_h.max(1))
}

/// Scale image to exact dimensions using bilinear interpolation.
pub fn bilinear_scale(image: &DecodedImage, new_width: u32, new_height: u32) -> DecodedImage {
    let mut pixels = vec![0u8; (new_width * new_height * 4) as usize];

    let x_ratio = image.width as f32 / new_width as f32;
    let y_ratio = image.height as f32 / new_height as f32;

    for y in 0..new_height {
        for x in 0..new_width {
            let src_x = x as f32 * x_ratio;
            let src_y = y as f32 * y_ratio;

            let x0 = src_x as u32;
            let y0 = src_y as u32;
            let x1 = (x0 + 1).min(image.width - 1);
            let y1 = (y0 + 1).min(image.height - 1);

            let fx = src_x - x0 as f32;
            let fy = src_y - y0 as f32;

            let dst = (y * new_width + x) as usize * 4;

            for c in 0..4u32 {
                let p00 = get_pixel(image, x0, y0, c);
                let p10 = get_pixel(image, x1, y0, c);
                let p01 = get_pixel(image, x0, y1, c);
                let p11 = get_pixel(image, x1, y1, c);

                let top = p00 * (1.0 - fx) + p10 * fx;
                let bottom = p01 * (1.0 - fx) + p11 * fx;
                let value = top * (1.0 - fy) + bottom * fy;

                pixels[dst + c as usize] = value.round() as u8;
            }
        }
    }

    DecodedImage {
        width: new_width,
        height: new_height,
        pixels,
    }
}

fn get_pixel(image: &DecodedImage, x: u32, y: u32, channel: u32) -> f32 {
    let idx = (y * image.width + x) as usize * 4 + channel as usize;
    if idx < image.pixels.len() {
        image.pixels[idx] as f32
    } else {
        0.0
    }
}

/// Create a placeholder image for broken/unsupported images.
pub fn broken_image_placeholder(width: u32, height: u32) -> DecodedImage {
    let w = width.max(16);
    let h = height.max(16);
    let mut pixels = vec![255u8; (w * h * 4) as usize];

    let border_color = Color::rgb(180, 180, 180);

    // Draw border.
    for x in 0..w {
        set_pixel(&mut pixels, w, x, 0, border_color);
        set_pixel(&mut pixels, w, x, h - 1, border_color);
    }
    for y in 0..h {
        set_pixel(&mut pixels, w, 0, y, border_color);
        set_pixel(&mut pixels, w, w - 1, y, border_color);
    }

    // Draw X across the placeholder.
    let x_color = Color::rgb(200, 50, 50);
    let min_dim = w.min(h);
    for i in 2..min_dim.saturating_sub(2) {
        let px = i * w / min_dim;
        let py = i * h / min_dim;
        let py2 = h - 1 - py;
        if px < w && py < h {
            set_pixel(&mut pixels, w, px, py, x_color);
        }
        if px < w && py2 < h {
            set_pixel(&mut pixels, w, px, py2, x_color);
        }
    }

    DecodedImage {
        width: w,
        height: h,
        pixels,
    }
}

fn set_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, color: Color) {
    let idx = (y * width + x) as usize * 4;
    if idx + 3 < pixels.len() {
        pixels[idx] = color.r;
        pixels[idx + 1] = color.g;
        pixels[idx + 2] = color.b;
        pixels[idx + 3] = color.a;
    }
}

/// Calculate display dimensions for an image, preserving aspect ratio.
///
/// If only one dimension is specified, scale the other proportionally.
pub fn calculate_display_size(
    intrinsic_w: u32,
    intrinsic_h: u32,
    attr_w: Option<u32>,
    attr_h: Option<u32>,
    max_width: u32,
) -> (u32, u32) {
    match (attr_w, attr_h) {
        (Some(w), Some(h)) => (w.min(max_width), h),
        (Some(w), None) => {
            let w = w.min(max_width);
            let h = if intrinsic_w > 0 {
                (intrinsic_h as f32 * w as f32 / intrinsic_w as f32) as u32
            } else {
                intrinsic_h
            };
            (w, h)
        },
        (None, Some(h)) => {
            let w = if intrinsic_h > 0 {
                (intrinsic_w as f32 * h as f32 / intrinsic_h as f32) as u32
            } else {
                intrinsic_w
            };
            (w.min(max_width), h)
        },
        (None, None) => {
            let w = intrinsic_w.min(max_width);
            let h = if intrinsic_w > 0 && w < intrinsic_w {
                (intrinsic_h as f32 * w as f32 / intrinsic_w as f32) as u32
            } else {
                intrinsic_h
            };
            (w, h)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_jpeg_magic_bytes() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_format(&data), ImageFormat::Jpeg);
    }

    #[test]
    fn detect_png_magic_bytes() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        assert_eq!(detect_format(&data), ImageFormat::Png);
    }

    #[test]
    fn detect_bmp_magic_bytes() {
        let data = [b'B', b'M', 0x00, 0x00, 0x00, 0x00];
        assert_eq!(detect_format(&data), ImageFormat::Bmp);
    }

    #[test]
    fn detect_gif_magic_bytes() {
        let data = b"GIF89a";
        assert_eq!(detect_format(data), ImageFormat::Gif);
    }

    #[test]
    fn detect_unknown_format() {
        let data = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_format(&data), ImageFormat::Unknown);
    }

    #[test]
    fn detect_too_short_data() {
        let data = [0xFF, 0xD8];
        assert_eq!(detect_format(&data), ImageFormat::Unknown);
    }

    /// Build a minimal valid 24-bit uncompressed BMP (2x2 pixels).
    fn make_test_bmp_24bit() -> Vec<u8> {
        let w: u32 = 2;
        let h: u32 = 2;
        let bpp: u16 = 24;
        let row_bytes = ((w * 3 + 3) / 4) * 4; // 8 bytes (padded)
        let pixel_data_size = row_bytes * h;
        let file_size = 54 + pixel_data_size;

        let mut bmp = vec![0u8; file_size as usize];

        // BMP file header (14 bytes).
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[2..6].copy_from_slice(&file_size.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());

        // DIB header (40 bytes).
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        bmp[22..26].copy_from_slice(&(h as i32).to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes()); // planes
        bmp[28..30].copy_from_slice(&bpp.to_le_bytes());
        bmp[30..34].copy_from_slice(&0u32.to_le_bytes()); // no compression

        // Pixel data (bottom-up, BGR).
        // Row 0 (bottom row): red, green.
        let off = 54;
        bmp[off] = 0;
        bmp[off + 1] = 0;
        bmp[off + 2] = 255; // BGR -> Red
        bmp[off + 3] = 0;
        bmp[off + 4] = 255;
        bmp[off + 5] = 0; // BGR -> Green

        // Row 1 (top row): blue, white.
        let off2 = 54 + row_bytes as usize;
        bmp[off2] = 255;
        bmp[off2 + 1] = 0;
        bmp[off2 + 2] = 0; // BGR -> Blue
        bmp[off2 + 3] = 255;
        bmp[off2 + 4] = 255;
        bmp[off2 + 5] = 255; // BGR -> White

        bmp
    }

    #[test]
    fn decode_bmp_24bit() {
        let bmp_data = make_test_bmp_24bit();
        let img = decode_bmp(&bmp_data).expect("should decode BMP");

        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.pixels.len(), 2 * 2 * 4);

        // Top-left pixel (row 0, col 0) should be blue
        // (bottom-up BMP: top row is last in file = row 1).
        assert_eq!(img.pixels[0], 0); // R
        assert_eq!(img.pixels[1], 0); // G
        assert_eq!(img.pixels[2], 255); // B
        assert_eq!(img.pixels[3], 255); // A

        // Top-right pixel (row 0, col 1) should be white.
        assert_eq!(img.pixels[4], 255); // R
        assert_eq!(img.pixels[5], 255); // G
        assert_eq!(img.pixels[6], 255); // B
        assert_eq!(img.pixels[7], 255); // A

        // Bottom-left pixel (row 1, col 0) should be red.
        assert_eq!(img.pixels[8], 255); // R
        assert_eq!(img.pixels[9], 0); // G
        assert_eq!(img.pixels[10], 0); // B
        assert_eq!(img.pixels[11], 255); // A

        // Bottom-right pixel (row 1, col 1) should be green.
        assert_eq!(img.pixels[12], 0); // R
        assert_eq!(img.pixels[13], 255); // G
        assert_eq!(img.pixels[14], 0); // B
        assert_eq!(img.pixels[15], 255); // A
    }

    #[test]
    fn decode_image_dispatches_to_bmp() {
        let bmp_data = make_test_bmp_24bit();
        let img = decode_image(&bmp_data);
        assert!(img.is_some());
        assert_eq!(img.unwrap().width, 2);
    }

    #[test]
    fn decode_image_returns_none_for_png() {
        // PNG magic followed by garbage.
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(decode_image(&data).is_none());
    }

    #[test]
    fn scale_to_fit_larger_than_max() {
        let img = DecodedImage {
            width: 200,
            height: 100,
            pixels: vec![128u8; 200 * 100 * 4],
        };
        let scaled = scale_to_fit(&img, 100, 100);

        assert_eq!(scaled.width, 100);
        assert_eq!(scaled.height, 50);
        assert_eq!(scaled.pixels.len(), (100 * 50 * 4) as usize);
    }

    #[test]
    fn scale_to_fit_already_fits() {
        let img = DecodedImage {
            width: 50,
            height: 30,
            pixels: vec![128u8; 50 * 30 * 4],
        };
        let scaled = scale_to_fit(&img, 100, 100);

        // Should return a clone, same dimensions.
        assert_eq!(scaled.width, 50);
        assert_eq!(scaled.height, 30);
    }

    #[test]
    fn bilinear_scale_produces_correct_dimensions() {
        let img = DecodedImage {
            width: 4,
            height: 4,
            pixels: vec![255u8; 4 * 4 * 4],
        };
        let scaled = bilinear_scale(&img, 8, 6);

        assert_eq!(scaled.width, 8);
        assert_eq!(scaled.height, 6);
        assert_eq!(scaled.pixels.len(), (8 * 6 * 4) as usize);
    }

    #[test]
    fn bilinear_scale_uniform_image() {
        // A solid-color image should remain solid after scaling.
        let img = DecodedImage {
            width: 2,
            height: 2,
            pixels: vec![
                100, 150, 200, 255, 100, 150, 200, 255, 100, 150, 200, 255, 100, 150, 200, 255,
            ],
        };
        let scaled = bilinear_scale(&img, 4, 4);

        for chunk in scaled.pixels.chunks(4) {
            assert_eq!(chunk[0], 100);
            assert_eq!(chunk[1], 150);
            assert_eq!(chunk[2], 200);
            assert_eq!(chunk[3], 255);
        }
    }

    #[test]
    fn broken_image_placeholder_dimensions() {
        let img = broken_image_placeholder(64, 48);
        assert_eq!(img.width, 64);
        assert_eq!(img.height, 48);
        assert_eq!(img.pixels.len(), (64 * 48 * 4) as usize);
    }

    #[test]
    fn broken_image_placeholder_minimum_size() {
        let img = broken_image_placeholder(4, 4);
        // Minimum enforced to 16x16.
        assert_eq!(img.width, 16);
        assert_eq!(img.height, 16);
    }

    #[test]
    fn calculate_display_size_both_dimensions() {
        let (w, h) = calculate_display_size(100, 200, Some(50), Some(80), 480);
        assert_eq!(w, 50);
        assert_eq!(h, 80);
    }

    #[test]
    fn calculate_display_size_both_dimensions_clamped() {
        // Width exceeds max_width.
        let (w, h) = calculate_display_size(100, 200, Some(600), Some(80), 480);
        assert_eq!(w, 480);
        assert_eq!(h, 80);
    }

    #[test]
    fn calculate_display_size_only_width() {
        let (w, h) = calculate_display_size(200, 100, Some(100), None, 480);
        assert_eq!(w, 100);
        // Height should be scaled proportionally: 100 * 100/200 = 50.
        assert_eq!(h, 50);
    }

    #[test]
    fn calculate_display_size_only_height() {
        let (w, h) = calculate_display_size(200, 100, None, Some(50), 480);
        // Width scaled proportionally: 200 * 50/100 = 100.
        assert_eq!(w, 100);
        assert_eq!(h, 50);
    }

    #[test]
    fn calculate_display_size_no_dimensions_fits() {
        let (w, h) = calculate_display_size(200, 100, None, None, 480);
        // Fits within max_width, so unchanged.
        assert_eq!(w, 200);
        assert_eq!(h, 100);
    }

    #[test]
    fn calculate_display_size_no_dimensions_constrained() {
        let (w, h) = calculate_display_size(960, 480, None, None, 480);
        // Constrained to max_width 480, height scaled: 480*480/960 = 240.
        assert_eq!(w, 480);
        assert_eq!(h, 240);
    }
}

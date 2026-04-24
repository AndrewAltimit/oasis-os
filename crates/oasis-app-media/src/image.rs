//! Minimal image decode for the Photo Viewer.
//!
//! Handles PNG and JPEG — the two formats that cover every sample and
//! essentially every photo in practice. Larger images are downscaled at
//! decode time so a 10-megapixel phone photo doesn't blow a 4 MB pixel
//! budget on an embedded target. The other formats (GIF/BMP/WebP) fall
//! through to text metadata like before.

/// Decoded RGBA pixel buffer, pre-scaled to fit inside a budget.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Maximum dimension (width or height) we keep in memory. Anything
/// larger is bilinearly downscaled during decode.
pub const MAX_DIMENSION: u32 = 1024;

/// Decode a PNG or JPEG from bytes, returning `None` for unsupported
/// formats or corrupt data (never panics — we catch decode panics from
/// the underlying crates).
pub fn decode(data: &[u8]) -> Option<DecodedImage> {
    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
            decode_png(data)
        } else if data.len() >= 3 && data[..3] == [0xFF, 0xD8, 0xFF] {
            decode_jpeg(data)
        } else {
            None
        }
    }))
    .ok()
    .flatten()?;

    Some(fit_within(decoded, MAX_DIMENSION))
}

fn decode_png(data: &[u8]) -> Option<DecodedImage> {
    let mut decoder = png::Decoder::new(data);
    // Expand palettes and sub-byte grayscale to full RGB/RGBA so we
    // always get a known-sized output.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    if w == 0 || h == 0 {
        return None;
    }
    let color = info.color_type;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut buf).ok()?;

    let rgba = match color {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        },
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for &v in &buf {
                out.extend_from_slice(&[v, v, v, 255]);
            }
            out
        },
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in buf.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        },
        // Palette would have been expanded to RGB/RGBA by
        // Transformations::EXPAND; bail for anything else.
        _ => return None,
    };
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

fn decode_jpeg(data: &[u8]) -> Option<DecodedImage> {
    let mut dec = jpeg_decoder::Decoder::new(data);
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in pixels.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        },
        jpeg_decoder::PixelFormat::L8 => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for &v in &pixels {
                out.extend_from_slice(&[v, v, v, 255]);
            }
            out
        },
        // Progressive CMYK, L16, RGB48 — rare; fall through.
        _ => return None,
    };
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

/// Downscale so neither dimension exceeds `max`. Uses nearest-neighbour —
/// good enough for a photo viewer and avoids the extra dependency on a
/// filter crate.
fn fit_within(img: DecodedImage, max: u32) -> DecodedImage {
    let (w, h) = (img.width, img.height);
    if w <= max && h <= max {
        return img;
    }
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(1);
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        let src_y = ((y * h) / nh).min(h - 1);
        for x in 0..nw {
            let src_x = ((x * w) / nw).min(w - 1);
            let idx = ((src_y * w + src_x) * 4) as usize;
            out.extend_from_slice(&img.rgba[idx..idx + 4]);
        }
    }
    DecodedImage {
        width: nw,
        height: nh,
        rgba: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        // 2x2 RGB PNG generated via png crate.
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, 2, 2);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128])
                .unwrap();
        }
        buf
    }

    #[test]
    fn decode_png_rgb() {
        let png = tiny_png();
        let img = decode(&png).expect("png decode");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.rgba.len(), 16);
        // First pixel is red with opaque alpha.
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn decode_unsupported_returns_none() {
        assert!(decode(b"not an image").is_none());
    }

    #[test]
    fn decode_empty_returns_none() {
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn fit_within_preserves_small() {
        let img = DecodedImage {
            width: 10,
            height: 10,
            rgba: vec![0; 400],
        };
        let out = fit_within(img, 100);
        assert_eq!(out.width, 10);
        assert_eq!(out.height, 10);
    }

    #[test]
    fn fit_within_scales_large() {
        let img = DecodedImage {
            width: 2000,
            height: 1000,
            rgba: vec![0; 2000 * 1000 * 4],
        };
        let out = fit_within(img, 1024);
        assert_eq!(out.width, 1024);
        assert_eq!(out.height, 512);
    }
}

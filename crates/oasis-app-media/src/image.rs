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
/// larger is downscaled after decode.
pub const MAX_DIMENSION: u32 = 1024;

/// Largest source image (in pixels) we agree to decode at all: the full
/// frame is decoded before downscaling, so a file claiming e.g.
/// 65535x65535 would otherwise ask for gigabytes (an allocation failure
/// aborts, which no `catch_unwind` can stop). 40 MP covers phone photos.
pub const MAX_SOURCE_PIXELS: u64 = 40_000_000;

/// Whether a `w` x `h` source passes [`MAX_SOURCE_PIXELS`] (and isn't
/// empty).
fn source_size_ok(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && u64::from(w) * u64::from(h) <= MAX_SOURCE_PIXELS
}

/// RGBA byte length of a `w` x `h` image.
fn rgba_len(w: u32, h: u32) -> usize {
    (w as usize) * (h as usize) * 4
}

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
    if decoded.rgba.len() != rgba_len(decoded.width, decoded.height) {
        return None;
    }

    Some(fit_within(decoded, MAX_DIMENSION))
}

fn decode_png(data: &[u8]) -> Option<DecodedImage> {
    let mut decoder = png::Decoder::new(data);
    // EXPAND: promote palettes and sub-byte grayscale to 8-bit channels.
    // STRIP_16: downsample 16-bit channels to 8-bit so the match arms
    // below can assume 1 byte per channel.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    // Checked before `output_buffer_size()` is allocated.
    if !source_size_ok(w, h) {
        return None;
    }
    // Use the post-transformation color type so palette-indexed PNGs
    // (expanded to RGB/RGBA by EXPAND) match the correct arm below.
    let (color, _) = reader.output_color_type();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut buf).ok()?;

    let rgba = match color {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(rgba_len(w, h));
            for px in buf.as_chunks::<3>().0.iter() {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        },
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(rgba_len(w, h));
            for &v in &buf {
                out.extend_from_slice(&[v, v, v, 255]);
            }
            out
        },
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(rgba_len(w, h));
            for px in buf.as_chunks::<2>().0.iter() {
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
    // Read the header first and refuse oversized frames before the
    // decoder allocates its buffers; the buffer cap backs that up.
    dec.read_info().ok()?;
    let info = dec.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    if !source_size_ok(w, h) {
        return None;
    }
    dec.set_max_decoding_buffer_size(MAX_SOURCE_PIXELS as usize * 3);
    let pixels = dec.decode().ok()?;
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let mut out = Vec::with_capacity(rgba_len(w, h));
            for px in pixels.as_chunks::<3>().0.iter() {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        },
        jpeg_decoder::PixelFormat::L8 => {
            let mut out = Vec::with_capacity(rgba_len(w, h));
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

/// Rotate `img` 90 degrees clockwise (width and height swap).
pub fn rotate_cw(img: &DecodedImage) -> DecodedImage {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = vec![0u8; img.rgba.len()];
    for y in 0..h {
        for x in 0..w {
            // Source (x, y) lands at (h - 1 - y, x) in the h-wide output.
            let src = (y * w + x) * 4;
            let dst = (x * h + (h - 1 - y)) * 4;
            out[dst..dst + 4].copy_from_slice(&img.rgba[src..src + 4]);
        }
    }
    DecodedImage {
        width: img.height,
        height: img.width,
        rgba: out,
    }
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
    let mut out = Vec::with_capacity(rgba_len(nw, nh));
    // Use u64 for the intermediate product: `y * h` wraps u32 for
    // inputs above ~4 megapixels in a single dimension, which is well
    // inside the `MAX_DIMENSION` cap but still worth being correct.
    let (w64, h64, nw64, nh64) = (w as u64, h as u64, nw as u64, nh as u64);
    for y in 0..nh {
        let src_y = (((y as u64) * h64) / nh64).min(h64 - 1) as u32;
        for x in 0..nw {
            let src_x = (((x as u64) * w64) / nw64).min(w64 - 1) as u32;
            let idx = ((src_y as u64 * w64 + src_x as u64) * 4) as usize;
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

    /// A PNG whose IHDR claims `w` x `h` (no image data needed: the size
    /// check runs right after the header).
    fn png_header_only(w: u32, h: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let _writer = enc.write_header().unwrap();
        }
        buf
    }

    #[test]
    fn rotate_cw_moves_pixels_and_swaps_dims() {
        // 2x1: red, green -> 1x2: red on top, green below.
        let img = DecodedImage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let r = rotate_cw(&img);
        assert_eq!((r.width, r.height), (1, 2));
        assert_eq!(r.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        // Four turns are the identity.
        let sq = DecodedImage {
            width: 2,
            height: 2,
            rgba: (0..16).collect(),
        };
        let back = rotate_cw(&rotate_cw(&rotate_cw(&rotate_cw(&sq))));
        assert_eq!(back.rgba, sq.rgba);
        assert_ne!(rotate_cw(&sq).rgba, sq.rgba);
    }

    #[test]
    fn oversized_png_is_rejected_before_allocating() {
        assert!(decode(&png_header_only(65_535, 65_535)).is_none());
        assert!(!source_size_ok(10_000, 10_000));
        assert!(source_size_ok(4000, 3000));
        assert!(!source_size_ok(0, 10));
    }

    #[test]
    fn oversized_jpeg_header_is_rejected() {
        // SOI + SOF0 claiming 65000x65000, 3 components; no scan data.
        let mut jpg = vec![0xFF, 0xD8];
        jpg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        jpg.extend_from_slice(&65_000u16.to_be_bytes());
        jpg.extend_from_slice(&65_000u16.to_be_bytes());
        jpg.extend_from_slice(&[0x03, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        jpg.extend_from_slice(&[0xFF, 0xD9]);
        assert!(decode(&jpg).is_none());
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
    fn decode_png_palette() {
        // Palette-indexed PNG. EXPAND converts it to RGB; the decoder
        // must use the post-transformation color type to match the buffer.
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, 2, 2);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128]);
            let mut writer = enc.write_header().expect("png header");
            writer
                .write_image_data(&[0, 1, 2, 3])
                .expect("palette png data");
        }
        let img = decode(&buf).expect("palette png decode");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.rgba.len(), 16);
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

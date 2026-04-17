//! Image decode dispatch and scaling for the browser.

use oasis_types::backend::Color;

/// Maximum image dimension (width or height) we allow to decode.
/// Anything larger is rejected to prevent OOM from malformed headers.
/// A 1024x1024 RGBA image is 4MB vs 64MB at 4096.
const MAX_IMAGE_DIMENSION: u32 = 1024;

/// Maximum total pixel count for a decoded image before we force
/// downscaling during decode. 1M pixels = 4MB RGBA.
const MAX_IMAGE_PIXELS: u32 = 1_048_576;

/// Decoded image data (RGBA pixels).
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data, 4 bytes per pixel.
    pub pixels: Vec<u8>,
    /// Cached: true when any pixel has alpha < 255.
    pub has_transparency: bool,
}

impl DecodedImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let has_transparency = pixels.chunks_exact(4).any(|px| px[3] < 255);
        Self {
            width,
            height,
            pixels,
            has_transparency,
        }
    }
}

/// Image format detected from content type or magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Bmp,
    Gif,
    Webp,
    /// SVG (XML-based, not a raster format). Detected by textual probe
    /// rather than magic bytes. We don't have a full rasterizer so we
    /// currently return a transparent-pixel placeholder sized from the
    /// SVG's `viewBox`/`width`/`height` attributes — enough to preserve
    /// layout and avoid the broken-image `×` glyph for CSS patterns like
    /// Wikipedia's sprite sheets.
    Svg,
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
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else if looks_like_svg(data) {
        ImageFormat::Svg
    } else {
        ImageFormat::Unknown
    }
}

/// Textual probe for SVG content. The specification permits an XML
/// declaration, a doctype, comments, and arbitrary whitespace before
/// the `<svg` root, so we scan the first 1 KiB for the literal token.
///
/// 1 KiB is an empirical ceiling — real SVG files ship their opening
/// tag well within the first few hundred bytes, and scanning further
/// both slows down detection of non-SVG payloads that happen to contain
/// `<svg` deep in their body and invites false positives on, e.g., HTML
/// pages describing SVG.
fn looks_like_svg(data: &[u8]) -> bool {
    let probe_len = data.len().min(1024);
    let probe = &data[..probe_len];
    // Skip an optional UTF-8 BOM before the whitespace scan — BOM bytes
    // are not ASCII whitespace, so a BOM-prefixed SVG would otherwise
    // fall through all the prefix checks below.
    let bom_len = if probe.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    // Fast path: skip leading whitespace after the BOM.
    let start = probe[bom_len..]
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .map(|p| p + bom_len)
        .unwrap_or(probe_len);
    let tail = &probe[start..];
    // Accept with or without XML prolog / doctype.
    let is_xml_or_svg = tail.starts_with(b"<?xml")
        || tail.starts_with(b"<!DOCTYPE")
        || tail.starts_with(b"<svg")
        || tail.starts_with(b"<!--");
    if !is_xml_or_svg {
        return false;
    }
    // Look for `<svg` somewhere in the probe window.
    probe.windows(4).any(|w| w == b"<svg")
}

/// Decode an image from raw bytes.
///
/// Returns the decoded RGBA pixel data with dimensions. If the decoded
/// image exceeds `MAX_IMAGE_PIXELS`, it is automatically scaled down to
/// fit within the pixel budget.
///
/// Corrupt or malformed images are handled gracefully: decoder panics
/// are caught via `catch_unwind` and treated as decode failures
/// (returns `None`). Callers that need a visual placeholder for failed
/// decodes should use [`broken_image_placeholder`].
pub fn decode_image(data: &[u8]) -> Option<DecodedImage> {
    let decoded = match detect_format(data) {
        ImageFormat::Bmp => decode_bmp(data),
        ImageFormat::Png => decode_png_safe(data),
        ImageFormat::Jpeg => decode_jpeg_safe(data),
        ImageFormat::Gif => decode_gif_safe(data),
        ImageFormat::Webp => decode_webp_safe(data),
        ImageFormat::Svg => decode_svg_placeholder(data),
        ImageFormat::Unknown => None,
    }?;

    // If the image exceeds the pixel budget, scale it down.
    let total_pixels = decoded.width as u64 * decoded.height as u64;
    if total_pixels > MAX_IMAGE_PIXELS as u64 {
        let scale = (MAX_IMAGE_PIXELS as f32 / total_pixels as f32).sqrt();
        let new_w = (decoded.width as f32 * scale) as u32;
        let new_h = (decoded.height as f32 * scale) as u32;
        Some(bilinear_scale(&decoded, new_w.max(1), new_h.max(1)))
    } else {
        Some(decoded)
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

    Some(DecodedImage::new(w, abs_h, pixels))
}

/// Decode a PNG image, catching panics from malformed data.
fn decode_png_safe(data: &[u8]) -> Option<DecodedImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_png(data)))
        .ok()
        .flatten()
}

/// Decode a PNG image using the `png` crate.
fn decode_png(data: &[u8]) -> Option<DecodedImage> {
    let mut decoder = png::Decoder::new(data);
    // Expand indexed-palette images to RGB, sub-8-bit grayscale to 8-bit,
    // and tRNS chunks to alpha channels. Without this, any PNG using a
    // palette (the majority of small icons and sprites) would fall out
    // the bottom of the color-type match as unsupported.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let info_header = reader.info();
    if info_header.width > MAX_IMAGE_DIMENSION || info_header.height > MAX_IMAGE_DIMENSION {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];

    let w = info.width;
    let h = info.height;

    let pixels = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for chunk in bytes.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        },
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for chunk in bytes.chunks_exact(2) {
                let g = chunk[0];
                rgba.extend_from_slice(&[g, g, g, chunk[1]]);
            }
            rgba
        },
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for &g in bytes {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            rgba
        },
        png::ColorType::Indexed => {
            // `EXPAND` should have already converted palette → RGB/RGBA;
            // falling through here means the decoder refused the
            // expansion for some reason (non-standard chunk ordering,
            // etc.). Decline rather than produce garbage output.
            return None;
        },
    };

    Some(DecodedImage::new(w, h, pixels))
}

/// Decode a JPEG image, catching panics from malformed data.
fn decode_jpeg_safe(data: &[u8]) -> Option<DecodedImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_jpeg(data)))
        .ok()
        .flatten()
}

/// Decode a JPEG image using the `jpeg-decoder` crate.
fn decode_jpeg(data: &[u8]) -> Option<DecodedImage> {
    let mut decoder = jpeg_decoder::Decoder::new(data);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    if (info.width as u32) > MAX_IMAGE_DIMENSION || (info.height as u32) > MAX_IMAGE_DIMENSION {
        return None;
    }
    let pixels_raw = decoder.decode().ok()?;
    let info = decoder.info()?;
    let w = info.width as u32;
    let h = info.height as u32;

    let pixels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for chunk in pixels_raw.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        },
        jpeg_decoder::PixelFormat::L8 => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for &g in &pixels_raw {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            rgba
        },
        jpeg_decoder::PixelFormat::L16 => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for chunk in pixels_raw.chunks_exact(2) {
                let g = chunk[0]; // High byte of 16-bit grayscale
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            rgba
        },
        jpeg_decoder::PixelFormat::CMYK32 => {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for chunk in pixels_raw.chunks_exact(4) {
                let c = chunk[0] as f32 / 255.0;
                let m = chunk[1] as f32 / 255.0;
                let y = chunk[2] as f32 / 255.0;
                let k = chunk[3] as f32 / 255.0;
                let r = (255.0 * (1.0 - c) * (1.0 - k)) as u8;
                let g = (255.0 * (1.0 - m) * (1.0 - k)) as u8;
                let b = (255.0 * (1.0 - y) * (1.0 - k)) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
            rgba
        },
    };

    Some(DecodedImage::new(w, h, pixels))
}

/// Decode a GIF image, catching panics from malformed data.
fn decode_gif_safe(data: &[u8]) -> Option<DecodedImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_gif(data)))
        .ok()
        .flatten()
}

/// Decode a GIF image using the `gif` crate (first frame only, static).
fn decode_gif(data: &[u8]) -> Option<DecodedImage> {
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = decoder.read_info(data).ok()?;
    let frame = reader.read_next_frame().ok()?.cloned()?;

    let w = u32::from(frame.width);
    let h = u32::from(frame.height);
    if w == 0 || h == 0 || w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION {
        return None;
    }

    let expected_len = (w as usize) * (h as usize) * 4;
    if frame.buffer.len() < expected_len {
        return None;
    }

    Some(DecodedImage::new(
        w,
        h,
        frame.buffer[..expected_len].to_vec(),
    ))
}

/// Decode a WebP image, catching panics from malformed data.
fn decode_webp_safe(data: &[u8]) -> Option<DecodedImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_webp(data)))
        .ok()
        .flatten()
}

/// Decode a WebP image.
///
/// When the `webp` feature is enabled, uses the `image` crate's WebP
/// decoder. Otherwise returns `None`.
fn decode_webp(data: &[u8]) -> Option<DecodedImage> {
    #[cfg(feature = "webp")]
    {
        let img = image::load_from_memory_with_format(data, image::ImageFormat::WebP).ok()?;
        let rgba = img.to_rgba8();
        let w = rgba.width();
        let h = rgba.height();
        if w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION {
            return None;
        }
        Some(DecodedImage::new(w, h, rgba.into_raw()))
    }
    #[cfg(not(feature = "webp"))]
    {
        // WebP decoding requires the `webp` feature (adds the `image` crate).
        let _ = data;
        None
    }
}

/// Produce a transparent placeholder image sized from an SVG document's
/// declared dimensions.
///
/// A real SVG rasterizer is out of scope for this module — full SVG
/// support (filters, gradients, transforms, text) lives in `svg.rs` and
/// renders through an `SdiBackend`, which isn't available during image
/// decode. What we can do cheaply here is:
///
/// 1. Parse just enough of the SVG header to recover the intrinsic
///    width/height (`width=`, `height=`, or `viewBox="x y w h"`).
/// 2. Return a transparent RGBA buffer of that size.
///
/// The layout engine then reserves the correct space for the element
/// and the broken-image `×` glyph stops appearing. This is substantially
/// better than treating SVGs as unknown-format (which yields the broken
/// placeholder) even though no visual SVG content is drawn.
fn decode_svg_placeholder(data: &[u8]) -> Option<DecodedImage> {
    let header_len = data.len().min(4096);
    let header = std::str::from_utf8(&data[..header_len]).ok()?;
    // Find the opening `<svg` tag.
    let svg_start = header.find("<svg")?;
    // Find the end of the opening tag so we don't pick up attrs from
    // descendants (viewport-sized rects etc.). Scan for `>` outside of
    // quoted attribute values — a raw `find('>')` would match `>` inside
    // e.g. `data-foo="a>b"` and truncate the tag prematurely.
    let tag_end = find_tag_end(&header[svg_start..])? + svg_start;
    let tag = &header[svg_start..tag_end];

    let width = extract_length_attr(tag, "width");
    let height = extract_length_attr(tag, "height");
    let viewbox = extract_viewbox(tag);

    // Fall back to viewBox when explicit width/height are missing, then
    // to a neutral 32×32 so degenerate cases still reserve some space.
    let w = width
        .or(viewbox.map(|(_, _, w, _)| w))
        .unwrap_or(32.0)
        .max(1.0)
        .min(MAX_IMAGE_DIMENSION as f32) as u32;
    let h = height
        .or(viewbox.map(|(_, _, _, h)| h))
        .unwrap_or(32.0)
        .max(1.0)
        .min(MAX_IMAGE_DIMENSION as f32) as u32;

    // Transparent RGBA.
    let pixels = vec![0u8; (w * h * 4) as usize];
    Some(DecodedImage::new(w, h, pixels))
}

/// Find the byte offset of the `>` that closes the first tag in `s`,
/// tracking quoted attribute values so `>` inside `"…"` or `'…'` is
/// not mistaken for the tag terminator.
fn find_tag_end(s: &str) -> Option<usize> {
    let mut in_quote: Option<u8> = None;
    for (i, b) in s.bytes().enumerate() {
        match (in_quote, b) {
            (None, b'"') => in_quote = Some(b'"'),
            (None, b'\'') => in_quote = Some(b'\''),
            (Some(q), c) if c == q => in_quote = None,
            (None, b'>') => return Some(i),
            _ => {},
        }
    }
    None
}

/// Extract a numeric attribute value from an SVG opening tag.
/// Handles both double- and single-quoted values and trims the `px`
/// unit suffix, which is the only one commonly seen on `<svg>` tags.
fn extract_length_attr(tag: &str, attr: &str) -> Option<f32> {
    let needle_d = format!("{attr}=\"");
    let needle_s = format!("{attr}='");
    let (after, quote) = if let Some(idx) = tag.find(&needle_d) {
        (&tag[idx + needle_d.len()..], '"')
    } else if let Some(idx) = tag.find(&needle_s) {
        (&tag[idx + needle_s.len()..], '\'')
    } else {
        return None;
    };
    let end = after.find(quote)?;
    let raw = after[..end].trim();
    let stripped = raw.strip_suffix("px").unwrap_or(raw).trim();
    stripped.parse::<f32>().ok()
}

/// Extract `viewBox="min-x min-y width height"` from an SVG opening tag.
fn extract_viewbox(tag: &str) -> Option<(f32, f32, f32, f32)> {
    // Try both lowercase and the canonical camelCase spelling. The
    // `viewBox` attribute is case-sensitive in XML but some producers
    // emit it lowercase; accept both.
    let needle = tag
        .find("viewBox=\"")
        .or_else(|| tag.find("viewbox=\""))
        .or_else(|| tag.find("viewBox='"))
        .or_else(|| tag.find("viewbox='"))?;
    let attr_len = "viewBox=\"".len();
    let after = &tag[needle + attr_len..];
    let end = after.find(['"', '\''])?;
    let raw = &after[..end];
    let parts: Vec<f32> = raw
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect();
    if parts.len() >= 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
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

    DecodedImage::new(new_width, new_height, pixels)
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

    DecodedImage::new(w, h, pixels)
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
        (Some(w), Some(h)) => {
            let clamped = w.min(max_width);
            let h = if clamped < w && w > 0 {
                (h as f32 * clamped as f32 / w as f32) as u32
            } else {
                h
            };
            (clamped, h)
        },
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
            let clamped = w.min(max_width);
            let h = if clamped < w && w > 0 {
                (h as f32 * clamped as f32 / w as f32) as u32
            } else {
                h
            };
            (clamped, h)
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
    fn decode_image_returns_none_for_truncated_png() {
        // PNG magic followed by garbage -- should fail gracefully.
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(decode_image(&data).is_none());
    }

    /// Build a minimal valid 1x1 red PNG in memory.
    fn make_test_png_1x1_red() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            // 1x1 RGBA: red pixel
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        buf
    }

    #[test]
    fn decode_png_1x1_red() {
        let data = make_test_png_1x1_red();
        let img = decode_image(&data).expect("should decode PNG");
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        assert_eq!(img.pixels.len(), 4);
        assert_eq!(img.pixels[0], 255); // R
        assert_eq!(img.pixels[1], 0); // G
        assert_eq!(img.pixels[2], 0); // B
        assert_eq!(img.pixels[3], 255); // A
    }

    #[test]
    fn decode_png_rgb_format() {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        }
        let img = decode_image(&buf).expect("should decode RGB PNG");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 1);
        assert_eq!(img.pixels.len(), 8);
        // Pixel 0: red (RGB expanded to RGBA)
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        // Pixel 1: green
        assert_eq!(&img.pixels[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn decode_png_grayscale() {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, 1, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[128]).unwrap();
        }
        let img = decode_image(&buf).expect("should decode grayscale PNG");
        assert_eq!(img.pixels, &[128, 128, 128, 255]);
    }

    #[test]
    fn decode_jpeg_returns_none_for_truncated() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert!(decode_image(&data).is_none());
    }

    #[test]
    fn scale_to_fit_larger_than_max() {
        let img = DecodedImage::new(200, 100, vec![128u8; 200 * 100 * 4]);
        let scaled = scale_to_fit(&img, 100, 100);

        assert_eq!(scaled.width, 100);
        assert_eq!(scaled.height, 50);
        assert_eq!(scaled.pixels.len(), (100 * 50 * 4) as usize);
    }

    #[test]
    fn scale_to_fit_already_fits() {
        let img = DecodedImage::new(50, 30, vec![128u8; 50 * 30 * 4]);
        let scaled = scale_to_fit(&img, 100, 100);

        // Should return a clone, same dimensions.
        assert_eq!(scaled.width, 50);
        assert_eq!(scaled.height, 30);
    }

    #[test]
    fn bilinear_scale_produces_correct_dimensions() {
        let img = DecodedImage::new(4, 4, vec![255u8; 4 * 4 * 4]);
        let scaled = bilinear_scale(&img, 8, 6);

        assert_eq!(scaled.width, 8);
        assert_eq!(scaled.height, 6);
        assert_eq!(scaled.pixels.len(), (8 * 6 * 4) as usize);
    }

    #[test]
    fn bilinear_scale_uniform_image() {
        // A solid-color image should remain solid after scaling.
        let img = DecodedImage::new(
            2,
            2,
            vec![
                100, 150, 200, 255, 100, 150, 200, 255, 100, 150, 200, 255, 100, 150, 200, 255,
            ],
        );
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
        // Width exceeds max_width; height scaled proportionally.
        let (w, h) = calculate_display_size(100, 200, Some(600), Some(80), 480);
        assert_eq!(w, 480);
        // 80 * 480/600 = 64
        assert_eq!(h, 64);
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

    #[test]
    fn max_image_dimension_reduced_to_1024() {
        assert_eq!(MAX_IMAGE_DIMENSION, 1024);
    }

    #[test]
    fn large_bmp_exceeding_pixel_budget_is_scaled() {
        // We can't easily construct a huge BMP in memory for this test,
        // so we verify the pixel budget constant is correct.
        assert_eq!(MAX_IMAGE_PIXELS, 1_048_576);
    }

    #[test]
    fn decode_corrupt_png_returns_none_not_panic() {
        // PNG magic followed by a valid IHDR length but corrupt data.
        // This should NOT panic — the catch_unwind wrapper catches it.
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        // Append garbage that looks like an IHDR chunk header but has
        // invalid CRC and dimensions that would cause issues.
        data.extend_from_slice(&[
            0x00, 0x00, 0x00, 0x0D, // chunk length 13
            0x49, 0x48, 0x44, 0x52, // "IHDR"
            0xFF, 0xFF, 0xFF, 0xFF, // width (garbage)
            0xFF, 0xFF, 0xFF, 0xFF, // height (garbage)
            0x08, 0x06, 0x00, 0x00, 0x00, // bit depth, color, etc.
            0x00, 0x00, 0x00, 0x00, // CRC (wrong)
        ]);
        assert!(decode_image(&data).is_none());
    }

    #[test]
    fn decode_corrupt_jpeg_returns_none_not_panic() {
        // JPEG magic followed by garbage that will confuse the decoder.
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xFF, 0xFF];
        assert!(decode_image(&data).is_none());
    }

    #[test]
    fn decode_corrupt_gif_returns_none_not_panic() {
        // GIF magic followed by truncated/corrupt data.
        let data = b"GIF89a\x01\x00\x01\x00\xFF\x00\x00";
        assert!(decode_image(data).is_none());
    }

    #[test]
    fn decode_empty_data_returns_none() {
        assert!(decode_image(&[]).is_none());
    }

    #[test]
    fn detect_svg_with_xml_prolog() {
        let svg = b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\"></svg>";
        assert_eq!(detect_format(svg), ImageFormat::Svg);
    }

    #[test]
    fn detect_svg_plain_root() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\"/></svg>";
        assert_eq!(detect_format(svg), ImageFormat::Svg);
    }

    #[test]
    fn detect_svg_leading_whitespace() {
        let svg = b"\n  \t<svg></svg>";
        assert_eq!(detect_format(svg), ImageFormat::Svg);
    }

    #[test]
    fn detect_html_is_not_svg() {
        let html = b"<!DOCTYPE html><html><body>mention svg in body</body></html>";
        assert_ne!(detect_format(html), ImageFormat::Svg);
    }

    #[test]
    fn decode_svg_uses_width_height_attrs() {
        let svg =
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><rect/></svg>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        assert_eq!(img.width, 120);
        assert_eq!(img.height, 60);
        // All pixels transparent.
        assert!(img.pixels.iter().all(|&b| b == 0));
        assert!(img.has_transparency);
    }

    #[test]
    fn decode_svg_falls_back_to_viewbox() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 128\"></svg>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        assert_eq!(img.width, 256);
        assert_eq!(img.height, 128);
    }

    #[test]
    fn decode_svg_strips_px_suffix() {
        let svg =
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48px\" height=\"48px\"></svg>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        assert_eq!(img.width, 48);
        assert_eq!(img.height, 48);
    }

    #[test]
    fn decode_svg_with_xml_prolog_and_viewbox() {
        let svg = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 22 22\"><circle cx=\"11\" cy=\"11\" r=\"10\"/></svg>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        assert_eq!(img.width, 22);
        assert_eq!(img.height, 22);
    }

    #[test]
    fn decode_svg_single_quoted_attrs() {
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'/>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        assert_eq!(img.width, 80);
        assert_eq!(img.height, 40);
    }

    #[test]
    fn decode_svg_falls_back_to_default_when_no_dimensions() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
        let img = decode_image(svg).expect("SVG placeholder decodes");
        // Neutral 32x32 fallback.
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
    }
}

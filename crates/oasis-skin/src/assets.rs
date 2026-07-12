//! Skin image assets.
//!
//! Skins can ship PNG images in an `assets/` subdirectory. Each image is
//! decoded to raw RGBA at load time and stored on the [`Skin`] keyed by its
//! path relative to the skin directory (e.g. `"assets/bar_top.png"`) — the
//! same string layout objects, wallpapers, and background layers use to
//! reference it.
//!
//! [`Skin`]: crate::Skin

use oasis_types::error::{OasisError, Result};

/// Soft budget for total decoded asset bytes per skin. PSIX shipped ~3.6 MB
/// of uncompressed sprites; PSP-friendly skins should stay well under this.
pub const ASSET_BUDGET_BYTES: usize = 2 * 1024 * 1024;

/// A decoded skin image: raw RGBA8 pixels ready for texture upload.
#[derive(Debug, Clone)]
pub struct SkinAsset {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixel data (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

impl SkinAsset {
    /// Decode a PNG byte stream to RGBA8.
    ///
    /// Grayscale, grayscale+alpha, RGB, and 16-bit images are expanded to
    /// 8-bit RGBA so every asset uploads through the same texture path.
    pub fn from_png_bytes(bytes: &[u8]) -> Result<Self> {
        let mut decoder = png::Decoder::new(bytes);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| OasisError::Config(format!("png: {e}").into()))?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| OasisError::Config(format!("png: {e}").into()))?;
        buf.truncate(info.buffer_size());

        let (width, height) = (info.width, info.height);
        let pixels = (width as usize) * (height as usize);
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => {
                let mut out = Vec::with_capacity(pixels * 4);
                for px in buf.chunks_exact(3) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
                out
            },
            png::ColorType::GrayscaleAlpha => {
                let mut out = Vec::with_capacity(pixels * 4);
                for px in buf.chunks_exact(2) {
                    out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
                out
            },
            png::ColorType::Grayscale => {
                let mut out = Vec::with_capacity(pixels * 4);
                for &g in &buf {
                    out.extend_from_slice(&[g, g, g, 255]);
                }
                out
            },
            other => {
                return Err(OasisError::Config(
                    format!("png: unsupported color type {other:?}").into(),
                ));
            },
        };

        if rgba.len() != pixels * 4 {
            return Err(OasisError::Config(
                format!(
                    "png: decoded size mismatch ({} bytes for {width}x{height})",
                    rgba.len()
                )
                .into(),
            ));
        }

        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Whether both dimensions are powers of two. PSP GU textures require
    /// power-of-two dimensions; desktop backends do not care.
    pub fn is_power_of_two(&self) -> bool {
        self.width.is_power_of_two() && self.height.is_power_of_two()
    }

    /// Decoded size in bytes.
    pub fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encode a small PNG in memory for decode tests.
    pub(crate) fn encode_png(width: u32, height: u32, color_type: png::ColorType) -> Vec<u8> {
        let channels = match color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => 4,
        };
        let data: Vec<u8> = (0..(width * height * channels) as usize)
            .map(|i| (i % 251) as u8)
            .collect();
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color_type);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer.write_image_data(&data).expect("png data");
        }
        out
    }

    #[test]
    fn decode_rgba_png() {
        let bytes = encode_png(8, 4, png::ColorType::Rgba);
        let asset = SkinAsset::from_png_bytes(&bytes).unwrap();
        assert_eq!(asset.width, 8);
        assert_eq!(asset.height, 4);
        assert_eq!(asset.rgba.len(), 8 * 4 * 4);
    }

    #[test]
    fn decode_rgb_png_expands_alpha() {
        let bytes = encode_png(4, 4, png::ColorType::Rgb);
        let asset = SkinAsset::from_png_bytes(&bytes).unwrap();
        assert_eq!(asset.rgba.len(), 4 * 4 * 4);
        // Every 4th byte is the synthesized opaque alpha.
        for px in asset.rgba.chunks_exact(4) {
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn decode_grayscale_png() {
        let bytes = encode_png(4, 2, png::ColorType::Grayscale);
        let asset = SkinAsset::from_png_bytes(&bytes).unwrap();
        assert_eq!(asset.rgba.len(), 4 * 2 * 4);
        for px in asset.rgba.chunks_exact(4) {
            assert_eq!(px[0], px[1]);
            assert_eq!(px[1], px[2]);
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn decode_grayscale_alpha_png() {
        let bytes = encode_png(2, 2, png::ColorType::GrayscaleAlpha);
        let asset = SkinAsset::from_png_bytes(&bytes).unwrap();
        assert_eq!(asset.rgba.len(), 2 * 2 * 4);
    }

    #[test]
    fn decode_garbage_fails() {
        assert!(SkinAsset::from_png_bytes(b"not a png").is_err());
    }

    #[test]
    fn power_of_two_check() {
        let pot = SkinAsset {
            width: 64,
            height: 32,
            rgba: vec![0; 64 * 32 * 4],
        };
        assert!(pot.is_power_of_two());
        let npot = SkinAsset {
            width: 48,
            height: 32,
            rgba: vec![0; 48 * 32 * 4],
        };
        assert!(!npot.is_power_of_two());
    }
}

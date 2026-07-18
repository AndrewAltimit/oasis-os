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

/// Header facts of a skin sound asset, parsed without decoding the PCM.
///
/// Mirrors what `oasis-audio`'s WAV decoder accepts (uncompressed PCM,
/// mono/stereo, 8- or 16-bit) so validation warnings match runtime
/// behaviour without pulling an audio dependency into `oasis-skin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Bits per sample (8 or 16).
    pub bits_per_sample: u16,
    /// Size of the PCM `data` chunk in bytes.
    pub data_bytes: usize,
}

impl WavInfo {
    /// Playback duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        let bytes_per_sec =
            self.sample_rate as f32 * self.channels as f32 * (self.bits_per_sample / 8) as f32;
        if bytes_per_sec <= 0.0 {
            return 0.0;
        }
        self.data_bytes as f32 / bytes_per_sec
    }
}

/// Probe a WAV byte stream's header. Returns `None` for anything the
/// runtime decoder would reject (non-RIFF, compressed, >2 channels,
/// unusual bit depths, missing chunks).
pub fn probe_wav(data: &[u8]) -> Option<WavInfo> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let (fmt_off, _) = find_riff_chunk(data, b"fmt ")?;
    if fmt_off + 16 > data.len() {
        return None;
    }
    let u16_le = |off: usize| u16::from_le_bytes([data[off], data[off + 1]]);
    let audio_format = u16_le(fmt_off);
    if audio_format != 1 {
        return None; // Not uncompressed PCM.
    }
    let channels = u16_le(fmt_off + 2);
    let sample_rate = u32::from_le_bytes([
        data[fmt_off + 4],
        data[fmt_off + 5],
        data[fmt_off + 6],
        data[fmt_off + 7],
    ]);
    let bits_per_sample = u16_le(fmt_off + 14);
    if channels == 0 || channels > 2 || sample_rate == 0 {
        return None;
    }
    if bits_per_sample != 8 && bits_per_sample != 16 {
        return None;
    }
    let (data_off, data_size) = find_riff_chunk(data, b"data")?;
    let data_bytes = data_size.min(data.len().saturating_sub(data_off));
    Some(WavInfo {
        sample_rate,
        channels,
        bits_per_sample,
        data_bytes,
    })
}

/// Find a RIFF sub-chunk by ID. Returns (data_offset, declared_size).
fn find_riff_chunk(data: &[u8], id: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = 12; // Skip the 12-byte RIFF header.
    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize;
        if chunk_id == id {
            return Some((pos + 8, chunk_size));
        }
        // Chunks are padded to even sizes.
        let padded = chunk_size.saturating_add(1) & !1;
        pos = pos.checked_add(8 + padded)?;
    }
    None
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

    /// Build a minimal valid 16-bit PCM WAV in memory (shared with the
    /// loader / validation tests).
    pub(crate) fn make_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample / 8);
        let block_align = channels * (bits_per_sample / 8);
        let data_size = (samples.len() * 2) as u32;
        let file_size = 36 + data_size;

        let mut buf = Vec::with_capacity(file_size as usize + 8);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        buf
    }

    #[test]
    fn probe_wav_reads_header() {
        let wav = make_wav(&[0i16; 4410], 44_100, 1);
        let info = probe_wav(&wav).expect("probe");
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.data_bytes, 8820);
        assert!((info.duration_secs() - 0.1).abs() < 0.001);
    }

    #[test]
    fn probe_wav_rejects_garbage() {
        assert!(probe_wav(b"not a wav").is_none());
        assert!(probe_wav(&[]).is_none());
    }

    #[test]
    fn probe_wav_rejects_compressed() {
        let mut wav = make_wav(&[0i16; 16], 22_050, 1);
        // Patch the format tag (offset 20) to something non-PCM.
        wav[20] = 2;
        assert!(probe_wav(&wav).is_none());
    }

    #[test]
    fn probe_wav_rejects_many_channels() {
        let mut wav = make_wav(&[0i16; 16], 22_050, 2);
        wav[22] = 6; // channels
        assert!(probe_wav(&wav).is_none());
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

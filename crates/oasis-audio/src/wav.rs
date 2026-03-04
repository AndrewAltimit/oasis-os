//! Minimal WAV file decoder.
//!
//! Supports uncompressed PCM (format tag 1) in 8-bit unsigned or 16-bit signed
//! little-endian, mono or stereo.  Returns interleaved i16 samples at the
//! source sample rate.

/// Decoded WAV audio data.
pub struct WavData {
    /// Interleaved i16 PCM samples.
    pub samples: Vec<i16>,
    /// Sample rate in Hz (e.g. 44100).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
}

/// Check if `data` starts with a RIFF/WAVE header.
pub fn is_wav(data: &[u8]) -> bool {
    data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE"
}

/// Decode a WAV file from raw bytes.
///
/// Only uncompressed PCM (format tag 1) is supported.
/// Returns `None` if the data is not a valid WAV or uses an unsupported format.
pub fn decode_wav(data: &[u8]) -> Option<WavData> {
    if !is_wav(data) {
        return None;
    }

    // Find the "fmt " sub-chunk.
    let (fmt_offset, _fmt_size) = find_chunk(data, b"fmt ")?;
    if fmt_offset + 16 > data.len() {
        return None;
    }

    let audio_format = u16_le(data, fmt_offset);
    if audio_format != 1 {
        // Not PCM.
        return None;
    }

    let channels = u16_le(data, fmt_offset + 2);
    let sample_rate = u32_le(data, fmt_offset + 4);
    // bytes 8..11 = byte rate (skip)
    // bytes 12..13 = block align (skip)
    let bits_per_sample = u16_le(data, fmt_offset + 14);

    if channels == 0 || channels > 2 {
        return None;
    }
    if bits_per_sample != 8 && bits_per_sample != 16 {
        return None;
    }

    // Find the "data" sub-chunk.
    let (data_offset, data_size) = find_chunk(data, b"data")?;
    let end = data_offset.saturating_add(data_size).min(data.len());
    let pcm_bytes = &data[data_offset..end];

    let samples = if bits_per_sample == 16 {
        // 16-bit signed little-endian.
        pcm_bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    } else {
        // 8-bit unsigned → i16.
        pcm_bytes
            .iter()
            .map(|&b| ((b as i16) - 128) * 256)
            .collect()
    };

    Some(WavData {
        samples,
        sample_rate,
        channels,
    })
}

/// Find a RIFF chunk by its 4-byte ID. Returns (data_offset, data_size).
fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = 12; // Skip RIFF header (12 bytes).
    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32_le(data, pos + 4) as usize;
        if chunk_id == id {
            return Some((pos + 8, chunk_size));
        }
        // Advance to next chunk (size is padded to even boundary).
        let padded = (chunk_size.saturating_add(1)) & !1;
        pos = match pos.checked_add(8 + padded) {
            Some(next) => next,
            None => break,
        };
    }
    None
}

fn u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid WAV file with the given PCM samples.
    fn make_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample / 8);
        let block_align = channels * (bits_per_sample / 8);
        let data_size = (samples.len() * 2) as u32;
        let file_size = 36 + data_size;

        let mut buf = Vec::with_capacity(file_size as usize + 8);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }

        buf
    }

    #[test]
    fn is_wav_valid() {
        let wav = make_wav(&[0, 1, 2, 3], 44100, 1);
        assert!(is_wav(&wav));
    }

    #[test]
    fn is_wav_invalid() {
        assert!(!is_wav(b"not a wav file"));
        assert!(!is_wav(&[]));
        assert!(!is_wav(&[0; 12]));
    }

    #[test]
    fn decode_mono_16bit() {
        let samples = vec![0i16, 1000, -1000, 32767, -32768];
        let wav = make_wav(&samples, 22050, 1);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.samples, samples);
        assert_eq!(decoded.sample_rate, 22050);
        assert_eq!(decoded.channels, 1);
    }

    #[test]
    fn decode_stereo_16bit() {
        let samples = vec![100, -100, 200, -200];
        let wav = make_wav(&samples, 44100, 2);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.samples, samples);
        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.channels, 2);
    }

    #[test]
    fn decode_8bit_unsigned() {
        // Build a WAV with 8-bit samples manually.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&36u32.to_le_bytes()); // file size placeholder
        buf.extend_from_slice(b"WAVE");

        // fmt chunk (8-bit, mono, 8000 Hz)
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
        buf.extend_from_slice(&8000u32.to_le_bytes()); // byte rate
        buf.extend_from_slice(&1u16.to_le_bytes()); // block align
        buf.extend_from_slice(&8u16.to_le_bytes()); // bits per sample

        // data chunk: 128 = silence in 8-bit unsigned
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.push(128); // silence → 0
        buf.push(255); // max → 127 * 256 = 32512

        // Fix file size.
        let file_size = (buf.len() - 8) as u32;
        buf[4..8].copy_from_slice(&file_size.to_le_bytes());

        let decoded = decode_wav(&buf).unwrap();
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.samples[0], 0); // 128 - 128 = 0, * 256 = 0
        assert!(decoded.samples[1] > 30000); // (255-128)*256 = 32512
    }

    #[test]
    fn decode_garbage_returns_none() {
        assert!(decode_wav(b"not a wav").is_none());
        assert!(decode_wav(&[]).is_none());
    }

    #[test]
    fn decode_truncated_returns_none() {
        // Valid RIFF header but no chunks.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        assert!(decode_wav(&buf).is_none());
    }
}

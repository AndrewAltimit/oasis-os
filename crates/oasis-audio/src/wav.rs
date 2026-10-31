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

/// `fmt ` format tag of uncompressed integer PCM.
const WAVE_FORMAT_PCM: u16 = 1;
/// `fmt ` format tag whose real format is in the SubFormat GUID.
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

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

    // Find the "fmt " sub-chunk; its 16 fixed bytes must be present and
    // inside the chunk (not read from whatever follows a short one).
    let (fmt_offset, fmt_size) = find_chunk(data, b"fmt ")?;
    if fmt_size < 16 || fmt_offset + 16 > data.len() {
        return None;
    }

    let audio_format = match u16_le(data, fmt_offset) {
        WAVE_FORMAT_PCM => WAVE_FORMAT_PCM,
        // WAVE_FORMAT_EXTENSIBLE: the real format is the first two bytes of
        // the SubFormat GUID at offset 24 (written by many tools for PCM).
        WAVE_FORMAT_EXTENSIBLE if fmt_size >= 40 && fmt_offset + 26 <= data.len() => {
            u16_le(data, fmt_offset + 24)
        },
        other => other,
    };
    if audio_format != WAVE_FORMAT_PCM {
        return None;
    }

    let channels = u16_le(data, fmt_offset + 2);
    let sample_rate = u32_le(data, fmt_offset + 4);
    // bytes 8..11 = byte rate (skip)
    // bytes 12..13 = block align (skip)
    let bits_per_sample = u16_le(data, fmt_offset + 14);

    // A 0 Hz rate would divide by zero in every player's position math.
    if channels == 0 || channels > 2 || sample_rate == 0 {
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
            .as_chunks::<2>()
            .0
            .iter()
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

    #[test]
    fn decode_extensible_pcm() {
        // WAVE_FORMAT_EXTENSIBLE with the PCM SubFormat GUID (as written by
        // many editors for plain 16-bit PCM) used to be rejected.
        let samples = [5i16, -5, 7, -7];
        let mut wav = make_wav(&samples, 48000, 2);
        let mut ext = Vec::new();
        ext.extend_from_slice(&0xFFFEu16.to_le_bytes());
        ext.extend_from_slice(&wav[22..36]); // channels..bits_per_sample
        ext.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        ext.extend_from_slice(&16u16.to_le_bytes()); // valid bits
        ext.extend_from_slice(&3u32.to_le_bytes()); // channel mask
        ext.extend_from_slice(&1u16.to_le_bytes()); // SubFormat: PCM GUID
        ext.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
        wav.splice(16..36, [40u32.to_le_bytes().as_slice(), &ext].concat());
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.samples, samples);
        assert_eq!((decoded.sample_rate, decoded.channels), (48000, 2));
        // Extensible with a non-PCM SubFormat (IEEE float = 3) is refused.
        let float_at = 20 + 24;
        wav[float_at] = 3;
        assert!(decode_wav(&wav).is_none());
    }

    #[test]
    fn short_fmt_chunk_and_zero_rate_are_rejected() {
        // Regression: a fmt chunk shorter than 16 bytes was read past its
        // end (into the next chunk), and a 0 Hz rate was accepted.
        let mut wav = make_wav(&[1, 2], 44100, 1);
        wav[16..20].copy_from_slice(&8u32.to_le_bytes());
        assert!(decode_wav(&wav).is_none());
        let mut wav = make_wav(&[1, 2], 0, 1);
        assert!(decode_wav(&wav).is_none());
        wav[24..28].copy_from_slice(&8000u32.to_le_bytes());
        assert!(decode_wav(&wav).is_some());
    }

    #[test]
    fn truncated_and_bit_flipped_wavs_never_panic() {
        let wav = make_wav(&[0, 1000, -1000, 32767, -32768, 5, 6, 7], 22050, 2);
        for len in 0..wav.len() {
            let _ = decode_wav(&wav[..len]);
        }
        for byte in 0..wav.len() {
            for bit in 0..8 {
                let mut m = wav.clone();
                m[byte] ^= 1 << bit;
                if let Some(d) = decode_wav(&m) {
                    assert!(d.channels > 0 && d.sample_rate > 0);
                }
            }
        }
    }
}

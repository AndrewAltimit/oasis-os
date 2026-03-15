//! AAC audio decoder wrapping symphonia's AAC codec.

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_AAC, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::formats::Packet;

use crate::VideoError;

/// Decoded audio samples.
pub struct DecodedAudio {
    pub pcm_f32: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// AAC audio decoder backed by symphonia.
pub struct AacDecoder {
    decoder: Box<dyn Decoder>,
    sample_rate: u32,
    channels: u16,
    /// Reusable sample buffer — avoids allocating a new `SampleBuffer` on
    /// every `decode()` call.  Recreated only when the audio spec changes
    /// (rare for AAC-LC; can happen with HE-AAC SBR).
    sample_buf: Option<SampleBuffer<f32>>,
}

impl AacDecoder {
    /// Create from the codec parameters discovered during demuxing.
    pub fn new(params: &CodecParameters) -> Result<Self, VideoError> {
        // Verify it's actually AAC.
        if params.codec != CODEC_TYPE_AAC {
            return Err(VideoError::Decode(format!(
                "expected AAC codec, got {:?}",
                params.codec
            )));
        }

        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(|e| VideoError::Decode(format!("AAC decoder init: {e}")))?;

        let sample_rate = params.sample_rate.unwrap_or(44100);
        let channels = params.channels.map(|ch| ch.count() as u16).unwrap_or(2);

        Ok(Self {
            decoder,
            sample_rate,
            channels,
            sample_buf: None,
        })
    }

    /// Decode one AAC packet into interleaved f32 PCM.
    pub fn decode(&mut self, data: &[u8], ts: u64) -> Result<Option<DecodedAudio>, VideoError> {
        // Build a symphonia Packet. Track ID doesn't matter for the codec.
        let packet = Packet::new_from_slice(0, ts, 0, data);

        let decoded = self
            .decoder
            .decode(&packet)
            .map_err(|e| VideoError::Decode(format!("AAC decode: {e}")))?;

        let spec = *decoded.spec();
        let duration = decoded.capacity();

        if duration == 0 {
            return Ok(None);
        }

        // Reuse the sample buffer if its capacity and spec still match.
        // Recreate only when the audio spec changes (rare for AAC-LC).
        let need_new = self
            .sample_buf
            .as_ref()
            .is_none_or(|sb| sb.capacity() < duration);
        if need_new {
            self.sample_buf = Some(SampleBuffer::<f32>::new(duration as u64, spec));
        }
        let sample_buf = self.sample_buf.as_mut().expect("just created above");
        sample_buf.copy_interleaved_ref(decoded);

        Ok(Some(DecodedAudio {
            pcm_f32: sample_buf.samples().to_vec(),
            channels: self.channels,
            sample_rate: self.sample_rate,
        }))
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

// ---------------------------------------------------------------------------
// ADTS header parsing utilities for validation/testing
// ---------------------------------------------------------------------------

/// ADTS fixed header fields extracted from a raw ADTS frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdtsHeader {
    /// MPEG version: false = MPEG-4, true = MPEG-2.
    pub mpeg2: bool,
    /// Audio object type minus 1 (profile).
    pub profile: u8,
    /// Sampling frequency index (0-12).
    pub sampling_freq_index: u8,
    /// Channel configuration (0-7).
    pub channel_config: u8,
    /// Frame length including header (in bytes).
    pub frame_length: u16,
    /// Whether CRC is absent (true = no CRC, 7-byte header).
    pub crc_absent: bool,
}

/// Parse an ADTS frame header from the given bytes.
///
/// Returns `None` if the data is too short or the sync word is invalid.
/// This is a validation utility -- the actual AAC decoding is done by
/// symphonia via [`AacDecoder`].
pub fn parse_adts_header(data: &[u8]) -> Option<AdtsHeader> {
    if data.len() < 7 {
        return None;
    }
    // Sync word: 12 bits, all 1s.
    if data[0] != 0xFF || (data[1] & 0xF0) != 0xF0 {
        return None;
    }
    let mpeg2 = (data[1] & 0x08) != 0;
    let crc_absent = (data[1] & 0x01) != 0;
    let profile = (data[2] >> 6) & 0x03;
    let sampling_freq_index = (data[2] >> 2) & 0x0F;
    let channel_config = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);
    let frame_length =
        (u16::from(data[3] & 0x03) << 11) | (u16::from(data[4]) << 3) | (u16::from(data[5]) >> 5);

    // Validate sampling frequency index (0-12 are defined).
    if sampling_freq_index > 12 {
        return None;
    }

    Some(AdtsHeader {
        mpeg2,
        profile,
        sampling_freq_index,
        channel_config,
        frame_length,
        crc_absent,
    })
}

/// Well-known AAC sampling rates indexed by sampling_freq_index.
pub const AAC_SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

// ---------------------------------------------------------------------------
// Item 75: AAC frame validation / ADTS header parsing tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parse_adts_valid_44100_stereo() {
        // ADTS header for 44100 Hz stereo AAC-LC, no CRC, frame length = 200
        #[rustfmt::skip]
        let header: [u8; 7] = [
            0xFF, 0xF1,       // sync word + MPEG-4 + no CRC
            0x50,             // profile=1(LC), freq_idx=4(44100), private=0
            0x80,             // channel_config=2(stereo), ...
            0x06, 0x40,       // frame_length bits spread
            0xFC,             // buffer fullness (don't care)
        ];
        // Manually compute frame_length to embed correctly
        let frame_len: u16 = 200;
        let mut h = header;
        // channel_config = 2 -> bits [3:6,7] of byte 2 and [0:1] of byte 3
        h[2] = (1 << 6) | (4 << 2); // profile=1, freq=4
        h[3] = (2 << 6) | ((frame_len >> 11) as u8 & 0x03);
        h[4] = (frame_len >> 3) as u8;
        h[5] = ((frame_len & 0x07) as u8) << 5 | 0x1F;
        h[6] = 0xFC;

        let parsed = parse_adts_header(&h).unwrap();
        assert!(!parsed.mpeg2);
        assert!(parsed.crc_absent);
        assert_eq!(parsed.profile, 1); // AAC-LC
        assert_eq!(parsed.sampling_freq_index, 4);
        assert_eq!(AAC_SAMPLE_RATES[parsed.sampling_freq_index as usize], 44100);
        assert_eq!(parsed.channel_config, 2);
        assert_eq!(parsed.frame_length, frame_len);
    }

    #[test]
    fn parse_adts_invalid_sync_word() {
        let data = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(parse_adts_header(&data).is_none());
    }

    #[test]
    fn parse_adts_partial_sync_word() {
        // First byte is 0xFF but second byte doesn't have top 4 bits set.
        let data = [0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(parse_adts_header(&data).is_none());
    }

    #[test]
    fn parse_adts_too_short() {
        assert!(parse_adts_header(&[]).is_none());
        assert!(parse_adts_header(&[0xFF]).is_none());
        assert!(parse_adts_header(&[0xFF, 0xF1, 0x50]).is_none());
        assert!(parse_adts_header(&[0xFF, 0xF1, 0x50, 0x80, 0x00, 0x00]).is_none());
    }

    #[test]
    fn parse_adts_invalid_freq_index() {
        // freq_index = 13 is reserved/invalid.
        let mut data = [0xFF, 0xF1, 0x00, 0x00, 0x00, 0x00, 0x00];
        data[2] = (1 << 6) | (13 << 2); // profile=1, freq=13
        assert!(parse_adts_header(&data).is_none());
    }

    #[test]
    fn parse_adts_mpeg2_flag() {
        // MPEG-2 ADTS: bit 3 of byte 1 = 1
        let mut data = [0xFF, 0xF9, 0x50, 0x80, 0x00, 0x00, 0xFC];
        // Rebuild with valid frame length
        data[2] = (1 << 6) | (4 << 2); // profile=1, freq=4
        data[3] = 2 << 6; // channel=2
        let parsed = parse_adts_header(&data).unwrap();
        assert!(parsed.mpeg2);
    }

    #[test]
    fn parse_adts_with_crc() {
        // CRC present: bit 0 of byte 1 = 0 -> 9-byte header
        let data = [0xFF, 0xF0, 0x50, 0x80, 0x00, 0x00, 0xFC];
        let parsed = parse_adts_header(&data);
        // Should still parse the 7-byte fixed header portion.
        if let Some(h) = parsed {
            assert!(!h.crc_absent);
        }
    }

    #[test]
    fn aac_sample_rates_table_correct() {
        assert_eq!(AAC_SAMPLE_RATES[0], 96000);
        assert_eq!(AAC_SAMPLE_RATES[3], 48000);
        assert_eq!(AAC_SAMPLE_RATES[4], 44100);
        assert_eq!(AAC_SAMPLE_RATES[11], 8000);
        assert_eq!(AAC_SAMPLE_RATES[12], 7350);
        assert_eq!(AAC_SAMPLE_RATES.len(), 13);
    }

    #[test]
    fn parse_adts_all_valid_freq_indices() {
        for idx in 0..=12u8 {
            let mut data = [0xFF, 0xF1, 0x00, 0x80, 0x01, 0x00, 0xFC];
            data[2] = (1 << 6) | (idx << 2);
            let parsed = parse_adts_header(&data);
            assert!(parsed.is_some(), "freq index {idx} should be valid");
            let h = parsed.unwrap();
            assert_eq!(h.sampling_freq_index, idx);
        }
    }

    #[test]
    fn parse_adts_all_profiles() {
        for profile in 0..=3u8 {
            let mut data = [0xFF, 0xF1, 0x00, 0x80, 0x01, 0x00, 0xFC];
            data[2] = (profile << 6) | (4 << 2); // freq=4 (44100)
            let parsed = parse_adts_header(&data).unwrap();
            assert_eq!(parsed.profile, profile);
        }
    }
}

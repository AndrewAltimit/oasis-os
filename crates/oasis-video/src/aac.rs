//! AAC audio decoder wrapping symphonia's AAC codec.

use symphonia::core::audio::{SampleBuffer, SignalSpec};
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
    /// Tracks the last audio spec to detect mid-stream spec changes (e.g. HE-AAC SBR).
    last_spec: Option<SignalSpec>,
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
        // The AudioSpecificConfig's channel configuration is authoritative:
        // the MP4 sample entry's channel count (what symphonia reports in
        // `params.channels`) is commonly 2 for mono streams, and labelling
        // mono PCM as stereo plays it at double speed.
        let channels = params
            .extra_data
            .as_deref()
            .and_then(asc_channel_config)
            .or_else(|| params.channels.map(|ch| ch.count() as u16))
            .unwrap_or(2);

        Ok(Self {
            decoder,
            sample_rate,
            channels,
            sample_buf: None,
            last_spec: None,
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
        // Recreate when the audio spec changes (rare for AAC-LC) or capacity is insufficient.
        let need_new = self
            .sample_buf
            .as_ref()
            .is_none_or(|sb| sb.capacity() < duration || self.last_spec.as_ref() != Some(&spec));
        if need_new {
            self.sample_buf = Some(SampleBuffer::<f32>::new(duration as u64, spec));
            self.last_spec = Some(spec);
        }
        let sample_buf = self.sample_buf.as_mut().expect("just created above");
        sample_buf.copy_interleaved_ref(decoded);
        // Describe the PCM by what was actually decoded.
        let decoded_channels = spec.channels.count() as u16;
        if decoded_channels > 0 {
            self.channels = decoded_channels;
        }
        if spec.rate > 0 {
            self.sample_rate = spec.rate;
        }

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

/// Channel count from an MPEG-4 AudioSpecificConfig (`esds` decoder
/// specific info): 5 bits object type, 4 bits frequency index (+24 bits
/// explicit frequency when the index is 15), then 4 bits channel
/// configuration.  Returns `None` for configurations 0 (defined in-band)
/// and above 7, or if the config is too short.
fn asc_channel_config(asc: &[u8]) -> Option<u16> {
    let bits = asc
        .iter()
        .take(8)
        .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    let total = asc.len().min(8) * 8;
    let read = |pos: usize, n: usize| -> Option<u64> {
        (pos + n <= total).then(|| (bits >> (total - pos - n)) & ((1 << n) - 1))
    };
    let mut pos = 5;
    if read(0, 5)? == 31 {
        pos += 6; // escaped object type
    }
    let freq_index = read(pos, 4)?;
    pos += 4;
    if freq_index == 15 {
        pos += 24;
    }
    match read(pos, 4)? {
        c @ 1..=6 => Some(c as u16),
        7 => Some(8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn asc_channel_config_reads_mono_and_stereo() {
        // AAC-LC (2), 22050 Hz (index 7), mono: 00010 0111 0001 000
        assert_eq!(super::asc_channel_config(&[0x13, 0x88]), Some(1));
        // AAC-LC, 44100 Hz (index 4), stereo: 00010 0100 0010 000
        assert_eq!(super::asc_channel_config(&[0x12, 0x10]), Some(2));
        // Explicit 24-bit frequency (index 15), mono.
        assert_eq!(
            super::asc_channel_config(&[0x17, 0x80, 0x2B, 0x11, 0x08]),
            Some(1)
        );
        // Channel config 0 (program config element) and short input.
        assert_eq!(super::asc_channel_config(&[0x12, 0x00]), None);
        assert_eq!(super::asc_channel_config(&[0x12]), None);
    }
}

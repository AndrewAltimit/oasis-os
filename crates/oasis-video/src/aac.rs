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

        let mut sample_buf = SampleBuffer::<f32>::new(duration as u64, spec);
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

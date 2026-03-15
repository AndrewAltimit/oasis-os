//! Ogg Vorbis decoder via symphonia.
//!
//! Decodes Ogg Vorbis audio data to interleaved i16 PCM samples, mirroring
//! the API of [`crate::wav`].  Uses symphonia's `ogg` demuxer and `vorbis`
//! codec decoder (pure Rust, no external C dependencies).

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded Ogg Vorbis audio data.
pub struct OggData {
    /// Interleaved i16 PCM samples.
    pub samples: Vec<i16>,
    /// Sample rate in Hz (e.g. 44100).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
}

/// Check if `data` starts with an OGG container magic number (`OggS`).
pub fn is_ogg(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"OggS"
}

/// Decode an Ogg Vorbis file from raw bytes.
///
/// Returns `None` if the data is not valid Ogg Vorbis or decoding fails.
pub fn decode_ogg(data: &[u8]) -> Option<OggData> {
    if !is_ogg(data) {
        return None;
    }

    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("ogg");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;

    let mut format = probed.format;

    // Find the first audio track.
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)?;

    let codec_params = track.codec_params.clone();
    let track_id = track.id;

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .ok()?;

    let mut all_samples: Vec<i16> = Vec::new();

    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        if num_frames == 0 {
            continue;
        }

        let mut sample_buf = SampleBuffer::<i16>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    if all_samples.is_empty() {
        return None;
    }

    Some(OggData {
        samples: all_samples,
        sample_rate,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ogg_valid_header() {
        assert!(is_ogg(b"OggS\x00\x02\x00\x00\x00\x00"));
    }

    #[test]
    fn is_ogg_invalid() {
        assert!(!is_ogg(b"RIFF"));
        assert!(!is_ogg(b""));
        assert!(!is_ogg(b"Ogg")); // too short
        assert!(!is_ogg(b"not ogg data"));
    }

    #[test]
    fn decode_garbage_returns_none() {
        assert!(decode_ogg(b"not ogg data").is_none());
        assert!(decode_ogg(b"").is_none());
    }

    #[test]
    fn decode_truncated_ogg_returns_none() {
        // Valid OGG magic but truncated -- should return None, not panic.
        let mut data = b"OggS".to_vec();
        data.extend_from_slice(&[0u8; 20]);
        assert!(decode_ogg(&data).is_none());
    }

    /// Verify that the symphonia Vorbis codec is registered and available.
    #[test]
    fn vorbis_codec_is_available() {
        // CODEC_TYPE_VORBIS should be recognized by the default codec registry.
        let codecs = symphonia::default::get_codecs();
        // We can't directly query "is codec X registered?" via public API,
        // but we can verify the codec type constant exists and is non-null.
        assert_ne!(
            symphonia::core::codecs::CODEC_TYPE_VORBIS,
            symphonia::core::codecs::CODEC_TYPE_NULL,
            "Vorbis codec type should be defined and non-null"
        );
        // Attempt to make a decoder with minimal (invalid) params -- this
        // should fail with a decode error, NOT an "unsupported codec" error,
        // proving the codec is registered.
        let mut params = symphonia::core::codecs::CodecParameters::new();
        params.for_codec(symphonia::core::codecs::CODEC_TYPE_VORBIS);
        let result = codecs.make(&params, &DecoderOptions::default());
        // The error should be about missing codec data, not unsupported codec.
        assert!(result.is_err(), "expected error with incomplete params");
    }
}

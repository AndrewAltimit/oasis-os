//! Safe MP3 frame decoder over `rmp3`'s public C FFI (minimp3).
//!
//! `rmp3` 0.3.1's safe `RawDecoder` wrapper has genuine undefined
//! behavior: it hands minimp3 an `mp3dec_frame_info_t` built from
//! `MaybeUninit::uninit().assume_init()`, and minimp3's "skipped
//! garbage but found no complete frame" path writes `frame_bytes`
//! without ever writing `frame_offset`. The wrapper then constructs
//! a source slice with `get_unchecked(frame_offset..frame_bytes)`
//! from that uninitialized offset. Feeding the decoder non-MP3 bytes
//! (exactly what streaming does while scanning for sync) trips the
//! nightly UB precondition checks in the ASAN CI job.
//!
//! This module calls `rmp3::ffi::mp3dec_decode_frame` directly with
//! zero-initialized structs and never builds the source slice, which
//! sidesteps the bug while keeping the same minimp3 code underneath.

/// PCM sample type produced by minimp3 (i16 unless rmp3's `float`
/// feature is enabled — this workspace builds without it).
pub type Sample = rmp3::ffi::mp3d_sample_t;

/// Maximum interleaved samples minimp3 writes per decoded frame
/// (1152 samples/channel × 2 channels).
pub const MAX_SAMPLES_PER_FRAME: usize = rmp3::ffi::MINIMP3_MAX_SAMPLES_PER_FRAME as usize;

/// A decoded audio frame borrowing the caller's PCM buffer.
pub struct AudioFrame<'pcm> {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples: `sample_count_per_channel * channels` long.
    pub samples: &'pcm [Sample],
}

/// One step of decoder progress.
pub enum Frame<'pcm> {
    /// A frame of decoded PCM audio.
    Audio(AudioFrame<'pcm>),
    /// Skipped bytes: garbage, ID3 tags, or a frame minimp3 chose not
    /// to decode. Nothing to play, but the consumed count advances.
    Other,
}

/// Streaming MP3 decoder state (heap-allocated: `mp3dec_t` is ~6.5 KB).
pub struct Mp3Decoder(Box<rmp3::ffi::mp3dec_t>);

impl Mp3Decoder {
    pub fn new() -> Self {
        // SAFETY: `mp3dec_t` is a plain C struct of floats, ints, and
        // byte arrays; the all-zero bit pattern is a valid value.
        let mut dec: Box<rmp3::ffi::mp3dec_t> = Box::new(unsafe { core::mem::zeroed() });
        // SAFETY: `dec` points to a valid, initialized `mp3dec_t`.
        unsafe { rmp3::ffi::mp3dec_init(&mut *dec) };
        Self(dec)
    }

    /// Decodes the next frame from `src` into `dest`.
    ///
    /// Returns the frame plus the number of bytes consumed from `src`
    /// (including any skipped garbage), or `None` when minimp3 needs
    /// more data before it can make progress.
    pub fn next<'pcm>(
        &mut self,
        src: &[u8],
        dest: &'pcm mut [Sample; MAX_SAMPLES_PER_FRAME],
    ) -> Option<(Frame<'pcm>, usize)> {
        // minimp3 takes `int` lengths; clamp instead of wrapping for
        // hypothetical >2 GB inputs (it just decodes the prefix).
        let src_len = src.len().min(core::ffi::c_int::MAX as usize) as core::ffi::c_int;
        let mut info = rmp3::ffi::mp3dec_frame_info_t {
            frame_bytes: 0,
            frame_offset: 0,
            channels: 0,
            hz: 0,
            layer: 0,
            bitrate_kbps: 0,
        };
        // SAFETY: all pointers are valid for the lengths passed:
        // `src` for `src_len` bytes, `dest` for
        // MINIMP3_MAX_SAMPLES_PER_FRAME samples (the most minimp3
        // writes per call), and `info`/decoder state are initialized.
        let samples_per_channel = unsafe {
            rmp3::ffi::mp3dec_decode_frame(
                &mut *self.0,
                src.as_ptr(),
                src_len,
                dest.as_mut_ptr(),
                &mut info,
            )
        };
        let consumed = usize::try_from(info.frame_bytes).unwrap_or(0);
        if samples_per_channel > 0 {
            let sample_len = samples_per_channel as usize * info.channels as usize;
            Some((
                Frame::Audio(AudioFrame {
                    sample_rate: info.hz as u32,
                    channels: info.channels as u16,
                    samples: &dest[..sample_len.min(MAX_SAMPLES_PER_FRAME)],
                }),
                consumed,
            ))
        } else if consumed > 0 {
            Some((Frame::Other, consumed))
        } else {
            None
        }
    }
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_input_is_skipped_not_ub() {
        // Regression test for the rmp3 0.3.1 uninitialized
        // `frame_offset` UB: pure garbage exercises minimp3's
        // "skipped bytes, no frame" path.
        let garbage = [0xAA_u8; 4096];
        let mut dec = Mp3Decoder::new();
        let mut pcm = [0 as Sample; MAX_SAMPLES_PER_FRAME];
        match dec.next(&garbage, &mut pcm) {
            Some((Frame::Other, consumed)) => {
                assert!(consumed > 0 && consumed <= garbage.len());
            },
            Some((Frame::Audio(_), _)) => panic!("garbage decoded as audio"),
            None => panic!("garbage should be reported as skipped bytes"),
        }
    }

    #[test]
    fn empty_input_returns_none() {
        let mut dec = Mp3Decoder::new();
        let mut pcm = [0 as Sample; MAX_SAMPLES_PER_FRAME];
        assert!(dec.next(&[], &mut pcm).is_none());
    }

    #[test]
    fn valid_frame_decodes_audio() {
        // A minimal valid MPEG-1 Layer III frame: 44.1 kHz, 128 kbps,
        // stereo → 417 bytes. Zeroed payload decodes to silence.
        // minimp3 wants the *next* frame's sync header visible before
        // committing, so provide two frames back to back.
        const FRAME_LEN: usize = 417;
        let header = [0xFF, 0xFB, 0x90, 0x00];
        let mut data = vec![0u8; FRAME_LEN * 2];
        data[..4].copy_from_slice(&header);
        data[FRAME_LEN..FRAME_LEN + 4].copy_from_slice(&header);

        let mut dec = Mp3Decoder::new();
        let mut pcm = [0 as Sample; MAX_SAMPLES_PER_FRAME];
        match dec.next(&data, &mut pcm) {
            Some((Frame::Audio(audio), consumed)) => {
                assert_eq!(audio.sample_rate, 44_100);
                assert_eq!(audio.channels, 2);
                assert_eq!(audio.samples.len(), 1152 * 2);
                assert_eq!(consumed, FRAME_LEN);
            },
            _ => panic!("expected an audio frame"),
        }
    }
}

//! Polyphonic one-shot sound-effect voices for skin-themed UI sounds.
//!
//! [`SfxPlayer`] holds short decoded PCM samples keyed by name (e.g.
//! `"click"`, `"open"`) and plays them as fire-and-forget voices mixed
//! over whatever music is already playing. All samples are normalized at
//! load time to interleaved stereo i16 at [`SFX_SAMPLE_RATE`] so the mix
//! loop is a pure integer accumulate — no per-frame format conversion.
//!
//! The player itself never touches an audio device: the shell calls
//! [`SfxPlayer::render`] once per frame and hands the mixed chunk to the
//! platform audio backend (a dedicated one-shot stream on SDL). This keeps
//! the crate dependency-free, like the rest of `oasis-audio`'s PCM math.

use std::collections::VecDeque;

use super::mixer::{mono_to_stereo_i16, resample_nearest_i16};
use super::wav::{WavData, decode_wav};

/// Output sample rate of the SFX mix, chosen to match the desktop audio
/// backend's fixed device rate so no further conversion happens downstream.
pub const SFX_SAMPLE_RATE: u32 = 48_000;

/// Maximum simultaneous voices. Triggering a sound while all slots are
/// busy steals the oldest voice.
pub const MAX_VOICES: usize = 8;

/// A playing one-shot: an index into the sample store plus a cursor.
#[derive(Debug, Clone, Copy)]
struct Voice {
    /// Index into `SfxPlayer::samples`.
    sample: usize,
    /// Read position in interleaved i16 samples (not frames).
    pos: usize,
}

/// Polyphonic one-shot sample player (see module docs).
#[derive(Debug, Default)]
pub struct SfxPlayer {
    /// Loaded samples: (name, interleaved stereo i16 at `SFX_SAMPLE_RATE`).
    samples: Vec<(String, Vec<i16>)>,
    /// Active voices, oldest first.
    voices: VecDeque<Voice>,
    /// Master volume applied at mix time, 0.0–1.0 (from the skin's
    /// `[sounds] volume`). Stored as an 8.8 fixed-point multiplier.
    volume_q8: i32,
}

impl SfxPlayer {
    /// Create an empty player (master volume 1.0).
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            voices: VecDeque::new(),
            volume_q8: 256,
        }
    }

    /// Remove all samples and stop all voices (skin swap-out).
    pub fn clear(&mut self) {
        self.samples.clear();
        self.voices.clear();
    }

    /// Number of loaded samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples are loaded.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Set the master volume (clamped to 0.0–1.0).
    pub fn set_master_volume(&mut self, volume: f32) {
        self.volume_q8 = (volume.clamp(0.0, 1.0) * 256.0) as i32;
    }

    /// Current master volume, 0.0–1.0.
    pub fn master_volume(&self) -> f32 {
        self.volume_q8 as f32 / 256.0
    }

    /// Whether a sample with this name is loaded.
    pub fn contains(&self, name: &str) -> bool {
        self.samples.iter().any(|(n, _)| n == name)
    }

    /// Register decoded WAV data under `name`, converting to the canonical
    /// stereo/48 kHz format. Replaces any existing sample of that name.
    pub fn insert(&mut self, name: &str, wav: &WavData) {
        let pcm = to_output_pcm(wav);
        if pcm.is_empty() {
            return;
        }
        if let Some(idx) = self.samples.iter().position(|(n, _)| n == name) {
            self.samples[idx].1 = pcm;
            // Drop voices playing the replaced sample so they don't play a
            // tail of the new sound from a stale position.
            self.voices.retain(|v| v.sample != idx);
        } else {
            self.samples.push((name.to_string(), pcm));
        }
    }

    /// Decode raw WAV bytes and register them under `name`.
    /// Returns `false` (and loads nothing) when the bytes don't decode.
    pub fn load_wav(&mut self, name: &str, bytes: &[u8]) -> bool {
        match decode_wav(bytes) {
            Some(wav) => {
                self.insert(name, &wav);
                self.contains(name)
            },
            None => false,
        }
    }

    /// Start a voice for the named sample. Returns `false` when no sample
    /// with that name is loaded (a skin without that sound stays silent).
    /// When all [`MAX_VOICES`] slots are busy the oldest voice is stolen.
    pub fn play(&mut self, name: &str) -> bool {
        let Some(idx) = self.samples.iter().position(|(n, _)| n == name) else {
            return false;
        };
        while self.voices.len() >= MAX_VOICES {
            self.voices.pop_front();
        }
        self.voices.push_back(Voice {
            sample: idx,
            pos: 0,
        });
        true
    }

    /// Number of currently playing voices.
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// Whether any voice is currently playing.
    pub fn has_active_voices(&self) -> bool {
        !self.voices.is_empty()
    }

    /// Mix `frames` stereo frames of all active voices into `out`
    /// (cleared and refilled with `frames * 2` interleaved i16 samples).
    /// Finished voices are retired. Returns `true` when at least one voice
    /// contributed audio; on `false`, `out` is left empty.
    pub fn render(&mut self, frames: usize, out: &mut Vec<i16>) -> bool {
        out.clear();
        if frames == 0 || self.voices.is_empty() {
            return false;
        }
        out.resize(frames * 2, 0);
        let vol = self.volume_q8;
        for voice in &mut self.voices {
            let Some((_, pcm)) = self.samples.get(voice.sample) else {
                voice.pos = usize::MAX;
                continue;
            };
            let avail = pcm.len().saturating_sub(voice.pos);
            let take = avail.min(out.len());
            for (dst, &src) in out[..take]
                .iter_mut()
                .zip(&pcm[voice.pos..voice.pos + take])
            {
                let scaled = (src as i32 * vol) >> 8;
                let mixed = (*dst as i32 + scaled).clamp(i16::MIN as i32, i16::MAX as i32);
                *dst = mixed as i16;
            }
            voice.pos = voice.pos.saturating_add(take);
        }
        let samples = &self.samples;
        self.voices.retain(|v| {
            samples
                .get(v.sample)
                .is_some_and(|(_, pcm)| v.pos < pcm.len())
        });
        true
    }
}

/// Convert decoded WAV data to interleaved stereo i16 at [`SFX_SAMPLE_RATE`].
///
/// Mono sources are resampled then duplicated to both channels. Stereo
/// sources are deinterleaved, resampled per channel (so the nearest-neighbor
/// resampler never mixes L/R), and re-interleaved.
fn to_output_pcm(wav: &WavData) -> Vec<i16> {
    if wav.samples.is_empty() || wav.sample_rate == 0 {
        return Vec::new();
    }
    match wav.channels {
        1 => {
            let resampled = resample_nearest_i16(&wav.samples, wav.sample_rate, SFX_SAMPLE_RATE);
            mono_to_stereo_i16(&resampled)
        },
        2 => {
            let frames = wav.samples.len() / 2;
            let mut left = Vec::with_capacity(frames);
            let mut right = Vec::with_capacity(frames);
            for pair in wav.samples.as_chunks::<2>().0.iter() {
                left.push(pair[0]);
                right.push(pair[1]);
            }
            let left = resample_nearest_i16(&left, wav.sample_rate, SFX_SAMPLE_RATE);
            let right = resample_nearest_i16(&right, wav.sample_rate, SFX_SAMPLE_RATE);
            let mut out = Vec::with_capacity(left.len().min(right.len()) * 2);
            for (l, r) in left.iter().zip(&right) {
                out.push(*l);
                out.push(*r);
            }
            out
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Build a minimal valid 16-bit PCM WAV in memory.
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

    fn loaded_player() -> SfxPlayer {
        let mut sfx = SfxPlayer::new();
        // 48 kHz stereo passes through untouched: 4 frames.
        let wav = make_wav(
            &[1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000],
            48_000,
            2,
        );
        assert!(sfx.load_wav("click", &wav));
        sfx
    }

    #[test]
    fn load_and_contains() {
        let sfx = loaded_player();
        assert!(sfx.contains("click"));
        assert!(!sfx.contains("open"));
        assert_eq!(sfx.len(), 1);
        assert!(!sfx.is_empty());
    }

    #[test]
    fn load_garbage_fails() {
        let mut sfx = SfxPlayer::new();
        assert!(!sfx.load_wav("bad", b"not a wav"));
        assert!(sfx.is_empty());
    }

    #[test]
    fn play_unknown_name_is_silent() {
        let mut sfx = loaded_player();
        assert!(!sfx.play("missing"));
        assert_eq!(sfx.active_voices(), 0);
    }

    #[test]
    fn render_passthrough_stereo_48k() {
        let mut sfx = loaded_player();
        assert!(sfx.play("click"));
        let mut out = Vec::new();
        assert!(sfx.render(4, &mut out));
        assert_eq!(
            out,
            vec![1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000]
        );
        // Voice finished exactly at the chunk boundary.
        assert!(!sfx.has_active_voices());
        assert!(!sfx.render(4, &mut out));
        assert!(out.is_empty());
    }

    #[test]
    fn render_spans_multiple_chunks() {
        let mut sfx = loaded_player();
        sfx.play("click");
        let mut out = Vec::new();
        sfx.render(3, &mut out);
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], 1000);
        assert!(sfx.has_active_voices());
        sfx.render(3, &mut out);
        // Last frame of the sample plus one frame of silence.
        assert_eq!(out, vec![4000, -4000, 0, 0, 0, 0]);
        assert!(!sfx.has_active_voices());
    }

    #[test]
    fn polyphony_mixes_voices() {
        let mut sfx = loaded_player();
        sfx.play("click");
        sfx.play("click");
        let mut out = Vec::new();
        sfx.render(1, &mut out);
        // Two identical voices sum.
        assert_eq!(out, vec![2000, -2000]);
    }

    #[test]
    fn mix_saturates() {
        let mut sfx = SfxPlayer::new();
        let wav = make_wav(&[i16::MAX, i16::MAX], 48_000, 2);
        sfx.load_wav("loud", &wav);
        sfx.play("loud");
        sfx.play("loud");
        let mut out = Vec::new();
        sfx.render(1, &mut out);
        assert_eq!(out, vec![i16::MAX, i16::MAX]);
    }

    #[test]
    fn voice_cap_steals_oldest() {
        let mut sfx = loaded_player();
        for _ in 0..MAX_VOICES {
            sfx.play("click");
        }
        assert_eq!(sfx.active_voices(), MAX_VOICES);
        // Advance the oldest voices so stealing is observable.
        let mut out = Vec::new();
        sfx.render(2, &mut out);
        sfx.play("click");
        assert_eq!(sfx.active_voices(), MAX_VOICES);
        // The newest voice starts at pos 0 (sample value 1000 next frame).
        sfx.render(1, &mut out);
        assert!(!out.is_empty());
    }

    #[test]
    fn master_volume_scales() {
        let mut sfx = loaded_player();
        sfx.set_master_volume(0.5);
        assert!((sfx.master_volume() - 0.5).abs() < 0.01);
        sfx.play("click");
        let mut out = Vec::new();
        sfx.render(1, &mut out);
        assert_eq!(out, vec![500, -500]);
    }

    #[test]
    fn master_volume_zero_is_silent() {
        let mut sfx = loaded_player();
        sfx.set_master_volume(0.0);
        sfx.play("click");
        let mut out = Vec::new();
        assert!(sfx.render(1, &mut out));
        assert_eq!(out, vec![0, 0]);
    }

    #[test]
    fn master_volume_clamped() {
        let mut sfx = SfxPlayer::new();
        sfx.set_master_volume(9.0);
        assert!((sfx.master_volume() - 1.0).abs() < 0.01);
        sfx.set_master_volume(-1.0);
        assert!(sfx.master_volume() < 0.01);
    }

    #[test]
    fn mono_upmixes_to_stereo() {
        let mut sfx = SfxPlayer::new();
        let wav = make_wav(&[123, 456], 48_000, 1);
        sfx.load_wav("mono", &wav);
        sfx.play("mono");
        let mut out = Vec::new();
        sfx.render(2, &mut out);
        assert_eq!(out, vec![123, 123, 456, 456]);
    }

    #[test]
    fn resamples_to_output_rate() {
        let mut sfx = SfxPlayer::new();
        // 24 kHz mono, 10 frames → ~20 frames at 48 kHz.
        let wav = make_wav(&[100; 10], 24_000, 1);
        sfx.load_wav("low", &wav);
        sfx.play("low");
        let mut out = Vec::new();
        sfx.render(30, &mut out);
        let nonzero = out.iter().filter(|&&s| s != 0).count();
        assert_eq!(nonzero, 40); // 20 stereo frames of value 100.
    }

    #[test]
    fn clear_stops_everything() {
        let mut sfx = loaded_player();
        sfx.play("click");
        sfx.clear();
        assert!(sfx.is_empty());
        assert!(!sfx.has_active_voices());
        let mut out = Vec::new();
        assert!(!sfx.render(4, &mut out));
    }

    #[test]
    fn insert_replaces_and_drops_stale_voices() {
        let mut sfx = loaded_player();
        sfx.play("click");
        let wav = make_wav(&[7, 7], 48_000, 2);
        let decoded = decode_wav(&wav).unwrap();
        sfx.insert("click", &decoded);
        assert_eq!(sfx.len(), 1);
        assert!(!sfx.has_active_voices());
        sfx.play("click");
        let mut out = Vec::new();
        sfx.render(1, &mut out);
        assert_eq!(out, vec![7, 7]);
    }

    #[test]
    fn render_zero_frames() {
        let mut sfx = loaded_player();
        sfx.play("click");
        let mut out = vec![9, 9];
        assert!(!sfx.render(0, &mut out));
        assert!(out.is_empty());
    }
}

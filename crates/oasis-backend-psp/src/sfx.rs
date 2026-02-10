//! Sound effects engine using `psp::audio_mixer::Mixer`.
//!
//! Provides UI sound effects (click, navigate, error) that can play
//! alongside music. Uses the PSP's multi-channel audio hardware via
//! the `Mixer` for volume control and mixing.

use psp::audio_mixer::{ChannelConfig, ChannelHandle, Mixer};

/// Sound effect identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxId {
    Click,
    Navigate,
    Error,
}

/// SFX engine backed by `psp::audio_mixer::Mixer`.
///
/// Allocates one mixer channel for SFX playback with pre-generated
/// PCM waveforms. Call `play()` to trigger a sound, then `pump()` to
/// mix and output. SFX uses a separate hardware audio channel from
/// the music player.
pub struct SfxEngine {
    mixer: Mixer,
    sfx_channel: ChannelHandle,
    mix_buffer: Vec<i16>,
    click_pcm: &'static [i16],
    navigate_pcm: &'static [i16],
    error_pcm: &'static [i16],
}

/// Sample count per mixer output (64-aligned, ~5.8ms at 44100Hz).
const SFX_SAMPLE_COUNT: i32 = 256;

impl SfxEngine {
    /// Create the SFX engine.
    ///
    /// Allocates a hardware audio channel, generates PCM waveforms, and
    /// prepares the mixer. Returns `None` if hardware resources are
    /// unavailable.
    pub fn new() -> Option<Self> {
        let mixer = Mixer::new(SFX_SAMPLE_COUNT).ok()?;
        mixer.reserve_hw_channel().ok()?;

        let sfx_channel = mixer
            .alloc_channel(ChannelConfig {
                volume_left: 0x6000,
                volume_right: 0x6000,
                looping: false,
            })
            .ok()?;

        // Generate PCM waveforms and leak them for 'static lifetime.
        // Total leaked: ~10KB (affordable on PSP's 32MB).
        let click_pcm = generate_click();
        let navigate_pcm = generate_navigate();
        let error_pcm = generate_error();

        let buf_size = (SFX_SAMPLE_COUNT * 2) as usize;
        Some(Self {
            mixer,
            sfx_channel,
            mix_buffer: vec![0i16; buf_size],
            click_pcm,
            navigate_pcm,
            error_pcm,
        })
    }

    /// Trigger a sound effect.
    ///
    /// Submits the PCM data to the mixer channel. Call `pump()` after
    /// this to actually output the audio.
    pub fn play(&self, sfx: SfxId) {
        let pcm = match sfx {
            SfxId::Click => self.click_pcm,
            SfxId::Navigate => self.navigate_pcm,
            SfxId::Error => self.error_pcm,
        };
        // SAFETY: PCM data is leaked Box<[i16]> with 'static lifetime.
        // It remains valid for the program's entire duration.
        unsafe {
            let _ = self.mixer.submit_samples(self.sfx_channel, pcm);
        }
    }

    /// Mix active channels and output to hardware (blocking).
    ///
    /// With 256 samples at 44100Hz, this blocks for ~5.8ms -- short
    /// enough to interleave with the music player's frame decoding.
    pub fn pump(&mut self) {
        self.mixer.mix_into(&mut self.mix_buffer);

        // Skip hardware output if the buffer is silence.
        if self.mix_buffer.iter().all(|&s| s == 0) {
            return;
        }

        let _ = self.mixer.output_blocking(&self.mix_buffer);
    }
}

// ---------------------------------------------------------------------------
// PCM waveform generators
// ---------------------------------------------------------------------------

/// Generate a click sound: short burst with fast decay (stereo, 512 samples).
fn generate_click() -> &'static [i16] {
    let samples = 512;
    let mut pcm = vec![0i16; samples * 2]; // stereo
    for i in 0..samples {
        let t = i as f32 / 44100.0;
        let decay = libm::expf(-t * 400.0);
        let wave = libm::sinf(t * 1200.0 * 2.0 * core::f32::consts::PI);
        let val = (12000.0 * decay * wave) as i16;
        pcm[i * 2] = val;
        pcm[i * 2 + 1] = val;
    }
    Box::leak(pcm.into_boxed_slice())
}

/// Generate a navigate sound: rising tone (stereo, 512 samples).
fn generate_navigate() -> &'static [i16] {
    let samples = 512;
    let mut pcm = vec![0i16; samples * 2];
    for i in 0..samples {
        let t = i as f32 / 44100.0;
        let freq = 800.0 + t * 8000.0; // rising pitch
        let decay = libm::expf(-t * 200.0);
        let wave = libm::sinf(t * freq * 2.0 * core::f32::consts::PI);
        let val = (8000.0 * decay * wave) as i16;
        pcm[i * 2] = val;
        pcm[i * 2 + 1] = val;
    }
    Box::leak(pcm.into_boxed_slice())
}

/// Generate an error sound: low buzz (stereo, 1024 samples).
fn generate_error() -> &'static [i16] {
    let samples = 1024;
    let mut pcm = vec![0i16; samples * 2];
    for i in 0..samples {
        let t = i as f32 / 44100.0;
        let decay = libm::expf(-t * 80.0);
        let wave = libm::sinf(t * 200.0 * 2.0 * core::f32::consts::PI);
        let val = (10000.0 * decay * wave) as i16;
        pcm[i * 2] = val;
        pcm[i * 2 + 1] = val;
    }
    Box::leak(pcm.into_boxed_slice())
}

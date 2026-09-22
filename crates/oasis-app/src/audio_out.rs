//! Shell audio output abstraction.
//!
//! [`AppState::audio_backend`](crate::app_state::AppState::audio_backend)
//! is a `Box<dyn ShellAudio>` rather than the concrete SDL type so the
//! same shell can run against a real audio device (the desktop binary)
//! or a recording fake (the headless e2e harness, which asserts on the
//! PCM that actually reaches the output).

use oasis_backend_sdl::SdlAudioBackend;
use oasis_core::backend::AudioBackend;
use oasis_core::error::Result;

/// Everything the shell needs from its audio output: the portable
/// [`AudioBackend`] trait plus the dedicated UI-sound (SFX) stream.
pub trait ShellAudio: AudioBackend {
    /// Queue interleaved stereo i16 PCM on the SFX stream.
    fn queue_sfx(&mut self, pcm: &[i16]) -> Result<()>;
    /// Bytes currently queued on the SFX stream.
    fn sfx_queued_bytes(&self) -> u32;
}

impl ShellAudio for SdlAudioBackend {
    fn queue_sfx(&mut self, pcm: &[i16]) -> Result<()> {
        SdlAudioBackend::queue_sfx(self, pcm)
    }

    fn sfx_queued_bytes(&self) -> u32 {
        SdlAudioBackend::sfx_queued_bytes(self)
    }
}

//! Audio backend trait.

use crate::error::{OasisError, Result};

/// Opaque handle to a loaded audio track in the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioTrackId(pub u64);

/// Audio playback backend trait.
///
/// Two implementations cover all deployment targets: rodio/SDL2_mixer (desktop/Pi)
/// and Media Engine offloading (PSP via PRX stubs).
pub trait AudioBackend {
    /// Initialize the audio subsystem (open device, set sample rate).
    fn init(&mut self) -> Result<()>;

    /// Load an audio file from raw bytes (MP3, WAV, OGG).
    /// Returns a handle for playback control.
    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId>;

    /// Start playing a loaded track from the beginning.
    fn play(&mut self, track: AudioTrackId) -> Result<()>;

    /// Pause the currently playing track.
    fn pause(&mut self) -> Result<()>;

    /// Resume a paused track.
    fn resume(&mut self) -> Result<()>;

    /// Stop playback and reset position to the beginning.
    fn stop(&mut self) -> Result<()>;

    /// Set volume (0 = silent, 100 = full).
    fn set_volume(&mut self, volume: u8) -> Result<()>;

    /// Get the current volume (0-100).
    fn get_volume(&self) -> u8;

    /// Return `true` if audio is currently playing.
    fn is_playing(&self) -> bool;

    /// Get the current playback position in milliseconds.
    fn position_ms(&self) -> u64;

    /// Get the total duration of the current track in milliseconds.
    /// Returns 0 if no track is loaded.
    fn duration_ms(&self) -> u64;

    /// Unload a previously loaded track and free its resources.
    fn unload_track(&mut self, track: AudioTrackId) -> Result<()>;

    /// Shut down the audio subsystem and release all resources.
    fn shutdown(&mut self) -> Result<()>;

    /// Begin a streaming audio session. Returns a track handle for feeding
    /// data incrementally via `feed_data()`.
    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        Err(OasisError::Backend("streaming not supported".into()))
    }

    /// Feed a chunk of streaming audio data to an active streaming track.
    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> Result<()> {
        let _ = (track, data);
        Err(OasisError::Backend("streaming not supported".into()))
    }

    /// Feed decoded PCM f32 samples directly to a streaming track.
    ///
    /// Used by the software video decoder path where audio is already decoded
    /// to interleaved f32 PCM (no MP3 re-encoding needed).
    fn feed_pcm_f32(
        &mut self,
        track: AudioTrackId,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> Result<()> {
        let _ = (track, samples, channels, sample_rate);
        Err(OasisError::Backend(
            "pcm f32 streaming not supported".into(),
        ))
    }
}

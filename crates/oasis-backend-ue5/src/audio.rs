//! UE5 audio backend.
//!
//! Wraps `NullAudioBackend` and adds an optional callback so the UE5 host
//! can be notified of playback events (play, pause, stop, volume changes).
//! The host engine is expected to handle actual audio output.

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::error::Result;

/// Audio event types forwarded to the UE5 host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AudioEvent {
    Play = 0,
    Pause = 1,
    Resume = 2,
    Stop = 3,
    VolumeChange = 4,
    TrackLoaded = 5,
    TrackUnloaded = 6,
    Shutdown = 7,
}

/// Callback function type for audio events.
///
/// Parameters: event type, track ID (0 if N/A), extra value (e.g. volume level).
pub type AudioEventCallback = extern "C" fn(event: u32, track_id: u64, value: u32);

/// UE5 audio backend with optional host callback.
///
/// Tracks playback state internally (silent no-ops) while forwarding events
/// to the host engine via an optional callback.
pub struct Ue5AudioBackend {
    inner: oasis_audio::NullAudioBackend,
    callback: Option<AudioEventCallback>,
}

impl Ue5AudioBackend {
    pub fn new() -> Self {
        Self {
            inner: oasis_audio::NullAudioBackend::new(),
            callback: None,
        }
    }

    /// Register a callback for audio events.
    pub fn set_callback(&mut self, cb: AudioEventCallback) {
        self.callback = Some(cb);
    }

    fn fire(&self, event: AudioEvent, track_id: u64, value: u32) {
        if let Some(cb) = self.callback {
            cb(event as u32, track_id, value);
        }
    }
}

impl Default for Ue5AudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for Ue5AudioBackend {
    fn init(&mut self) -> Result<()> {
        self.inner.init()
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        let id = self.inner.load_track(data)?;
        self.fire(AudioEvent::TrackLoaded, id.0, data.len() as u32);
        Ok(id)
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        self.inner.play(track)?;
        self.fire(AudioEvent::Play, track.0, 0);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.inner.pause()?;
        self.fire(AudioEvent::Pause, 0, 0);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.inner.resume()?;
        self.fire(AudioEvent::Resume, 0, 0);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.inner.stop()?;
        self.fire(AudioEvent::Stop, 0, 0);
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.inner.set_volume(volume)?;
        self.fire(AudioEvent::VolumeChange, 0, volume as u32);
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.inner.get_volume()
    }

    fn is_playing(&self) -> bool {
        self.inner.is_playing()
    }

    fn position_ms(&self) -> u64 {
        self.inner.position_ms()
    }

    fn duration_ms(&self) -> u64 {
        self.inner.duration_ms()
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        self.inner.unload_track(track)?;
        self.fire(AudioEvent::TrackUnloaded, track.0, 0);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.fire(AudioEvent::Shutdown, 0, 0);
        self.inner.shutdown()
    }

    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        self.inner.load_streaming()
    }

    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> Result<()> {
        self.inner.feed_data(track, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static LAST_EVENT: AtomicU32 = AtomicU32::new(u32::MAX);
    static LAST_TRACK: AtomicU32 = AtomicU32::new(u32::MAX);
    static LAST_VALUE: AtomicU32 = AtomicU32::new(u32::MAX);

    extern "C" fn test_cb(event: u32, track_id: u64, value: u32) {
        LAST_EVENT.store(event, Ordering::SeqCst);
        LAST_TRACK.store(track_id as u32, Ordering::SeqCst);
        LAST_VALUE.store(value, Ordering::SeqCst);
    }

    fn reset_globals() {
        LAST_EVENT.store(u32::MAX, Ordering::SeqCst);
        LAST_TRACK.store(u32::MAX, Ordering::SeqCst);
        LAST_VALUE.store(u32::MAX, Ordering::SeqCst);
    }

    fn init_backend() -> Ue5AudioBackend {
        let mut b = Ue5AudioBackend::new();
        b.init().unwrap();
        b.set_callback(test_cb);
        reset_globals();
        b
    }

    #[test]
    fn default_state() {
        let b = Ue5AudioBackend::new();
        assert!(!b.is_playing());
        assert_eq!(b.get_volume(), 80);
        assert_eq!(b.position_ms(), 0);
        assert_eq!(b.duration_ms(), 0);
    }

    #[test]
    fn play_fires_callback() {
        let mut b = init_backend();
        let track = b.load_track(b"data").unwrap();
        reset_globals();
        b.play(track).unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Play as u32);
        assert_eq!(LAST_TRACK.load(Ordering::SeqCst), track.0 as u32);
    }

    #[test]
    fn pause_fires_callback() {
        let mut b = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        reset_globals();
        b.pause().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Pause as u32);
    }

    #[test]
    fn resume_fires_callback() {
        let mut b = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        b.pause().unwrap();
        reset_globals();
        b.resume().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Resume as u32);
    }

    #[test]
    fn stop_fires_callback() {
        let mut b = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        reset_globals();
        b.stop().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Stop as u32);
    }

    #[test]
    fn volume_fires_callback() {
        let mut b = init_backend();
        b.set_volume(42).unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::VolumeChange as u32
        );
        assert_eq!(LAST_VALUE.load(Ordering::SeqCst), 42);
        assert_eq!(b.get_volume(), 42);
    }

    #[test]
    fn load_track_fires_callback() {
        let mut b = init_backend();
        let data = b"fake audio";
        let track = b.load_track(data).unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::TrackLoaded as u32
        );
        assert_eq!(LAST_TRACK.load(Ordering::SeqCst), track.0 as u32);
        assert_eq!(LAST_VALUE.load(Ordering::SeqCst), data.len() as u32);
    }

    #[test]
    fn unload_track_fires_callback() {
        let mut b = init_backend();
        let track = b.load_track(b"data").unwrap();
        reset_globals();
        b.unload_track(track).unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::TrackUnloaded as u32
        );
        assert_eq!(LAST_TRACK.load(Ordering::SeqCst), track.0 as u32);
    }

    #[test]
    fn shutdown_fires_callback() {
        let mut b = init_backend();
        b.shutdown().unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::Shutdown as u32
        );
    }

    #[test]
    fn no_callback_does_not_crash() {
        let mut b = Ue5AudioBackend::new();
        b.init().unwrap();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        b.pause().unwrap();
        b.resume().unwrap();
        b.stop().unwrap();
        b.set_volume(50).unwrap();
        b.unload_track(track).unwrap();
        b.shutdown().unwrap();
    }

    #[test]
    fn streaming_works() {
        let mut b = init_backend();
        let track = b.load_streaming().unwrap();
        b.feed_data(track, b"streaming chunk").unwrap();
    }

    #[test]
    fn play_missing_track_fails() {
        let mut b = init_backend();
        assert!(b.play(AudioTrackId(999)).is_err());
    }

    #[test]
    fn lifecycle() {
        let mut b = init_backend();
        let t = b.load_track(b"mp3 data").unwrap();
        b.play(t).unwrap();
        assert!(b.is_playing());
        b.pause().unwrap();
        assert!(!b.is_playing());
        b.resume().unwrap();
        assert!(b.is_playing());
        b.stop().unwrap();
        assert!(!b.is_playing());
        b.unload_track(t).unwrap();
        b.shutdown().unwrap();
    }
}

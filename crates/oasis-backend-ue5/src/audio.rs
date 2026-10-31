//! UE5 audio backend.
//!
//! Wraps `NullAudioBackend` and adds an optional callback so the UE5 host
//! can be notified of playback events (play, pause, stop, volume changes).
//! The host engine is expected to handle actual audio output.
//!
//! Streaming tracks (internet radio, `load_streaming` + `feed_data`) carry
//! bytes the host never saw, so they are buffered here (bounded by
//! [`MAX_STREAM_BUFFER`], with `streaming_can_accept` back-pressure) and
//! announced with [`AudioEvent::StreamOpened`] / [`AudioEvent::StreamData`] /
//! [`AudioEvent::StreamEnd`]; the host pulls the bytes with
//! [`Ue5AudioBackend::drain_stream`].

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
    /// A streaming track was opened (`track_id` = the stream).
    StreamOpened = 8,
    /// Encoded stream bytes were buffered; `value` = bytes now waiting
    /// for [`Ue5AudioBackend::drain_stream`].
    StreamData = 9,
    /// The stream's source ended; drain what is left.
    StreamEnd = 10,
}

/// Most encoded stream bytes buffered for the host (1 MiB, about a minute
/// of 128 kbit/s MP3). Past it `streaming_can_accept` reports `false` and
/// `feed_data` drops what does not fit.
pub const MAX_STREAM_BUFFER: usize = 1024 * 1024;

/// Callback function type for audio events.
///
/// Parameters: event type, track ID (0 if N/A), extra value (e.g. volume level).
pub type AudioEventCallback = extern "C" fn(event: u32, track_id: u64, value: u32);

/// [`AudioEventCallback`] with the host's context pointer, registered with
/// [`Ue5AudioBackend::set_callback_ex`] and passed back unchanged.
pub type AudioEventCallbackEx =
    extern "C" fn(event: u32, track_id: u64, value: u32, user_data: *mut std::ffi::c_void);

/// The registered host callback.
#[derive(Clone, Copy)]
enum HostCallback {
    Plain(AudioEventCallback),
    /// The context pointer is kept as its exposed address so the backend
    /// stays `Send`; it is only ever handed back to the host.
    WithData(AudioEventCallbackEx, usize),
}

/// UE5 audio backend with optional host callback.
///
/// Tracks playback state internally (silent no-ops) while forwarding events
/// to the host engine via an optional callback.
pub struct Ue5AudioBackend {
    inner: oasis_audio::NullAudioBackend,
    callback: Option<HostCallback>,
    /// A track was paused mid-playback and can be resumed.
    paused: bool,
    /// The open streaming track, if any.
    stream_track: Option<AudioTrackId>,
    /// Encoded bytes fed to [`Self::stream_track`] the host has not drained.
    stream_buf: Vec<u8>,
}

impl Ue5AudioBackend {
    pub fn new() -> Self {
        Self {
            inner: oasis_audio::NullAudioBackend::new(),
            callback: None,
            paused: false,
            stream_track: None,
            stream_buf: Vec::new(),
        }
    }

    /// Move up to `max` buffered stream bytes into `out` (appended) and
    /// return how many were moved. The host calls this after a
    /// [`AudioEvent::StreamData`] / [`AudioEvent::StreamEnd`] event.
    pub fn drain_stream(&mut self, out: &mut Vec<u8>, max: usize) -> usize {
        let n = max.min(self.stream_buf.len());
        out.extend(self.stream_buf.drain(..n));
        n
    }

    /// Move up to `out.len()` buffered stream bytes into the front of `out`
    /// and return how many were moved (the C export's allocation-free form
    /// of [`Self::drain_stream`]).
    pub fn drain_stream_into(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.stream_buf.len());
        out[..n].copy_from_slice(&self.stream_buf[..n]);
        self.stream_buf.drain(..n);
        n
    }

    /// Encoded stream bytes waiting to be drained.
    pub fn stream_buffered(&self) -> usize {
        self.stream_buf.len()
    }

    /// Register a callback for audio events.
    pub fn set_callback(&mut self, cb: AudioEventCallback) {
        self.callback = Some(HostCallback::Plain(cb));
    }

    /// Register a callback that also receives `user_data` (any pointer the
    /// host likes, e.g. its audio component; never dereferenced here).
    /// Replaces a callback set with either setter.
    pub fn set_callback_ex(&mut self, cb: AudioEventCallbackEx, user_data: *mut std::ffi::c_void) {
        self.callback = Some(HostCallback::WithData(cb, user_data.expose_provenance()));
    }

    /// Unregister the audio event callback.
    pub fn clear_callback(&mut self) {
        self.callback = None;
    }

    /// Return whether this backend can produce audio output.
    ///
    /// Always returns `false` because the UE5 backend delegates
    /// audio to the host engine via callbacks. OASIS_OS itself does
    /// not drive audio hardware in this configuration.
    pub fn has_audio(&self) -> bool {
        false
    }

    fn fire(&self, event: AudioEvent, track_id: u64, value: u32) {
        match self.callback {
            Some(HostCallback::Plain(cb)) => cb(event as u32, track_id, value),
            Some(HostCallback::WithData(cb, user_data)) => cb(
                event as u32,
                track_id,
                value,
                std::ptr::with_exposed_provenance_mut(user_data),
            ),
            None => {},
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
        self.paused = false;
        self.fire(AudioEvent::Play, track.0, 0);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        let was_playing = self.inner.is_playing();
        self.inner.pause()?;
        self.paused |= was_playing;
        self.fire(AudioEvent::Pause, 0, 0);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        // Only a paused track can resume. Without this, resume after stop
        // (or before anything was played) reported "playing" and told the
        // host to start output with no track.
        if !self.paused {
            return Err(oasis_types::error::OasisError::Backend(
                "resume: no paused track".into(),
            ));
        }
        self.inner.resume()?;
        self.paused = false;
        self.fire(AudioEvent::Resume, 0, 0);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.inner.stop()?;
        self.paused = false;
        self.fire(AudioEvent::Stop, 0, 0);
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.inner.set_volume(volume)?;
        // Report the volume actually applied (clamped to 0-100), so a host
        // mirroring it never sees an out-of-range level.
        self.fire(
            AudioEvent::VolumeChange,
            0,
            u32::from(self.inner.get_volume()),
        );
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
        if self.stream_track == Some(track) {
            self.stream_track = None;
            self.stream_buf.clear();
        }
        self.fire(AudioEvent::TrackUnloaded, track.0, 0);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.paused = false;
        self.stream_track = None;
        self.stream_buf.clear();
        self.fire(AudioEvent::Shutdown, 0, 0);
        self.inner.shutdown()
    }

    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        let id = self.inner.load_streaming()?;
        self.stream_track = Some(id);
        self.stream_buf.clear();
        self.fire(AudioEvent::StreamOpened, id.0, 0);
        Ok(id)
    }

    /// Buffer the bytes for the host (they used to stop at the null
    /// backend, so streamed audio never reached UE5).
    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> Result<()> {
        self.inner.feed_data(track, data)?;
        if self.stream_track != Some(track) || data.is_empty() {
            return Ok(());
        }
        let room = MAX_STREAM_BUFFER.saturating_sub(self.stream_buf.len());
        if room < data.len() {
            log::debug!(
                "UE5 audio: stream buffer full, dropping {} bytes",
                data.len() - room
            );
        }
        self.stream_buf
            .extend_from_slice(&data[..room.min(data.len())]);
        let buffered = u32::try_from(self.stream_buf.len()).unwrap_or(u32::MAX);
        self.fire(AudioEvent::StreamData, track.0, buffered);
        Ok(())
    }

    fn finalize_streaming(&mut self, track: AudioTrackId) -> Result<()> {
        if self.stream_track == Some(track) {
            let buffered = u32::try_from(self.stream_buf.len()).unwrap_or(u32::MAX);
            self.fire(AudioEvent::StreamEnd, track.0, buffered);
        }
        Ok(())
    }

    fn streaming_can_accept(&self, track: AudioTrackId) -> bool {
        self.stream_track == Some(track) && self.stream_buf.len() < MAX_STREAM_BUFFER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::{Mutex, MutexGuard};

    /// Serializes tests that use the shared global callback atomics.
    /// Without this, parallel tests race on `LAST_EVENT`/`LAST_TRACK`/`LAST_VALUE`.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

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

    fn init_backend() -> (MutexGuard<'static, ()>, Ue5AudioBackend) {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = Ue5AudioBackend::new();
        b.init().unwrap();
        b.set_callback(test_cb);
        reset_globals();
        (guard, b)
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
        let (_guard, mut b) = init_backend();
        let track = b.load_track(b"data").unwrap();
        reset_globals();
        b.play(track).unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Play as u32);
        assert_eq!(LAST_TRACK.load(Ordering::SeqCst), track.0 as u32);
    }

    #[test]
    fn pause_fires_callback() {
        let (_guard, mut b) = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        reset_globals();
        b.pause().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Pause as u32);
    }

    #[test]
    fn resume_fires_callback() {
        let (_guard, mut b) = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        b.pause().unwrap();
        reset_globals();
        b.resume().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Resume as u32);
    }

    #[test]
    fn stop_fires_callback() {
        let (_guard, mut b) = init_backend();
        let track = b.load_track(b"data").unwrap();
        b.play(track).unwrap();
        reset_globals();
        b.stop().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Stop as u32);
    }

    #[test]
    fn volume_fires_callback() {
        let (_guard, mut b) = init_backend();
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
        let (_guard, mut b) = init_backend();
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
        let (_guard, mut b) = init_backend();
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
        let (_guard, mut b) = init_backend();
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
    fn resume_requires_a_paused_track() {
        let (_guard, mut b) = init_backend();
        reset_globals();
        assert!(b.resume().is_err(), "resume with nothing loaded");
        assert!(!b.is_playing());
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), u32::MAX);
        let t = b.load_track(b"x").unwrap();
        b.play(t).unwrap();
        b.stop().unwrap();
        assert!(b.resume().is_err(), "resume after stop");
        assert!(!b.is_playing());
        b.play(t).unwrap();
        b.pause().unwrap();
        b.pause().unwrap();
        b.resume().unwrap();
        assert!(b.is_playing());
        assert!(b.resume().is_err(), "second resume without pause");
    }

    #[test]
    fn volume_callback_reports_clamped_value() {
        let (_guard, mut b) = init_backend();
        b.set_volume(250).unwrap();
        assert_eq!(b.get_volume(), 100);
        assert_eq!(LAST_VALUE.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn streaming_works() {
        let (_guard, mut b) = init_backend();
        let track = b.load_streaming().unwrap();
        b.feed_data(track, b"streaming chunk").unwrap();
    }

    #[test]
    fn streamed_bytes_reach_the_host() {
        let (_guard, mut b) = init_backend();
        let track = b.load_streaming().unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::StreamOpened as u32
        );
        b.feed_data(track, b"abc").unwrap();
        b.feed_data(track, b"def").unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::StreamData as u32
        );
        assert_eq!(LAST_VALUE.load(Ordering::SeqCst), 6);
        let mut out = Vec::new();
        assert_eq!(b.drain_stream(&mut out, 4), 4);
        assert_eq!(b.drain_stream(&mut out, 100), 2);
        assert_eq!(out, b"abcdef");
        b.finalize_streaming(track).unwrap();
        assert_eq!(
            LAST_EVENT.load(Ordering::SeqCst),
            AudioEvent::StreamEnd as u32
        );
    }

    #[test]
    fn drain_stream_into_fills_a_slice() {
        let (_guard, mut b) = init_backend();
        let track = b.load_streaming().unwrap();
        b.feed_data(track, b"abcdef").unwrap();
        let mut out = [0u8; 4];
        assert_eq!(b.drain_stream_into(&mut out), 4);
        assert_eq!(&out, b"abcd");
        assert_eq!(b.drain_stream_into(&mut out), 2);
        assert_eq!(&out[..2], b"ef");
        assert_eq!(b.drain_stream_into(&mut out), 0);
        assert_eq!(b.drain_stream_into(&mut []), 0);
    }

    static EX_USER_DATA: AtomicUsize = AtomicUsize::new(0);
    static EX_EVENT: AtomicU32 = AtomicU32::new(u32::MAX);

    extern "C" fn ex_cb(event: u32, _track: u64, _value: u32, user_data: *mut std::ffi::c_void) {
        EX_EVENT.store(event, Ordering::SeqCst);
        EX_USER_DATA.store(user_data as usize, Ordering::SeqCst);
    }

    #[test]
    fn callback_ex_receives_user_data() {
        let (_guard, mut b) = init_backend();
        reset_globals();
        let mut context = 7u32;
        let ptr = (&raw mut context).cast::<std::ffi::c_void>();
        b.set_callback_ex(ex_cb, ptr);
        b.set_volume(10).unwrap();
        assert_eq!(
            EX_EVENT.load(Ordering::SeqCst),
            AudioEvent::VolumeChange as u32
        );
        assert_eq!(EX_USER_DATA.load(Ordering::SeqCst), ptr as usize);
        // The plain setter replaces it.
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), u32::MAX);
        b.set_callback(test_cb);
        b.stop().unwrap();
        assert_eq!(LAST_EVENT.load(Ordering::SeqCst), AudioEvent::Stop as u32);
        assert_eq!(
            EX_EVENT.load(Ordering::SeqCst),
            AudioEvent::VolumeChange as u32
        );
    }

    #[test]
    fn stream_buffer_is_bounded_with_back_pressure() {
        let (_guard, mut b) = init_backend();
        let track = b.load_streaming().unwrap();
        assert!(b.streaming_can_accept(track));
        b.feed_data(track, &vec![0u8; MAX_STREAM_BUFFER + 10])
            .unwrap();
        assert_eq!(b.stream_buffered(), MAX_STREAM_BUFFER);
        assert!(!b.streaming_can_accept(track));
        b.unload_track(track).unwrap();
        assert_eq!(b.stream_buffered(), 0);
    }

    #[test]
    fn play_missing_track_fails() {
        let (_guard, mut b) = init_backend();
        assert!(b.play(AudioTrackId(999)).is_err());
    }

    #[test]
    fn lifecycle() {
        let (_guard, mut b) = init_backend();
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

    #[test]
    fn has_audio_returns_false() {
        let b = Ue5AudioBackend::new();
        assert!(!b.has_audio());
    }

    #[test]
    fn has_audio_false_after_init() {
        let mut b = Ue5AudioBackend::new();
        b.init().unwrap();
        assert!(!b.has_audio());
    }

    #[test]
    fn init_succeeds_gracefully() {
        let mut b = Ue5AudioBackend::new();
        assert!(b.init().is_ok());
    }

    #[test]
    fn stop_without_play_succeeds() {
        let (_guard, mut b) = init_backend();
        // Stopping when nothing is playing should succeed gracefully.
        assert!(b.stop().is_ok());
    }

    #[test]
    fn position_always_zero() {
        let (_guard, mut b) = init_backend();
        assert_eq!(b.position_ms(), 0);
        let t = b.load_track(b"data").unwrap();
        b.play(t).unwrap();
        assert_eq!(b.position_ms(), 0);
    }

    #[test]
    fn duration_always_zero() {
        let (_guard, mut b) = init_backend();
        assert_eq!(b.duration_ms(), 0);
        let t = b.load_track(b"data").unwrap();
        b.play(t).unwrap();
        assert_eq!(b.duration_ms(), 0);
    }

    #[test]
    fn multiple_tracks_load_and_unload() {
        let (_guard, mut b) = init_backend();
        let t1 = b.load_track(b"track1").unwrap();
        let t2 = b.load_track(b"track2").unwrap();
        let t3 = b.load_track(b"track3").unwrap();
        assert_ne!(t1, t2);
        assert_ne!(t2, t3);

        b.unload_track(t2).unwrap();
        // t1 and t3 should still be playable.
        b.play(t1).unwrap();
        b.stop().unwrap();
        b.play(t3).unwrap();
    }

    #[test]
    fn streaming_feed_data_succeeds() {
        let (_guard, mut b) = init_backend();
        let t = b.load_streaming().unwrap();
        assert!(b.feed_data(t, b"chunk 1").is_ok());
        assert!(b.feed_data(t, b"chunk 2").is_ok());
    }

    #[test]
    fn double_init_succeeds() {
        let mut b = Ue5AudioBackend::new();
        b.init().unwrap();
        // Double init should succeed gracefully.
        b.init().unwrap();
    }
}

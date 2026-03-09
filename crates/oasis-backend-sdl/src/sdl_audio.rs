//! SDL3 audio backend for OASIS_OS.
//!
//! Implements `AudioBackend` using SDL3's audio stream API and the `rmp3` MP3
//! decoder.  Incoming MP3 data (from `feed_data` or `load_track`) is decoded
//! to PCM i16 samples via `rmp3::RawDecoder`, volume-scaled, and pushed to an
//! `sdl3::audio::AudioStream` bound to the default playback device.
//!
//! The SDL3 audio device and stream are opened lazily on the first decoded
//! frame so that the sample rate and channel count come from the actual stream.
//!
//! Note: Real audio output requires hardware. In CI (Docker without audio
//! devices), `init()` may fail gracefully. The `NullAudioBackend` in
//! `oasis-core` is used for testing without hardware.

use std::collections::HashMap;

use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::{OasisError, Result};

/// Maximum PCM bytes to keep in the SDL3 audio stream before we stop accepting
/// more samples.  At 44 100 Hz stereo i16 this is roughly 9 seconds — enough
/// runway to absorb decode jitter and video frame blocking without audio gaps.
const MAX_QUEUE_BYTES: u32 = 1_600_000;

/// SDL3-based audio backend with real MP3 decoding.
pub struct SdlAudioBackend {
    /// Whether the audio subsystem has been initialized.
    initialized: bool,
    /// SDL3 audio stream owner (owns both device and stream).
    stream_owner: Option<sdl3::audio::AudioStreamOwner>,
    /// SDL3 audio subsystem (kept alive for device lifetime).
    audio_subsystem: Option<sdl3::AudioSubsystem>,
    /// MP3 decoder state.
    decoder: rmp3::RawDecoder,
    /// Pending MP3 bytes not yet decoded (streaming track).
    mp3_buffer: Vec<u8>,
    /// Reusable staging buffer for decoded PCM (avoids per-call allocation).
    pcm_staging: Vec<i16>,
    /// Loaded non-streaming tracks (raw MP3 data).
    tracks: HashMap<u64, Vec<u8>>,
    /// Next track ID to assign.
    next_id: u64,
    /// Currently playing track ID (if any).
    current_track: Option<u64>,
    /// Active streaming track ID (if any).
    stream_track: Option<u64>,
    /// Current volume (0-100).
    volume: u8,
    /// Whether playback is active.
    playing: bool,
    /// Whether playback is paused.
    paused: bool,
    /// Sample rate detected from first decoded frame.
    sample_rate: i32,
    /// Channel count detected from first decoded frame.
    channels: usize,
    /// Total PCM samples queued (for position_ms calculation).
    samples_queued: u64,
}

impl SdlAudioBackend {
    /// Create a new SDL3 audio backend (not yet initialized).
    pub fn new() -> Self {
        Self {
            initialized: false,
            stream_owner: None,
            audio_subsystem: None,
            decoder: rmp3::RawDecoder::new(),
            mp3_buffer: Vec::new(),
            pcm_staging: Vec::new(),
            tracks: HashMap::new(),
            next_id: 0,
            current_track: None,
            stream_track: None,
            volume: 80,
            playing: false,
            paused: false,
            sample_rate: 0,
            channels: 0,
            samples_queued: 0,
        }
    }

    /// Open an SDL3 audio device and stream with the given sample rate and
    /// channels.
    fn open_device(&mut self, sample_rate: i32, channels: u8) -> Result<()> {
        let audio = self
            .audio_subsystem
            .as_ref()
            .ok_or_else(|| OasisError::Backend("audio subsystem not available".into()))?;

        let spec = sdl3::audio::AudioSpec::new(
            Some(sample_rate),
            Some(channels as i32),
            Some(sdl3::audio::AudioFormat::S16LE),
        );
        let device = audio
            .open_playback_device(&spec)
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        let stream = device
            .open_device_stream(Some(&spec))
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        stream
            .resume()
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        self.sample_rate = sample_rate;
        self.channels = channels as usize;
        log::info!(
            "SDL3 audio device opened: {}Hz, {} channels",
            self.sample_rate,
            self.channels,
        );
        self.stream_owner = Some(stream);
        Ok(())
    }

    /// Queue PCM i16 samples to the SDL3 audio stream.
    fn queue_pcm(&mut self, pcm: &[i16]) -> Result<()> {
        if let Some(ref stream) = self.stream_owner {
            stream
                .put_data_i16(pcm)
                .map_err(|e| OasisError::Backend(e.to_string().into()))?;
            self.samples_queued += pcm.len() as u64;
        }
        Ok(())
    }

    /// Get the number of bytes currently queued in the audio stream.
    fn queued_bytes(&self) -> u32 {
        self.stream_owner
            .as_ref()
            .and_then(|s| s.queued_bytes().ok())
            .unwrap_or(0) as u32
    }

    /// Decode available MP3 frames from `mp3_buffer` and queue PCM to the
    /// SDL3 audio stream.
    ///
    /// Throttle is applied **before** decoding: if the SDL3 stream already has
    /// enough audio we leave the MP3 bytes in the buffer for next time,
    /// avoiding the old bug where decoded PCM was silently dropped.
    fn decode_buffered(&mut self) -> Result<()> {
        // If the SDL3 stream already has plenty of audio, skip decoding and
        // keep the MP3 bytes for later.  This bounds latency without losing
        // any decoded audio.
        //
        // Exception: when the mp3_buffer is small (< 16 KB), always decode.
        // This ensures the last frames of a finite track get decoded even if
        // the queue is momentarily full — otherwise those bytes would be
        // stranded when the source reaches EOF and feed_data is never called
        // again.
        if self.queued_bytes() > MAX_QUEUE_BYTES && self.mp3_buffer.len() >= 16 * 1024 {
            return Ok(());
        }

        let mut pcm_out = [0i16; 2304];
        let mut offset = 0;
        self.pcm_staging.clear();
        let mut detected_rate = 0u32;
        let mut detected_channels = 0u16;

        loop {
            let remaining = self.mp3_buffer.len() - offset;
            // Need at least 16 bytes for rmp3 to safely scan for a frame
            // header (works around an unchecked slice bounds bug in rmp3).
            if remaining < 16 {
                break;
            }
            match self.decoder.next(&self.mp3_buffer[offset..], &mut pcm_out) {
                Some((frame, consumed)) => {
                    offset += consumed;
                    if let rmp3::Frame::Audio(audio) = frame {
                        detected_rate = audio.sample_rate();
                        detected_channels = audio.channels();
                        self.pcm_staging.extend_from_slice(audio.samples());
                    }
                },
                None => break,
            }
        }

        self.mp3_buffer.drain(..offset);

        // Open device on first decoded frame (need sample rate).
        if self.stream_owner.is_none() && detected_rate > 0 && self.audio_subsystem.is_some() {
            self.open_device(detected_rate as i32, detected_channels as u8)?;
        }

        if !self.pcm_staging.is_empty() {
            if detected_rate > 0 {
                self.sample_rate = detected_rate as i32;
                self.channels = detected_channels as usize;
            }

            // Apply volume scaling.
            let vol = self.volume as i32;
            for s in &mut self.pcm_staging {
                *s = ((*s as i32 * vol) / 100) as i16;
            }

            // Always queue decoded PCM — never drop it.
            // Clone the staging buffer since queue_pcm borrows self mutably.
            let staging = std::mem::take(&mut self.pcm_staging);
            self.queue_pcm(&staging)?;
            self.pcm_staging = staging;
        }

        Ok(())
    }

    /// Decode an entire MP3 buffer and queue all PCM at once (for static
    /// tracks loaded via `load_track`).
    fn decode_and_queue_all(&mut self, audio_data: &[u8]) -> Result<()> {
        // Try WAV first (cheap header check), then fall back to MP3.
        if oasis_audio::wav::is_wav(audio_data) {
            return self.decode_wav_and_queue(audio_data);
        }
        self.decode_mp3_and_queue(audio_data)
    }

    fn decode_wav_and_queue(&mut self, wav_data: &[u8]) -> Result<()> {
        let wav = oasis_audio::wav::decode_wav(wav_data)
            .ok_or_else(|| OasisError::Backend("invalid WAV data".into()))?;

        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(wav.sample_rate as i32, wav.channels as u8)?;
        }

        self.sample_rate = wav.sample_rate as i32;
        self.channels = wav.channels as usize;

        let mut pending_pcm = wav.samples;
        let vol = self.volume as i32;
        for s in &mut pending_pcm {
            *s = ((*s as i32 * vol) / 100) as i16;
        }

        self.queue_pcm(&pending_pcm)?;
        Ok(())
    }

    fn decode_mp3_and_queue(&mut self, mp3_data: &[u8]) -> Result<()> {
        let mut decoder = rmp3::RawDecoder::new();
        let mut pcm_out = [0i16; 2304];
        let mut offset = 0;
        let mut pending_pcm: Vec<i16> = Vec::new();
        let mut detected_rate = 0u32;
        let mut detected_channels = 0u16;

        loop {
            let remaining = mp3_data.len() - offset;
            if remaining < 16 {
                break;
            }
            match decoder.next(&mp3_data[offset..], &mut pcm_out) {
                Some((frame, consumed)) => {
                    offset += consumed;
                    if let rmp3::Frame::Audio(audio) = frame {
                        detected_rate = audio.sample_rate();
                        detected_channels = audio.channels();
                        pending_pcm.extend_from_slice(audio.samples());
                    }
                },
                None => break,
            }
        }

        // Open device if needed.
        if self.stream_owner.is_none() && detected_rate > 0 && self.audio_subsystem.is_some() {
            self.open_device(detected_rate as i32, detected_channels as u8)?;
        }

        if !pending_pcm.is_empty() {
            if detected_rate > 0 {
                self.sample_rate = detected_rate as i32;
                self.channels = detected_channels as usize;
            }

            let vol = self.volume as i32;
            for s in &mut pending_pcm {
                *s = ((*s as i32 * vol) / 100) as i16;
            }

            self.queue_pcm(&pending_pcm)?;
        }

        Ok(())
    }
}

impl Default for SdlAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for SdlAudioBackend {
    fn init(&mut self) -> Result<()> {
        // Try to initialize SDL3 audio subsystem.
        // In CI/headless environments this may fail — we log a warning
        // but still mark as initialized so non-audio functionality works.
        match sdl3::init() {
            Ok(sdl) => match sdl.audio() {
                Ok(audio) => {
                    self.audio_subsystem = Some(audio);
                    log::info!("SDL3 audio subsystem initialized");
                },
                Err(e) => {
                    log::warn!("SDL3 audio unavailable: {e}");
                },
            },
            Err(e) => {
                log::warn!("SDL3 init failed (headless?): {e}");
            },
        }
        self.initialized = true;
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tracks.insert(id, data.to_vec());
        log::debug!("Loaded audio track {id} ({} bytes)", data.len());
        Ok(AudioTrackId(id))
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }

        let is_stream = self.stream_track == Some(track.0);
        if !is_stream && !self.tracks.contains_key(&track.0) {
            return Err(OasisError::Backend(
                format!("track {} not loaded", track.0).into(),
            ));
        }

        self.current_track = Some(track.0);
        self.playing = true;
        self.paused = false;

        // For static tracks, decode the full MP3 and queue all PCM.
        if !is_stream && let Some(data) = self.tracks.get(&track.0).cloned() {
            self.decode_and_queue_all(&data)?;
        }

        // Resume the SDL3 audio device so queued samples play.
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.resume();
        }

        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if !self.playing {
            return Err(OasisError::Backend("not playing".into()));
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.pause();
        }
        self.playing = false;
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if !self.paused {
            return Err(OasisError::Backend("not paused".into()));
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.resume();
        }
        self.playing = true;
        self.paused = false;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.pause();
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.clear();
        }
        self.mp3_buffer.clear();
        self.playing = false;
        self.paused = false;
        self.samples_queued = 0;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn position_ms(&self) -> u64 {
        if self.sample_rate > 0 && self.channels > 0 {
            // Subtract unplayed samples still sitting in the SDL audio stream
            // so the position reflects what the user actually hears, not what
            // has been decoded and queued.  `stream.available()` returns queued
            // bytes; each sample is i16 = 2 bytes.
            let unplayed = self.queued_bytes() as u64 / 2;
            let played = self.samples_queued.saturating_sub(unplayed);
            (played / self.channels as u64) * 1000 / self.sample_rate as u64
        } else {
            0
        }
    }

    fn duration_ms(&self) -> u64 {
        // For streaming tracks, duration is unknown.
        if self.stream_track.is_some() {
            return 0;
        }
        // Rough estimate from MP3 data size (assume 128kbps).
        if let Some(id) = self.current_track
            && let Some(data) = self.tracks.get(&id)
        {
            return (data.len() as u64 * 8) / 128;
        }
        0
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        if self.current_track == Some(track.0) {
            self.stop()?;
            self.current_track = None;
        }
        if self.stream_track == Some(track.0) {
            self.stream_track = None;
            self.mp3_buffer.clear();
            self.decoder = rmp3::RawDecoder::new();
        }
        self.tracks.remove(&track.0);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        self.stream_owner = None;
        self.audio_subsystem = None;
        self.tracks.clear();
        self.stream_track = None;
        self.mp3_buffer.clear();
        self.pcm_staging.clear();
        self.decoder = rmp3::RawDecoder::new();
        self.initialized = false;
        log::info!("SDL3 audio backend shut down");
        Ok(())
    }

    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.mp3_buffer.clear();
        self.decoder = rmp3::RawDecoder::new();
        self.stream_track = Some(id);
        self.samples_queued = 0;
        log::debug!("Created streaming audio track {id}");
        Ok(AudioTrackId(id))
    }

    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> Result<()> {
        if self.stream_track != Some(track.0) {
            return Err(OasisError::Backend(
                format!("streaming track {} not found", track.0).into(),
            ));
        }

        self.mp3_buffer.extend_from_slice(data);

        // Cap MP3 buffer at 256KB to prevent unbounded growth.
        const MAX_MP3_BUF: usize = 256 * 1024;
        if self.mp3_buffer.len() > MAX_MP3_BUF {
            let drain = self.mp3_buffer.len() - MAX_MP3_BUF;
            self.mp3_buffer.drain(..drain);
        }

        self.decode_buffered()
    }

    fn feed_pcm_f32(
        &mut self,
        track: AudioTrackId,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> Result<()> {
        if self.stream_track != Some(track.0) {
            return Err(OasisError::Backend(
                format!("streaming track {} not found", track.0).into(),
            ));
        }

        if samples.is_empty() {
            return Ok(());
        }

        // Open device lazily with the stream's format, or reopen if format changed.
        let format_changed = self.stream_owner.is_some()
            && (self.sample_rate != sample_rate as i32 || self.channels != channels as usize);
        if format_changed {
            log::info!(
                "SDL audio: format change {}Hz/{}ch -> {}Hz/{}ch, reopening device",
                self.sample_rate,
                self.channels,
                sample_rate,
                channels,
            );
            self.stream_owner = None;
        }
        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(sample_rate as i32, channels as u8)?;
            // Resume immediately — play() was called before the device existed.
            if self.playing
                && let Some(ref stream) = self.stream_owner
            {
                let _ = stream.resume();
            }
        }

        self.sample_rate = sample_rate as i32;
        self.channels = channels as usize;

        // Convert f32 → i16 with volume scaling.
        let vol = self.volume as f32 / 100.0;
        self.pcm_staging.clear();
        self.pcm_staging.reserve(samples.len());
        for &s in samples {
            let scaled = s * vol * 32767.0;
            self.pcm_staging
                .push(scaled.clamp(-32768.0, 32767.0) as i16);
        }

        // Apply backpressure: skip if queue is already full.
        if self.queued_bytes() < MAX_QUEUE_BYTES {
            let staging = std::mem::take(&mut self.pcm_staging);
            self.queue_pcm(&staging)?;
            self.pcm_staging = staging;
        } else {
            log::trace!(
                "SDL audio: queue full ({} bytes), dropping {} samples",
                self.queued_bytes(),
                self.pcm_staging.len(),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_backend() -> SdlAudioBackend {
        let mut backend = SdlAudioBackend::new();
        backend.init().unwrap();
        backend
    }

    #[test]
    fn init_and_shutdown() {
        let mut backend = SdlAudioBackend::new();
        assert!(!backend.initialized);
        backend.init().unwrap();
        assert!(backend.initialized);
        backend.shutdown().unwrap();
        assert!(!backend.initialized);
    }

    #[test]
    fn load_and_play_track() {
        let mut backend = init_backend();
        let track = backend.load_track(b"fake mp3 data here").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
        assert_eq!(backend.current_track, Some(track.0));
    }

    #[test]
    fn load_without_init_fails() {
        let mut backend = SdlAudioBackend::new();
        assert!(backend.load_track(b"data").is_err());
    }

    #[test]
    fn play_missing_track_fails() {
        let mut backend = init_backend();
        assert!(backend.play(AudioTrackId(999)).is_err());
    }

    #[test]
    fn pause_and_resume() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();

        backend.pause().unwrap();
        assert!(!backend.is_playing());
        assert!(backend.paused);

        backend.resume().unwrap();
        assert!(backend.is_playing());
        assert!(!backend.paused);
    }

    #[test]
    fn pause_when_not_playing_fails() {
        let mut backend = init_backend();
        assert!(backend.pause().is_err());
    }

    #[test]
    fn resume_when_not_paused_fails() {
        let mut backend = init_backend();
        assert!(backend.resume().is_err());
    }

    #[test]
    fn stop_playback() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();
        backend.stop().unwrap();
        assert!(!backend.is_playing());
        assert_eq!(backend.position_ms(), 0);
    }

    #[test]
    fn volume_control() {
        let mut backend = init_backend();
        backend.set_volume(42).unwrap();
        assert_eq!(backend.get_volume(), 42);

        // Clamp to 100.
        backend.set_volume(200).unwrap();
        assert_eq!(backend.get_volume(), 100);
    }

    #[test]
    fn unload_track() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();

        backend.unload_track(track).unwrap();
        assert!(!backend.is_playing());
        assert!(!backend.tracks.contains_key(&track.0));
    }

    #[test]
    fn multiple_tracks() {
        let mut backend = init_backend();
        let t1 = backend.load_track(b"track 1").unwrap();
        let t2 = backend.load_track(b"track 2").unwrap();
        assert_ne!(t1, t2);
        assert_eq!(backend.tracks.len(), 2);

        backend.play(t1).unwrap();
        assert!(backend.is_playing());

        backend.stop().unwrap();
        backend.play(t2).unwrap();
        assert!(backend.is_playing());
        assert_eq!(backend.current_track, Some(t2.0));
    }

    #[test]
    fn shutdown_clears_tracks() {
        let mut backend = init_backend();
        backend.load_track(b"track 1").unwrap();
        backend.load_track(b"track 2").unwrap();
        backend.shutdown().unwrap();
        assert!(backend.tracks.is_empty());
    }

    #[test]
    fn volume_clamps_to_zero() {
        let mut backend = init_backend();
        backend.set_volume(0).unwrap();
        assert_eq!(backend.get_volume(), 0);
    }

    #[test]
    fn volume_clamps_to_max() {
        let mut backend = init_backend();
        backend.set_volume(255).unwrap();
        assert_eq!(backend.get_volume(), 100);
    }

    #[test]
    fn rapid_play_pause_cycle() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();

        for _ in 0..10 {
            backend.play(track).unwrap();
            assert!(backend.is_playing());
            backend.pause().unwrap();
            assert!(!backend.is_playing());
            backend.resume().unwrap();
            assert!(backend.is_playing());
            backend.stop().unwrap();
            assert!(!backend.is_playing());
        }
    }

    #[test]
    fn double_stop_is_idempotent() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();
        backend.stop().unwrap();
        // Second stop should not error.
        backend.stop().unwrap();
        assert!(!backend.is_playing());
    }

    #[test]
    fn unload_nonexistent_track_is_silent() {
        let mut backend = init_backend();
        // Unloading a track that doesn't exist just removes nothing.
        backend.unload_track(AudioTrackId(999)).unwrap();
    }

    #[test]
    fn play_after_unload_fails() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.unload_track(track).unwrap();
        assert!(backend.play(track).is_err());
    }

    #[test]
    fn duration_estimate_scales_with_data() {
        let mut backend = init_backend();
        let small = backend.load_track(&[0u8; 1_000]).unwrap();
        backend.play(small).unwrap();
        let dur_small = backend.duration_ms();

        backend.stop().unwrap();
        let large = backend.load_track(&[0u8; 10_000]).unwrap();
        backend.play(large).unwrap();
        let dur_large = backend.duration_ms();

        assert!(
            dur_large > dur_small,
            "larger track should have longer duration: {dur_large} > {dur_small}"
        );
    }

    // ---------------------------------------------------------------
    // Streaming tests
    // ---------------------------------------------------------------

    #[test]
    fn streaming_lifecycle() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        assert_eq!(backend.stream_track, Some(track.0));
        backend.feed_data(track, b"chunk 1").unwrap();
        backend.feed_data(track, b"chunk 2").unwrap();
        // Decoder consumes garbage bytes while scanning for sync words,
        // so mp3_buffer may be smaller than total fed bytes.
        backend.unload_track(track).unwrap();
        assert_eq!(backend.stream_track, None);
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_buffer_is_bounded() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed more than 256KB to verify the buffer is capped.
        for _ in 0..80 {
            backend.feed_data(track, &[0xAA; 4096]).unwrap();
        }
        // 80 * 4096 = 320KB, should be capped to 256KB.
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_without_init_fails() {
        let mut backend = SdlAudioBackend::new();
        assert!(backend.load_streaming().is_err());
    }

    #[test]
    fn feed_data_invalid_track_fails() {
        let mut backend = init_backend();
        assert!(backend.feed_data(AudioTrackId(999), b"data").is_err());
    }

    #[test]
    fn reinit_after_shutdown() {
        let mut backend = init_backend();
        backend.load_track(b"data").unwrap();
        backend.shutdown().unwrap();
        assert!(!backend.initialized);

        // Re-init should work.
        backend.init().unwrap();
        assert!(backend.initialized);
        let track = backend.load_track(b"new data").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
    }

    #[test]
    fn streaming_starts_empty() {
        let mut backend = init_backend();
        let _track = backend.load_streaming().unwrap();
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_play_after_feed() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"audio data").unwrap();
        // Should be able to play a streaming track.
        backend.play(track).unwrap();
        assert!(backend.is_playing());
    }

    #[test]
    fn streaming_unload_stops_playback() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"data").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
        backend.unload_track(track).unwrap();
        assert!(!backend.is_playing());
        assert_eq!(backend.stream_track, None);
    }

    #[test]
    fn streaming_feed_empty_data() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"").unwrap();
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_buffer_cap_prevents_oom() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed a large chunk; the decoder will consume garbage data as it
        // scans, and the 256KB cap prevents unbounded growth in between.
        let chunk = vec![0xBBu8; 256 * 1024];
        backend.feed_data(track, &chunk).unwrap();
        // After decoding, buffer should not exceed 256KB.
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_overflow_does_not_error() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed >256KB to verify the cap + decode cycle stays stable.
        let first = vec![0xAAu8; 256 * 1024];
        backend.feed_data(track, &first).unwrap();
        backend.feed_data(track, &[0xBBu8; 100]).unwrap();
        // Buffer should never exceed cap (decoder consumes garbage).
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_shutdown_clears_streaming() {
        let mut backend = init_backend();
        let t1 = backend.load_streaming().unwrap();
        backend.feed_data(t1, b"data").unwrap();
        backend.shutdown().unwrap();
        assert_eq!(backend.stream_track, None);
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_feed_after_unload_fails() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.unload_track(track).unwrap();
        assert!(backend.feed_data(track, b"data").is_err());
    }

    #[test]
    fn streaming_incremental_feed() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed many small chunks; decoder consumes garbage bytes, so the
        // buffer may not grow monotonically, but no errors should occur.
        for i in 0..10 {
            backend.feed_data(track, &[i as u8; 100]).unwrap();
        }
        // Buffer stays bounded (decoder consumed garbage).
        assert!(backend.mp3_buffer.len() <= 1000);
    }

    #[test]
    fn position_ms_is_zero_before_playback() {
        let backend = init_backend();
        assert_eq!(backend.position_ms(), 0);
    }
}

//! Audio playback (MP3 via psp::mp3 + psp::audio) and `AudioBackend` trait.

use psp::audio::{AudioChannel, AudioFormat};
use psp::mp3::Mp3Decoder;

use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::{OasisError, Result};

use crate::threading::{AudioCmd, AudioHandle, send_audio_cmd};

/// Standard MP3 frame size (MPEG1 Layer 3).
const MP3_FRAME_SAMPLES: i32 = 1152;

/// MP3 playback engine using the PSP's hardware MP3 decoder.
///
/// Uses RAII wrappers from `psp::mp3::Mp3Decoder` and
/// `psp::audio::AudioChannel`. Call `load_and_play()` to start,
/// `update()` each frame to pump decoded audio, and `stop()` to halt.
pub struct AudioPlayer {
    decoder: Option<Mp3Decoder>,
    channel: Option<AudioChannel>,
    playing: bool,
    paused: bool,
    /// Cached MP3 info.
    pub sample_rate: u32,
    pub bitrate: u32,
    pub channels: u32,
    /// Count of decoded MP3 frames (for position tracking).
    pub frames_decoded: u32,
    /// Total file size in bytes (for duration estimation).
    pub data_size: u32,
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            decoder: None,
            channel: None,
            playing: false,
            paused: false,
            sample_rate: 0,
            bitrate: 0,
            channels: 0,
            frames_decoded: 0,
            data_size: 0,
        }
    }

    /// Initialize the audio subsystem.
    ///
    /// With the new SDK, `Mp3Decoder` handles resource init on construction,
    /// so this is a no-op kept for API compatibility with the worker thread.
    pub fn init(&mut self) -> bool {
        true
    }

    /// Load an MP3 file from the Memory Stick and start playback.
    pub fn load_and_play(&mut self, path: &str) -> bool {
        let data = match psp::io::read_to_vec(path) {
            Ok(d) => d,
            Err(_) => return false,
        };
        self.load_and_play_data(&data)
    }

    /// Start playback from raw MP3 data already in memory.
    pub fn load_and_play_data(&mut self, data: &[u8]) -> bool {
        self.stop();

        if data.is_empty() {
            return false;
        }

        let decoder = match Mp3Decoder::new(data) {
            Ok(d) => d,
            Err(e) => {
                psp::dprintln!("OASIS_OS: Mp3Decoder failed: {:?}", e);
                return false;
            }
        };

        self.sample_rate = decoder.sample_rate();
        self.bitrate = decoder.bitrate();
        self.channels = decoder.channels() as u32;
        self.frames_decoded = 0;
        self.data_size = data.len() as u32;

        let fmt = if self.channels == 1 {
            AudioFormat::Mono
        } else {
            AudioFormat::Stereo
        };

        let channel = match AudioChannel::reserve(MP3_FRAME_SAMPLES, fmt) {
            Ok(ch) => ch,
            Err(e) => {
                psp::dprintln!(
                    "OASIS_OS: AudioChannel::reserve failed: {:?}",
                    e,
                );
                return false;
            }
        };

        self.decoder = Some(decoder);
        self.channel = Some(channel);
        self.playing = true;
        self.paused = false;

        psp::dprintln!(
            "OASIS_OS: MP3 loaded - {}Hz, {}kbps, {}ch",
            self.sample_rate,
            self.bitrate,
            self.channels,
        );
        true
    }

    /// Pump decoded audio to the output channel. Call each frame.
    pub fn update(&mut self) {
        if !self.playing || self.paused {
            return;
        }

        let (Some(decoder), Some(channel)) =
            (&mut self.decoder, &self.channel)
        else {
            return;
        };

        match decoder.decode_frame() {
            Ok(samples) if !samples.is_empty() => {
                self.frames_decoded += 1;
                // output_blocking paces playback to hardware timing.
                let _ = channel.output_blocking(0x8000, samples);
            }
            _ => {
                // End of stream or decode error.
                self.playing = false;
            }
        }
    }

    /// Stop playback and release resources.
    pub fn stop(&mut self) {
        // Drop order: channel first (stops hardware output), then decoder.
        self.channel = None;
        self.decoder = None;
        self.playing = false;
        self.paused = false;
    }

    /// Toggle pause/resume.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Estimated playback position in milliseconds.
    pub fn position_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.frames_decoded as u64 * MP3_FRAME_SAMPLES as u64 * 1000)
            / self.sample_rate as u64
    }

    /// Estimated total duration in milliseconds (from bitrate + file size).
    pub fn duration_ms(&self) -> u64 {
        if self.bitrate == 0 {
            return 0;
        }
        // bitrate is in kbps, data_size in bytes.
        (self.data_size as u64 * 8) / self.bitrate as u64
    }
}

// ---------------------------------------------------------------------------
// AudioBackend trait implementation (delegates to worker thread)
// ---------------------------------------------------------------------------

/// PSP audio backend that delegates to the audio worker thread.
///
/// Stores loaded track data locally and sends it to the audio thread
/// on `play()`. Reads playback state from the shared `SpinMutex` via
/// `AudioHandle`.
pub struct PspAudioBackend {
    audio: AudioHandle,
    tracks: Vec<Option<Vec<u8>>>,
    current_track: Option<u64>,
    volume: u8,
}

impl PspAudioBackend {
    /// Create a new PSP audio backend.
    pub fn new() -> Self {
        Self {
            audio: AudioHandle,
            tracks: Vec::new(),
            current_track: None,
            volume: 80,
        }
    }
}

impl AudioBackend for PspAudioBackend {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        let id = self.tracks.len() as u64;
        self.tracks.push(Some(data.to_vec()));
        Ok(AudioTrackId(id))
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        let idx = track.0 as usize;
        let data = self
            .tracks
            .get(idx)
            .and_then(|slot| slot.as_ref())
            .ok_or_else(|| {
                OasisError::Backend(format!("track {} not loaded", track.0))
            })?
            .clone();
        send_audio_cmd(AudioCmd::LoadAndPlayData(data));
        self.current_track = Some(track.0);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Pause);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Resume);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Stop);
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
        self.audio.is_playing()
    }

    fn position_ms(&self) -> u64 {
        self.audio.state().position_ms
    }

    fn duration_ms(&self) -> u64 {
        self.audio.state().duration_ms
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        let idx = track.0 as usize;
        if self.current_track == Some(track.0) {
            self.stop()?;
            self.current_track = None;
        }
        if let Some(slot) = self.tracks.get_mut(idx) {
            *slot = None;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        self.tracks.clear();
        Ok(())
    }
}

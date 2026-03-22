//! Audio playback (MP3 via sceAudiocodec + psp::audio) and `AudioBackend` trait.
//!
//! Uses the low-level `sceAudiocodec` frame-by-frame decoder instead of the
//! high-level `sceMp3*` API, which crashes on real PSP hardware after ~2-3
//! handle reuse cycles.
//!
//! MP3 data is **streamed from file** using a small fixed-size read buffer
//! (32 KB) to avoid large heap allocations that cause heap fragmentation
//! and crashes on PSP's limited 24 MB user memory.

mod frame_parser;
mod player;
mod radio;

pub(crate) use player::AudioPlayer;
pub(crate) use radio::RadioStreamer;

use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::{OasisError, Result};

use crate::threading::{AudioCmd, AudioHandle, send_audio_cmd};

/// Standard MP3 frame size (MPEG1 Layer 3).
const MP3_FRAME_SAMPLES: i32 = 1152;

/// Size of the read buffer for streaming MP3 from file.
/// 32 KB is enough for many MP3 frames and avoids large heap allocations.
const READ_BUF_SIZE: usize = 32 * 1024;

/// Load AV codec modules once (idempotent). Called lazily on first play
/// to avoid conflicts with the PRX overlay at boot time.
///
/// Also exposed as `pub` for use by the video AAC decoder in threading.rs.
pub fn load_av_modules_once_pub() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOADED: AtomicBool = AtomicBool::new(false);
    if LOADED.swap(true, Ordering::Relaxed) {
        return; // Already loaded.
    }
    // SAFETY: sceUtilityLoadModule loads firmware modules into memory.
    // AvCodec and AvMpegBase are required for sceAudiocodec and
    // sceVideocodec. AvMp3 is for MP3 decoding.
    // The AtomicBool guard ensures these are loaded at most once.
    // Log return values to diagnose video codec init failures.
    // SAFETY: sceIo log + sceUtilityLoadModule calls.
    unsafe {
        let r1 = psp::sys::sceUtilityLoadModule(psp::sys::Module::AvCodec);
        let r2 = psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMpegBase);
        let r3 = psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMp3);
        // Log to file (dprintln goes to screen, not eboot.log).
        let msg = format!(
            "[AV] modules: AvCodec={r1:#x} AvMpegBase={r2:#x} AvMp3={r3:#x}\n",
        );
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }
}

/// Internal alias.
fn load_av_modules_once() {
    load_av_modules_once_pub();
}

// ---------------------------------------------------------------------------
// AudioBackend trait implementation (delegates to worker thread)
// ---------------------------------------------------------------------------

/// PSP audio backend that delegates to the audio worker thread.
///
/// Track data is moved (not cloned) to the audio thread on play to
/// minimize peak memory. Only one track's data lives in this struct
/// at a time — previous tracks are freed on load.
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
        // Free all previous track data to conserve PSP memory.
        for slot in &mut self.tracks {
            *slot = None;
        }
        self.tracks.push(Some(data.to_vec()));
        Ok(AudioTrackId(id))
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        let idx = track.0 as usize;
        let data = self
            .tracks
            .get_mut(idx)
            .and_then(|slot| slot.take())
            .ok_or_else(|| OasisError::Backend(format!("track {} not loaded", track.0).into()))?;
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
        send_audio_cmd(AudioCmd::SetVolume(self.volume));
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        self.audio.is_playing()
    }

    fn position_ms(&self) -> u64 {
        self.audio.position_ms()
    }

    fn duration_ms(&self) -> u64 {
        self.audio.duration_ms()
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

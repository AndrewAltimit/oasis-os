//! Backend-agnostic consumer of the `/var/audio/*` VFS IPC.
//!
//! Requests ([`MediaRequest`] lines) arrive at [`AUDIO_REQUEST_PATH`] from
//! the Music Player app (`play_file <path>` when the user opens a track,
//! `stop` when they back out), from the `music` terminal command and from
//! the MCP `play_media` tool. [`AudioManager`] keeps the playlist and the
//! playback state and says what to do; [`MusicController`] carries that out
//! on the host's [`AudioBackend`] and publishes, every tick:
//!
//! - [`AUDIO_POSITION_PATH`]: the music track's real playback position
//!   (`<position_ms> <duration_ms> <path>`), for the Music Player's
//!   progress bar;
//! - [`AUDIO_STATUS_PATH`]: the document `music status` prints (rewritten
//!   only when it changes).
//!
//! Every host that runs the Music Player drives one of these: the desktop
//! shell (`oasis-app/src/media_controller.rs`, SDL audio) and the WASM
//! backend (Web Audio / `<audio>`). The host keeps the [`AudioManager`] and
//! the music track id in its own state and lends them to a
//! [`MusicController`] for the duration of a call, together with the
//! two things only it knows: whether another consumer currently shares the
//! process-global audio stream, and what else must stop before music
//! starts (internet radio).

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::ipc::{AUDIO_POSITION_PATH, AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, MediaRequest};
use oasis_vfs::Vfs;

use crate::manager::{AudioManager, MediaAction};
use crate::types::PlaybackState;

/// Position-report granularity: the Music Player's readout is m:ss and
/// its bar a few hundred pixels wide, so 250 ms steps are plenty and
/// avoid rewriting the IPC file every frame.
pub const POSITION_STEP_MS: u64 = 250;

/// The host's music state and audio backend, borrowed for one call.
pub struct MusicController<'a> {
    /// Playlist + playback state.
    pub manager: &'a mut AudioManager,
    /// The loaded music track, if any (owned by the host's state).
    pub track: &'a mut Option<AudioTrackId>,
    /// Where the music plays.
    pub audio: &'a mut dyn AudioBackend,
    /// Another consumer (desktop: TV Guide video audio) currently plays
    /// through the same backend. `AudioBackend::pause` / `resume` / `stop`
    /// are process-global, so while this is set the controller leaves
    /// them alone and only unloads its own track.
    pub stream_shared: bool,
}

impl MusicController<'_> {
    /// Publish the position, carry out every pending request, publish the
    /// status.
    ///
    /// `before_play` runs right before a new track loads; hosts use it to
    /// tear down internet radio so the two never play at once.
    pub fn tick(&mut self, vfs: &mut dyn Vfs, before_play: &mut dyn FnMut(&mut dyn AudioBackend)) {
        self.publish_position(vfs);
        for request in drain_requests(vfs) {
            self.handle(vfs, &request, before_play);
        }
        self.publish_status(vfs);
    }

    /// Stop and unload the music track (the Music Player window closed:
    /// its own `stop` request only runs on its Cancel path).
    pub fn shutdown(&mut self) {
        self.stop_track();
        self.manager.set_state(PlaybackState::Stopped);
    }

    /// Apply one request to the playlist model and carry out its action.
    pub fn handle(
        &mut self,
        vfs: &mut dyn Vfs,
        request: &MediaRequest,
        before_play: &mut dyn FnMut(&mut dyn AudioBackend),
    ) {
        match self.manager.apply(request) {
            MediaAction::None => {},
            MediaAction::PlayPath(path) => {
                self.play_file(vfs, &path, before_play);
                if self.track.is_some() {
                    self.write_position(vfs, &path);
                } else {
                    self.manager.set_state(PlaybackState::Stopped);
                }
            },
            MediaAction::Pause => {
                if self.track.is_some() && !self.stream_shared {
                    if let Err(e) = self.audio.pause() {
                        log::warn!("music: pause failed: {e}");
                    }
                } else if self.track.is_none() {
                    self.manager.set_state(PlaybackState::Stopped);
                }
            },
            MediaAction::Resume => {
                if self.track.is_some() && !self.stream_shared {
                    if let Err(e) = self.audio.resume() {
                        log::warn!("music: resume failed: {e}");
                    }
                } else if self.track.is_none() {
                    self.manager.set_state(PlaybackState::Stopped);
                }
            },
            MediaAction::Stop => {
                self.stop_track();
                let _ = vfs.write(AUDIO_POSITION_PATH, b"");
            },
            MediaAction::SetVolume(volume) => {
                if let Err(e) = self.audio.set_volume(volume) {
                    log::warn!("music: set_volume({volume}) failed: {e}");
                }
            },
        }
    }

    /// Publish the `music status` document (written only when it changes).
    fn publish_status(&self, vfs: &mut dyn Vfs) {
        let (pos, dur) = if self.track.is_some() {
            (self.audio.position_ms(), self.audio.duration_ms())
        } else {
            (0, 0)
        };
        let volume = self.audio.get_volume();
        if let Err(e) = self.manager.publish_status(vfs, volume, pos, dur) {
            log::warn!("music: cannot publish {AUDIO_STATUS_PATH}: {e}");
        }
    }

    /// Write `<position_ms> <duration_ms> <path>` to
    /// [`AUDIO_POSITION_PATH`] (skipped when unchanged).
    fn write_position(&self, vfs: &mut dyn Vfs, path: &str) {
        let pos = self.audio.position_ms() / POSITION_STEP_MS * POSITION_STEP_MS;
        let line = format!("{pos} {} {path}", self.audio.duration_ms());
        let unchanged = vfs
            .read(AUDIO_POSITION_PATH)
            .is_ok_and(|prev| prev == line.as_bytes());
        if !unchanged {
            let _ = vfs.write(AUDIO_POSITION_PATH, line.as_bytes());
        }
    }

    /// Refresh the position report for the track that is playing (its
    /// path is carried in the report written when it started).
    fn publish_position(&self, vfs: &mut dyn Vfs) {
        if self.track.is_none() {
            return;
        }
        let Ok(prev) = vfs.read(AUDIO_POSITION_PATH) else {
            return;
        };
        let prev = String::from_utf8_lossy(&prev).into_owned();
        if let Some(path) = prev.trim().splitn(3, ' ').nth(2) {
            self.write_position(vfs, path);
        }
    }

    fn play_file(
        &mut self,
        vfs: &dyn Vfs,
        path: &str,
        before_play: &mut dyn FnMut(&mut dyn AudioBackend),
    ) {
        // Unload any previous track so memory doesn't grow across songs.
        self.stop_track();
        before_play(&mut *self.audio);

        let data = match vfs.read(path) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("music: cannot read {path} from VFS: {e}");
                return;
            },
        };
        match self.audio.load_track(&data) {
            Ok(track) => {
                if let Err(e) = self.audio.play(track) {
                    log::warn!("music: play failed for {path}: {e}");
                    let _ = self.audio.unload_track(track);
                    return;
                }
                *self.track = Some(track);
                log::info!("music: playing {path} ({} bytes)", data.len());
            },
            Err(e) => log::warn!("music: load_track failed for {path}: {e}"),
        }
    }

    fn stop_track(&mut self) {
        if let Some(track) = self.track.take() {
            // `AudioBackend::stop()` is process-global: with another
            // consumer on the stream it would silence that one too.
            // `unload_track` already stops the music track when it is the
            // current one, so the explicit call is only for the sole-owner
            // case.
            if !self.stream_shared {
                let _ = self.audio.stop();
            }
            let _ = self.audio.unload_track(track);
        }
    }
}

/// Take every recognised request line out of [`AUDIO_REQUEST_PATH`],
/// leaving unrecognised lines in place (so a future controller sharing the
/// path still sees its own payload).
///
/// The file is newline-delimited and drained whole every tick, so two
/// requests queued in one frame are both served.
pub fn drain_requests(vfs: &mut dyn Vfs) -> Vec<MediaRequest> {
    let Ok(data) = vfs.read(AUDIO_REQUEST_PATH) else {
        return Vec::new();
    };
    if data.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut requests = Vec::new();
    let mut unhandled: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        match MediaRequest::parse(line) {
            Some(req) => requests.push(req),
            None => unhandled.push(line),
        }
    }
    if !requests.is_empty() {
        let remainder = unhandled.join("\n");
        let _ = vfs.write(AUDIO_REQUEST_PATH, remainder.as_bytes());
    }
    requests
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullAudioBackend;
    use oasis_vfs::MemoryVfs;

    struct Host {
        manager: AudioManager,
        track: Option<AudioTrackId>,
        audio: NullAudioBackend,
        vfs: MemoryVfs,
        before_play_calls: usize,
    }

    impl Host {
        fn new() -> Self {
            let mut vfs = MemoryVfs::new();
            vfs.mkdir("/var").unwrap();
            vfs.mkdir("/var/audio").unwrap();
            vfs.mkdir("/music").unwrap();
            vfs.write("/music/a.mp3", b"ID3a").unwrap();
            vfs.write("/music/b.mp3", b"ID3b").unwrap();
            Self {
                manager: AudioManager::new(),
                track: None,
                audio: NullAudioBackend::new(),
                vfs,
                before_play_calls: 0,
            }
        }

        fn request(&mut self, line: &str) {
            self.vfs.write(AUDIO_REQUEST_PATH, line.as_bytes()).unwrap();
            self.tick(false);
        }

        fn tick(&mut self, stream_shared: bool) {
            let calls = &mut self.before_play_calls;
            MusicController {
                manager: &mut self.manager,
                track: &mut self.track,
                audio: &mut self.audio,
                stream_shared,
            }
            .tick(&mut self.vfs, &mut |_| *calls += 1);
        }

        fn status(&self) -> String {
            String::from_utf8(self.vfs.read(AUDIO_STATUS_PATH).unwrap()).unwrap()
        }
    }

    #[test]
    fn play_file_loads_plays_and_reports() {
        let mut h = Host::new();
        h.request("play_file /music/a.mp3");
        assert!(h.track.is_some());
        assert!(h.audio.is_playing());
        assert_eq!(h.before_play_calls, 1);
        assert_eq!(h.manager.state(), PlaybackState::Playing);
        let pos = h.vfs.read(AUDIO_POSITION_PATH).unwrap();
        assert_eq!(pos, b"0 0 /music/a.mp3");
        assert!(
            h.status().to_lowercase().contains("playing"),
            "{}",
            h.status()
        );
        // The request file was drained.
        assert!(h.vfs.read(AUDIO_REQUEST_PATH).unwrap().is_empty());
    }

    #[test]
    fn two_requests_in_one_tick_are_both_served() {
        let mut h = Host::new();
        h.request("play_file /music/a.mp3\nplay_file /music/b.mp3");
        assert_eq!(h.manager.playlist.len(), 2);
        assert_eq!(h.manager.current_path(), Some("/music/b.mp3"));
        // The first track was unloaded when the second started.
        assert_eq!(h.audio.loaded_count(), 1);
    }

    #[test]
    fn missing_file_leaves_playback_stopped() {
        let mut h = Host::new();
        h.request("play_file /music/nope.mp3");
        assert!(h.track.is_none());
        assert_eq!(h.manager.state(), PlaybackState::Stopped);
    }

    #[test]
    fn pause_resume_stop_drive_the_backend() {
        let mut h = Host::new();
        h.request("play_file /music/a.mp3");
        h.request("pause");
        assert!(!h.audio.is_playing());
        assert_eq!(h.manager.state(), PlaybackState::Paused);
        h.request("resume");
        assert!(h.audio.is_playing());
        h.request("stop");
        assert!(h.track.is_none());
        assert_eq!(h.audio.loaded_count(), 0);
        assert!(h.vfs.read(AUDIO_POSITION_PATH).unwrap().is_empty());
    }

    #[test]
    fn shared_stream_is_not_paused() {
        let mut h = Host::new();
        h.request("play_file /music/a.mp3");
        h.vfs.write(AUDIO_REQUEST_PATH, b"pause").unwrap();
        h.tick(true);
        // The model pauses, but the process-global backend keeps playing
        // for the other consumer.
        assert_eq!(h.manager.state(), PlaybackState::Paused);
        assert!(h.audio.is_playing());
    }

    #[test]
    fn volume_and_unknown_lines() {
        let mut h = Host::new();
        h.request("vol 30\nfrobnicate");
        assert_eq!(h.audio.get_volume(), 30);
        // The unrecognised line stays for whoever understands it.
        assert_eq!(h.vfs.read(AUDIO_REQUEST_PATH).unwrap(), b"frobnicate");
    }

    #[test]
    fn shutdown_unloads_and_stops() {
        let mut h = Host::new();
        h.request("play_file /music/a.mp3");
        MusicController {
            manager: &mut h.manager,
            track: &mut h.track,
            audio: &mut h.audio,
            stream_shared: false,
        }
        .shutdown();
        assert!(h.track.is_none());
        assert_eq!(h.audio.loaded_count(), 0);
        assert_eq!(h.manager.state(), PlaybackState::Stopped);
    }
}

//! Music playback state machine behind the `/var/audio/*` VFS IPC.
//!
//! [`AudioManager`] owns the playlist, repeat/shuffle modes and the
//! playback state, and turns each [`MediaRequest`] into a [`MediaAction`]
//! for the host to carry out on its `AudioBackend`. It never touches a
//! backend itself: the host knows things the manager does not (which
//! loaded track is the music track, whether another consumer such as a
//! video player currently owns the shared audio stream), so the split
//! keeps those decisions in one place and keeps this type testable
//! without audio hardware.
//!
//! The desktop shell drives it from `oasis-app/src/media_controller.rs`;
//! the `music` terminal command and the Music Player app produce the
//! requests.

use oasis_types::error::Result;
use oasis_types::ipc::{AUDIO_STATUS_PATH, MediaRequest};
use oasis_vfs::Vfs;

use super::playlist::{Playlist, format_duration};
use super::types::{PlaybackState, TrackInfo};

/// What the host must do on its audio backend after
/// [`AudioManager::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaAction {
    /// Nothing to do on the backend (state-only change or a no-op).
    None,
    /// Release the current music track, then load and play this VFS path.
    PlayPath(String),
    /// Pause the music track.
    Pause,
    /// Resume the paused music track.
    Resume,
    /// Stop and release the music track.
    Stop,
    /// Set the output volume (0-100).
    SetVolume(u8),
}

/// Playlist + playback-state model for the host's music controller.
#[derive(Debug, Default)]
pub struct AudioManager {
    /// Current playback state.
    state: PlaybackState,
    /// The playlist (every file played via `play_file`, in first-play order).
    pub playlist: Playlist,
}

impl AudioManager {
    /// Create an empty, stopped manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current playback state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Override the playback state. The host calls this when an action
    /// could not be carried out (e.g. the file failed to load) or when
    /// playback ended outside the manager (the app was closed).
    pub fn set_state(&mut self, state: PlaybackState) {
        self.state = state;
    }

    /// VFS path of the current playlist entry.
    pub fn current_path(&self) -> Option<&str> {
        self.playlist.current_track().map(|t| t.path.as_str())
    }

    /// Apply one request and return the backend action it needs.
    pub fn apply(&mut self, request: &MediaRequest) -> MediaAction {
        match request {
            MediaRequest::PlayFile(path) => {
                let idx = match self.playlist.position_of(path) {
                    Some(i) => i,
                    None => {
                        self.playlist.add(TrackInfo::from_path(path));
                        self.playlist.len() - 1
                    },
                };
                self.playlist.set_current(idx);
                self.state = PlaybackState::Playing;
                MediaAction::PlayPath(path.clone())
            },
            MediaRequest::Play => match self.state {
                PlaybackState::Playing => MediaAction::None,
                PlaybackState::Paused => {
                    self.state = PlaybackState::Playing;
                    MediaAction::Resume
                },
                PlaybackState::Stopped => {
                    if self.playlist.current_track().is_none() && !self.playlist.set_current(0) {
                        return MediaAction::None;
                    }
                    self.play_current()
                },
            },
            MediaRequest::Pause => {
                if self.state != PlaybackState::Playing {
                    return MediaAction::None;
                }
                self.state = PlaybackState::Paused;
                MediaAction::Pause
            },
            MediaRequest::Resume => {
                if self.state != PlaybackState::Paused {
                    return MediaAction::None;
                }
                self.state = PlaybackState::Playing;
                MediaAction::Resume
            },
            MediaRequest::Stop => {
                self.state = PlaybackState::Stopped;
                MediaAction::Stop
            },
            MediaRequest::Next => {
                let moved = self.playlist.advance();
                self.after_skip(moved)
            },
            MediaRequest::Prev => {
                let moved = self.playlist.go_back();
                self.after_skip(moved)
            },
            MediaRequest::Volume(v) => MediaAction::SetVolume((*v).min(100)),
            MediaRequest::Repeat(mode) => {
                self.playlist.repeat = *mode;
                MediaAction::None
            },
            MediaRequest::Shuffle => {
                self.playlist.set_shuffle(!self.playlist.shuffle);
                MediaAction::None
            },
        }
    }

    /// Start the current playlist entry.
    fn play_current(&mut self) -> MediaAction {
        match self.current_path().map(str::to_string) {
            Some(path) => {
                self.state = PlaybackState::Playing;
                MediaAction::PlayPath(path)
            },
            None => MediaAction::None,
        }
    }

    /// After `next` / `prev`: play the new entry unless playback is
    /// stopped (then only the selection moves); at either end of a
    /// non-repeating playlist, stop.
    fn after_skip(&mut self, moved: bool) -> MediaAction {
        if !moved {
            if self.state == PlaybackState::Stopped {
                return MediaAction::None;
            }
            self.state = PlaybackState::Stopped;
            return MediaAction::Stop;
        }
        if self.state == PlaybackState::Stopped {
            return MediaAction::None;
        }
        self.play_current()
    }

    /// Format the status document published at [`AUDIO_STATUS_PATH`].
    ///
    /// `volume`, `position_ms` and `duration_ms` come from the backend.
    pub fn format_status(&self, volume: u8, position_ms: u64, duration_ms: u64) -> String {
        let mut lines = Vec::new();
        lines.push(format!("State: {}", self.state));
        lines.push(format!("Volume: {volume}%"));

        if let Some(track) = self.playlist.current_track() {
            lines.push(format!("Track: {}", track.title));
            if !track.artist.is_empty() {
                lines.push(format!("Artist: {}", track.artist));
            }
            lines.push(format!("Path: {}", track.path));
            if self.state != PlaybackState::Stopped {
                let pos = format_duration(position_ms);
                let dur = format_duration(duration_ms);
                lines.push(format!("Position: {pos} / {dur}"));
            }
        }

        lines.push(format!("Repeat: {}", self.playlist.repeat));
        lines.push(format!(
            "Shuffle: {}",
            if self.playlist.shuffle { "on" } else { "off" }
        ));
        lines.push(format!("Playlist: {} tracks", self.playlist.len()));

        lines.join("\n")
    }

    /// Write [`AudioManager::format_status`] to [`AUDIO_STATUS_PATH`],
    /// skipping the write when the document is unchanged. Returns whether
    /// it wrote.
    pub fn publish_status(
        &self,
        vfs: &mut dyn Vfs,
        volume: u8,
        position_ms: u64,
        duration_ms: u64,
    ) -> Result<bool> {
        let status = self.format_status(volume, position_ms, duration_ms);
        if vfs
            .read(AUDIO_STATUS_PATH)
            .is_ok_and(|prev| prev == status.as_bytes())
        {
            return Ok(false);
        }
        vfs.write(AUDIO_STATUS_PATH, status.as_bytes())?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::ipc::RepeatMode;
    use oasis_vfs::MemoryVfs;

    fn play(path: &str) -> MediaRequest {
        MediaRequest::PlayFile(path.to_string())
    }

    /// A manager with three tracks queued (played once each, then stopped).
    fn setup() -> AudioManager {
        let mut mgr = AudioManager::new();
        for p in ["/music/a.mp3", "/music/b.mp3", "/music/c.mp3"] {
            mgr.apply(&play(p));
        }
        mgr.apply(&MediaRequest::Stop);
        mgr.playlist.set_current(0);
        mgr
    }

    #[test]
    fn new_is_stopped_and_empty() {
        let mgr = AudioManager::new();
        assert_eq!(mgr.state(), PlaybackState::Stopped);
        assert!(mgr.playlist.is_empty());
        assert_eq!(mgr.current_path(), None);
    }

    #[test]
    fn play_file_adds_once_and_plays() {
        let mut mgr = AudioManager::new();
        assert_eq!(
            mgr.apply(&play("/music/a.mp3")),
            MediaAction::PlayPath("/music/a.mp3".into())
        );
        assert_eq!(mgr.state(), PlaybackState::Playing);
        mgr.apply(&play("/music/b.mp3"));
        mgr.apply(&play("/music/a.mp3"));
        assert_eq!(
            mgr.playlist.len(),
            2,
            "re-playing a file does not duplicate it"
        );
        assert_eq!(mgr.current_path(), Some("/music/a.mp3"));
    }

    #[test]
    fn play_on_empty_playlist_does_nothing() {
        let mut mgr = AudioManager::new();
        assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::None);
        assert_eq!(mgr.state(), PlaybackState::Stopped);
    }

    #[test]
    fn play_when_stopped_plays_current() {
        let mut mgr = setup();
        assert_eq!(
            mgr.apply(&MediaRequest::Play),
            MediaAction::PlayPath("/music/a.mp3".into())
        );
        assert_eq!(mgr.state(), PlaybackState::Playing);
        assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::None);
    }

    #[test]
    fn pause_resume_cycle() {
        let mut mgr = setup();
        assert_eq!(
            mgr.apply(&MediaRequest::Pause),
            MediaAction::None,
            "not playing"
        );
        mgr.apply(&MediaRequest::Play);
        assert_eq!(mgr.apply(&MediaRequest::Pause), MediaAction::Pause);
        assert_eq!(mgr.state(), PlaybackState::Paused);
        assert_eq!(mgr.apply(&MediaRequest::Pause), MediaAction::None);
        assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::Resume);
        assert_eq!(mgr.state(), PlaybackState::Playing);
        assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::None);
        // `play` while paused resumes.
        mgr.apply(&MediaRequest::Pause);
        assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::Resume);
    }

    #[test]
    fn stop_always_stops() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Play);
        assert_eq!(mgr.apply(&MediaRequest::Stop), MediaAction::Stop);
        assert_eq!(mgr.state(), PlaybackState::Stopped);
    }

    #[test]
    fn next_and_prev_while_playing() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Play);
        assert_eq!(
            mgr.apply(&MediaRequest::Next),
            MediaAction::PlayPath("/music/b.mp3".into())
        );
        assert_eq!(
            mgr.apply(&MediaRequest::Prev),
            MediaAction::PlayPath("/music/a.mp3".into())
        );
    }

    #[test]
    fn next_while_paused_starts_the_new_track() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Play);
        mgr.apply(&MediaRequest::Pause);
        assert_eq!(
            mgr.apply(&MediaRequest::Next),
            MediaAction::PlayPath("/music/b.mp3".into())
        );
        assert_eq!(mgr.state(), PlaybackState::Playing);
    }

    #[test]
    fn next_while_stopped_only_moves_selection() {
        let mut mgr = setup();
        assert_eq!(mgr.apply(&MediaRequest::Next), MediaAction::None);
        assert_eq!(mgr.current_path(), Some("/music/b.mp3"));
        assert_eq!(mgr.state(), PlaybackState::Stopped);
    }

    #[test]
    fn next_past_the_end_stops_unless_repeating() {
        let mut mgr = setup();
        mgr.playlist.set_current(2);
        mgr.apply(&MediaRequest::Play);
        assert_eq!(mgr.apply(&MediaRequest::Next), MediaAction::Stop);
        assert_eq!(mgr.state(), PlaybackState::Stopped);

        mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
        mgr.apply(&MediaRequest::Play);
        assert_eq!(
            mgr.apply(&MediaRequest::Next),
            MediaAction::PlayPath("/music/a.mp3".into())
        );
    }

    #[test]
    fn repeat_one_replays_the_same_track() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Play);
        mgr.apply(&MediaRequest::Repeat(RepeatMode::One));
        assert_eq!(
            mgr.apply(&MediaRequest::Next),
            MediaAction::PlayPath("/music/a.mp3".into())
        );
    }

    #[test]
    fn volume_is_clamped_and_forwarded() {
        let mut mgr = AudioManager::new();
        assert_eq!(
            mgr.apply(&MediaRequest::Volume(42)),
            MediaAction::SetVolume(42)
        );
        assert_eq!(
            mgr.apply(&MediaRequest::Volume(250)),
            MediaAction::SetVolume(100)
        );
    }

    #[test]
    fn shuffle_toggles() {
        let mut mgr = setup();
        assert_eq!(mgr.apply(&MediaRequest::Shuffle), MediaAction::None);
        assert!(mgr.playlist.shuffle);
        mgr.apply(&MediaRequest::Shuffle);
        assert!(!mgr.playlist.shuffle);
    }

    #[test]
    fn shuffle_visits_every_track() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Shuffle);
        mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
        mgr.apply(&MediaRequest::Play);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3 {
            if let MediaAction::PlayPath(p) = mgr.apply(&MediaRequest::Next) {
                seen.insert(p);
            }
        }
        assert_eq!(seen.len(), 3, "{seen:?}");
    }

    #[test]
    fn format_status_reports_everything() {
        let mut mgr = setup();
        mgr.apply(&MediaRequest::Play);
        let status = mgr.format_status(65, 61_000, 180_000);
        assert!(status.contains("State: playing"), "{status}");
        assert!(status.contains("Volume: 65%"), "{status}");
        assert!(status.contains("Track: a.mp3"), "{status}");
        assert!(status.contains("Path: /music/a.mp3"), "{status}");
        assert!(status.contains("Position: 1:01 / 3:00"), "{status}");
        assert!(status.contains("Repeat: off"), "{status}");
        assert!(status.contains("Shuffle: off"), "{status}");
        assert!(status.contains("Playlist: 3 tracks"), "{status}");

        mgr.apply(&MediaRequest::Stop);
        let status = mgr.format_status(65, 0, 0);
        assert!(status.contains("State: stopped"), "{status}");
        assert!(!status.contains("Position:"), "{status}");
    }

    #[test]
    fn publish_status_writes_only_on_change() {
        let mut mgr = setup();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/audio").unwrap();

        assert!(mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
        assert!(!mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
        mgr.apply(&MediaRequest::Play);
        assert!(mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
        let text = String::from_utf8(vfs.read(AUDIO_STATUS_PATH).unwrap()).unwrap();
        assert!(text.contains("State: playing"));
    }
}

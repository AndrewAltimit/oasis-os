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
//! [`MusicController`](crate::MusicController) carries the actions out
//! (the desktop shell and the WASM backend both drive one); the `music`
//! terminal command and the Music Player app produce the requests.

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
    /// Release the current music track and select this VFS path without
    /// starting it (`next` / `prev` while paused): playback stays paused
    /// and the next `play` / `resume` starts the new track
    /// ([`MediaAction::PlayPath`]).
    Cue(String),
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
    /// Paused on a cued entry ([`MediaAction::Cue`]): nothing is loaded,
    /// so resuming must start the current entry rather than resume the
    /// backend.
    cued: bool,
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
        if state != PlaybackState::Paused {
            self.cued = false;
        }
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
                self.set_state(PlaybackState::Playing);
                MediaAction::PlayPath(path.clone())
            },
            MediaRequest::Play => match self.state {
                PlaybackState::Playing => MediaAction::None,
                PlaybackState::Paused => self.resume(),
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
                self.resume()
            },
            MediaRequest::Stop => {
                self.set_state(PlaybackState::Stopped);
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
                self.playlist.set_shuffle(!self.playlist.shuffle());
                MediaAction::None
            },
        }
    }

    /// The music track ended on its own (the host saw the backend stop
    /// while the model says playing). Advance per repeat / shuffle like
    /// `next`: repeat-one replays the track, repeat-all wraps, and at the
    /// end of a non-repeating playlist playback stops.
    pub fn track_finished(&mut self) -> MediaAction {
        if self.state != PlaybackState::Playing {
            return MediaAction::None;
        }
        if self.playlist.advance() {
            return self.play_current();
        }
        self.set_state(PlaybackState::Stopped);
        MediaAction::Stop
    }

    /// Leave the paused state: start a cued entry, or resume the paused
    /// track.
    fn resume(&mut self) -> MediaAction {
        if self.cued {
            return self.play_current();
        }
        self.set_state(PlaybackState::Playing);
        MediaAction::Resume
    }

    /// Start the current playlist entry.
    fn play_current(&mut self) -> MediaAction {
        match self.current_path().map(str::to_string) {
            Some(path) => {
                self.set_state(PlaybackState::Playing);
                MediaAction::PlayPath(path)
            },
            None => MediaAction::None,
        }
    }

    /// After `next` / `prev`: play the new entry while playing, cue it
    /// while paused (the conventional media-player behaviour: skipping
    /// does not unpause), only move the selection while stopped; at
    /// either end of a non-repeating playlist, stop.
    fn after_skip(&mut self, moved: bool) -> MediaAction {
        if !moved {
            if self.state == PlaybackState::Stopped {
                return MediaAction::None;
            }
            self.set_state(PlaybackState::Stopped);
            return MediaAction::Stop;
        }
        match self.state {
            PlaybackState::Stopped => MediaAction::None,
            PlaybackState::Paused => match self.current_path().map(str::to_string) {
                Some(path) => {
                    self.cued = true;
                    MediaAction::Cue(path)
                },
                None => MediaAction::None,
            },
            PlaybackState::Playing => self.play_current(),
        }
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
            if self.playlist.shuffle() { "on" } else { "off" }
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
mod tests;

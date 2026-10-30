//! Music playback controller: the consumer of the `/var/audio/*` VFS IPC.
//!
//! Requests ([`MediaRequest`] lines) arrive at [`AUDIO_REQUEST_PATH`] from
//! the Music Player app (which lives in a pure-data crate with no backend
//! access: `play_file <path>` when the user opens a track, `stop` when they
//! back out), from the `music` terminal command (`play [file]`, `pause`,
//! `resume`, `stop`, `next`, `prev`, `vol`, `repeat`, `shuffle`) and from
//! the MCP `play_media` tool. [`oasis_audio::AudioManager`] (in
//! `state.media`) keeps the playlist and playback state and says what to do
//! on the `AudioBackend`; this module carries that out.
//!
//! Every tick the controller also publishes:
//! - [`AUDIO_POSITION_PATH`]: the music track's real playback position, for
//!   the Music Player's progress bar;
//! - [`AUDIO_STATUS_PATH`]: the `State/Volume/Track/Position/Repeat/
//!   Shuffle/Playlist` document `music status` prints (rewritten only
//!   when it changes).
//!
//! Format is deliberately line-based and human readable — mirrors the
//! radio_controller pattern.

use oasis_audio::{
    AUDIO_POSITION_PATH, AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, MediaAction, MediaRequest,
    PlaybackState,
};
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::AppState;

/// Poll the audio IPC path, action every pending request, and publish
/// position + status.
///
/// The IPC file is newline-delimited: each request occupies one line,
/// and the controller drains them all per tick, then rewrites the
/// buffer with any lines it did not recognise (so a future controller
/// sharing the path still sees its own payload). This prevents the
/// "rapidly click two tracks in one tick and one of them gets silently
/// dropped" race.
pub fn tick(state: &mut AppState, vfs: &mut MemoryVfs) {
    publish_position(state, vfs);
    for request in drain_requests(vfs) {
        handle(state, vfs, &request);
    }
    publish_status(state, vfs);
}

/// Take every recognised request line out of [`AUDIO_REQUEST_PATH`],
/// leaving unrecognised lines in place.
fn drain_requests(vfs: &mut MemoryVfs) -> Vec<MediaRequest> {
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

/// Apply one request to the playlist model and carry out its action.
fn handle(state: &mut AppState, vfs: &mut MemoryVfs, request: &MediaRequest) {
    match state.media.apply(request) {
        MediaAction::None => {},
        MediaAction::PlayPath(path) => {
            play_file(state, vfs, &path);
            if state.media_track.is_some() {
                write_position(state, vfs, &path);
            } else {
                state.media.set_state(PlaybackState::Stopped);
            }
        },
        MediaAction::Pause => {
            // `AudioBackend::pause` is process-global: leave a concurrently
            // streaming TV Guide alone (see `stop_track`).
            if state.media_track.is_some() && state.tv.audio_track.is_none() {
                if let Err(e) = state.audio_backend.pause() {
                    log::warn!("media_controller: pause failed: {e}");
                }
            } else if state.media_track.is_none() {
                state.media.set_state(PlaybackState::Stopped);
            }
        },
        MediaAction::Resume => {
            if state.media_track.is_some() && state.tv.audio_track.is_none() {
                if let Err(e) = state.audio_backend.resume() {
                    log::warn!("media_controller: resume failed: {e}");
                }
            } else if state.media_track.is_none() {
                state.media.set_state(PlaybackState::Stopped);
            }
        },
        MediaAction::Stop => {
            stop_track(state);
            let _ = vfs.write(AUDIO_POSITION_PATH, b"");
        },
        MediaAction::SetVolume(volume) => {
            if let Err(e) = state.audio_backend.set_volume(volume) {
                log::warn!("media_controller: set_volume({volume}) failed: {e}");
            }
        },
    }
}

/// Publish the `music status` document (written only when it changes).
fn publish_status(state: &AppState, vfs: &mut MemoryVfs) {
    let (pos, dur) = if state.media_track.is_some() {
        (
            state.audio_backend.position_ms(),
            state.audio_backend.duration_ms(),
        )
    } else {
        (0, 0)
    };
    let volume = state.audio_backend.get_volume();
    if let Err(e) = state.media.publish_status(vfs, volume, pos, dur) {
        log::warn!("media_controller: cannot publish {AUDIO_STATUS_PATH}: {e}");
    }
}

/// Position-report granularity: the Music Player's readout is m:ss and
/// its bar a few hundred pixels wide, so 250 ms steps are plenty and
/// avoid rewriting the IPC file every frame.
const POSITION_STEP_MS: u64 = 250;

/// Write `<position_ms> <duration_ms> <path>` to [`AUDIO_POSITION_PATH`]
/// so the Music Player can draw a real progress bar.
fn write_position(state: &AppState, vfs: &mut MemoryVfs, path: &str) {
    let pos = state.audio_backend.position_ms() / POSITION_STEP_MS * POSITION_STEP_MS;
    let line = format!("{pos} {} {path}", state.audio_backend.duration_ms());
    let unchanged = vfs
        .read(AUDIO_POSITION_PATH)
        .is_ok_and(|prev| prev == line.as_bytes());
    if !unchanged {
        let _ = vfs.write(AUDIO_POSITION_PATH, line.as_bytes());
    }
}

/// Refresh the position report for the track that is playing (its path
/// is carried in the report written by `play_file`).
fn publish_position(state: &AppState, vfs: &mut MemoryVfs) {
    if state.media_track.is_none() {
        return;
    }
    let Ok(prev) = vfs.read(AUDIO_POSITION_PATH) else {
        return;
    };
    let prev = String::from_utf8_lossy(&prev).into_owned();
    if let Some(path) = prev.trim().splitn(3, ' ').nth(2) {
        write_position(state, vfs, path);
    }
}

fn play_file(state: &mut AppState, vfs: &MemoryVfs, path: &str) {
    // Unload any previous track so memory doesn't grow across songs.
    stop_track(state);
    // If Internet Radio is currently streaming, tear it down so its UI
    // reflects reality after music takes over. `RadioManager::stop`
    // internally calls `AudioBackend::stop()` which is process-global
    // (it clears the shared SDL audio stream), so guard it with
    // `state.tv.audio_track.is_none()` — otherwise TV Guide video
    // audio would be silenced mid-playback in windowed multi-window
    // mode while its state machine stays oblivious. If TV Guide is the
    // active audio consumer, we skip the radio teardown entirely;
    // radio's RadioState stays `Playing` until the user explicitly
    // stops it, but the subsequent `play(music_track)` swaps
    // `current_track` away from radio so no audio interleaves.
    if state.radio.manager.state() != oasis_audio::radio::RadioState::Stopped
        && state.tv.audio_track.is_none()
    {
        let _ = state
            .radio
            .manager
            .process_request("stop", state.audio_backend.as_mut());
        state.radio.archive_catalog = None;
        state.radio.pending_catalog_fetch = None;
        state.radio.pending_source_fetch = None;
        if let Some(mut src) = state.radio.source.take() {
            src.disconnect();
        }
    }

    let data = match vfs.read(path) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("media_controller: cannot read {path} from VFS: {e}");
            return;
        },
    };

    match state.audio_backend.load_track(&data) {
        Ok(track) => {
            if let Err(e) = state.audio_backend.play(track) {
                log::warn!("media_controller: play failed for {path}: {e}");
                return;
            }
            state.media_track = Some(track);
            log::info!("media_controller: playing {path} ({} bytes)", data.len());
        },
        Err(e) => {
            log::warn!("media_controller: load_track failed for {path}: {e}");
        },
    }
}

fn stop_track(state: &mut AppState) {
    if let Some(track) = state.media_track.take() {
        // `AudioBackend::stop()` is process-global: it clears the shared
        // audio stream buffer. If TV Guide is concurrently streaming
        // (windowed multi-window mode), calling it here would silence
        // the video's audio mid-playback while leaving its state machine
        // thinking it's still live. `unload_track` already issues an
        // internal `stop()` when the music track is the current one, so
        // we only need the explicit call when no other audio consumer
        // is active.
        if state.tv.audio_track.is_none() {
            let _ = state.audio_backend.stop();
        }
        let _ = state.audio_backend.unload_track(track);
    }
}

/// Unload the music track when the Music Player window/app closes.
/// Invoked from the input dispatcher when the closing runner is the
/// Music Player — the app's own Cancel handler emits a `stop` IPC
/// but the window-manager close button bypasses that path.
pub fn shutdown(state: &mut AppState) {
    stop_track(state);
    state.media.set_state(PlaybackState::Stopped);
}

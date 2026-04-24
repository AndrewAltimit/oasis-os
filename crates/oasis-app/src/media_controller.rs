//! Music Player audio controller.
//!
//! Bridges the Music Player app (which lives in a pure-data crate with no
//! backend access) to the `AudioBackend` by watching for VFS IPC requests
//! at [`MEDIA_REQUEST_PATH`]. The app writes `play_file <path>` when the
//! user opens a track and `stop` when they back out of the view.
//!
//! Format is deliberately line-based and human readable — mirrors the
//! radio_controller pattern.

use oasis_app_media::MEDIA_REQUEST_PATH;
use oasis_core::backend::AudioBackend;
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::AppState;

/// Poll the Music Player IPC path and action every pending request.
///
/// The IPC file is newline-delimited: each request occupies one line,
/// and the controller drains them all per tick, then rewrites the
/// buffer with any lines it did not recognise (so a future controller
/// sharing the path — e.g. a radio consumer — still sees its own
/// payload). This prevents the "rapidly click two tracks in one tick
/// and one of them gets silently dropped" race.
pub fn tick(state: &mut AppState, vfs: &mut MemoryVfs) {
    let Ok(data) = vfs.read(MEDIA_REQUEST_PATH) else {
        return;
    };
    if data.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(&data).into_owned();

    let mut unhandled: Vec<&str> = Vec::new();
    let mut touched = false;
    // Collect owned copies of consumed lines first so the borrow on
    // `text` (from the `split` iterator) ends before we call
    // `play_file`, which needs `vfs` mutably.
    let mut to_process: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("play_file ") || trimmed == "stop" {
            to_process.push(trimmed.to_string());
            touched = true;
        } else {
            unhandled.push(line);
        }
    }

    if !touched {
        return;
    }
    let remainder = unhandled.join("\n");
    let _ = vfs.write(MEDIA_REQUEST_PATH, remainder.as_bytes());

    for req in to_process {
        if let Some(path) = req.strip_prefix("play_file ") {
            play_file(state, vfs, path);
        } else if req == "stop" {
            stop_track(state);
        }
    }
}

fn play_file(state: &mut AppState, vfs: &MemoryVfs, path: &str) {
    // Unload any previous track so memory doesn't grow across songs.
    stop_track(state);
    // If Internet Radio is currently streaming, tear it down so its UI
    // reflects reality after music takes over. `RadioManager::stop`
    // internally calls `AudioBackend::stop()` which is process-global
    // (it clears the shared SDL audio stream), so guard it with
    // `state.tv_audio_track.is_none()` — otherwise TV Guide video
    // audio would be silenced mid-playback in windowed multi-window
    // mode while its state machine stays oblivious. If TV Guide is the
    // active audio consumer, we skip the radio teardown entirely;
    // radio's RadioState stays `Playing` until the user explicitly
    // stops it, but the subsequent `play(music_track)` swaps
    // `current_track` away from radio so no audio interleaves.
    if state.radio_manager.state() != oasis_audio::radio::RadioState::Stopped
        && state.tv_audio_track.is_none()
    {
        let _ = state
            .radio_manager
            .process_request("stop", &mut state.audio_backend);
        state.archive_catalog = None;
        state.pending_catalog_fetch = None;
        state.pending_source_fetch = None;
        if let Some(mut src) = state.radio_source.take() {
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
        if state.tv_audio_track.is_none() {
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
}

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

/// Poll the Music Player IPC path and action any request.
pub fn tick(state: &mut AppState, vfs: &mut MemoryVfs) {
    let Ok(data) = vfs.read(MEDIA_REQUEST_PATH) else {
        return;
    };
    if data.is_empty() {
        return;
    }
    let request = String::from_utf8_lossy(&data).trim().to_string();
    // Only consume lines we recognize; leave unknown payloads for other
    // controllers that share the path.
    if !(request.starts_with("play_file ") || request == "stop") {
        return;
    }
    let _ = vfs.write(MEDIA_REQUEST_PATH, b"");

    if let Some(path) = request.strip_prefix("play_file ") {
        play_file(state, vfs, path);
    } else if request == "stop" {
        stop_track(state);
    }
}

fn play_file(state: &mut AppState, vfs: &MemoryVfs, path: &str) {
    // Unload any previous track so memory doesn't grow across songs.
    stop_track(state);
    // The `AudioBackend::stop` below is process-global; if the Internet
    // Radio is currently streaming, starting a music track would silence
    // it without updating the radio state machine. Tear the radio down
    // explicitly so its UI reflects reality.
    if state.radio_manager.state() != oasis_audio::radio::RadioState::Stopped {
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

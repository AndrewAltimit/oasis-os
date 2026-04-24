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
        let _ = state.audio_backend.stop();
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

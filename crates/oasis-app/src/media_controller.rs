//! Music playback controller: the desktop shell's consumer of the
//! `/var/audio/*` VFS IPC.
//!
//! The request handling, position and status publishing live in
//! [`oasis_audio::MusicController`] (shared with the WASM backend); this
//! module lends it the shell's music state (`state.media`,
//! `state.media_track`) and SDL audio backend, and supplies the two
//! desktop-specific rules:
//!
//! - TV Guide video audio streams through the same process-global SDL audio
//!   stream, so while it is live the controller must not pause / resume /
//!   stop the backend (`stream_shared`).
//! - Starting a track tears down internet radio first (unless TV Guide owns
//!   the stream: `RadioManager::stop` calls the process-global
//!   `AudioBackend::stop()`, which would silence the video mid-playback).

use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, RadioLayer};

/// Poll the audio IPC path, action every pending request, and publish
/// position + status.
pub fn tick(state: &mut AppState, vfs: &mut MemoryVfs) {
    let stream_shared = state.tv.audio_track.is_some();
    let AppState {
        media,
        media_track,
        audio_backend,
        radio,
        ..
    } = state;
    let mut stop_radio = |audio: &mut dyn oasis_core::backend::AudioBackend| {
        if !stream_shared {
            stop_radio_for_music(radio, audio);
        }
    };
    oasis_audio::MusicController {
        manager: media,
        track: media_track,
        audio: audio_backend.as_mut(),
        stream_shared,
    }
    .tick(vfs, &mut stop_radio);
}

/// Append one request line to the audio request path. Producers in the
/// shell (app requests, MCP `play_media`) must append like the `music`
/// command does: a plain write would drop lines queued earlier in the
/// same frame.
pub(crate) fn queue_request(vfs: &mut MemoryVfs, line: &str) -> oasis_core::error::Result<()> {
    oasis_audio::queue_request(vfs, line)
}

/// If Internet Radio is streaming, tear it down so its UI reflects reality
/// once music takes over.
fn stop_radio_for_music(radio: &mut RadioLayer, audio: &mut dyn oasis_core::backend::AudioBackend) {
    if radio.manager.state() == oasis_audio::radio::RadioState::Stopped {
        return;
    }
    let _ = radio.manager.process_request("stop", audio);
    radio.archive_catalog = None;
    radio.pending_catalog_fetch = None;
    radio.pending_source_fetch = None;
    if let Some(mut src) = radio.source.take() {
        src.disconnect();
    }
}

/// Unload the music track when the Music Player window/app closes.
/// Invoked from the window-close path when the closing runner is the
/// Music Player — the app's own Cancel handler emits a `stop` IPC but the
/// window-manager close button bypasses that path.
pub fn shutdown(state: &mut AppState) {
    oasis_audio::MusicController {
        manager: &mut state.media,
        track: &mut state.media_track,
        audio: state.audio_backend.as_mut(),
        stream_shared: state.tv.audio_track.is_some(),
    }
    .shutdown();
}

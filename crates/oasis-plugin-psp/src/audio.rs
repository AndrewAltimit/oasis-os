//! Background MP3 playback -- STUBBED OUT.
//!
//! Audio imports (sceMp3*, sceAudio*) cause PRX load failure because
//! those modules aren't loaded in the game's kernel context. All public
//! functions are no-ops until we implement dynamic module loading.

use crate::overlay;

/// Get the current track's display name (stub).
pub fn current_track_name() -> &'static [u8] {
    b"\0"
}

/// Toggle play/pause (stub).
pub fn toggle_playback() {
    overlay::show_osd(b"Audio: not available");
}

/// Skip to next track (stub).
pub fn next_track() {
    overlay::show_osd(b"Audio: not available");
}

/// Skip to previous track (stub).
pub fn prev_track() {
    overlay::show_osd(b"Audio: not available");
}

/// Increase volume (stub).
pub fn volume_up() {
    overlay::show_osd(b"Audio: not available");
}

/// Decrease volume (stub).
pub fn volume_down() {
    overlay::show_osd(b"Audio: not available");
}

/// Start the background audio thread (stub).
pub fn start_audio_thread() {
    // No-op: sceMp3/sceAudio imports removed to prevent PRX load failure.
}

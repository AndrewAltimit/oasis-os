//! Skin-themed UI sound playback glue.
//!
//! Bridges the three pieces of the UI sound system:
//! - the skin's `[sounds]` table + WAV assets (`oasis-skin`),
//! - the polyphonic one-shot mixer (`oasis_audio::sfx::SfxPlayer`),
//! - the SDL backend's dedicated SFX stream (`SdlAudioBackend::queue_sfx`).
//!
//! `reload_for_skin` runs at boot and on every skin swap: it drops the old
//! skin's samples and decodes the new skin's WAVs (mirroring how image
//! assets are re-uploaded). `tick` runs once per frame: it derives toast
//! sounds from the toast counters, drains the event queue into voices, and
//! pumps a small backlog of mixed PCM into the SFX stream.

use oasis_core::ui_sound::UiSound;

use crate::app_state::AppState;

/// Target backlog of mixed SFX in the SDL stream: ~100 ms at 48 kHz
/// stereo i16. Small enough that triggers feel immediate, large enough
/// that a preempted main thread doesn't underrun mid-sample.
const TARGET_QUEUE_BYTES: u32 = 19_200;

/// Load the active skin's `[sounds]` samples into the SFX player,
/// replacing whatever the previous skin loaded. A skin without a
/// `[sounds]` table leaves the player empty — fully silent, today's
/// default behavior.
pub fn reload_for_skin(state: &mut AppState) {
    state.sfx.clear();
    let Some(ref sounds) = state.skin.theme.sounds else {
        return;
    };
    state.sfx.set_master_volume(sounds.effective_volume());
    for sound in UiSound::ALL {
        let Some(path) = sounds.path_for(sound.key()) else {
            continue;
        };
        match state.skin.sound_assets.get(path) {
            Some(bytes) => {
                if !state.sfx.load_wav(sound.key(), bytes) {
                    log::warn!(
                        "skin '{}': sound '{}' asset '{path}' failed to decode",
                        state.skin.manifest.name,
                        sound.key(),
                    );
                }
            },
            None => {
                log::warn!(
                    "skin '{}': sound '{}' references missing asset '{path}'",
                    state.skin.manifest.name,
                    sound.key(),
                );
            },
        }
    }
    if !state.sfx.is_empty() {
        log::info!(
            "skin '{}': loaded {} UI sound(s)",
            state.skin.manifest.name,
            state.sfx.len(),
        );
    }
}

/// Per-frame UI sound pump: derive toast sounds, trigger queued events,
/// and top the SFX stream up with freshly mixed PCM. Cheap no-op when the
/// skin ships no sounds.
pub fn tick(state: &mut AppState) {
    // Toast/Error sounds come from the toast counters so every show()
    // call site is covered without instrumentation.
    let (shown, errors) = state.toasts.shown_counts();
    state.ui_sounds.observe_toasts(shown, errors);

    let events = state.ui_sounds.drain();
    if state.sfx.is_empty() {
        return; // Silent skin: drop events, skip the pump entirely.
    }
    for sound in events {
        state.sfx.play(sound.key());
    }

    if !state.sfx.has_active_voices() {
        return;
    }
    let queued = state.audio_backend.sfx_queued_bytes();
    if queued >= TARGET_QUEUE_BYTES {
        return;
    }
    // 4 bytes per stereo i16 frame.
    let deficit_frames = ((TARGET_QUEUE_BYTES - queued) as usize) / 4;
    let mut chunk = Vec::new();
    if state.sfx.render(deficit_frames, &mut chunk)
        && let Err(e) = state.audio_backend.queue_sfx(&chunk)
    {
        log::debug!("SFX queue failed: {e}");
    }
}

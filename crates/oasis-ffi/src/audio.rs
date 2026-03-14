//! Audio FFI functions.

use oasis_core::backend::AudioBackend;

use crate::handle::{OasisInstance, with_instance, with_instance_ref};
use crate::types::OasisAudioCallback;

/// Register an audio event callback.
///
/// The callback fires on play, pause, stop, volume changes, and track load/unload.
/// This lets the UE5 host handle actual audio output.
///
/// # Safety
///
/// `handle` must be valid. `cb` must be a valid function pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_set_audio_callback(
    handle: *mut OasisInstance,
    cb: OasisAudioCallback,
) {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            instance.audio.set_callback(cb);
        });
    }
}

/// Load audio data and return a track ID. Returns `u64::MAX` on failure.
///
/// # Safety
///
/// `handle` must be valid. `data` must point to `data_len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_load(
    handle: *mut OasisInstance,
    data: *const u8,
    data_len: u32,
) -> u64 {
    if data.is_null() || data_len == 0 {
        return u64::MAX;
    }
    // SAFETY: Caller guarantees `data` is valid for `data_len` bytes; null check above.
    let slice = unsafe { std::slice::from_raw_parts(data, data_len as usize) };

    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, u64::MAX, |instance| {
            match instance.audio.load_track(slice) {
                Ok(id) => id.0,
                Err(_) => u64::MAX,
            }
        })
    }
}

/// Start playing a loaded audio track. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_play(handle: *mut OasisInstance, track_id: u64) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, false, |instance| {
            instance
                .audio
                .play(oasis_core::backend::AudioTrackId(track_id))
                .is_ok()
        })
    }
}

/// Pause audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_pause(handle: *mut OasisInstance) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance(handle, false, |instance| instance.audio.pause().is_ok()) }
}

/// Resume audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_resume(handle: *mut OasisInstance) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance(handle, false, |instance| instance.audio.resume().is_ok()) }
}

/// Stop audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_stop(handle: *mut OasisInstance) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance(handle, false, |instance| instance.audio.stop().is_ok()) }
}

/// Set audio volume (0-100). Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_set_volume(handle: *mut OasisInstance, volume: u8) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, false, |instance| {
            instance.audio.set_volume(volume).is_ok()
        })
    }
}

/// Get the current audio volume (0-100).
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_get_volume(handle: *mut OasisInstance) -> u8 {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance_ref(handle, 0, |instance| instance.audio.get_volume()) }
}

/// Check if audio is currently playing.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_is_playing(handle: *mut OasisInstance) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance_ref(handle, false, |instance| instance.audio.is_playing()) }
}

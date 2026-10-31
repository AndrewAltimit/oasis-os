//! Audio FFI functions.

use oasis_core::backend::AudioBackend;

use crate::handle::{OasisInstance, ffi_guard, with_instance, with_instance_ref};
use crate::types::{OasisAudioCallback, OasisAudioCallbackEx};

/// Register an audio event callback.
///
/// The callback fires on play, pause, stop, volume changes, and track load/unload.
/// This lets the UE5 host handle actual audio output. Null unregisters it.
/// It runs synchronously inside the `oasis_audio_*` / `oasis_destroy` call
/// that fired it and must not call back into the API with the same handle.
///
/// # Safety
///
/// `handle` must be null or valid. `cb` must be null or a valid function
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_set_audio_callback(
    handle: *mut OasisInstance,
    cb: Option<OasisAudioCallback>,
) {
    ffi_guard("oasis_set_audio_callback", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| match cb {
                Some(cb) => instance.audio.set_callback(cb),
                None => instance.audio.clear_callback(),
            });
        }
    })
}

/// [`oasis_set_audio_callback`] for a callback that also receives
/// `user_data` (any host pointer, e.g. the audio component; passed back
/// unchanged and never dereferenced). Replaces a callback set with either
/// export; null `cb` unregisters.
///
/// # Safety
///
/// `handle` must be null or valid. `cb` must be null or a valid function
/// pointer. `user_data` must stay valid for as long as `cb` may use it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_set_audio_callback_ex(
    handle: *mut OasisInstance,
    cb: Option<OasisAudioCallbackEx>,
    user_data: *mut std::ffi::c_void,
) {
    ffi_guard("oasis_set_audio_callback_ex", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| match cb {
                Some(cb) => instance.audio.set_callback_ex(cb, user_data),
                None => instance.audio.clear_callback(),
            });
        }
    })
}

/// Pull streamed (internet radio) audio bytes into `buf`.
///
/// Streaming tracks carry encoded bytes (e.g. MP3) the host never loaded
/// itself. They are buffered (up to 1 MiB) and announced through the audio
/// callback: `StreamOpened` (8), `StreamData` (9, `value` = bytes waiting)
/// and `StreamEnd` (10). Call this after those events -- outside the
/// callback, which must not re-enter the API -- to move up to `buf_len`
/// bytes into `buf`. Returns the number of bytes written (0 when nothing
/// is buffered, or for a null handle / buffer).
///
/// # Safety
///
/// `handle` must be null or valid. `buf` must be null or valid for
/// `buf_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_drain_stream(
    handle: *mut OasisInstance,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    ffi_guard("oasis_audio_drain_stream", 0, || {
        if buf.is_null() || buf_len == 0 {
            return 0;
        }
        // SAFETY: Caller guarantees `buf` is valid for `buf_len` writable
        // bytes; null was checked above.
        let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len as usize) };
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, 0, |instance| {
                // `n <= buf_len`, so it fits a u32.
                instance.audio.drain_stream_into(out) as u32
            })
        }
    })
}

/// Bytes of streamed audio waiting for [`oasis_audio_drain_stream`].
///
/// # Safety
///
/// `handle` must be null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_stream_buffered(handle: *mut OasisInstance) -> u32 {
    ffi_guard("oasis_audio_stream_buffered", 0, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance_ref(handle, 0, |instance| {
                u32::try_from(instance.audio.stream_buffered()).unwrap_or(u32::MAX)
            })
        }
    })
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
    ffi_guard("oasis_audio_load", u64::MAX, || {
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
    })
}

/// Start playing a loaded audio track. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_play(handle: *mut OasisInstance, track_id: u64) -> bool {
    ffi_guard("oasis_audio_play", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, false, |instance| {
                instance
                    .audio
                    .play(oasis_core::backend::AudioTrackId(track_id))
                    .is_ok()
            })
        }
    })
}

/// Pause audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_pause(handle: *mut OasisInstance) -> bool {
    ffi_guard("oasis_audio_pause", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance(handle, false, |instance| instance.audio.pause().is_ok()) }
    })
}

/// Resume audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_resume(handle: *mut OasisInstance) -> bool {
    ffi_guard("oasis_audio_resume", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance(handle, false, |instance| instance.audio.resume().is_ok()) }
    })
}

/// Stop audio playback. Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_stop(handle: *mut OasisInstance) -> bool {
    ffi_guard("oasis_audio_stop", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance(handle, false, |instance| instance.audio.stop().is_ok()) }
    })
}

/// Set audio volume (0-100). Returns true on success.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_set_volume(handle: *mut OasisInstance, volume: u8) -> bool {
    ffi_guard("oasis_audio_set_volume", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, false, |instance| {
                instance.audio.set_volume(volume).is_ok()
            })
        }
    })
}

/// Get the current audio volume (0-100).
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_get_volume(handle: *mut OasisInstance) -> u8 {
    ffi_guard("oasis_audio_get_volume", 0, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance_ref(handle, 0, |instance| instance.audio.get_volume()) }
    })
}

/// Check if audio is currently playing.
///
/// # Safety
///
/// `handle` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_audio_is_playing(handle: *mut OasisInstance) -> bool {
    ffi_guard("oasis_audio_is_playing", false, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance_ref(handle, false, |instance| instance.audio.is_playing()) }
    })
}

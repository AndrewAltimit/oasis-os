//! Callback registration: `oasis_register_callback` and
//! `oasis_register_callback_ex` (the same, plus a host context pointer).

use std::ffi::c_void;

use crate::handle::{OasisInstance, ffi_guard, with_instance};
use crate::types::{OasisCallback, OasisCallbackEx, RegisteredCallback, is_callback_event};

/// Register a callback for OS events.
///
/// `event` is one of the `OASIS_CB_*` constants; any other code (including
/// the retired 1/4/5/6) is ignored with a warning, since it would never fire.
/// `cb` is the function to call when the event fires; null unregisters the
/// event. Registering the same event again (with this or
/// `oasis_register_callback_ex`) replaces the previous callback.
///
/// The callback runs synchronously inside the `oasis_*` call that fired it
/// and must not call back into the API with the same handle; `detail` is
/// only valid while it runs.
///
/// # Safety
///
/// `handle` must be null or valid. `cb` must be null or a valid function
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_register_callback(
    handle: *mut OasisInstance,
    event: u32,
    cb: Option<OasisCallback>,
) {
    ffi_guard("oasis_register_callback", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { register(handle, event, cb.map(RegisteredCallback::Plain)) }
    })
}

/// [`oasis_register_callback`] for a callback that also receives
/// `user_data` -- any pointer the host likes (e.g. the owning UObject),
/// passed back unchanged on every call and never dereferenced by the
/// library. Null `cb` unregisters the event.
///
/// # Safety
///
/// `handle` must be null or valid. `cb` must be null or a valid function
/// pointer. `user_data` must stay valid for as long as `cb` may use it
/// (until the event is re-registered or unregistered, or the handle is
/// destroyed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_register_callback_ex(
    handle: *mut OasisInstance,
    event: u32,
    cb: Option<OasisCallbackEx>,
    user_data: *mut c_void,
) {
    ffi_guard("oasis_register_callback_ex", (), || {
        let cb = cb.map(|cb| RegisteredCallback::WithData(cb, user_data.expose_provenance()));
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { register(handle, event, cb) }
    })
}

/// Shared body of the two registration exports.
///
/// # Safety
///
/// `handle` must be null or valid.
unsafe fn register(handle: *mut OasisInstance, event: u32, cb: Option<RegisteredCallback>) {
    // SAFETY: Caller guarantees `handle` is null or valid.
    unsafe {
        with_instance(handle, (), |instance| {
            if !is_callback_event(event) {
                log::warn!("oasis_register_callback: unknown event code {event} ignored");
            } else if let Some(cb) = cb {
                instance.callbacks.insert(event, cb);
            } else {
                instance.callbacks.remove(&event);
            }
        });
    }
}

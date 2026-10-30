//! Callback registration: `oasis_register_callback`.

use crate::handle::{OasisInstance, ffi_guard, with_instance};
use crate::types::{OasisCallback, is_callback_event};

/// Register a callback for OS events.
///
/// `event` is one of the `OASIS_CB_*` constants; any other code (including
/// the retired 1/4/5/6) is ignored with a warning, since it would never fire.
/// `cb` is the function to call when the event fires. Registering the same
/// event again replaces the previous callback.
///
/// # Safety
///
/// `handle` must be valid. `cb` must be a valid function pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_register_callback(
    handle: *mut OasisInstance,
    event: u32,
    cb: OasisCallback,
) {
    ffi_guard("oasis_register_callback", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                if is_callback_event(event) {
                    instance.callbacks.insert(event, cb);
                } else {
                    log::warn!("oasis_register_callback: unknown event code {event} ignored");
                }
            });
        }
    })
}

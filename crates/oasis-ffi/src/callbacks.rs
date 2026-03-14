//! Callback registration: `oasis_register_callback`.

use crate::handle::{OasisInstance, with_instance};
use crate::types::OasisCallback;

/// Register a callback for OS events.
///
/// `event` is one of the `OASIS_CB_*` constants.
/// `cb` is the function to call when the event fires.
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
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            instance.callbacks.insert(event, cb);
        });
    }
}

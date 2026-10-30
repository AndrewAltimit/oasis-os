//! VFS operations: `oasis_set_vfs_root` and `oasis_add_vfs_file`.

use std::os::raw::c_char;

use crate::handle::{OasisInstance, c_str_to_str, ffi_guard, with_instance};

/// Reset the VFS to a fresh one: empty `/home`, `/etc` and `/tmp`, the
/// dashboard's `/apps` entries re-seeded, and the working directory back
/// at `/`. `path` is reserved and currently ignored. Repopulate with
/// [`oasis_add_vfs_file`] (or terminal commands) afterwards.
///
/// # Safety
///
/// `handle` must be null or valid. `path` must be a valid C string or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_set_vfs_root(handle: *mut OasisInstance, _path: *const c_char) {
    ffi_guard("oasis_set_vfs_root", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                instance.vfs = crate::lifecycle::fresh_vfs();
                instance.cwd = "/".to_string();
            });
        }
    })
}

/// Add a file to the instance's game asset VFS base layer.
///
/// Useful for pre-populating the VFS from the host application before
/// the first tick. The file is read-only from the terminal's perspective
/// (writes create overlay entries). `data` may be null only when
/// `data_len` is 0 (an empty file); the bytes are copied.
///
/// # Safety
///
/// `handle` must be null or valid. `path` must be null or a valid C
/// string; `data` must be valid for `data_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_add_vfs_file(
    handle: *mut OasisInstance,
    path: *const c_char,
    data: *const u8,
    data_len: u32,
) {
    ffi_guard("oasis_add_vfs_file", (), || {
        // SAFETY: Caller guarantees pointer is null or a valid C string per function safety contract.
        let Some(path_str) = (unsafe { c_str_to_str(path) }) else {
            return;
        };
        let slice: &[u8] = if data_len == 0 {
            &[]
        } else if data.is_null() {
            log::warn!("oasis_add_vfs_file: null data with length {data_len} for {path_str}");
            return;
        } else {
            // SAFETY: Caller guarantees `data` is valid for `data_len` bytes; non-null
            // checked above.
            unsafe { std::slice::from_raw_parts(data, data_len as usize) }
        };

        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                instance.vfs.add_base_file(path_str, slice);
            });
        }
    })
}

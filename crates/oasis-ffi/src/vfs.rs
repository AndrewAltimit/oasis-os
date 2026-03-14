//! VFS operations: `oasis_set_vfs_root` and `oasis_add_vfs_file`.

use std::os::raw::c_char;

use oasis_core::vfs::GameAssetVfs;

use crate::handle::{OasisInstance, c_str_to_str, with_instance};

/// Change the VFS root by populating the game asset VFS with content.
///
/// `path` is a virtual path prefix. Files should be added via
/// `oasis_send_command("write ...")` or the VFS will be pre-populated
/// by the host application before ticking.
///
/// This resets the current working directory to "/".
///
/// # Safety
///
/// `handle` must be valid. `path` must be a valid C string or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_set_vfs_root(handle: *mut OasisInstance, _path: *const c_char) {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            // Reset to clean VFS state.
            instance.vfs = GameAssetVfs::new();
            instance.vfs.add_base_dir("/home");
            instance.vfs.add_base_dir("/etc");
            instance.vfs.add_base_dir("/tmp");
            instance.cwd = "/".to_string();
        });
    }
}

/// Add a file to the instance's game asset VFS base layer.
///
/// Useful for pre-populating the VFS from the host application before
/// the first tick. The file is read-only from the terminal's perspective
/// (writes create overlay entries).
///
/// # Safety
///
/// `handle` must be valid. `path` and `data` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_add_vfs_file(
    handle: *mut OasisInstance,
    path: *const c_char,
    data: *const u8,
    data_len: u32,
) {
    // SAFETY: Caller guarantees pointer is null or a valid C string per function safety contract.
    let Some(path_str) = (unsafe { c_str_to_str(path) }) else {
        return;
    };
    if data.is_null() {
        return;
    }
    // SAFETY: Caller guarantees `data` is valid for `data_len` bytes; null check above.
    let slice = unsafe { std::slice::from_raw_parts(data, data_len as usize) };

    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            instance.vfs.add_base_file(path_str, slice);
        });
    }
}

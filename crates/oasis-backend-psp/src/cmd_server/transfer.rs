//! File transfer commands: `deploy`, `upload`, `log` / `logfull`.

use core::ffi::c_void;

use psp::sys::{IoOpenFlags, IoWhence, SceUid};

use super::log_msg;
use super::wire::{crc32_update, recv, send_response};

/// Largest accepted `deploy` / `upload` payload.
pub(super) const MAX_TRANSFER_BYTES: u32 = 24_000_000;

const EBOOT_TEMP_PATH: &[u8] = b"ms0:/PSP/GAME/OASISOS/EBOOT.PBP.tmp\0";
const EBOOT_PATH: &[u8] = b"ms0:/PSP/GAME/OASISOS/EBOOT.PBP\0";
const LOG_PATH: &[u8] = b"ms0:/PSP/GAME/OASISOS/eboot.log\0";

/// Open `path` (NUL-terminated) for writing, truncating it.
fn create(path: &[u8]) -> Option<SceUid> {
    debug_assert_eq!(path.last(), Some(&0));
    // SAFETY: `path` is a NUL-terminated byte string that outlives the call.
    let fd = unsafe {
        psp::sys::sceIoOpen(
            path.as_ptr(),
            IoOpenFlags::CREAT | IoOpenFlags::WR_ONLY | IoOpenFlags::TRUNC,
            0o777,
        )
    };
    (fd >= SceUid(0)).then_some(fd)
}

/// Write all of `data` to `fd`. Returns false on a short or failed write.
fn write_all(fd: SceUid, data: &[u8]) -> bool {
    // SAFETY: `data` is a live slice and the call only reads its bytes.
    let n = unsafe { psp::sys::sceIoWrite(fd, data.as_ptr() as *const c_void, data.len()) };
    n >= 0 && n as usize == data.len()
}

fn io_close(fd: SceUid) {
    // SAFETY: `fd` came from `sceIoOpen` and is closed exactly once.
    unsafe { psp::sys::sceIoClose(fd) };
}

fn io_remove(path: &[u8]) {
    // SAFETY: `path` is a NUL-terminated byte string.
    unsafe { psp::sys::sceIoRemove(path.as_ptr()) };
}

/// Stream `size` bytes from `cfd` into `fd`, starting with `leftover`
/// (the part of the payload that arrived with the command line).
///
/// Returns the CRC32 of the payload, or `None` if the transfer was cut
/// short or the memory stick write failed.
fn recv_to_file(cfd: i32, fd: SceUid, size: u32, leftover: &[u8]) -> Option<u32> {
    let size = size as usize;
    // Never write more than the announced size, even if the client sent
    // extra bytes along with the command line.
    let leftover = &leftover[..leftover.len().min(size)];
    if !write_all(fd, leftover) {
        return None;
    }
    let mut crc = crc32_update(0, leftover);
    let mut received = leftover.len();

    let mut buf = [0u8; 4096];
    while received < size {
        let want = buf.len().min(size - received);
        let n = recv(cfd, &mut buf[..want])?;
        let chunk = &buf[..n];
        if !write_all(fd, chunk) {
            return None;
        }
        crc = crc32_update(crc, chunk);
        received += n;
    }
    Some(crc)
}

/// `upload <size> <path>`: receive an arbitrary file and write it to
/// `path` on the memory stick.
pub(super) fn receive_file(cfd: i32, size: u32, path: &[u8], leftover: &[u8]) {
    let mut path_z = Vec::with_capacity(path.len() + 1);
    path_z.extend_from_slice(path);
    path_z.push(0);

    log_msg(&format!(
        "[CMD] upload: {} bytes -> {}",
        size,
        core::str::from_utf8(path).unwrap_or("?")
    ));

    let Some(fd) = create(&path_z) else {
        send_response(cfd, b"err: can't create file\n");
        return;
    };
    let ok = recv_to_file(cfd, fd, size, leftover).is_some();
    io_close(fd);

    if ok {
        log_msg("[CMD] upload: OK");
        send_response(cfd, b"ok\n");
    } else {
        log_msg("[CMD] upload: incomplete");
        send_response(cfd, b"err: incomplete transfer\n");
    }
}

/// `deploy <size> [crc32]`: receive a new EBOOT.
///
/// Writes to a temp file first and renames it over the live EBOOT only
/// once the whole payload (and, if given, its CRC32) checks out, so an
/// interrupted transfer never leaves a truncated EBOOT behind.
pub(super) fn receive_deploy(cfd: i32, size: u32, leftover: &[u8], expected_crc: Option<u32>) {
    log_msg("[CMD] deploy: receiving EBOOT");

    let Some(fd) = create(EBOOT_TEMP_PATH) else {
        send_response(cfd, b"err: can't create temp file\n");
        return;
    };
    let crc = recv_to_file(cfd, fd, size, leftover);
    io_close(fd);

    let Some(crc) = crc else {
        io_remove(EBOOT_TEMP_PATH);
        log_msg("[CMD] deploy: incomplete");
        send_response(cfd, b"err: incomplete transfer\n");
        return;
    };
    if let Some(expected) = expected_crc
        && crc != expected
    {
        io_remove(EBOOT_TEMP_PATH);
        log_msg(&format!(
            "[CMD] deploy: CRC mismatch (got {crc:08x}, expected {expected:08x})"
        ));
        send_response(cfd, b"err: crc mismatch\n");
        return;
    }

    // Rename temp -> final (atomic-ish on FAT).
    io_remove(EBOOT_PATH);
    // SAFETY: both paths are NUL-terminated byte strings.
    unsafe { psp::sys::sceIoRename(EBOOT_TEMP_PATH.as_ptr(), EBOOT_PATH.as_ptr()) };
    log_msg("[CMD] deploy: OK");
    send_response(cfd, format!("ok {crc:08x}\n").as_bytes());
}

/// `log` / `logfull`: send the last `max_bytes` of `eboot.log`.
pub(super) fn send_log(cfd: i32, max_bytes: usize) {
    // SAFETY: LOG_PATH is a NUL-terminated byte string.
    let fd = unsafe { psp::sys::sceIoOpen(LOG_PATH.as_ptr(), IoOpenFlags::RD_ONLY, 0) };
    if fd < SceUid(0) {
        send_response(cfd, b"err: no log\n");
        return;
    }

    // SAFETY: `fd` is an open file; seeking has no memory effects.
    unsafe {
        let size = psp::sys::sceIoLseek(fd, 0, IoWhence::End);
        let offset = (size - max_bytes as i64).max(0);
        psp::sys::sceIoLseek(fd, offset, IoWhence::Set);
    }

    // Read and send in chunks (stack-friendly).
    let mut buf = [0u8; 2048];
    let mut remaining = max_bytes;
    while remaining > 0 {
        let to_read = remaining.min(buf.len());
        // SAFETY: `buf` is writable for `to_read <= buf.len()` bytes.
        let n = unsafe { psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, to_read as u32) };
        if n <= 0 {
            break;
        }
        send_response(cfd, &buf[..n as usize]);
        remaining -= n as usize;
    }

    io_close(fd);
}

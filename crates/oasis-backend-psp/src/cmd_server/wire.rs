//! Socket and parsing helpers shared by the command handlers.

use core::ffi::c_void;

/// Send all of `data` on `cfd`, looping over short writes.
///
/// Stops early (silently) if the peer goes away: every caller is a
/// best-effort reply on a connection that is closed right after.
pub(super) fn send_response(cfd: i32, data: &[u8]) {
    let mut sent = 0;
    while sent < data.len() {
        let rest = &data[sent..];
        // SAFETY: `rest` is a live slice; the pointer/length pair covers
        // exactly its bytes and the call only reads them.
        let n =
            unsafe { psp::sys::sceNetInetSend(cfd, rest.as_ptr() as *const c_void, rest.len(), 0) };
        if n <= 0 {
            return;
        }
        sent += n as usize;
    }
}

/// Receive into `buf`. Returns the byte count, or `None` on EOF / error /
/// timeout.
pub(super) fn recv(cfd: i32, buf: &mut [u8]) -> Option<usize> {
    // SAFETY: `buf` is a live, writable slice of exactly `buf.len()` bytes.
    let n = unsafe { psp::sys::sceNetInetRecv(cfd, buf.as_mut_ptr() as *mut c_void, buf.len(), 0) };
    (n > 0).then_some(n as usize)
}

/// Close a socket descriptor.
pub(super) fn close(fd: i32) {
    // SAFETY: closing a descriptor this module opened; double closes only
    // return an error code.
    unsafe { psp::sys::sceNetInetClose(fd) };
}

/// Set `SO_RCVTIMEO` on a socket (timeout in seconds).
pub(super) fn set_recv_timeout(fd: i32, secs: u32) {
    // struct timeval { long tv_sec; long tv_usec; } -- 32-bit longs on PSP.
    let timeval: [i32; 2] = [secs as i32, 0];
    // SAFETY: `timeval` outlives the call and is the 8 bytes passed.
    unsafe {
        psp::sys::sceNetInetSetsockopt(
            fd,
            0xFFFF, // SOL_SOCKET
            0x1006, // SO_RCVTIMEO
            timeval.as_ptr() as *const c_void,
            core::mem::size_of_val(&timeval) as u32,
        );
    }
}

/// Incrementally update a running CRC32 (ISO 3309, the polynomial of the
/// host's `zlib.crc32()`) with a new chunk. Start from 0.
pub(super) fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Parse a hex string (e.g. `b"1a2b3c4d"`) into a u32, skipping non-hex
/// bytes.
pub(super) fn parse_hex_u32(data: &[u8]) -> u32 {
    data.iter()
        .filter_map(|&b| (b as char).to_digit(16))
        .fold(0u32, |acc, d| (acc << 4) | d)
}

/// Parse the decimal digits in `data` into a u32 (non-digits are skipped,
/// overflow wraps -- callers range-check the result).
pub(super) fn parse_u32(data: &[u8]) -> u32 {
    data.iter()
        .filter(|b| b.is_ascii_digit())
        .fold(0u32, |acc, &b| {
            acc.wrapping_mul(10).wrapping_add((b - b'0') as u32)
        })
}

/// Split `"<a> <b>"` at the first space.
pub(super) fn split_arg(args: &[u8]) -> Option<(&[u8], &[u8])> {
    let pos = args.iter().position(|&b| b == b' ')?;
    Some((&args[..pos], &args[pos + 1..]))
}

//! PSK challenge-response for the recovery file server.
//!
//! The recovery server writes and deletes arbitrary `ms0:` files and can
//! reboot the device, so no connection gets a command in without proving it
//! holds the pre-shared key. The handshake is the EBOOT command server's
//! (and the desktop remote terminal's, `oasis_net::auth`), so
//! `scripts/psp_remote.py` talks to both with the same `PSP_PSK`:
//!
//! ```text
//! server: AUTH_REQUIRED <nonce>\n        (32 hex digits, fresh per connection)
//! client: AUTH <hex(HMAC-SHA256(key = psk, message = nonce))>\n
//! server: AUTH_OK\n                      (or AUTH_FAIL\n and close)
//! client: <command>\n ...
//! ```
//!
//! ## Key lookup
//!
//! Recovery mode exists for when the shell is broken, so the key must not
//! depend on the shell's install or config being intact. [`KEY_PATHS`] is
//! tried in order and the first file holding [`MIN_PSK_LEN`]+ bytes wins:
//!
//! 1. `ms0:/PSP/SAVEDATA/ARK_01234/oasis-recovery.key` -- recovery's own key,
//!    next to `RECOVERY.PBP` and outside the shell's directory, so wiping or
//!    corrupting `ms0:/PSP/GAME/OASISOS/` does not lock recovery out.
//! 2. `ms0:/PSP/GAME/OASISOS/cmdserver.key` -- the EBOOT command server's
//!    key, so a device already set up for remote development needs nothing
//!    extra. It is read as a plain file; no shell config is parsed.
//!
//! With neither, the server still listens (so the screen shows the IP and
//! the host gets a clear reason) but refuses every connection with
//! `AUTH_FAIL no key`. There is no unauthenticated mode.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use oasis_net::verify_auth_line;

/// Key files, most specific first (see the module docs).
pub const KEY_PATHS: [&str; 2] = [
    "ms0:/PSP/SAVEDATA/ARK_01234/oasis-recovery.key",
    "ms0:/PSP/GAME/OASISOS/cmdserver.key",
];

/// Shortest PSK accepted (the EBOOT command server's limit).
pub const MIN_PSK_LEN: usize = 8;

/// Longest accepted `AUTH ...` line (the answer is 69 bytes plus CR/LF).
const MAX_AUTH_LINE: usize = 128;

/// Longest a peer may take to send its whole `AUTH` line, in microseconds.
const AUTH_DEADLINE_US: i64 = 10_000_000;

/// Connections answered so far; folded into every nonce.
static NONCE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// The first usable key in [`KEY_PATHS`] and the path it came from.
pub fn load_psk() -> Option<(String, &'static str)> {
    KEY_PATHS.iter().find_map(|&path| {
        let raw = std::fs::read_to_string(path).ok()?;
        let psk = raw.trim();
        (psk.len() >= MIN_PSK_LEN).then(|| (psk.to_string(), path))
    })
}

/// A fresh challenge nonce: 128 bits as 32 hex digits, condensed with
/// SHA-256 from the RTC tick, the microsecond timer, a per-boot counter and
/// an MT19937 draw (the PSP has no OS entropy source; nonces need to be
/// unique and unpredictable, they are not keys).
fn new_nonce() -> String {
    let count = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tick: u64 = 0;
    // SAFETY: `tick` is a valid out-pointer for the duration of the call.
    unsafe { psp::sys::sceRtcGetCurrentTick(&mut tick) };
    // SAFETY: argument-less timer read.
    let now = unsafe { psp::sys::sceKernelGetSystemTimeWide() };
    let mt = {
        let mut ctx = core::mem::MaybeUninit::uninit();
        // SAFETY: the context is initialised by Mt19937Init before the read,
        // and both calls get a pointer to the same live context.
        unsafe {
            psp::sys::sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), (now as u32) ^ count);
            psp::sys::sceKernelUtilsMt19937UInt(ctx.as_mut_ptr())
        }
    };
    let mut seed = [0u8; 24];
    seed[..8].copy_from_slice(&tick.to_le_bytes());
    seed[8..16].copy_from_slice(&now.to_le_bytes());
    seed[16..20].copy_from_slice(&count.to_le_bytes());
    seed[20..].copy_from_slice(&mt.to_le_bytes());
    oasis_types::crypto::hex(&oasis_types::crypto::sha256(&seed)[..16])
}

/// Run the challenge-response on a fresh connection. `true` when the peer
/// proved it holds `psk`; otherwise the peer has been told `AUTH_FAIL` and
/// the caller closes the socket. `None` = no key configured: refuse.
pub fn authenticate(cfd: i32, psk: Option<&str>) -> bool {
    let Some(psk) = psk else {
        crate::send(cfd, b"AUTH_FAIL no key\n");
        return false;
    };
    let nonce = new_nonce();
    crate::send(cfd, format!("AUTH_REQUIRED {nonce}\n").as_bytes());

    // Read up to the first newline. Clients wait for `AUTH_OK` before they
    // send their command, so nothing after the newline belongs to us.
    let mut line = [0u8; MAX_AUTH_LINE];
    let mut len = 0;
    // SAFETY: argument-less timer read.
    let started = unsafe { psp::sys::sceKernelGetSystemTimeWide() };
    while !line[..len].contains(&b'\n') && len < line.len() {
        // SO_RCVTIMEO bounds each recv, not the whole answer: without an
        // overall deadline a peer trickling one byte per timeout could
        // hold the single-threaded server for ~20 minutes.
        // SAFETY: argument-less timer read.
        if unsafe { psp::sys::sceKernelGetSystemTimeWide() } - started > AUTH_DEADLINE_US {
            return false;
        }
        let rest = &mut line[len..];
        // SAFETY: `rest` is a live, writable slice of exactly `rest.len()` bytes.
        let n = unsafe {
            psp::sys::sceNetInetRecv(cfd, rest.as_mut_ptr() as *mut c_void, rest.len(), 0)
        };
        if n <= 0 {
            return false;
        }
        len += n as usize;
    }
    let line = core::str::from_utf8(&line[..len]).unwrap_or("");
    if verify_auth_line(psk, &nonce, line) {
        crate::send(cfd, b"AUTH_OK\n");
        true
    } else {
        crate::send(cfd, b"AUTH_FAIL\n");
        crate::print("[!] auth failed\n");
        // The accept loop is single-threaded, so this delay throttles online
        // PSK guessing for every peer.
        psp::thread::sleep_ms(2000);
        false
    }
}

/// Set `SO_RCVTIMEO` on a socket (timeout in seconds), so a peer that never
/// answers cannot hold the single-threaded server.
pub fn set_recv_timeout(fd: i32, secs: u32) {
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

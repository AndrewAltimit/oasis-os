//! Opt-in switch and PSK challenge-response for the command server.
//!
//! The server can deploy EBOOTs, write arbitrary `ms0:` files, inject input
//! and run JavaScript, so it never listens without a key. It is enabled only
//! when [`KEY_PATH`] holds a pre-shared key (at least [`MIN_PSK_LEN`]
//! printable bytes; surrounding whitespace is trimmed). Every connection
//! then has to answer the same challenge as the desktop remote terminal
//! (`oasis_net::auth`, docs/networking.md) before its command is read:
//!
//! ```text
//! server: AUTH_REQUIRED <nonce>\n        (32 hex digits, fresh per connection)
//! client: AUTH <hex(HMAC-SHA256(key = psk, message = nonce))>\n
//! server: AUTH_OK\n                      (or AUTH_FAIL\n and close)
//! client: <command>\n ...                (the unchanged one-shot protocol)
//! ```
//!
//! The PSK never crosses the wire and a captured answer cannot be replayed.
//! The session itself is plaintext, so use it on a trusted network.

use core::sync::atomic::{AtomicU32, Ordering};

use oasis_core::net::verify_auth_line;

use super::wire::{recv, send_response};

/// File holding the pre-shared key. Absent or too short = server off.
pub(super) const KEY_PATH: &str = "ms0:/PSP/GAME/OASISOS/cmdserver.key";

/// Shortest PSK accepted; shorter keys leave the server disabled.
pub(super) const MIN_PSK_LEN: usize = 8;

/// Longest accepted `AUTH ...` line (the answer is 69 bytes plus CR/LF).
const MAX_AUTH_LINE: usize = 128;

/// Connections answered so far; folded into every nonce.
static NONCE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Read the PSK from [`KEY_PATH`]. `None` (server stays off) when the file
/// is missing, unreadable, or holds a key shorter than [`MIN_PSK_LEN`].
pub(super) fn load_psk() -> Option<String> {
    let raw = std::fs::read_to_string(KEY_PATH).ok()?;
    let psk = raw.trim();
    if psk.len() < MIN_PSK_LEN {
        super::log_msg("[CMD] cmdserver.key shorter than 8 bytes; server disabled");
        return None;
    }
    Some(psk.to_string())
}

/// A fresh challenge nonce: 128 bits as 32 hex digits.
///
/// The PSP has no OS entropy source, so this condenses the RTC wall clock,
/// the microsecond system timer, a per-boot counter and an MT19937 draw with
/// SHA-256. Nonces only have to be unique (so answers cannot be replayed)
/// and hard to predict -- they are not keys.
fn new_nonce() -> String {
    let count = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tick: u64 = 0;
    // SAFETY: `tick` is a valid out-pointer for the duration of the call.
    unsafe { psp::sys::sceRtcGetCurrentTick(&mut tick) };
    // SAFETY: argument-less timer read.
    let now = unsafe { psp::sys::sceKernelGetSystemTimeWide() };
    let mt = {
        let mut ctx = core::mem::MaybeUninit::uninit();
        // SAFETY: the context is initialised by Mt19937Init before the
        // read, and both calls get a pointer to the same live context.
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
/// proved it holds `psk`; on failure the peer has been told `AUTH_FAIL`
/// and the caller closes the socket.
pub(super) fn authenticate(cfd: i32, psk: &str) -> bool {
    let nonce = new_nonce();
    send_response(cfd, format!("AUTH_REQUIRED {nonce}\n").as_bytes());

    // Read up to the first newline. Clients wait for `AUTH_OK` before they
    // send their command, so nothing after the newline belongs to us.
    let mut line = [0u8; MAX_AUTH_LINE];
    let mut len = 0;
    while !line[..len].contains(&b'\n') {
        if len == line.len() {
            break;
        }
        match recv(cfd, &mut line[len..]) {
            Some(n) => len += n,
            None => return false,
        }
    }
    let line = core::str::from_utf8(&line[..len]).unwrap_or("");
    if verify_auth_line(psk, &nonce, line) {
        send_response(cfd, b"AUTH_OK\n");
        true
    } else {
        send_response(cfd, b"AUTH_FAIL\n");
        super::log_msg("[CMD] auth failed");
        // The accept loop is single-threaded, so this delay throttles
        // online PSK guessing for every peer.
        psp::thread::sleep_ms(2000);
        false
    }
}

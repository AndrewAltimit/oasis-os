//! PSK challenge-response for the devloop TCP server.
//!
//! The same protocol as the desktop remote terminal and the EBOOT command
//! server (`oasis_net::auth`, docs/networking.md), self-contained here
//! because the PRX cannot link the std-based workspace crates:
//!
//! ```text
//! server: AUTH_REQUIRED <nonce>\n        (32 hex digits, fresh per connection)
//! client: AUTH <hex(HMAC-SHA256(key = psk, message = nonce))>\n
//! server: AUTH_OK\n                      (or AUTH_FAIL\n and close)
//! ```
//!
//! SHA-256 here is a one-shot, bounded-input version of
//! `oasis_types::crypto` (no allocator, ~1 KB of code): HMAC only ever
//! hashes one 64-byte pad block plus a 32-byte nonce or digest.

/// Round constants (first 32 bits of the cube roots of the first 64 primes).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Longest message [`sha256`] accepts (a pad block plus a digest/nonce
/// leaves room to spare).
const MAX_MSG: usize = 128;

/// Length of a nonce in hex digits (128 bits).
pub const NONCE_HEX_LEN: usize = 32;

/// Length of an HMAC answer in hex digits.
const ANSWER_HEX_LEN: usize = 64;

fn compress(state: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut v = *state;
    for i in 0..64 {
        let [a, b, c, d, e, f, g, h] = v;
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        v = [
            t1.wrapping_add(s0.wrapping_add(maj)),
            a,
            b,
            c,
            d.wrapping_add(t1),
            e,
            f,
            g,
        ];
    }
    for (slot, x) in state.iter_mut().zip(v) {
        *slot = slot.wrapping_add(x);
    }
}

/// SHA-256 of the concatenation `a || b` (`a.len() + b.len() <= MAX_MSG`;
/// longer input is truncated, which no caller produces).
fn sha256(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut buf = [0u8; MAX_MSG + 72];
    let a_len = a.len().min(MAX_MSG);
    let b_len = b.len().min(MAX_MSG - a_len);
    buf[..a_len].copy_from_slice(&a[..a_len]);
    buf[a_len..a_len + b_len].copy_from_slice(&b[..b_len]);
    let len = a_len + b_len;
    buf[len] = 0x80;
    let total = (len + 9).div_ceil(64) * 64;
    let bits = (len as u64) * 8;
    buf[total - 8..total].copy_from_slice(&bits.to_be_bytes());
    let mut state = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    for block in buf[..total].chunks_exact(64) {
        compress(&mut state, block);
    }
    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// HMAC-SHA256 (RFC 2104) for keys of at most one block.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for (i, &k) in key.iter().take(64).enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let inner = sha256(&ipad, msg);
    sha256(&opad, &inner)
}

/// Lowercase hex of `bytes` into `out` (`out.len() >= 2 * bytes.len()`).
pub fn hex_into(bytes: &[u8], out: &mut [u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for (i, &b) in bytes.iter().enumerate() {
        out[i * 2] = DIGITS[(b >> 4) as usize];
        out[i * 2 + 1] = DIGITS[(b & 0xf) as usize];
    }
}

/// A fresh nonce (hex digits) derived from `seed` (clock, counter, the
/// previous nonce): only uniqueness and unpredictability matter.
pub fn nonce_from_seed(seed: &[u8]) -> [u8; NONCE_HEX_LEN] {
    let digest = sha256(b"oasis-devloop-nonce", seed);
    let mut out = [0u8; NONCE_HEX_LEN];
    hex_into(&digest[..NONCE_HEX_LEN / 2], &mut out);
    out
}

/// Whether `line` is `AUTH <hex HMAC-SHA256(psk, nonce)>` (surrounding
/// whitespace ignored). The comparison runs in constant time.
pub fn verify_auth_line(psk: &[u8], nonce: &[u8], line: &[u8]) -> bool {
    let line = trim(line);
    let Some(answer) = line.strip_prefix(b"AUTH ") else {
        return false;
    };
    let answer = trim(answer);
    let mut expected = [0u8; ANSWER_HEX_LEN];
    hex_into(&hmac_sha256(psk, nonce), &mut expected);
    if answer.len() != ANSWER_HEX_LEN {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in answer.iter().zip(expected.iter()) {
        diff |= a ^ b;
    }
    // Read through a volatile so the loop is not turned into an
    // early-exit comparison.
    // SAFETY: `&diff` is a valid, aligned pointer to a live local.
    unsafe { core::ptr::read_volatile(&diff) == 0 }
}

fn trim(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |e| e + 1);
    &s[start..end]
}

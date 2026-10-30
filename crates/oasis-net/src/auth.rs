//! Authentication primitives shared by the remote terminal and the FTP-like
//! transfer server: the PSK challenge-response and the failed-attempt
//! throttle.
//!
//! # PSK challenge-response
//!
//! The remote terminal never sends its pre-shared key over the wire. On
//! accept the listener sends `AUTH_REQUIRED <nonce>` with a fresh random
//! nonce (32 lowercase hex digits); the client answers
//! `AUTH <hex(HMAC-SHA256(key = psk, message = nonce))>` and the listener
//! compares that in constant time. A passive observer learns neither the
//! PSK nor a reusable answer (every connection gets a new nonce), so
//! captured traffic cannot be replayed. The session itself is not
//! encrypted: commands and output travel in clear, so use the listener on
//! trusted networks or tunnel it (e.g. over SSH).
//!
//! The answer can be computed by hand with OpenSSL:
//!
//! ```text
//! printf %s "$NONCE" | openssl dgst -sha256 -hmac "$PSK"
//! ```

use std::time::{Duration, Instant};

/// Failed attempts before the throttle locks authentication out.
pub(crate) const MAX_AUTH_FAILURES: u32 = 5;

/// Lockout once [`MAX_AUTH_FAILURES`] is reached; doubles with each
/// further failure (30 s, 60 s, 120 s, ... capped at 64x).
pub(crate) const AUTH_RATE_LIMIT_BASE_SECS: u64 = 30;

/// How long a connection may stay unauthenticated before it is dropped,
/// so idle unauthenticated peers cannot hold every connection slot.
pub(crate) const AUTH_TIMEOUT: Duration = Duration::from_secs(20);

/// The answer to an `AUTH_REQUIRED <nonce>` challenge: lowercase hex of
/// HMAC-SHA256 keyed with `psk` over the nonce text.
pub fn psk_response(psk: &str, nonce: &str) -> String {
    hex(&hmac_sha256(psk.as_bytes(), nonce.as_bytes()))
}

/// Lowercase hex encoding.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// HMAC-SHA256 (RFC 2104).
pub(crate) fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    inner.extend(block.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(msg);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend(block.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&sha256(&inner));
    sha256(&outer)
}

/// SHA-256 (FIPS 180-4) of `data`.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = word.to_be_bytes();
    }
    out
}

/// Server-wide throttle for failed authentication attempts.
///
/// Failures are counted until a quiet period of one [`window`](Self::window)
/// passes without any. Reaching [`MAX_AUTH_FAILURES`] locks authentication
/// out for one window, measured from the failure that reached the limit;
/// each further failure (from connections already admitted) doubles the
/// window, capped at 64x. After a lockout the count is kept, so the next
/// failure locks again immediately with a longer window, until a quiet
/// window clears it.
///
/// The throttle is global, not per peer: the network backend does not
/// expose peer addresses. A flood of bad attempts therefore also delays
/// legitimate clients -- the trade-off for bounding online guessing.
pub(crate) struct AuthThrottle {
    /// Failures since the last quiet window.
    pub(crate) count: u32,
    last_failure: Option<Instant>,
    locked_until: Option<Instant>,
}

impl AuthThrottle {
    pub(crate) fn new() -> Self {
        Self {
            count: 0,
            last_failure: None,
            locked_until: None,
        }
    }

    /// The current lockout length: base * 2^(failures - threshold).
    pub(crate) fn window(&self) -> Duration {
        let excess = self.count.saturating_sub(MAX_AUTH_FAILURES);
        let multiplier = 1u64 << excess.min(6); // cap at 64x
        Duration::from_secs(AUTH_RATE_LIMIT_BASE_SECS.saturating_mul(multiplier))
    }

    /// Whether new authentication attempts are refused right now.
    pub(crate) fn is_locked(&self) -> bool {
        self.locked_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// Record one failed attempt.
    pub(crate) fn record_failure(&mut self, what: &str) {
        let now = Instant::now();
        let quiet = !self.is_locked()
            && self
                .last_failure
                .is_none_or(|last| now.duration_since(last) > self.window());
        if quiet {
            self.count = 0;
        }
        self.count = self.count.saturating_add(1);
        self.last_failure = Some(now);
        if self.count >= MAX_AUTH_FAILURES {
            let window = self.window();
            self.locked_until = Some(now + window);
            log::warn!(
                "{what}: auth rate limit active ({} failures, locked {:.0}s)",
                self.count,
                window.as_secs_f64(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Two-block message (56 bytes forces a second padding block).
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_vectors() {
        // Test case 1.
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: key longer than the block size.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn psk_response_depends_on_key_and_nonce() {
        let a = psk_response("s3cret", "00112233");
        assert_eq!(a.len(), 64);
        assert_eq!(a, psk_response("s3cret", "00112233"));
        assert_ne!(a, psk_response("s3cret", "00112234"));
        assert_ne!(a, psk_response("s3cres", "00112233"));
    }

    #[test]
    fn throttle_locks_at_threshold_and_escalates() {
        let mut t = AuthThrottle::new();
        for _ in 0..MAX_AUTH_FAILURES - 1 {
            t.record_failure("test");
        }
        assert!(!t.is_locked());
        t.record_failure("test");
        assert!(t.is_locked());
        assert_eq!(t.window().as_secs(), AUTH_RATE_LIMIT_BASE_SECS);
        t.record_failure("test");
        assert_eq!(t.window().as_secs(), AUTH_RATE_LIMIT_BASE_SECS * 2);
    }

    #[test]
    fn throttle_lock_is_anchored_at_the_limiting_failure() {
        let mut t = AuthThrottle::new();
        for _ in 0..MAX_AUTH_FAILURES {
            t.record_failure("test");
        }
        let until = t.locked_until.unwrap();
        let remaining = until.duration_since(Instant::now());
        assert!(remaining > Duration::from_secs(AUTH_RATE_LIMIT_BASE_SECS - 2));
    }

    #[test]
    fn throttle_window_is_capped() {
        let mut t = AuthThrottle::new();
        t.count = 1000;
        assert_eq!(t.window().as_secs(), AUTH_RATE_LIMIT_BASE_SECS * 64);
    }
}

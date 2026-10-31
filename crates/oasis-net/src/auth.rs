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

use oasis_types::crypto::{hex, hmac_sha256};
use oasis_types::util::ct_eq;

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

/// Whether `line` is a correct `AUTH <response>` answer to the challenge
/// `nonce` under `psk`, compared in constant time. Surrounding whitespace
/// (the line terminator included) is ignored.
///
/// Shared by the remote terminal listener and the PSP EBOOT's dev command
/// server, so both speak exactly one challenge-response protocol.
pub fn verify_auth_line(psk: &str, nonce: &str, line: &str) -> bool {
    let expected = psk_response(psk, nonce);
    line.trim()
        .strip_prefix("AUTH ")
        .map(str::trim)
        .is_some_and(|answer| ct_eq(answer.as_bytes(), expected.as_bytes()))
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
    fn psk_response_depends_on_key_and_nonce() {
        let a = psk_response("s3cret", "00112233");
        assert_eq!(a.len(), 64);
        assert_eq!(a, psk_response("s3cret", "00112233"));
        assert_ne!(a, psk_response("s3cret", "00112234"));
        // RFC 2104 HMAC-SHA256 over the nonce text; scripts/test_psp_remote.py
        // pins the same vector for the Python client.
        assert_eq!(
            psk_response("key", "abc"),
            "9c196e32dc0175f86f4b1cb89289d6619de6bee699e4c378e68309ed97a1a6ab"
        );
        assert_ne!(a, psk_response("s3cres", "00112233"));
    }

    #[test]
    fn verify_auth_line_accepts_only_the_right_answer() {
        let nonce = "0123456789abcdef0123456789abcdef";
        let good = format!("AUTH {}\r\n", psk_response("s3cret", nonce));
        assert!(verify_auth_line("s3cret", nonce, &good));
        assert!(!verify_auth_line("s3cres", nonce, &good));
        assert!(!verify_auth_line("s3cret", "00", &good));
        // The bare PSK (what pre-challenge clients sent) is rejected.
        assert!(!verify_auth_line("s3cret", nonce, "AUTH s3cret"));
        assert!(!verify_auth_line("s3cret", nonce, &good[5..]));
        assert!(!verify_auth_line("s3cret", nonce, ""));
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

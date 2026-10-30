//! Small dependency-free helpers shared across crates.
//!
//! - [`ct_eq`]: constant-time byte comparison for secrets (PSK, bearer tokens).
//! - [`base64`]: standard RFC 4648 base64 encode/decode.

/// Constant-time comparison of two byte slices.
///
/// Compares every byte when the lengths match, so the time taken does not
/// reveal how long a matching prefix is. A length mismatch returns early:
/// the attacker can observe the secret's length from packet sizes anyway.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Standard base64 (RFC 4648 alphabet, `=` padding), hand-rolled to avoid a
/// dependency.
pub mod base64 {
    use std::fmt;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    /// Why [`decode`] rejected its input.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum DecodeError {
        /// A byte outside the base64 alphabet (padding `=` is only allowed at
        /// the end).
        InvalidByte(u8),
        /// The data length leaves a dangling 6-bit group that encodes no byte.
        InvalidLength,
    }

    impl fmt::Display for DecodeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidByte(b) => write!(f, "invalid base64 char: {}", *b as char),
                Self::InvalidLength => write!(f, "invalid base64 length"),
            }
        }
    }

    impl std::error::Error for DecodeError {}

    /// Encode `data` as padded base64.
    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(n & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn sextet(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    /// Decode base64 text.
    ///
    /// Lenient in the ways real-world input needs: ASCII whitespace (line
    /// breaks in MIME / `data:` URLs) is skipped and trailing `=` padding is
    /// optional. Any other non-alphabet byte, or `=` before the end, is an
    /// error.
    pub fn decode(input: &str) -> Result<Vec<u8>, DecodeError> {
        let mut digits: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        while digits.last() == Some(&b'=') {
            digits.pop();
        }
        if digits.len() % 4 == 1 {
            return Err(DecodeError::InvalidLength);
        }
        let mut out = Vec::with_capacity(digits.len() * 3 / 4);
        for chunk in digits.chunks(4) {
            let mut n = 0u32;
            for (i, &b) in chunk.iter().enumerate() {
                let v = sextet(b).ok_or(DecodeError::InvalidByte(b))?;
                n |= u32::from(v) << (18 - 6 * i);
            }
            out.push((n >> 16) as u8);
            if chunk.len() > 2 {
                out.push((n >> 8) as u8);
            }
            if chunk.len() > 3 {
                out.push(n as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::base64::{DecodeError, decode, encode};
    use super::ct_eq;

    #[test]
    fn ct_eq_cases() {
        assert!(ct_eq(b"", b""));
        assert!(ct_eq(b"secret", b"secret"));
        assert!(!ct_eq(b"secret", b"secreT"));
        assert!(!ct_eq(b"secret", b"secret1"));
        assert!(!ct_eq(b"", b"x"));
        assert!(!ct_eq(b"Secret", b"secret"));
    }

    const VECTORS: [(&[u8], &str); 7] = [
        (b"", ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
    ];

    #[test]
    fn rfc4648_vectors() {
        for (raw, enc) in VECTORS {
            assert_eq!(encode(raw), enc);
            assert_eq!(decode(enc).unwrap(), raw, "decode {enc}");
        }
    }

    #[test]
    fn binary_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
        assert_eq!(encode(&[0x00, 0xff, 0x10]), "AP8Q");
    }

    #[test]
    fn decode_is_lenient_about_whitespace_and_padding() {
        assert_eq!(decode("aGVs\nbG8=\r\n").unwrap(), b"hello");
        assert_eq!(decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode("  Zg  ").unwrap(), b"f");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert_eq!(decode("ab$d"), Err(DecodeError::InvalidByte(b'$')));
        assert_eq!(decode("Zg==Zg=="), Err(DecodeError::InvalidByte(b'=')));
        assert_eq!(decode("Zm9vY"), Err(DecodeError::InvalidLength));
        assert_eq!(
            decode("ab$d").unwrap_err().to_string(),
            "invalid base64 char: $"
        );
    }
}

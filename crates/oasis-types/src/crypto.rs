//! Dependency-free hashing for the places that need a digest but must not
//! pull in a crypto crate (the PSP build in particular): SHA-256 (FIPS
//! 180-4), HMAC-SHA256 (RFC 2104) and lowercase hex.
//!
//! Users: the remote-terminal PSK challenge-response (`oasis-net`) and CSP
//! `sha256-...` source hashes (`oasis-http`, which still carries its own
//! copy at the time of writing). Compare secrets and MACs with
//! [`ct_eq`](crate::util::ct_eq), not `==`.

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

/// Initial hash value (first 32 bits of the square roots of the first 8
/// primes).
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 block size in bytes (also the HMAC block size).
pub const SHA256_BLOCK_LEN: usize = 64;

/// SHA-256 digest size in bytes.
pub const SHA256_LEN: usize = 32;

/// Incremental SHA-256: feed data with [`update`](Self::update), then
/// [`finalize`](Self::finalize). Memory use is constant (one block).
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    /// Bytes not yet compressed (always fewer than one block).
    buf: [u8; SHA256_BLOCK_LEN],
    buf_len: usize,
    /// Total message length in bytes.
    len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A hasher over the empty message.
    pub fn new() -> Self {
        Self {
            state: H0,
            buf: [0; SHA256_BLOCK_LEN],
            buf_len: 0,
            len: 0,
        }
    }

    /// Append `data` to the message.
    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (SHA256_BLOCK_LEN - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < SHA256_BLOCK_LEN {
                return;
            }
            let block = self.buf;
            compress(&mut self.state, &block);
            self.buf_len = 0;
        }
        let (blocks, rest) = data.as_chunks::<SHA256_BLOCK_LEN>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.buf_len = rest.len();
    }

    /// Finish the message (padding + length) and return the digest.
    pub fn finalize(mut self) -> [u8; SHA256_LEN] {
        let bit_len = self.len.wrapping_mul(8);
        let mut pad = [0u8; SHA256_BLOCK_LEN + 8];
        pad[0] = 0x80;
        // Pad to 56 mod 64, then append the 64-bit big-endian bit length.
        let pad_len = if self.buf_len < 56 {
            56 - self.buf_len
        } else {
            120 - self.buf_len
        };
        self.update(&pad[..pad_len]);
        self.update(&bit_len.to_be_bytes());
        debug_assert_eq!(self.buf_len, 0);
        let mut out = [0u8; SHA256_LEN];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }
}

/// The SHA-256 compression function over one 64-byte block.
fn compress(state: &mut [u32; 8], block: &[u8; SHA256_BLOCK_LEN]) {
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
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (k, w) in K.iter().zip(w) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(*k)
            .wrapping_add(w);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (slot, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(v);
    }
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; SHA256_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

/// HMAC-SHA256 of `msg` under `key` (RFC 2104; keys longer than one block
/// are hashed first).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; SHA256_LEN] {
    let mut block = [0u8; SHA256_BLOCK_LEN];
    if key.len() > SHA256_BLOCK_LEN {
        block[..SHA256_LEN].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(&block.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(&block.map(|b| b ^ 0x5c));
    outer.update(&inner.finalize());
    outer.finalize()
}

/// Lowercase hex encoding of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_fips_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56 bytes: the length no longer fits, forcing a second block.
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmn\
                  hijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
            )),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
    }

    #[test]
    fn sha256_million_a() {
        let mut h = Sha256::new();
        let chunk = [b'a'; 1000];
        for _ in 0..1000 {
            h.update(&chunk);
        }
        assert_eq!(
            hex(&h.finalize()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_boundary_lengths_match_across_split_points() {
        // Lengths around the padding boundaries, fed in two pieces at every
        // split point, must equal the one-shot digest.
        let data: Vec<u8> = (0..200u8).collect();
        for len in [0, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 129, 200] {
            let msg = &data[..len];
            let whole = sha256(msg);
            for split in 0..=len {
                let mut h = Sha256::new();
                h.update(&msg[..split]);
                h.update(&msg[split..]);
                assert_eq!(h.finalize(), whole, "len {len} split {split}");
            }
        }
    }

    #[test]
    fn hmac_sha256_rfc4231_vectors() {
        // Test case 1.
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2: short key.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 3: 50 bytes of 0xdd.
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 20], &[0xdd; 50])),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );
        // Test case 6: key longer than the block size.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
        // Test case 7: long key and long message.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"This is a test using a larger than block-size key and a larger \
                  than block-size data. The key needs to be hashed before being \
                  used by the HMAC algorithm."
            )),
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"
        );
    }

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex(&[]), "");
    }
}

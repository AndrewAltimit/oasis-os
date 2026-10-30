//! Hashers for the name index and per-object change detection.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::object::SdiObject;

/// FNV-1a hasher for the name index.
///
/// Object names are short ASCII keys (`term_line_12`, `dash_icon_3_label`)
/// looked up many times per frame; SipHash's per-call setup and
/// finalization dominate at that size. The index is not exposed to
/// untrusted keys, so HashDoS resistance buys nothing here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FnvHasher(u64);

impl Default for FnvHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for FnvHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub(super) type NameIndex = HashMap<String, usize, BuildHasherDefault<FnvHasher>>;

/// Word-at-a-time multiply/rotate hasher for per-object change detection.
///
/// Every step `h = (rotl(h, 5) ^ word) * K` (odd `K`) is a bijection of
/// `h` for a fixed input word, so two object states that differ in exactly
/// one hashed word — the common per-frame case (a moved x, a new text, a
/// toggled flag) — are *guaranteed* to hash differently. It only compares
/// an object against its own previous state (never used as a table key),
/// so the weak low-bit distribution of this construction is irrelevant,
/// and it is several times cheaper than SipHash on the ~30 small fields
/// an object feeds it.
#[derive(Debug, Clone, Copy, Default)]
struct RenderHasher(u64);

impl RenderHasher {
    const K: u64 = 0xf135_7aea_2e62_a9c5;

    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::K);
    }
}

impl Hasher for RenderHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for &w in words {
            self.add(u64::from_le_bytes(w));
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            // Tag the tail length so "ab" + "" and "a" + "b" differ.
            self.add(u64::from_le_bytes(w) ^ ((rest.len() as u64) << 59));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Render-state hash of a single object (see [`SdiObject::hash_render_state`]).
pub(super) fn object_hash(obj: &SdiObject) -> u64 {
    let mut h = RenderHasher::default();
    hash_object_into(obj, &mut h);
    h.finish()
}

/// Hash an object's render state into `h`.
///
/// Hidden objects contribute only their name and visibility: nothing else
/// about them reaches the screen, so rewriting properties of a hidden
/// object (the per-frame "hide pass" idiom) never registers as a change.
pub(super) fn hash_object_into<H: Hasher>(obj: &SdiObject, h: &mut H) {
    use std::hash::Hash;
    if obj.visible {
        obj.hash_render_state(h);
    } else {
        obj.name.hash(h);
        false.hash(h);
    }
}

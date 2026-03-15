//! Content-addressed texture deduplication with LRU eviction and reference
//! counting.
//!
//! When `load_texture()` is called with RGBA data that has already been loaded,
//! the cache returns the existing `TextureId` instead of allocating a duplicate.
//! A fast content hash (FNV-1a over sampled bytes + length) keeps lookup cheap
//! even for large textures. Reference counting ensures textures shared by
//! multiple callers are not destroyed until all references are released.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Content hash
// ---------------------------------------------------------------------------

/// A content fingerprint for RGBA texture data.
///
/// Uses dimensions + a sampled FNV-1a hash of the pixel data. The sampling
/// strategy (first 256 bytes, last 256 bytes, every-Nth byte in between)
/// keeps cost O(1) for arbitrarily large textures while providing strong
/// collision resistance in practice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash {
    width: u32,
    height: u32,
    data_len: usize,
    hash: u64,
}

/// Number of sample bytes at head/tail of the data.
const SAMPLE_EDGE: usize = 256;

/// Maximum total bytes to hash (head + tail + middle samples).
const MAX_HASH_BYTES: usize = 2048;

impl ContentHash {
    /// Compute a content hash from RGBA data.
    pub fn new(width: u32, height: u32, rgba_data: &[u8]) -> Self {
        let hash = sampled_fnv1a(rgba_data);
        Self {
            width,
            height,
            data_len: rgba_data.len(),
            hash,
        }
    }
}

/// FNV-1a over sampled bytes of the input.
fn sampled_fnv1a(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut h = FNV_OFFSET;
    let len = data.len();

    if len <= MAX_HASH_BYTES {
        // Small enough to hash entirely.
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    } else {
        // Sample head.
        for &b in &data[..SAMPLE_EDGE] {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        // Sample middle (evenly spaced).
        let middle_start = SAMPLE_EDGE;
        let middle_end = len - SAMPLE_EDGE;
        let middle_samples = MAX_HASH_BYTES - SAMPLE_EDGE * 2;
        let step = (middle_end - middle_start) / middle_samples.max(1);
        let step = step.max(1);
        let mut i = middle_start;
        let mut count = 0;
        while i < middle_end && count < middle_samples {
            h ^= data[i] as u64;
            h = h.wrapping_mul(FNV_PRIME);
            i += step;
            count += 1;
        }
        // Sample tail.
        for &b in &data[len - SAMPLE_EDGE..] {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

// ---------------------------------------------------------------------------
// Cache entry
// ---------------------------------------------------------------------------

/// Per-texture metadata tracked by the dedup cache.
struct CacheEntry {
    /// The backend texture id (the raw `u64` inside `TextureId`).
    texture_id: u64,
    /// Number of active references to this texture.
    refcount: u32,
    /// Monotonic access timestamp for LRU eviction.
    last_access: u64,
}

// ---------------------------------------------------------------------------
// TextureDedup
// ---------------------------------------------------------------------------

/// Default maximum number of deduplicated textures before LRU eviction.
const DEFAULT_MAX_TEXTURES: usize = 512;

/// Content-addressed texture cache with reference counting and LRU eviction.
///
/// Backends call [`TextureDedup::acquire`] when loading a texture and
/// [`TextureDedup::release`] when destroying one. The cache is generic over
/// the actual GPU/canvas texture storage -- it only tracks `u64` ids and
/// content hashes.
pub struct TextureDedup {
    /// Forward map: content hash -> cache entry.
    by_hash: HashMap<ContentHash, CacheEntry>,
    /// Reverse map: texture id -> content hash (for O(1) release).
    by_id: HashMap<u64, ContentHash>,
    /// Maximum cached textures before eviction.
    max_textures: usize,
    /// Monotonic counter for LRU timestamps.
    access_counter: u64,
}

impl TextureDedup {
    /// Create a new dedup cache with the default capacity.
    pub fn new() -> Self {
        Self {
            by_hash: HashMap::new(),
            by_id: HashMap::new(),
            max_textures: DEFAULT_MAX_TEXTURES,
            access_counter: 0,
        }
    }

    /// Try to acquire a texture for the given RGBA content.
    ///
    /// Returns `Some(texture_id)` if an identical texture is already cached
    /// (bumps refcount + LRU timestamp). Returns `None` if the caller must
    /// create a new backend texture.
    pub fn acquire(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Option<u64> {
        let hash = ContentHash::new(width, height, rgba_data);
        if let Some(entry) = self.by_hash.get_mut(&hash) {
            entry.refcount += 1;
            self.access_counter += 1;
            entry.last_access = self.access_counter;
            return Some(entry.texture_id);
        }
        None
    }

    /// Register a newly created backend texture with the cache.
    ///
    /// Called by the backend immediately after creating the GPU/canvas texture,
    /// so future `acquire` calls for the same content will return this id.
    pub fn insert(&mut self, texture_id: u64, width: u32, height: u32, rgba_data: &[u8]) {
        let hash = ContentHash::new(width, height, rgba_data);
        self.access_counter += 1;
        self.by_hash.insert(
            hash,
            CacheEntry {
                texture_id,
                refcount: 1,
                last_access: self.access_counter,
            },
        );
        self.by_id.insert(texture_id, hash);
    }

    /// Release a reference to a texture.
    ///
    /// Returns `true` if the refcount reached zero and the backend should
    /// destroy the actual GPU/canvas texture. Returns `false` if the texture
    /// still has other references or if the texture is kept in the cache for
    /// potential LRU reuse (refcount == 0).
    pub fn release(&mut self, texture_id: u64) -> bool {
        let Some(hash) = self.by_id.get(&texture_id).copied() else {
            // Not tracked by dedup (e.g. glyph cache textures).
            return true;
        };
        if let Some(entry) = self.by_hash.get_mut(&hash) {
            if entry.refcount > 0 {
                entry.refcount -= 1;
            }
            // Keep the entry in the cache even at refcount 0 so that
            // future `acquire` calls can reuse it (LRU caching) and
            // `evict` can safely reclaim it.
            return false;
        }
        // Hash entry was already removed (e.g. by eviction); clean up
        // the reverse map and let the backend destroy the texture.
        self.by_id.remove(&texture_id);
        true
    }

    /// Evict least-recently-used entries until the cache is within capacity.
    ///
    /// Returns a list of texture ids that should be destroyed by the backend.
    /// Only textures with refcount == 0 (no active references) are eligible
    /// for eviction -- active textures are never evicted.
    pub fn evict(&mut self) -> Vec<u64> {
        let mut evicted = Vec::new();
        while self.by_hash.len() > self.max_textures {
            // Find the unreferenced entry with the smallest last_access.
            let victim = self
                .by_hash
                .iter()
                .filter(|(_, e)| e.refcount == 0)
                .min_by_key(|(_, e)| e.last_access)
                .map(|(h, e)| (*h, e.texture_id));

            if let Some((hash, tid)) = victim {
                self.by_hash.remove(&hash);
                self.by_id.remove(&tid);
                evicted.push(tid);
            } else {
                // All entries have active references; cannot evict further.
                break;
            }
        }
        evicted
    }

    /// Number of deduplicated textures currently tracked.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }
}

impl Default for TextureDedup {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rgba(w: u32, h: u32, fill: u8) -> Vec<u8> {
        vec![fill; (w * h * 4) as usize]
    }

    #[test]
    fn acquire_returns_none_for_new_content() {
        let mut dedup = TextureDedup::new();
        let data = make_rgba(4, 4, 0xFF);
        assert!(dedup.acquire(4, 4, &data).is_none());
    }

    #[test]
    fn acquire_returns_id_after_insert() {
        let mut dedup = TextureDedup::new();
        let data = make_rgba(4, 4, 0xFF);
        dedup.insert(42, 4, 4, &data);
        assert_eq!(dedup.acquire(4, 4, &data), Some(42));
    }

    #[test]
    fn different_content_gets_different_entries() {
        let mut dedup = TextureDedup::new();
        let data_a = make_rgba(4, 4, 0xFF);
        let data_b = make_rgba(4, 4, 0x00);
        dedup.insert(1, 4, 4, &data_a);
        dedup.insert(2, 4, 4, &data_b);
        assert_eq!(dedup.acquire(4, 4, &data_a), Some(1));
        assert_eq!(dedup.acquire(4, 4, &data_b), Some(2));
    }

    #[test]
    fn different_dimensions_same_data_are_distinct() {
        let mut dedup = TextureDedup::new();
        let data = make_rgba(2, 2, 0xAA);
        dedup.insert(1, 2, 2, &data);
        // Same bytes but different width/height.
        assert!(dedup.acquire(1, 4, &data).is_none());
    }

    #[test]
    fn refcount_prevents_destruction() {
        let mut dedup = TextureDedup::new();
        let data = make_rgba(4, 4, 0xFF);
        dedup.insert(10, 4, 4, &data);
        // Second acquire bumps refcount to 2.
        assert_eq!(dedup.acquire(4, 4, &data), Some(10));
        // First release: refcount 2 -> 1, should NOT destroy.
        assert!(!dedup.release(10));
        // Second release: refcount 1 -> 0, kept in cache for LRU reuse.
        assert!(!dedup.release(10));
        // Texture is still acquirable at refcount 0 (LRU cached).
        assert_eq!(dedup.acquire(4, 4, &data), Some(10));
        // Release again to get back to refcount 0.
        assert!(!dedup.release(10));
    }

    #[test]
    fn release_unknown_id_returns_true() {
        let mut dedup = TextureDedup::new();
        assert!(dedup.release(999));
    }

    #[test]
    fn evict_removes_lru_entries() {
        let mut dedup = TextureDedup {
            max_textures: 3,
            ..TextureDedup::new()
        };
        for i in 0..5u64 {
            let data = make_rgba(2, 2, i as u8);
            dedup.insert(i, 2, 2, &data);
        }
        // All entries have refcount 1 (active); evict cannot remove any.
        let evicted = dedup.evict();
        assert_eq!(evicted.len(), 0);
        assert_eq!(dedup.len(), 5);

        // Release the two oldest entries to make them evictable.
        dedup.release(0);
        dedup.release(1);
        let evicted = dedup.evict();
        assert_eq!(evicted.len(), 2);
        assert!(evicted.contains(&0));
        assert!(evicted.contains(&1));
        assert_eq!(dedup.len(), 3);
    }

    #[test]
    fn evict_skips_active_entries() {
        let mut dedup = TextureDedup {
            max_textures: 1,
            ..TextureDedup::new()
        };
        let data_a = make_rgba(2, 2, 0xAA);
        let data_b = make_rgba(2, 2, 0xBB);
        dedup.insert(1, 2, 2, &data_a);
        dedup.insert(2, 2, 2, &data_b);
        // Release entry 2 so it becomes evictable (refcount 0).
        dedup.release(2);
        let evicted = dedup.evict();
        // Only entry 2 can be evicted (refcount == 0).
        assert_eq!(evicted, vec![2]);
    }

    #[test]
    fn content_hash_large_data() {
        // Verify the sampled hash works for data larger than MAX_HASH_BYTES.
        let data = make_rgba(64, 64, 0xCC); // 16384 bytes
        let h1 = ContentHash::new(64, 64, &data);
        let h2 = ContentHash::new(64, 64, &data);
        assert_eq!(h1, h2);

        let mut data2 = data.clone();
        // Flip a byte in the head region (always sampled).
        data2[100] = 0x00;
        let h3 = ContentHash::new(64, 64, &data2);
        assert_ne!(h1.hash, h3.hash);

        let mut data3 = data.clone();
        // Flip a byte in the tail region (always sampled).
        let tail_idx = data3.len() - 100;
        data3[tail_idx] = 0x00;
        let h4 = ContentHash::new(64, 64, &data3);
        assert_ne!(h1.hash, h4.hash);
    }
}

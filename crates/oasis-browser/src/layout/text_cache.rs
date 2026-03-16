//! Caching wrapper for [`TextMeasurer`].
//!
//! [`CachingMeasurer`] wraps any `TextMeasurer` implementation and
//! caches results in a hash map keyed by `(text_hash, font_size)`.
//! This avoids redundant per-character iteration for text runs that
//! appear multiple times during a single layout pass (e.g. repeated
//! words, space characters, emergency word-break re-measurement).
//!
//! The cache uses a `u64` hash of the text string rather than cloning
//! the string itself, keeping lookup overhead minimal. Collisions are
//! astronomically unlikely with the 64-bit `DefaultHasher` and do not
//! affect correctness -- they would only cause a rare width mismatch
//! for a single layout pass.

use super::block::TextMeasurer;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Compute a 64-bit hash of a text string for use as a cache key.
fn text_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Shared text measurement cache that persists across layout passes.
///
/// The cache is only cleared when the base font size changes (e.g.
/// due to zoom level changes).  This avoids re-measuring the same
/// text strings on every relayout.
pub type SharedTextCache = std::rc::Rc<RefCell<HashMap<(u64, u16), u32>>>;

/// Create a new empty shared text cache.
pub fn new_shared_cache() -> SharedTextCache {
    std::rc::Rc::new(RefCell::new(HashMap::new()))
}

/// A text measurer that caches results from an inner measurer.
///
/// Wraps any `&dyn TextMeasurer` and stores computed widths in a
/// `HashMap<(u64, u16), u32>` keyed by `(text_hash, font_size)`.
/// Because `TextMeasurer::measure_text` takes `&self`, interior
/// mutability via `RefCell` is used for the cache.
///
/// # Usage
///
/// ```ignore
/// let inner = SimpleTextMeasurer;
/// let cached = CachingMeasurer::new(&inner);
/// // Pass &cached wherever &dyn TextMeasurer is expected.
/// build_layout_tree(doc, styles, &cached, ...);
/// ```
pub struct CachingMeasurer<'a> {
    inner: &'a dyn TextMeasurer,
    cache: RefCell<HashMap<(u64, u16), u32>>,
    /// Optional shared cache that persists across layout passes.
    shared: Option<SharedTextCache>,
}

impl<'a> CachingMeasurer<'a> {
    /// Create a new caching measurer wrapping the given inner measurer.
    pub fn new(inner: &'a dyn TextMeasurer) -> Self {
        Self {
            inner,
            cache: RefCell::new(HashMap::new()),
            shared: None,
        }
    }

    /// Create a caching measurer backed by a shared persistent cache.
    ///
    /// Results are stored in the shared cache and survive across
    /// layout passes. The local `cache` field is unused in this mode.
    pub fn with_shared(inner: &'a dyn TextMeasurer, shared: SharedTextCache) -> Self {
        Self {
            inner,
            cache: RefCell::new(HashMap::new()),
            shared: Some(shared),
        }
    }

    /// Number of cached entries (useful for testing/benchmarking).
    pub fn len(&self) -> usize {
        if let Some(shared) = &self.shared {
            shared.borrow().len()
        } else {
            self.cache.borrow().len()
        }
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        if let Some(shared) = &self.shared {
            shared.borrow().is_empty()
        } else {
            self.cache.borrow().is_empty()
        }
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        if let Some(shared) = &self.shared {
            shared.borrow_mut().clear();
        } else {
            self.cache.borrow_mut().clear();
        }
    }
}

impl TextMeasurer for CachingMeasurer<'_> {
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        let hash = text_hash(text);
        let key = (hash, font_size);

        if let Some(shared) = &self.shared {
            // Shared persistent cache path.
            if let Some(&width) = shared.borrow().get(&key) {
                return width;
            }
            let width = self.inner.measure_text(text, font_size);
            shared.borrow_mut().insert(key, width);
            width
        } else {
            // Local per-pass cache path.
            if let Some(&width) = self.cache.borrow().get(&key) {
                return width;
            }
            let width = self.inner.measure_text(text, font_size);
            self.cache.borrow_mut().insert(key, width);
            width
        }
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Counting measurer that tracks how many times `measure_text` is
    /// called on the inner implementation.
    struct CountingMeasurer {
        calls: Cell<u32>,
    }

    impl CountingMeasurer {
        fn new() -> Self {
            Self {
                calls: Cell::new(0),
            }
        }

        fn call_count(&self) -> u32 {
            self.calls.get()
        }
    }

    impl TextMeasurer for CountingMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            self.calls.set(self.calls.get() + 1);
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    #[test]
    fn cache_miss_delegates_to_inner() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let result = cached.measure_text("hello", 16);
        assert_eq!(inner.call_count(), 1, "first call should delegate");
        assert_eq!(
            result,
            oasis_types::backend::bitmap_measure_text("hello", 16),
        );
    }

    #[test]
    fn cache_hit_avoids_inner_call() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let first = cached.measure_text("hello", 16);
        let second = cached.measure_text("hello", 16);

        assert_eq!(inner.call_count(), 1, "second call should use cache");
        assert_eq!(first, second, "cached result should match original");
    }

    #[test]
    fn different_text_produces_separate_entries() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let a = cached.measure_text("hello", 16);
        let b = cached.measure_text("world", 16);

        assert_eq!(inner.call_count(), 2, "different text should miss cache");
        assert_ne!(a, b, "different text should produce different widths");
        assert_eq!(cached.len(), 2);
    }

    #[test]
    fn different_font_size_produces_separate_entries() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let small = cached.measure_text("hello", 8);
        let large = cached.measure_text("hello", 16);

        assert_eq!(
            inner.call_count(),
            2,
            "different font size should miss cache",
        );
        assert_ne!(
            small, large,
            "different font sizes should produce different widths",
        );
        assert_eq!(cached.len(), 2);
    }

    #[test]
    fn repeated_calls_with_mixed_inputs() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        // First round: all misses.
        cached.measure_text("hello", 16);
        cached.measure_text("world", 16);
        cached.measure_text("hello", 8);
        assert_eq!(inner.call_count(), 3, "three unique inputs = 3 misses");

        // Second round: all hits.
        cached.measure_text("hello", 16);
        cached.measure_text("world", 16);
        cached.measure_text("hello", 8);
        assert_eq!(inner.call_count(), 3, "repeat calls should all hit cache");

        assert_eq!(cached.len(), 3);
    }

    #[test]
    fn empty_string_is_cached() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let first = cached.measure_text("", 16);
        let second = cached.measure_text("", 16);

        assert_eq!(inner.call_count(), 1);
        assert_eq!(first, 0);
        assert_eq!(second, 0);
    }

    #[test]
    fn single_space_is_cached() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        let first = cached.measure_text(" ", 16);
        let second = cached.measure_text(" ", 16);

        assert_eq!(inner.call_count(), 1, "space should be cached on repeat");
        assert_eq!(first, second);
        assert!(first > 0, "space should have nonzero width");
    }

    #[test]
    fn clear_resets_cache() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        cached.measure_text("hello", 16);
        assert_eq!(cached.len(), 1);

        cached.clear();
        assert!(cached.is_empty());

        // After clear, should miss again.
        cached.measure_text("hello", 16);
        assert_eq!(inner.call_count(), 2, "cleared cache should miss");
    }

    #[test]
    fn values_match_bitmap_measure_text() {
        let inner = CountingMeasurer::new();
        let cached = CachingMeasurer::new(&inner);

        // Verify a variety of inputs produce correct results.
        for (text, size) in &[
            ("a", 8),
            ("hello world", 12),
            ("test", 16),
            ("x", 24),
            ("longer text here", 10),
        ] {
            let expected = oasis_types::backend::bitmap_measure_text(text, *size);
            let got = cached.measure_text(text, *size);
            assert_eq!(
                got, expected,
                "CachingMeasurer({text:?}, {size}) = {got}, expected {expected}",
            );
        }
    }
}

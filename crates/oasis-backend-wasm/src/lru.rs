//! A small bounded LRU map for the glyph-canvas cache.
//!
//! Each entry carries its last-use tick, and a `BTreeMap` from tick to key
//! keeps the recency order, so a hit, an insert and an eviction are all
//! O(log n). The cache used to find its victim with a linear `min_by_key`
//! over every entry on each miss at capacity (2048 glyphs per miss).

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// A map holding at most `capacity` entries, evicting the least recently
/// used one to make room.
pub(crate) struct LruMap<K, V> {
    entries: HashMap<K, (V, u64)>,
    /// Last-use tick -> key, oldest first.
    order: BTreeMap<u64, K>,
    tick: u64,
    capacity: usize,
}

impl<K: Copy + Eq + Hash, V> LruMap<K, V> {
    /// An empty map holding up to `capacity` (at least 1) entries.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            capacity: capacity.max(1),
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// The value for `key` without touching its recency.
    pub(crate) fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(v, _)| v)
    }

    /// Whether `key` is cached; a hit marks it most recently used.
    pub(crate) fn touch(&mut self, key: &K) -> bool {
        let tick = self.next_tick();
        let Some((_, last)) = self.entries.get_mut(key) else {
            return false;
        };
        self.order.remove(last);
        *last = tick;
        self.order.insert(tick, *key);
        true
    }

    /// Insert (or replace) `key`, evicting least recently used entries
    /// while the map is full.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if let Some((_, last)) = self.entries.remove(&key) {
            self.order.remove(&last);
        }
        while self.entries.len() >= self.capacity {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        let tick = self.next_tick();
        self.entries.insert(key, (value, tick));
        self.order.insert(tick, key);
    }

    /// Drop every entry.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Number of entries.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let mut m = LruMap::new(2);
        m.insert('a', 1);
        m.insert('b', 2);
        assert!(m.touch(&'a'));
        m.insert('c', 3);
        assert_eq!(m.len(), 2);
        assert_eq!(m.peek(&'a'), Some(&1));
        assert_eq!(m.peek(&'b'), None, "b was least recently used");
        assert_eq!(m.peek(&'c'), Some(&3));
    }

    #[test]
    fn reinsert_replaces_without_growing() {
        let mut m = LruMap::new(2);
        m.insert('a', 1);
        m.insert('a', 9);
        m.insert('b', 2);
        assert_eq!(m.len(), 2);
        assert_eq!(m.peek(&'a'), Some(&9));
        assert!(!m.touch(&'z'));
        m.clear();
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn misses_at_capacity_stay_cheap() {
        // 200k inserts into a full 2048-entry map: the old linear victim
        // search made this ~400M comparisons.
        let mut m = LruMap::new(2048);
        for i in 0..200_000u32 {
            m.insert(i, i);
        }
        assert_eq!(m.len(), 2048);
        assert_eq!(m.peek(&199_999), Some(&199_999));
        assert_eq!(m.peek(&0), None);
    }
}

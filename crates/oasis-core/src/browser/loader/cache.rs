//! LRU resource cache.
//!
//! Bounds memory usage by evicting the least-recently-used entries when
//! the total cached body size exceeds a configurable limit.

use std::collections::{HashMap, VecDeque};

use crate::backend::TextureId;

use super::ResourceResponse;

/// An entry in the resource cache.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The loaded resource data.
    pub response: ResourceResponse,
    /// If the resource is an image, the decoded texture handle.
    pub texture: Option<TextureId>,
}

/// LRU resource cache with bounded size (measured in body bytes).
pub struct ResourceCache {
    entries: HashMap<String, CacheEntry>,
    /// Front = most recently used, back = least recently used.
    order: VecDeque<String>,
    current_size: usize,
    max_size: usize,
}

impl ResourceCache {
    /// Create a new cache with the given maximum size in bytes.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            current_size: 0,
            max_size,
        }
    }

    /// Look up a cached resource by URL, promoting it to the
    /// most-recently-used position.
    pub fn get(&mut self, url: &str) -> Option<&CacheEntry> {
        if self.entries.contains_key(url) {
            // Move to front of LRU order.
            self.order.retain(|u| u != url);
            self.order.push_front(url.to_string());
            self.entries.get(url)
        } else {
            None
        }
    }

    /// Insert a resource into the cache, evicting least-recently-used
    /// entries as needed to stay within the size limit.
    ///
    /// Entries whose body is larger than `max_size` are silently
    /// dropped (never cached).
    pub fn insert(&mut self, url: String, entry: CacheEntry) {
        let entry_size = entry.response.body.len();

        // Don't cache entries larger than the entire budget.
        if entry_size > self.max_size {
            return;
        }

        // If the URL is already cached, remove the old version first.
        if let Some(old) = self.entries.remove(&url) {
            self.current_size -= old.response.body.len();
            self.order.retain(|u| u != &url);
        }

        // Evict until there is room.
        while self.current_size + entry_size > self.max_size {
            if let Some(evicted_url) = self.order.pop_back() {
                if let Some(evicted) = self.entries.remove(&evicted_url) {
                    self.current_size -= evicted.response.body.len();
                }
            } else {
                break;
            }
        }

        self.current_size += entry_size;
        self.order.push_front(url.clone());
        self.entries.insert(url, entry);
    }

    /// Check whether `url` is present in the cache (without promoting
    /// it).
    pub fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }

    /// Drop all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.current_size = 0;
    }

    /// Current total body size in bytes.
    pub fn size(&self) -> usize {
        self.current_size
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::loader::ContentType;

    /// Helper: build a minimal `CacheEntry` with the given body size.
    fn make_entry(url: &str, size: usize) -> (String, CacheEntry) {
        let body = vec![0u8; size];
        let entry = CacheEntry {
            response: ResourceResponse {
                url: url.to_string(),
                content_type: ContentType::Html,
                body,
                status: 200,
            },
            texture: None,
        };
        (url.to_string(), entry)
    }

    #[test]
    fn insert_and_retrieve() {
        let mut cache = ResourceCache::new(1024);
        let (url, entry) = make_entry("http://a.com/1", 100);
        cache.insert(url, entry);

        assert!(cache.contains("http://a.com/1"));
        let got = cache.get("http://a.com/1").unwrap();
        assert_eq!(got.response.status, 200);
        assert_eq!(got.response.body.len(), 100);
    }

    #[test]
    fn lru_eviction_oldest_first() {
        // Cache fits exactly two 50-byte entries (max = 100).
        let mut cache = ResourceCache::new(100);
        let (u1, e1) = make_entry("http://a.com/1", 50);
        let (u2, e2) = make_entry("http://a.com/2", 50);
        cache.insert(u1, e1);
        cache.insert(u2, e2);

        // Both present.
        assert!(cache.contains("http://a.com/1"));
        assert!(cache.contains("http://a.com/2"));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.size(), 100);

        // Insert a third -- should evict the oldest (#1).
        let (u3, e3) = make_entry("http://a.com/3", 50);
        cache.insert(u3, e3);

        assert!(!cache.contains("http://a.com/1"));
        assert!(cache.contains("http://a.com/2"));
        assert!(cache.contains("http://a.com/3"));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.size(), 100);
    }

    #[test]
    fn lru_access_promotes_entry() {
        let mut cache = ResourceCache::new(100);
        let (u1, e1) = make_entry("http://a.com/1", 50);
        let (u2, e2) = make_entry("http://a.com/2", 50);
        cache.insert(u1, e1);
        cache.insert(u2, e2);

        // Access #1 to promote it.
        let _ = cache.get("http://a.com/1");

        // Now inserting #3 should evict #2 (the actual LRU).
        let (u3, e3) = make_entry("http://a.com/3", 50);
        cache.insert(u3, e3);

        assert!(cache.contains("http://a.com/1"));
        assert!(!cache.contains("http://a.com/2"));
        assert!(cache.contains("http://a.com/3"));
    }

    #[test]
    fn size_tracking() {
        let mut cache = ResourceCache::new(1024);
        let (u1, e1) = make_entry("http://a.com/1", 100);
        let (u2, e2) = make_entry("http://a.com/2", 200);
        cache.insert(u1, e1);
        cache.insert(u2, e2);
        assert_eq!(cache.size(), 300);

        cache.clear();
        assert_eq!(cache.size(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn oversized_entry_not_cached() {
        let mut cache = ResourceCache::new(50);
        let (url, entry) = make_entry("http://big.com/huge", 100);
        cache.insert(url, entry);

        assert!(!cache.contains("http://big.com/huge"));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn update_existing_entry() {
        let mut cache = ResourceCache::new(1024);
        let (url, entry) = make_entry("http://a.com/1", 100);
        cache.insert(url, entry);
        assert_eq!(cache.size(), 100);

        // Re-insert with a different size.
        let (url, entry) = make_entry("http://a.com/1", 200);
        cache.insert(url, entry);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.size(), 200);

        let got = cache.get("http://a.com/1").unwrap();
        assert_eq!(got.response.body.len(), 200);
    }

    #[test]
    fn clear_cache() {
        let mut cache = ResourceCache::new(1024);
        let (u1, e1) = make_entry("http://a.com/1", 100);
        let (u2, e2) = make_entry("http://a.com/2", 200);
        cache.insert(u1, e1);
        cache.insert(u2, e2);
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.size(), 0);
        assert!(cache.is_empty());
        assert!(!cache.contains("http://a.com/1"));
    }

    #[test]
    fn get_missing_returns_none() {
        let mut cache = ResourceCache::new(1024);
        assert!(cache.get("http://missing.com/x").is_none());
    }

    #[test]
    fn multiple_evictions_for_large_insert() {
        let mut cache = ResourceCache::new(200);
        // Insert four 50-byte entries (fills to 200).
        for i in 0..4 {
            let (u, e) = make_entry(&format!("http://a.com/{i}"), 50);
            cache.insert(u, e);
        }
        assert_eq!(cache.len(), 4);
        assert_eq!(cache.size(), 200);

        // Insert one 150-byte entry -- should evict three oldest.
        let (u, e) = make_entry("http://a.com/big", 150);
        cache.insert(u, e);

        assert_eq!(cache.size(), 200);
        assert!(cache.contains("http://a.com/big"));
        assert!(cache.contains("http://a.com/3"));
        assert!(!cache.contains("http://a.com/0"));
        assert!(!cache.contains("http://a.com/1"));
        assert!(!cache.contains("http://a.com/2"));
    }
}

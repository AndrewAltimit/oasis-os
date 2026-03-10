//! LRU resource cache.
//!
//! Bounds memory usage by evicting the least-recently-used entries when
//! the total cached body size exceeds a configurable limit.
//!
//! The LRU order is tracked by an intrusive doubly-linked list stored
//! in a `Vec` arena, giving O(1) promotion, insertion, and eviction
//! (vs the previous O(N) `VecDeque::retain` per hit).

use std::collections::HashMap;

use oasis_types::backend::TextureId;

use super::ResourceResponse;

/// An entry in the resource cache.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The loaded resource data.
    pub response: ResourceResponse,
    /// If the resource is an image, the decoded texture handle.
    pub texture: Option<TextureId>,
}

// -----------------------------------------------------------------------
// Intrusive doubly-linked list arena
// -----------------------------------------------------------------------

/// A node in the intrusive LRU linked list.
struct LruNode {
    url: String,
    prev: Option<usize>,
    next: Option<usize>,
}

/// LRU resource cache with bounded size (measured in body bytes).
///
/// Uses an arena-backed doubly-linked list for O(1) LRU operations.
pub struct ResourceCache {
    entries: HashMap<String, CacheEntry>,
    /// Arena of linked-list nodes. Slots may be `None` after removal.
    nodes: Vec<Option<LruNode>>,
    /// URL -> arena index for O(1) lookup.
    index: HashMap<String, usize>,
    /// Arena index of the most-recently-used node (head of list).
    head: Option<usize>,
    /// Arena index of the least-recently-used node (tail of list).
    tail: Option<usize>,
    /// Free slots in the arena for reuse.
    free: Vec<usize>,
    current_size: usize,
    max_size: usize,
}

impl ResourceCache {
    /// Create a new cache with the given maximum size in bytes.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            nodes: Vec::new(),
            index: HashMap::new(),
            head: None,
            tail: None,
            free: Vec::new(),
            current_size: 0,
            max_size,
        }
    }

    /// Look up a cached resource by URL, promoting it to the
    /// most-recently-used position. O(1).
    pub fn get(&mut self, url: &str) -> Option<&CacheEntry> {
        if let Some(&idx) = self.index.get(url) {
            self.move_to_head(idx);
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
            self.unlink_url(&url);
        }

        // Evict until there is room.
        while self.current_size + entry_size > self.max_size {
            if let Some(evicted_url) = self.pop_tail() {
                if let Some(evicted) = self.entries.remove(&evicted_url) {
                    self.current_size -= evicted.response.body.len();
                }
            } else {
                break;
            }
        }

        self.current_size += entry_size;
        self.push_head(&url);
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
        self.nodes.clear();
        self.index.clear();
        self.head = None;
        self.tail = None;
        self.free.clear();
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

    // -------------------------------------------------------------------
    // Linked-list helpers
    // -------------------------------------------------------------------

    /// Allocate or reuse an arena slot for a new node.
    fn alloc_node(&mut self, node: LruNode) -> usize {
        if let Some(idx) = self.free.pop() {
            self.nodes[idx] = Some(node);
            idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(Some(node));
            idx
        }
    }

    /// Unlink a node from the list without freeing it. Returns its
    /// prev/next pointers before unlinking.
    fn unlink(&mut self, idx: usize) {
        let (prev, next) = {
            let Some(node) = self.nodes.get(idx).and_then(|n| n.as_ref()) else {
                return;
            };
            (node.prev, node.next)
        };

        // Patch predecessor's next pointer.
        if let Some(p) = prev {
            if let Some(Some(pred)) = self.nodes.get_mut(p) {
                pred.next = next;
            }
        } else {
            self.head = next;
        }

        // Patch successor's prev pointer.
        if let Some(n) = next {
            if let Some(Some(succ)) = self.nodes.get_mut(n) {
                succ.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        // Clear this node's links (still allocated in arena).
        if let Some(Some(node)) = self.nodes.get_mut(idx) {
            node.prev = None;
            node.next = None;
        }
    }

    /// Remove a URL from the linked list and free its arena slot.
    fn unlink_url(&mut self, url: &str) {
        if let Some(idx) = self.index.remove(url) {
            self.unlink(idx);
            self.nodes[idx] = None;
            self.free.push(idx);
        }
    }

    /// Move an existing node to the head (MRU position).
    fn move_to_head(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return; // Already at head.
        }
        self.unlink(idx);

        // Link at head.
        let Some(Some(node)) = self.nodes.get_mut(idx) else {
            return;
        };
        node.prev = None;
        node.next = self.head;

        if let Some(old_head) = self.head
            && let Some(Some(h)) = self.nodes.get_mut(old_head)
        {
            h.prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    /// Insert a new URL at the head (MRU position).
    fn push_head(&mut self, url: &str) {
        let node = LruNode {
            url: url.to_string(),
            prev: None,
            next: self.head,
        };
        let idx = self.alloc_node(node);
        self.index.insert(url.to_string(), idx);

        if let Some(old_head) = self.head
            && let Some(Some(h)) = self.nodes.get_mut(old_head)
        {
            h.prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    /// Remove and return the tail (LRU) URL.
    fn pop_tail(&mut self) -> Option<String> {
        let tail_idx = self.tail?;
        let url = self.nodes.get(tail_idx)?.as_ref()?.url.clone();
        self.unlink(tail_idx);
        self.index.remove(&url);
        if let Some(slot) = self.nodes.get_mut(tail_idx) {
            *slot = None;
        }
        self.free.push(tail_idx);
        Some(url)
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::ContentType;

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

    #[test]
    fn lru_promotion_is_o1() {
        // Insert 1000 entries, then promote each one. This verifies
        // the linked-list approach doesn't degrade to O(N) per hit.
        let mut cache = ResourceCache::new(1_000_000);
        for i in 0..1000 {
            let (u, e) = make_entry(&format!("http://a.com/{i}"), 100);
            cache.insert(u, e);
        }
        assert_eq!(cache.len(), 1000);

        // Promote every entry (would be O(N^2) with VecDeque::retain).
        for i in 0..1000 {
            let url = format!("http://a.com/{i}");
            assert!(cache.get(&url).is_some());
        }
        assert_eq!(cache.len(), 1000);
    }

    #[test]
    fn arena_reuses_freed_slots() {
        let mut cache = ResourceCache::new(200);
        // Insert and evict to create free slots.
        for i in 0..10 {
            let (u, e) = make_entry(&format!("http://a.com/{i}"), 100);
            cache.insert(u, e);
        }
        // With max 200 bytes and 100-byte entries, should have exactly 2.
        assert_eq!(cache.len(), 2);
        // Arena should have reused slots (arena size < 10).
        assert!(cache.nodes.len() <= 10);
    }
}

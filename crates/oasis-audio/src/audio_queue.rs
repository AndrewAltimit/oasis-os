//! Bounded audio chunk queue for streaming backends.
//!
//! Models the pending-chunk queue pattern used by the WASM MSE streaming
//! backend and other streaming audio implementations. Provides bounded
//! FIFO semantics with automatic eviction of oldest chunks when the queue
//! exceeds capacity.

use std::collections::VecDeque;

/// A bounded FIFO queue for streaming audio chunks.
///
/// When the queue exceeds `max_chunks`, oldest entries are evicted on push.
/// This prevents unbounded memory growth when the audio output can't keep
/// up with incoming data (e.g. MSE SourceBuffer QuotaExceededError).
pub struct AudioChunkQueue {
    chunks: VecDeque<Vec<u8>>,
    max_chunks: usize,
    total_bytes: usize,
}

impl AudioChunkQueue {
    /// Create a new queue with the given maximum chunk count.
    pub fn new(max_chunks: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            max_chunks,
            total_bytes: 0,
        }
    }

    /// Push a chunk into the queue. If the queue exceeds `max_chunks`,
    /// the oldest chunk(s) are evicted. Returns the number of chunks evicted.
    pub fn push(&mut self, data: Vec<u8>) -> usize {
        self.total_bytes += data.len();
        self.chunks.push_back(data);
        let mut evicted = 0;
        while self.chunks.len() > self.max_chunks {
            if let Some(old) = self.chunks.pop_front() {
                self.total_bytes -= old.len();
                evicted += 1;
            }
        }
        evicted
    }

    /// Pop the oldest chunk from the queue.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let chunk = self.chunks.pop_front()?;
        self.total_bytes -= chunk.len();
        Some(chunk)
    }

    /// Push a chunk back to the front of the queue (re-queue on failure).
    pub fn push_front(&mut self, data: Vec<u8>) {
        self.total_bytes += data.len();
        self.chunks.push_front(data);
        // Don't evict on push_front -- this is a retry, not new data.
    }

    /// Number of chunks in the queue.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Total bytes across all queued chunks.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Maximum number of chunks before eviction.
    pub fn max_chunks(&self) -> usize {
        self.max_chunks
    }

    /// Clear all queued chunks.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.total_bytes = 0;
    }

    /// Peek at the oldest chunk without removing it.
    pub fn peek(&self) -> Option<&[u8]> {
        self.chunks.front().map(|v| v.as_slice())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn new_queue_is_empty() {
        let q = AudioChunkQueue::new(10);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert_eq!(q.total_bytes(), 0);
    }

    #[test]
    fn push_and_pop_fifo() {
        let mut q = AudioChunkQueue::new(10);
        q.push(vec![1, 2, 3]);
        q.push(vec![4, 5]);
        assert_eq!(q.len(), 2);
        assert_eq!(q.total_bytes(), 5);

        let first = q.pop().unwrap();
        assert_eq!(first, vec![1, 2, 3]);
        let second = q.pop().unwrap();
        assert_eq!(second, vec![4, 5]);
        assert!(q.is_empty());
        assert_eq!(q.total_bytes(), 0);
    }

    #[test]
    fn pop_empty_returns_none() {
        let mut q = AudioChunkQueue::new(10);
        assert!(q.pop().is_none());
    }

    #[test]
    fn push_evicts_oldest_when_full() {
        let mut q = AudioChunkQueue::new(3);
        q.push(vec![1]);
        q.push(vec![2]);
        q.push(vec![3]);
        assert_eq!(q.len(), 3);

        let evicted = q.push(vec![4]);
        assert_eq!(evicted, 1);
        assert_eq!(q.len(), 3);

        // Oldest (1) was evicted; next pop should give (2).
        assert_eq!(q.pop().unwrap(), vec![2]);
    }

    #[test]
    fn push_evicts_multiple() {
        let mut q = AudioChunkQueue::new(2);
        q.push(vec![1]);
        q.push(vec![2]);
        q.push(vec![3]);
        // After push(3), evicts 1, leaving [2, 3].
        assert_eq!(q.len(), 2);
        q.push(vec![4]);
        // After push(4), evicts 2, leaving [3, 4].
        assert_eq!(q.pop().unwrap(), vec![3]);
        assert_eq!(q.pop().unwrap(), vec![4]);
    }

    #[test]
    fn push_front_requeues() {
        let mut q = AudioChunkQueue::new(10);
        q.push(vec![1]);
        q.push(vec![2]);

        let chunk = q.pop().unwrap();
        assert_eq!(chunk, vec![1]);

        // Re-queue on failure.
        q.push_front(chunk);
        assert_eq!(q.len(), 2);

        // Should get the re-queued chunk back first.
        assert_eq!(q.pop().unwrap(), vec![1]);
        assert_eq!(q.pop().unwrap(), vec![2]);
    }

    #[test]
    fn total_bytes_tracking() {
        let mut q = AudioChunkQueue::new(10);
        q.push(vec![0; 100]);
        q.push(vec![0; 200]);
        assert_eq!(q.total_bytes(), 300);

        q.pop();
        assert_eq!(q.total_bytes(), 200);

        q.clear();
        assert_eq!(q.total_bytes(), 0);
    }

    #[test]
    fn eviction_updates_total_bytes() {
        let mut q = AudioChunkQueue::new(2);
        q.push(vec![0; 50]);
        q.push(vec![0; 75]);
        assert_eq!(q.total_bytes(), 125);

        // This evicts the 50-byte chunk.
        q.push(vec![0; 100]);
        assert_eq!(q.total_bytes(), 175); // 75 + 100
    }

    #[test]
    fn peek_returns_oldest() {
        let mut q = AudioChunkQueue::new(10);
        assert!(q.peek().is_none());

        q.push(vec![1, 2, 3]);
        q.push(vec![4, 5]);
        assert_eq!(q.peek().unwrap(), &[1, 2, 3]);

        // Peek doesn't consume.
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn clear_empties_queue() {
        let mut q = AudioChunkQueue::new(10);
        q.push(vec![1]);
        q.push(vec![2]);
        q.clear();
        assert!(q.is_empty());
        assert_eq!(q.total_bytes(), 0);
    }

    #[test]
    fn max_chunks_is_correct() {
        let q = AudioChunkQueue::new(42);
        assert_eq!(q.max_chunks(), 42);
    }
}

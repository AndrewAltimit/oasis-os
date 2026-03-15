//! Portable synchronization primitives.
//!
//! Provides a lock-free single-producer single-consumer (SPSC) queue and
//! related utilities that work across all platforms. On PSP, the equivalent
//! `psp::sync::SpscQueue` is used directly; this module provides a portable
//! implementation for desktop, WASM, and testing.

use std::sync::atomic::{AtomicUsize, Ordering};

/// A lock-free, bounded single-producer single-consumer (SPSC) ring buffer.
///
/// Designed for passing messages between exactly one producer thread and one
/// consumer thread without locks. Uses atomic operations for the head/tail
/// indices and a pre-allocated array for storage.
///
/// The capacity is fixed at construction time. One slot is always kept empty
/// to distinguish full from empty, so the usable capacity is `capacity - 1`.
pub struct SpscQueue<T> {
    buffer: Vec<Option<T>>,
    capacity: usize,
    head: AtomicUsize, // write position (producer)
    tail: AtomicUsize, // read position (consumer)
}

// SAFETY: SpscQueue is designed for single-producer, single-consumer use.
// The atomic head/tail indices ensure correct visibility between threads.
// The buffer slots are only written by the producer (at head) and read by
// the consumer (at tail), with no overlap when the queue is correctly used.
unsafe impl<T: Send> Send for SpscQueue<T> {}
// SAFETY: Same reasoning -- the atomics provide the necessary synchronization.
unsafe impl<T: Send> Sync for SpscQueue<T> {}

impl<T> SpscQueue<T> {
    /// Create a new SPSC queue with the given capacity.
    ///
    /// The actual usable capacity is `capacity` items. Internally one extra
    /// slot is allocated to distinguish full from empty.
    pub fn new(capacity: usize) -> Self {
        let actual_cap = capacity + 1;
        let mut buffer = Vec::with_capacity(actual_cap);
        for _ in 0..actual_cap {
            buffer.push(None);
        }
        Self {
            buffer,
            capacity: actual_cap,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Try to push an item into the queue. Returns `Err(item)` if the queue
    /// is full.
    pub fn push(&self, item: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let next_head = (head + 1) % self.capacity;
        let tail = self.tail.load(Ordering::Acquire);

        if next_head == tail {
            return Err(item); // Full.
        }

        // SAFETY: Only the producer writes to buffer[head], and we've verified
        // the slot is not occupied (next_head != tail).
        let slot = unsafe { &mut *(self.buffer.as_ptr().add(head) as *mut Option<T>) };
        *slot = Some(item);

        self.head.store(next_head, Ordering::Release);
        Ok(())
    }

    /// Try to pop an item from the queue. Returns `None` if the queue is empty.
    pub fn pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        if tail == head {
            return None; // Empty.
        }

        // SAFETY: Only the consumer reads from buffer[tail], and we've verified
        // the slot is occupied (tail != head).
        let slot = unsafe { &mut *(self.buffer.as_ptr().add(tail) as *mut Option<T>) };
        let item = slot.take();

        let next_tail = (tail + 1) % self.capacity;
        self.tail.store(next_tail, Ordering::Release);

        item
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        tail == head
    }

    /// Check if the queue is full.
    pub fn is_full(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        (head + 1) % self.capacity == tail
    }

    /// Return the usable capacity (number of items that can be stored).
    pub fn capacity(&self) -> usize {
        self.capacity - 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // ---- Basic single-threaded tests ----

    #[test]
    fn new_queue_is_empty() {
        let q = SpscQueue::<i32>::new(8);
        assert!(q.is_empty());
        assert!(!q.is_full());
        assert_eq!(q.capacity(), 8);
    }

    #[test]
    fn push_and_pop_single() {
        let q = SpscQueue::new(4);
        assert!(q.push(42).is_ok());
        assert_eq!(q.pop(), Some(42));
        assert!(q.is_empty());
    }

    #[test]
    fn push_and_pop_multiple() {
        let q = SpscQueue::new(4);
        q.push(1).unwrap();
        q.push(2).unwrap();
        q.push(3).unwrap();
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
        assert!(q.is_empty());
    }

    #[test]
    fn pop_empty_returns_none() {
        let q = SpscQueue::<i32>::new(4);
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn push_full_returns_err() {
        let q = SpscQueue::new(2);
        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());
        let result = q.push(3);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 3);
    }

    #[test]
    fn is_full_when_capacity_reached() {
        let q = SpscQueue::new(2);
        q.push(1).unwrap();
        assert!(!q.is_full());
        q.push(2).unwrap();
        assert!(q.is_full());
    }

    #[test]
    fn fifo_order() {
        let q = SpscQueue::new(8);
        for i in 0..8 {
            q.push(i).unwrap();
        }
        for i in 0..8 {
            assert_eq!(q.pop(), Some(i));
        }
    }

    #[test]
    fn wrap_around() {
        let q = SpscQueue::new(4);
        // Fill and drain to advance head/tail past the start.
        q.push(1).unwrap();
        q.push(2).unwrap();
        q.pop();
        q.pop();
        // Now head=2, tail=2. Push wraps around.
        q.push(3).unwrap();
        q.push(4).unwrap();
        q.push(5).unwrap();
        q.push(6).unwrap();
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(4));
        assert_eq!(q.pop(), Some(5));
        assert_eq!(q.pop(), Some(6));
    }

    #[test]
    fn capacity_one() {
        let q = SpscQueue::new(1);
        assert_eq!(q.capacity(), 1);
        assert!(q.push(42).is_ok());
        assert!(q.is_full());
        assert!(q.push(99).is_err());
        assert_eq!(q.pop(), Some(42));
        assert!(q.is_empty());
    }

    #[test]
    fn interleaved_push_pop() {
        let q = SpscQueue::new(2);
        for i in 0..100 {
            q.push(i).unwrap();
            assert_eq!(q.pop(), Some(i));
        }
    }

    // ---- Multi-threaded tests (SPSC pattern) ----

    #[test]
    fn spsc_producer_consumer() {
        let q = Arc::new(SpscQueue::new(64));
        let count = 10_000;

        let q_producer = Arc::clone(&q);
        let producer = thread::spawn(move || {
            for i in 0..count {
                loop {
                    if q_producer.push(i).is_ok() {
                        break;
                    }
                    // Queue full, spin.
                    thread::yield_now();
                }
            }
        });

        let q_consumer = Arc::clone(&q);
        let consumer = thread::spawn(move || {
            let mut received = Vec::with_capacity(count);
            while received.len() < count {
                if let Some(item) = q_consumer.pop() {
                    received.push(item);
                } else {
                    thread::yield_now();
                }
            }
            received
        });

        producer.join().unwrap();
        let received = consumer.join().unwrap();
        // Verify all items received in order.
        let expected: Vec<usize> = (0..count).collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn spsc_no_data_loss() {
        let q = Arc::new(SpscQueue::new(16));
        let count = 5_000;

        let q_prod = Arc::clone(&q);
        let prod = thread::spawn(move || {
            for i in 0u64..count {
                loop {
                    if q_prod.push(i).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        });

        let q_cons = Arc::clone(&q);
        let cons = thread::spawn(move || {
            let mut sum = 0u64;
            let mut n = 0u64;
            while n < count {
                if let Some(val) = q_cons.pop() {
                    sum += val;
                    n += 1;
                } else {
                    thread::yield_now();
                }
            }
            sum
        });

        prod.join().unwrap();
        let sum = cons.join().unwrap();
        let expected = count * (count - 1) / 2;
        assert_eq!(sum, expected);
    }

    #[test]
    fn spsc_small_buffer_stress() {
        // Capacity 1 forces frequent full/empty transitions.
        let q = Arc::new(SpscQueue::new(1));
        let count = 1_000;

        let q_prod = Arc::clone(&q);
        let prod = thread::spawn(move || {
            for i in 0..count {
                loop {
                    if q_prod.push(i).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        });

        let q_cons = Arc::clone(&q);
        let cons = thread::spawn(move || {
            let mut received = Vec::with_capacity(count);
            while received.len() < count {
                if let Some(item) = q_cons.pop() {
                    received.push(item);
                } else {
                    thread::yield_now();
                }
            }
            received
        });

        prod.join().unwrap();
        let received = cons.join().unwrap();
        let expected: Vec<usize> = (0..count).collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn spsc_empty_after_drain() {
        let q = SpscQueue::new(128);
        let count = 100;

        // Push all items (fits within capacity).
        for i in 0..count {
            q.push(i).unwrap();
        }

        // Drain all items.
        let mut n = 0;
        while q.pop().is_some() {
            n += 1;
        }
        assert_eq!(n, count);
        assert!(q.is_empty());
    }
}

//! Fixed-capacity ring buffer for streaming audio data.
//!
//! Accumulates incoming audio chunks from the network source and provides
//! them to the audio backend for decoding. The default capacity (128 KB)
//! balances memory usage against buffering latency.

/// Default buffer capacity in bytes (128 KB).
const DEFAULT_CAPACITY: usize = 128 * 1024;

/// Fixed-capacity ring buffer for streaming audio.
pub struct StreamBuffer {
    buf: Vec<u8>,
    capacity: usize,
    /// Write position (where new data goes).
    head: usize,
    /// Read position (where data is consumed from).
    tail: usize,
    /// Number of bytes currently stored.
    len: usize,
}

impl StreamBuffer {
    /// Create a new buffer with the default capacity (128 KB).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a new buffer with a specific capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity],
            capacity,
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    /// Write data into the buffer. Returns the number of bytes actually written.
    /// If the buffer is full, excess bytes are silently dropped.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let free = self.free_space();
        let to_write = data.len().min(free);
        if to_write == 0 {
            return 0;
        }
        let first = to_write.min(self.capacity - self.head);
        self.buf[self.head..self.head + first].copy_from_slice(&data[..first]);
        if first < to_write {
            self.buf[..to_write - first].copy_from_slice(&data[first..to_write]);
        }
        self.head = (self.head + to_write) % self.capacity;
        self.len += to_write;
        to_write
    }

    /// Read up to `dst.len()` bytes from the buffer. Returns the number of
    /// bytes actually read.
    pub fn read(&mut self, dst: &mut [u8]) -> usize {
        let to_read = dst.len().min(self.len);
        if to_read == 0 {
            return 0;
        }
        let first = to_read.min(self.capacity - self.tail);
        dst[..first].copy_from_slice(&self.buf[self.tail..self.tail + first]);
        if first < to_read {
            dst[first..to_read].copy_from_slice(&self.buf[..to_read - first]);
        }
        self.tail = (self.tail + to_read) % self.capacity;
        self.len -= to_read;
        to_read
    }

    /// Number of bytes available for reading.
    pub fn available(&self) -> usize {
        self.len
    }

    /// Number of bytes of free space for writing.
    pub fn free_space(&self) -> usize {
        self.capacity - self.len
    }

    /// Total capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear the buffer, discarding all data.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
    }

    /// Estimate buffered duration in milliseconds given a bitrate in kbps.
    /// Returns 0 if bitrate is 0.
    pub fn buffered_ms(&self, bitrate_kbps: u32) -> u64 {
        if bitrate_kbps == 0 {
            return 0;
        }
        // bytes * 8 = bits; bits / (kbps * 1000) = seconds; * 1000 = ms
        // Simplified: bytes * 8 / kbps = ms
        (self.len as u64 * 8) / bitrate_kbps as u64
    }
}

impl Default for StreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let buf = StreamBuffer::new();
        assert_eq!(buf.available(), 0);
        assert_eq!(buf.free_space(), DEFAULT_CAPACITY);
    }

    #[test]
    fn write_and_read() {
        let mut buf = StreamBuffer::with_capacity(16);
        let written = buf.write(b"hello");
        assert_eq!(written, 5);
        assert_eq!(buf.available(), 5);

        let mut out = [0u8; 5];
        let read = buf.read(&mut out);
        assert_eq!(read, 5);
        assert_eq!(&out, b"hello");
        assert_eq!(buf.available(), 0);
    }

    #[test]
    fn write_overflow_drops_excess() {
        let mut buf = StreamBuffer::with_capacity(4);
        let written = buf.write(b"abcdef");
        assert_eq!(written, 4);
        assert_eq!(buf.available(), 4);
        assert_eq!(buf.free_space(), 0);
    }

    #[test]
    fn read_partial() {
        let mut buf = StreamBuffer::with_capacity(16);
        buf.write(b"hello world");
        let mut out = [0u8; 5];
        let read = buf.read(&mut out);
        assert_eq!(read, 5);
        assert_eq!(&out, b"hello");
        assert_eq!(buf.available(), 6);
    }

    #[test]
    fn wrap_around() {
        let mut buf = StreamBuffer::with_capacity(8);
        buf.write(b"abcd");
        let mut out = [0u8; 4];
        buf.read(&mut out);
        assert_eq!(&out, b"abcd");

        // Now head=4, tail=4, write wraps around.
        buf.write(b"efghij");
        assert_eq!(buf.available(), 6);

        let mut out2 = [0u8; 6];
        buf.read(&mut out2);
        assert_eq!(&out2, b"efghij");
    }

    #[test]
    fn clear_resets() {
        let mut buf = StreamBuffer::with_capacity(16);
        buf.write(b"data");
        assert_eq!(buf.available(), 4);
        buf.clear();
        assert_eq!(buf.available(), 0);
        assert_eq!(buf.free_space(), 16);
    }

    #[test]
    fn buffered_ms_calculation() {
        let mut buf = StreamBuffer::with_capacity(1024);
        buf.write(&[0u8; 128]); // 128 bytes
        // 128 bytes * 8 bits / 128 kbps = 8 ms
        assert_eq!(buf.buffered_ms(128), 8);
    }

    #[test]
    fn buffered_ms_zero_bitrate() {
        let mut buf = StreamBuffer::with_capacity(1024);
        buf.write(&[0u8; 128]);
        assert_eq!(buf.buffered_ms(0), 0);
    }

    #[test]
    fn read_empty_returns_zero() {
        let mut buf = StreamBuffer::with_capacity(16);
        let mut out = [0u8; 4];
        assert_eq!(buf.read(&mut out), 0);
    }

    #[test]
    fn multiple_write_read_cycles() {
        let mut buf = StreamBuffer::with_capacity(8);
        for i in 0u8..20 {
            let data = [i; 3];
            buf.write(&data);
            let mut out = [0u8; 3];
            let n = buf.read(&mut out);
            assert_eq!(n, 3);
            assert_eq!(out, data);
        }
    }

    #[test]
    fn capacity_is_correct() {
        let buf = StreamBuffer::with_capacity(42);
        assert_eq!(buf.capacity(), 42);
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn write_read_preserves_data(data in proptest::collection::vec(any::<u8>(), 0..256)) {
                let mut buf = StreamBuffer::with_capacity(512);
                let written = buf.write(&data);
                prop_assert_eq!(written, data.len());

                let mut out = vec![0u8; data.len()];
                let read = buf.read(&mut out);
                prop_assert_eq!(read, data.len());
                prop_assert_eq!(out, data);
            }

            #[test]
            fn available_plus_free_equals_capacity(
                writes in proptest::collection::vec(
                    proptest::collection::vec(any::<u8>(), 0..32),
                    0..10,
                ),
            ) {
                let cap = 64;
                let mut buf = StreamBuffer::with_capacity(cap);
                for w in &writes {
                    buf.write(w);
                }
                prop_assert_eq!(buf.available() + buf.free_space(), cap);
            }

            #[test]
            fn write_never_exceeds_capacity(data in proptest::collection::vec(any::<u8>(), 0..512)) {
                let cap = 32;
                let mut buf = StreamBuffer::with_capacity(cap);
                buf.write(&data);
                prop_assert!(buf.available() <= cap);
            }
        }
    }
}

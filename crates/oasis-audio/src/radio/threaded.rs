//! Background-thread wrapper for radio sources.
//!
//! `ThreadedSource` decorates any [`RadioSource`] with a dedicated pump
//! thread that performs all network I/O (connect, request write, socket
//! reads) and accumulates decoded chunks in a bounded readahead queue.
//! The main-loop `poll()` becomes a lock-free `try_recv` with no
//! syscalls, so frame hitches no longer starve audio ingest and network
//! jitter (TLS record bursts, CDN stalls) no longer blocks the frame
//! loop.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};

use oasis_types::error::{OasisError, Result};

use super::source::{AudioChunk, RadioSource, SourceState};

/// Readahead cap in bytes. At a typical 128 kbps MP3 stream (16 KB/s)
/// this holds ~16 s of audio between the network and the frame loop.
const READAHEAD_BYTES: usize = 256 * 1024;

/// Pump-thread sleep when the socket has no data ready.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// Pump-thread sleep when the readahead queue is full.
const FULL_BACKOFF: std::time::Duration = std::time::Duration::from_millis(20);

// SourceState encoding for the shared atomic.
const STATE_CONNECTING: u8 = 0;
const STATE_ACTIVE: u8 = 1;
const STATE_ENDED: u8 = 2;
const STATE_ERROR: u8 = 3;

fn decode_state(v: u8) -> SourceState {
    match v {
        STATE_CONNECTING => SourceState::Connecting,
        STATE_ACTIVE => SourceState::Active,
        STATE_ENDED => SourceState::Ended,
        _ => SourceState::Error,
    }
}

/// State shared between the pump thread and the main-loop handle.
struct Shared {
    state: AtomicU8,
    stop: AtomicBool,
    /// Bytes currently queued between pump and consumer.
    buffered: AtomicUsize,
    /// Error message from the pump thread ("backend error: " prefix
    /// already stripped so re-wrapping doesn't double it).
    error: Mutex<Option<String>>,
    /// Inner source's `source_type()`, set once the source exists.
    source_type: OnceLock<String>,
    /// Inner source's `streaming_url()`, set once the source exists.
    url: OnceLock<String>,
}

/// A [`RadioSource`] that runs its inner source on a background thread.
pub struct ThreadedSource {
    shared: Arc<Shared>,
    rx: Receiver<AudioChunk>,
}

impl ThreadedSource {
    /// Wrap an already-connected source; all further I/O happens on the
    /// pump thread.
    pub fn spawn_from(inner: Box<dyn RadioSource + Send>) -> Self {
        Self::spawn_connect_inner(move |_| Ok(inner))
    }

    /// Run `connect` on the background thread to build the source (TCP +
    /// TLS + header exchange stay off the main loop), then pump it.
    pub fn spawn_connect<F>(connect: F) -> Self
    where
        F: FnOnce() -> std::result::Result<Box<dyn RadioSource + Send>, String> + Send + 'static,
    {
        Self::spawn_connect_inner(move |_| connect())
    }

    fn spawn_connect_inner<F>(connect: F) -> Self
    where
        F: FnOnce(&Shared) -> std::result::Result<Box<dyn RadioSource + Send>, String>
            + Send
            + 'static,
    {
        let shared = Arc::new(Shared {
            state: AtomicU8::new(STATE_CONNECTING),
            stop: AtomicBool::new(false),
            buffered: AtomicUsize::new(0),
            error: Mutex::new(None),
            source_type: OnceLock::new(),
            url: OnceLock::new(),
        });
        let (tx, rx) = channel();
        let thread_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let inner = match connect(&thread_shared) {
                Ok(src) => src,
                Err(msg) => {
                    thread_shared.fail(&msg);
                    return;
                },
            };
            let _ = thread_shared.source_type.set(inner.source_type().into());
            if let Some(url) = inner.streaming_url() {
                let _ = thread_shared.url.set(url.to_string());
            }
            pump(inner, &thread_shared, &tx);
        });
        Self { shared, rx }
    }

    /// Bytes currently buffered ahead of the consumer.
    pub fn buffered_bytes(&self) -> usize {
        self.shared.buffered.load(Ordering::Acquire)
    }
}

impl Shared {
    fn fail(&self, msg: &str) {
        let stripped = msg.strip_prefix("backend error: ").unwrap_or(msg);
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(stripped.to_string());
        self.state.store(STATE_ERROR, Ordering::Release);
    }
}

/// Pump loop: poll the inner source, forward chunks, honour readahead
/// backpressure and the stop flag.
fn pump(mut inner: Box<dyn RadioSource + Send>, shared: &Shared, tx: &Sender<AudioChunk>) {
    loop {
        if shared.stop.load(Ordering::Acquire) {
            inner.disconnect();
            return;
        }
        if shared.buffered.load(Ordering::Acquire) >= READAHEAD_BYTES {
            std::thread::sleep(FULL_BACKOFF);
            continue;
        }
        match inner.poll() {
            Ok(Some(chunk)) => {
                if shared.state.load(Ordering::Acquire) == STATE_CONNECTING {
                    shared.state.store(STATE_ACTIVE, Ordering::Release);
                }
                shared
                    .buffered
                    .fetch_add(chunk.data.len(), Ordering::AcqRel);
                if tx.send(chunk).is_err() {
                    // Consumer dropped -- shut down quietly.
                    inner.disconnect();
                    return;
                }
                // No sleep: drain the socket while data is flowing.
            },
            Ok(None) => match inner.state() {
                SourceState::Ended => {
                    shared.state.store(STATE_ENDED, Ordering::Release);
                    return;
                },
                SourceState::Error => {
                    shared.fail("source error");
                    return;
                },
                s => {
                    if s == SourceState::Active
                        && shared.state.load(Ordering::Acquire) == STATE_CONNECTING
                    {
                        shared.state.store(STATE_ACTIVE, Ordering::Release);
                    }
                    std::thread::sleep(IDLE_POLL);
                },
            },
            Err(e) => {
                shared.fail(&format!("{e}"));
                return;
            },
        }
    }
}

impl RadioSource for ThreadedSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        match self.rx.try_recv() {
            Ok(chunk) => {
                self.shared
                    .buffered
                    .fetch_sub(chunk.data.len(), Ordering::AcqRel);
                Ok(Some(chunk))
            },
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {
                // Queue drained: surface the pump thread's terminal state.
                if self.shared.state.load(Ordering::Acquire) == STATE_ERROR {
                    let msg = self
                        .shared
                        .error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                        .unwrap_or_else(|| "stream error".to_string());
                    return Err(OasisError::Backend(msg.into()));
                }
                Ok(None)
            },
        }
    }

    fn disconnect(&mut self) {
        // Signal the pump thread and detach: joining here could block the
        // main loop on an in-flight connect (up to its own timeout), and
        // an orphaned pump exits on its own once poll/connect returns.
        self.shared.stop.store(true, Ordering::Release);
    }

    fn state(&self) -> SourceState {
        let s = decode_state(self.shared.state.load(Ordering::Acquire));
        // Report queued chunks before surfacing Ended so the consumer
        // drains the tail of the stream.
        if s == SourceState::Ended && self.shared.buffered.load(Ordering::Acquire) > 0 {
            return SourceState::Active;
        }
        s
    }

    fn source_type(&self) -> &str {
        self.shared
            .source_type
            .get()
            .map_or("threaded", |s| s.as_str())
    }

    fn streaming_url(&self) -> Option<&str> {
        self.shared.url.get().map(|s| s.as_str())
    }
}

impl Drop for ThreadedSource {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radio::source::VfsSource;

    fn drain(src: &mut ThreadedSource, max_polls: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..max_polls {
            match src.poll() {
                Ok(Some(chunk)) => out.extend_from_slice(&chunk.data),
                Ok(None) => {
                    if src.state() == SourceState::Ended {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                },
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        out
    }

    #[test]
    fn delivers_all_data_and_ends() {
        let data: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();
        let mut src = ThreadedSource::spawn_from(Box::new(VfsSource::new(data.clone())));
        let out = drain(&mut src, 100_000);
        assert_eq!(out, data);
        assert_eq!(src.state(), SourceState::Ended);
    }

    #[test]
    fn readahead_backpressure_holds_queue_bounded() {
        // 1 MB source: pump must stop around READAHEAD_BYTES if we don't
        // consume.
        let data = vec![0xAB; 1024 * 1024];
        let src = ThreadedSource::spawn_from(Box::new(VfsSource::new(data)));
        // Give the pump time to fill the queue.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while src.buffered_bytes() < READAHEAD_BYTES && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Small slack: one chunk may be in flight past the cap.
        assert!(
            src.buffered_bytes() <= READAHEAD_BYTES + 8192,
            "buffered {} exceeded readahead cap",
            src.buffered_bytes()
        );
    }

    #[test]
    fn connect_failure_surfaces_error() {
        let mut src = ThreadedSource::spawn_connect(|| Err("connect refused".to_string()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match src.poll() {
                Err(e) => {
                    assert!(format!("{e}").contains("connect refused"));
                    break;
                },
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                },
                other => panic!("expected error, got {other:?}"),
            }
        }
        assert_eq!(src.state(), SourceState::Error);
    }

    #[test]
    fn error_message_strips_backend_prefix_once() {
        // Redirect errors from ArchiveSource format as
        // "backend error: redirect:<url>"; callers strip one prefix and
        // then match "redirect:". The wrapper must not double the prefix.
        struct RedirectSource;
        impl RadioSource for RedirectSource {
            fn poll(&mut self) -> Result<Option<AudioChunk>> {
                Err(OasisError::Backend(
                    "redirect:https://cdn.example.com/x.mp3".into(),
                ))
            }
            fn disconnect(&mut self) {}
            fn state(&self) -> SourceState {
                SourceState::Error
            }
            fn source_type(&self) -> &str {
                "redirect-test"
            }
        }
        let mut src = ThreadedSource::spawn_from(Box::new(RedirectSource));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match src.poll() {
                Err(e) => {
                    let msg = format!("{e}");
                    let inner = msg.strip_prefix("backend error: ").unwrap_or(&msg);
                    assert!(
                        inner.starts_with("redirect:"),
                        "prefix not stripped exactly once: {msg}"
                    );
                    break;
                },
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                },
                other => panic!("expected redirect error, got {other:?}"),
            }
        }
    }

    #[test]
    fn ended_reported_only_after_drain() {
        let data = vec![0x55; 8192];
        let mut src = ThreadedSource::spawn_from(Box::new(VfsSource::new(data)));
        // Wait until the pump finishes the (tiny) source.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while src.shared.state.load(Ordering::Acquire) != STATE_ENDED
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Chunks are still queued: state must read Active until drained.
        assert_eq!(src.state(), SourceState::Active);
        let out = drain(&mut src, 10_000);
        assert_eq!(out.len(), 8192);
        assert_eq!(src.state(), SourceState::Ended);
    }

    #[test]
    fn source_type_passthrough() {
        let src = ThreadedSource::spawn_from(Box::new(VfsSource::new(vec![0; 16])));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while src.source_type() == "threaded" && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(src.source_type(), "vfs");
    }

    #[test]
    fn disconnect_stops_pump() {
        let data = vec![0xCD; 1024 * 1024];
        let mut src = ThreadedSource::spawn_from(Box::new(VfsSource::new(data)));
        src.disconnect();
        assert!(src.shared.stop.load(Ordering::Acquire));
    }
}

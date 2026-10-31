//! Minimal HTTP/1.x plumbing shared by the network sources: response-head
//! parsing, the header size cap, and bounded request writes.

use std::time::{Duration, Instant};

use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

/// A parsed HTTP (or ICY) response head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HttpHead {
    /// Status code from the first line (`HTTP/1.1 200 OK`, `ICY 200 OK`);
    /// `None` when the status line is malformed.
    pub status: Option<u16>,
    /// The head as text (status line + header lines, without the blank
    /// line), lossily decoded.
    pub text: String,
    /// Offset of the first body byte in the input buffer.
    pub body_offset: usize,
}

impl HttpHead {
    /// Value of the first header named `name` (case-insensitive), trimmed.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.text.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Parse an HTTP response head from the start of `buf`.
///
/// Returns `None` until the terminating `\r\n\r\n` has arrived. The
/// separator is found in the raw bytes so a non-UTF-8 body (MP3 frames)
/// cannot shift the body offset.
pub(super) fn parse_http_head(buf: &[u8]) -> Option<HttpHead> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let text = String::from_utf8_lossy(&buf[..end]).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok());
    Some(HttpHead {
        status,
        text,
        body_offset: end + 4,
    })
}

/// Most response-header bytes a source buffers while waiting for the
/// blank line that ends them.  A server that never sends one (or streams
/// garbage) would otherwise grow the buffer forever, rescanning all of it
/// on every poll.
pub(super) const MAX_HEADER_BYTES: usize = 16 * 1024;

/// The error for a response whose headers exceed [`MAX_HEADER_BYTES`].
pub(super) fn headers_too_large() -> OasisError {
    OasisError::Backend(format!("response headers exceed {MAX_HEADER_BYTES} bytes").into())
}

/// How long a request write may keep hitting `WouldBlock` before the peer
/// is treated as dead. A request is a few hundred bytes, so a healthy
/// socket accepts it on the first write.
const REQUEST_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Write a request to a non-blocking `stream`, retrying `WouldBlock`
/// until [`REQUEST_WRITE_TIMEOUT`] elapses.
pub(super) fn write_request(stream: &mut dyn NetworkStream, bytes: &[u8]) -> Result<()> {
    write_all_bounded(stream, bytes, REQUEST_WRITE_TIMEOUT)
}

/// Write all of `bytes` to a non-blocking `stream`, retrying `WouldBlock`
/// until `timeout` elapses.
fn write_all_bounded(
    stream: &mut dyn NetworkStream,
    bytes: &[u8],
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut written = 0;
    while written < bytes.len() {
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(OasisError::Backend("request write: peer closed".into())),
            Ok(n) => written += n,
            Err(e) if e.is_would_block() => {
                if Instant::now() >= deadline {
                    return Err(OasisError::Backend("request write timed out".into()));
                }
                std::thread::sleep(Duration::from_millis(1));
            },
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read one chunk from a non-blocking `stream` into `buf`.
///
/// `Ok(Some(0))` is end of stream, `Ok(None)` means no data yet.
pub(super) fn read_some(stream: &mut dyn NetworkStream, buf: &mut [u8]) -> Result<Option<usize>> {
    match stream.read(buf) {
        Ok(n) => Ok(Some(n)),
        Err(e) if e.is_would_block() => Ok(None),
        Err(e) => Err(OasisError::Backend(format!("stream read: {e}").into())),
    }
}

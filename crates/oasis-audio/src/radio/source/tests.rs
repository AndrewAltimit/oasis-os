use super::http::{MAX_HEADER_BYTES, parse_http_head};
use super::*;
use oasis_types::error::OasisError;

#[test]
fn vfs_source_delivers_chunks() {
    let data = vec![0xAAu8; 10_000];
    let mut src = VfsSource::new(data.clone()).with_chunk_size(4096);

    assert_eq!(src.state(), SourceState::Active);
    assert_eq!(src.source_type(), "vfs");

    let chunk1 = src.poll().unwrap().unwrap();
    assert_eq!(chunk1.data.len(), 4096);
    assert!(chunk1.metadata.is_none());

    let chunk2 = src.poll().unwrap().unwrap();
    assert_eq!(chunk2.data.len(), 4096);

    let chunk3 = src.poll().unwrap().unwrap();
    assert_eq!(chunk3.data.len(), 1808); // Remaining.

    let chunk4 = src.poll().unwrap();
    assert!(chunk4.is_none());
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn vfs_source_empty_data() {
    let mut src = VfsSource::new(Vec::new());
    assert!(src.poll().unwrap().is_none());
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn vfs_source_disconnect() {
    let mut src = VfsSource::new(vec![0; 100]);
    src.disconnect();
    assert_eq!(src.state(), SourceState::Ended);
    assert!(src.poll().unwrap().is_none());
}

#[test]
fn vfs_source_custom_chunk_size() {
    let data = vec![0u8; 100];
    let mut src = VfsSource::new(data).with_chunk_size(25);
    let chunk = src.poll().unwrap().unwrap();
    assert_eq!(chunk.data.len(), 25);
}

#[test]
fn vfs_source_total_data_matches() {
    let data = vec![0xBBu8; 5000];
    let mut src = VfsSource::new(data.clone()).with_chunk_size(1024);
    let mut total = Vec::new();
    while let Some(chunk) = src.poll().unwrap() {
        total.extend_from_slice(&chunk.data);
    }
    assert_eq!(total.len(), 5000);
    assert_eq!(total, data);
}

/// Mock network stream that replays pre-loaded data.
struct MockStream {
    data: Vec<u8>,
    pos: usize,
    written: Vec<u8>,
}

impl MockStream {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            pos: 0,
            written: Vec::new(),
        }
    }
}

impl oasis_types::backend::NetworkStream for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.pos >= self.data.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.written.extend_from_slice(data);
        Ok(data.len())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn archive_source_sends_request_and_parses_headers() {
    let body = vec![0xFFu8; 100];
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec();
    response.extend_from_slice(&body);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "archive.org", "/download/x/y.mp3", "Song", "Artist");

    assert_eq!(src.state(), SourceState::Connecting);
    assert_eq!(src.source_type(), "archive");

    // First poll sends request, transitions to Active.
    let r = src.poll().unwrap();
    assert!(r.is_none());
    assert_eq!(src.state(), SourceState::Active);

    // Subsequent polls return audio data.
    let mut total = Vec::new();
    while let Some(chunk) = src.poll().unwrap() {
        total.extend_from_slice(&chunk.data);
    }
    assert_eq!(total.len(), 100);
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn request_targets_with_whitespace_or_controls_are_refused() {
    // Nothing is written for a path / host that would split the request
    // line (header injection); the source fails instead.
    for (host, path) in [
        ("h", "/a\r\nX-Injected: 1"),
        ("h", "/a b"),
        ("h\r\nX: 1", "/"),
        ("h", "relative"),
        ("", "/"),
    ] {
        let mut icecast = IcecastSource::new(Box::new(MockStream::new(Vec::new())), host, path);
        assert!(icecast.poll().is_err(), "{host:?} {path:?}");
        assert_eq!(icecast.state(), SourceState::Error);
        let stream = Box::new(MockStream::new(Vec::new()));
        let mut archive = ArchiveSource::new(stream, host, path, "T", "");
        assert!(archive.poll().is_err(), "{host:?} {path:?}");
        assert_eq!(archive.state(), SourceState::Error);
    }
    assert!(super::http::is_request_token("/ok?x=%20"));
}

#[test]
fn archive_redirect_without_location_is_an_error() {
    // Its body is an HTML stub, not audio.
    let response = b"HTTP/1.1 302 Found\r\n\r\n<html>moved</html>".to_vec();
    let mut src = ArchiveSource::new(Box::new(MockStream::new(response)), "h", "/p", "T", "");
    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    assert!(err.to_string().contains("HTTP 302"), "{err}");
    assert_eq!(src.state(), SourceState::Error);
}

#[test]
fn archive_source_metadata_on_first_chunk() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 100]);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "archive.org", "/download/x/y.mp3", "Song", "Artist");

    // Send request.
    src.poll().unwrap();

    // First data chunk should have metadata.
    let chunk = src.poll().unwrap().unwrap();
    assert!(chunk.metadata.is_some());
    assert_eq!(chunk.metadata.unwrap().title, "Artist - Song");

    // Drain remaining (may get more data or end).
    while let Some(chunk) = src.poll().unwrap() {
        // Subsequent chunks should NOT have metadata.
        assert!(chunk.metadata.is_none());
    }
}

#[test]
fn archive_source_content_length_tracking() {
    let body = vec![0xBB; 50];
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\n".to_vec();
    response.extend_from_slice(&body);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "");

    src.poll().unwrap(); // Send request.
    let mut received = 0;
    while let Some(chunk) = src.poll().unwrap() {
        received += chunk.data.len();
    }
    assert_eq!(received, 50);
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn archive_source_disconnect() {
    let response = b"HTTP/1.1 200 OK\r\n\r\ndata".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");
    src.disconnect();
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn archive_source_302_redirect() {
    let response =
        b"HTTP/1.1 302 Found\r\nLocation: https://cdn.archive.org/file.mp3\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "archive.org", "/download/x/y.mp3", "T", "C");

    // Connecting → Active.
    src.poll().unwrap();
    // Header parse returns redirect error.
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("redirect:"),
        "expected redirect error, got: {msg}"
    );
    assert!(msg.contains("cdn.archive.org"));
    assert_eq!(src.state(), SourceState::Error);
}

#[test]
fn archive_source_404_error() {
    let response = b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Connecting → Active.
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("HTTP 404"),
        "expected HTTP 404 error, got: {msg}"
    );
    assert_eq!(src.state(), SourceState::Error);
}

#[test]
fn archive_source_500_error() {
    let response = b"HTTP/1.1 500 Internal Server Error\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("HTTP 500"),
        "expected HTTP 500 error, got: {msg}"
    );
}

#[test]
fn archive_source_streams_without_content_length() {
    // No Content-Length header → stream until EOF.
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 200]);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Connecting.
    let mut total = 0;
    while let Some(chunk) = src.poll().unwrap() {
        total += chunk.data.len();
    }
    assert_eq!(total, 200);
    assert_eq!(src.state(), SourceState::Ended);
}

// ---------------------------------------------------------------
// Redirect error format tests (critical for connect_archive_source)
// ---------------------------------------------------------------

#[test]
fn redirect_error_format_has_backend_prefix() {
    // Verify the exact format of redirect errors — callers depend on this.
    let response =
        b"HTTP/1.1 302 Found\r\nLocation: https://cdn.example.com/file.mp3\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Connecting → Active.
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");

    // OasisError::Backend("redirect:...") formats with "backend error: " prefix.
    assert!(
        msg.starts_with("backend error: redirect:"),
        "unexpected format: {msg}"
    );
    // Callers strip prefix then check for "redirect:".
    let inner = msg.strip_prefix("backend error: ").unwrap();
    let url = inner.strip_prefix("redirect:").unwrap();
    assert_eq!(url, "https://cdn.example.com/file.mp3");
}

#[test]
fn redirect_301_permanent() {
    let response =
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://new.host/a.mp3\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    let inner = msg.strip_prefix("backend error: ").unwrap();
    assert!(inner.starts_with("redirect:"));
    assert!(inner.contains("new.host"));
}

#[test]
fn redirect_307_temporary() {
    let response =
        b"HTTP/1.1 307 Temporary Redirect\r\nLocation: https://tmp.host/b.mp3\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    // 307 has no Location extraction because status_code >= 400 check doesn't catch it,
    // but redirect_url IS set via Location header parsing.
    let inner = msg.strip_prefix("backend error: ").unwrap();
    assert!(inner.starts_with("redirect:"));
}

#[test]
fn http_error_format() {
    let response = b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.starts_with("backend error: HTTP 403"),
        "unexpected format: {msg}"
    );
}

// ---------------------------------------------------------------
// Partial header accumulation tests
// ---------------------------------------------------------------

/// Mock stream that delivers data in configurable-sized chunks.
struct ChunkedMockStream {
    data: Vec<u8>,
    pos: usize,
    chunk_size: usize,
    written: Vec<u8>,
}

impl ChunkedMockStream {
    fn new(data: Vec<u8>, chunk_size: usize) -> Self {
        Self {
            data,
            pos: 0,
            chunk_size,
            written: Vec::new(),
        }
    }
}

impl oasis_types::backend::NetworkStream for ChunkedMockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.pos >= self.data.len() {
            return Ok(0);
        }
        let available = self.data.len() - self.pos;
        let n = buf.len().min(available).min(self.chunk_size);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.written.extend_from_slice(data);
        Ok(data.len())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn archive_source_partial_headers_across_polls() {
    // Headers arrive in small chunks (e.g. slow network).
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xCC; 50]);

    // Deliver 10 bytes at a time — headers span multiple reads.
    let stream = Box::new(ChunkedMockStream::new(response, 10));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    // First poll sends request.
    src.poll().unwrap();

    // Keep polling until we get data (headers accumulate across polls).
    let mut got_data = false;
    let mut total = 0;
    for _ in 0..50 {
        match src.poll().unwrap() {
            Some(chunk) => {
                got_data = true;
                total += chunk.data.len();
            },
            None => {
                if src.state() == SourceState::Ended {
                    break;
                }
            },
        }
    }
    assert!(got_data, "should have received audio data");
    assert_eq!(total, 50);
}

#[test]
fn archive_source_partial_headers_redirect() {
    // Redirect response arrives in small chunks.
    let response =
        b"HTTP/1.1 302 Found\r\nLocation: https://cdn.example.com/file.mp3\r\n\r\n".to_vec();
    // 5 bytes at a time.
    let stream = Box::new(ChunkedMockStream::new(response, 5));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Send request.

    // Poll until redirect error is detected.
    let mut found_redirect = false;
    for _ in 0..50 {
        match src.poll() {
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("redirect:"), "unexpected error: {msg}");
                found_redirect = true;
                break;
            },
            Ok(_) => continue,
        }
    }
    assert!(found_redirect, "should have detected redirect");
}

// ---------------------------------------------------------------
// WouldBlock handling
// ---------------------------------------------------------------

/// Mock stream that returns WouldBlock on every other read.
struct WouldBlockStream {
    inner: MockStream,
    call_count: usize,
}

impl WouldBlockStream {
    fn new(data: Vec<u8>) -> Self {
        Self {
            inner: MockStream::new(data),
            call_count: 0,
        }
    }
}

impl oasis_types::backend::NetworkStream for WouldBlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.call_count += 1;
        if self.call_count % 2 == 1 {
            return Err(OasisError::Backend("WouldBlock".into()));
        }
        self.inner.read(buf)
    }
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.inner.write(data)
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn archive_source_handles_would_block() {
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xDD; 100]);

    let stream = Box::new(WouldBlockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Send request.

    // Poll with WouldBlock interspersed — should still eventually get all data.
    let mut total = 0;
    for _ in 0..200 {
        match src.poll() {
            Ok(Some(chunk)) => total += chunk.data.len(),
            Ok(None) => {
                if src.state() == SourceState::Ended {
                    break;
                }
            },
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    assert_eq!(total, 100);
}

// ---------------------------------------------------------------
// Metadata formatting edge cases
// ---------------------------------------------------------------

#[test]
fn archive_source_metadata_title_only() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 50]);

    // Empty creator → title only (no " - " prefix).
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "My Song", "");

    src.poll().unwrap();
    let chunk = src.poll().unwrap().unwrap();
    let meta = chunk.metadata.unwrap();
    assert_eq!(meta.title, "My Song");
}

#[test]
fn archive_source_metadata_creator_and_title() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 50]);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "Concerto", "Bach");

    src.poll().unwrap();
    let chunk = src.poll().unwrap().unwrap();
    let meta = chunk.metadata.unwrap();
    assert_eq!(meta.title, "Bach - Concerto");
}

#[test]
fn archive_source_metadata_sent_only_once() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 10000]);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "Title", "Artist");

    src.poll().unwrap(); // Send request.

    let mut meta_count = 0;
    for _ in 0..100 {
        match src.poll().unwrap() {
            Some(chunk) => {
                if chunk.metadata.is_some() {
                    meta_count += 1;
                }
            },
            None => break,
        }
    }
    assert_eq!(meta_count, 1, "metadata should be sent exactly once");
}

// ---------------------------------------------------------------
// Full lifecycle tests
// ---------------------------------------------------------------

#[test]
fn archive_source_full_lifecycle() {
    // Test complete lifecycle: Connecting → Active → data → Ended.
    let body = vec![0xEE; 1000];
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec();
    response.extend_from_slice(&body);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(
        stream,
        "archive.org",
        "/download/item/file.mp3",
        "Song",
        "Band",
    );

    // Phase 1: Connecting.
    assert_eq!(src.state(), SourceState::Connecting);
    let r = src.poll().unwrap();
    assert!(r.is_none());
    assert_eq!(src.state(), SourceState::Active);

    // Phase 2: Streaming data.
    let mut total_data = Vec::new();
    let mut got_metadata = false;
    while let Some(chunk) = src.poll().unwrap() {
        if let Some(ref meta) = chunk.metadata {
            assert_eq!(meta.title, "Band - Song");
            got_metadata = true;
        }
        total_data.extend_from_slice(&chunk.data);
    }

    // Phase 3: Verification.
    assert!(got_metadata, "should have received metadata");
    assert_eq!(total_data.len(), 1000);
    assert_eq!(total_data, body);
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn archive_source_large_file_content_length() {
    // Simulate a 64KB file download.
    let body = vec![0x42; 65536];
    let mut response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(&body);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "Big File", "");

    src.poll().unwrap(); // Send request.
    let mut received = 0;
    let mut polls = 0;
    while let Some(chunk) = src.poll().unwrap() {
        received += chunk.data.len();
        polls += 1;
    }
    assert_eq!(received, 65536);
    assert!(polls > 1, "should take multiple polls for 64KB");
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn archive_source_disconnect_during_streaming() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAA; 10000]);

    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(stream, "host", "/path", "T", "C");

    src.poll().unwrap(); // Connecting → Active.
    let _ = src.poll().unwrap(); // Read some data.

    // Disconnect mid-stream.
    src.disconnect();
    assert_eq!(src.state(), SourceState::Ended);
    // Further polls should return None.
    assert!(src.poll().unwrap().is_none());
}

#[test]
fn archive_source_verifies_request_format() {
    let response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = ArchiveSource::new(
        stream,
        "archive.org",
        "/download/item-123/my%20song.mp3",
        "T",
        "C",
    );

    src.poll().unwrap();

    // Inspect what was written to the stream (the HTTP request).
    // We can't easily access MockStream's `written` field after boxing,
    // so this test verifies the source transitions correctly.
    assert_eq!(src.state(), SourceState::Active);
}

// ---------------------------------------------------------------
// Icecast source tests
// ---------------------------------------------------------------

#[test]
fn icecast_source_sends_request_with_icy_headers() {
    let response = b"HTTP/1.0 200 OK\r\nicy-metaint:8192\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "radio.example.com", "/stream.mp3");

    assert_eq!(src.state(), SourceState::Connecting);
    assert_eq!(src.source_type(), "icecast");

    // First poll sends request.
    let r = src.poll().unwrap();
    assert!(r.is_none());
    assert_eq!(src.state(), SourceState::Active);
}

#[test]
fn endless_response_headers_are_an_error() {
    // Regression: a server that never ends its headers grew the header
    // buffer without bound (and rescanned all of it every poll).
    let garbage = vec![b'x'; MAX_HEADER_BYTES + 8192];
    let mut icecast = IcecastSource::new(Box::new(MockStream::new(garbage.clone())), "h", "/");
    let mut archive = ArchiveSource::new(Box::new(MockStream::new(garbage)), "h", "/", "T", "A");
    for src in [&mut icecast as &mut dyn RadioSource, &mut archive] {
        src.poll().unwrap(); // Send the request.
        let err = loop {
            match src.poll() {
                Ok(_) => continue,
                Err(e) => break e,
            }
        };
        assert!(err.to_string().contains("headers exceed"), "{err}");
        assert_eq!(src.state(), SourceState::Error);
    }
}

#[test]
fn icecast_source_no_icy_metadata() {
    // Response without icy-metaint → treat all data as audio.
    let mut response = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xFF; 100]);

    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/stream");

    src.poll().unwrap(); // Send request.
    let chunk = src.poll().unwrap().unwrap();
    assert!(!chunk.data.is_empty());
    assert!(chunk.metadata.is_none());
}

#[test]
fn icecast_source_disconnect() {
    let response = b"HTTP/1.0 200 OK\r\n\r\ndata".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/stream");

    src.disconnect();
    assert_eq!(src.state(), SourceState::Ended);
    assert!(src.poll().unwrap().is_none());
}

#[test]
fn icecast_source_eof_transitions_to_ended() {
    let response = b"HTTP/1.0 200 OK\r\n\r\nshort".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/stream");

    src.poll().unwrap(); // Send request.

    // Drain all data.
    for _ in 0..10 {
        if src.poll().unwrap().is_none() && src.state() == SourceState::Ended {
            break;
        }
    }
    assert_eq!(src.state(), SourceState::Ended);
}

#[test]
fn icecast_source_rejects_http_error_status() {
    let mut response = b"HTTP/1.0 404 Not Found\r\nContent-Type: text/html\r\n\r\n".to_vec();
    response.extend_from_slice(b"<html>not audio</html>");
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/missing");

    src.poll().unwrap(); // Send request.
    let err = src.poll().unwrap_err();
    assert!(format!("{err}").contains("HTTP 404"), "got: {err}");
    assert_eq!(src.state(), SourceState::Error);
}

#[test]
fn icecast_source_reports_redirect() {
    let response = b"HTTP/1.1 302 Found\r\nLocation: http://cdn.example.com/s.mp3\r\n\r\n".to_vec();
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/stream");

    src.poll().unwrap();
    let err = src.poll().unwrap_err();
    assert!(
        format!("{err}").contains("redirect:http://cdn.example.com/s.mp3"),
        "got: {err}"
    );
    assert_eq!(src.state(), SourceState::Error);
}

#[test]
fn icecast_source_accepts_icy_status_line() {
    let mut response = b"ICY 200 OK\r\nicy-name: Test\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xFF; 64]);
    let stream = Box::new(MockStream::new(response));
    let mut src = IcecastSource::new(stream, "host", "/stream");

    src.poll().unwrap();
    let chunk = src.poll().unwrap().unwrap();
    assert_eq!(chunk.data.len(), 64);
}

#[test]
fn icecast_source_rejects_garbage_status_line() {
    let stream = Box::new(MockStream::new(b"hello there\r\n\r\nbody".to_vec()));
    let mut src = IcecastSource::new(stream, "host", "/stream");
    src.poll().unwrap();
    assert!(src.poll().is_err());
    assert_eq!(src.state(), SourceState::Error);
}

// ---------------------------------------------------------------
// parse_http_head
// ---------------------------------------------------------------

#[test]
fn parse_http_head_incomplete_returns_none() {
    assert!(parse_http_head(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n").is_none());
}

#[test]
fn parse_http_head_fields() {
    let mut buf = b"HTTP/1.1 206 Partial\r\nContent-Length: 4\r\nX-Pad:  v \r\n\r\n".to_vec();
    buf.extend_from_slice(&[0xFF, 0xFE, b'\r', b'\n']);
    let head = parse_http_head(&buf).unwrap();
    assert_eq!(head.status, Some(206));
    assert_eq!(head.header("content-length"), Some("4"));
    assert_eq!(head.header("CONTENT-LENGTH"), Some("4"));
    assert_eq!(head.header("x-pad"), Some("v"));
    assert_eq!(head.header("location"), None);
    assert_eq!(&buf[head.body_offset..], &[0xFF, 0xFE, b'\r', b'\n']);
}

#[test]
fn parse_http_head_status_line_is_not_a_header() {
    // "HTTP/1.1 200 OK" contains no ':' but a status line like
    // "HTTP/1.1 200 location: x" must not be read as a header.
    let head = parse_http_head(b"HTTP/1.1 200 location: x\r\n\r\n").unwrap();
    assert_eq!(head.header("location"), None);
    assert_eq!(parse_http_head(b"junk\r\n\r\n").unwrap().status, None);
}

// ---------------------------------------------------------------
// Real non-blocking sockets (StdNetworkBackend over loopback)
// ---------------------------------------------------------------

/// Serve one connection on loopback: read the request head, wait for
/// `go`, then write `response` and close.
fn loopback_server(
    response: Vec<u8>,
) -> (
    u16,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<Vec<u8>>,
) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = conn.read(&mut buf).unwrap();
            assert!(n > 0, "client closed before sending a request");
            request.extend_from_slice(&buf[..n]);
        }
        go_rx.recv().unwrap();
        conn.write_all(&response).unwrap();
        request
    });
    (port, go_tx, handle)
}

fn connect_loopback(port: u16) -> Box<dyn oasis_types::backend::NetworkStream> {
    use oasis_types::backend::NetworkBackend;
    let mut backend = oasis_net::StdNetworkBackend::new();
    backend.connect("127.0.0.1", port).unwrap()
}

/// Poll until a chunk arrives or the source leaves `Active`.
fn poll_until_chunk(src: &mut IcecastSource) -> Result<Option<AudioChunk>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match src.poll() {
            Ok(None) if src.state() == SourceState::Active => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            },
            other => return other,
        }
    }
    panic!("timed out waiting for loopback data");
}

#[test]
fn icecast_source_survives_os_would_block_on_plain_tcp() {
    let mut response = b"HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xAB; 256]);
    let (port, go, server) = loopback_server(response);

    let mut src = IcecastSource::new(connect_loopback(port), "127.0.0.1", "/live");
    assert!(src.poll().unwrap().is_none()); // Sends the request.

    // The server is holding its response, so the non-blocking socket has
    // nothing to read: the OS reports WouldBlock (whose text does not say
    // "would block"). That must be "no data yet", not a stream error.
    for _ in 0..5 {
        assert!(src.poll().unwrap().is_none());
        assert_eq!(src.state(), SourceState::Active);
    }

    go.send(()).unwrap();
    let chunk = poll_until_chunk(&mut src).unwrap().unwrap();
    assert!(chunk.data.iter().all(|&b| b == 0xAB));
    let request = server.join().unwrap();
    let request = String::from_utf8_lossy(&request);
    assert!(request.starts_with("GET /live HTTP/1.0\r\n"), "{request}");
    assert!(request.contains("Icy-MetaData: 1"));
}

#[test]
fn icecast_source_loopback_http_error_is_not_audio() {
    let response = b"HTTP/1.0 503 Service Unavailable\r\n\r\nserver full".to_vec();
    let (port, go, server) = loopback_server(response);

    let mut src = IcecastSource::new(connect_loopback(port), "127.0.0.1", "/live");
    src.poll().unwrap();
    go.send(()).unwrap();
    let err = poll_until_chunk(&mut src).unwrap_err();
    assert!(format!("{err}").contains("HTTP 503"), "got: {err}");
    assert_eq!(src.state(), SourceState::Error);
    server.join().unwrap();
}

/// A `Location` header on a 2xx response (e.g. a CDN that echoes the
/// canonical URL) used to be followed as a redirect, failing the download.
#[test]
fn archive_source_location_on_200_is_not_a_redirect() {
    let mut response =
        b"HTTP/1.1 200 OK\r\nLocation: https://cdn.example.com/a.mp3\r\n\r\n".to_vec();
    response.extend_from_slice(&[0xCC; 64]);
    let mut src = ArchiveSource::new(Box::new(MockStream::new(response)), "h", "/p", "T", "");
    src.poll().unwrap(); // Send request.
    let chunk = src.poll().unwrap().expect("audio after a 200");
    assert_eq!(chunk.data, vec![0xCC; 64]);
    assert_eq!(src.state(), SourceState::Active);
}

/// A zero chunk size used to return empty chunks forever without
/// advancing.
#[test]
fn vfs_source_zero_chunk_size_still_progresses() {
    let mut src = VfsSource::new(vec![1, 2, 3]).with_chunk_size(0);
    let mut total = Vec::new();
    for _ in 0..10 {
        match src.poll().unwrap() {
            Some(chunk) => total.extend(chunk.data),
            None => break,
        }
    }
    assert_eq!(total, vec![1, 2, 3]);
    assert_eq!(src.state(), SourceState::Ended);
}

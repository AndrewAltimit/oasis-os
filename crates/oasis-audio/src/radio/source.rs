//! Radio source trait and implementations.
//!
//! `RadioSource` abstracts audio data providers for the radio manager.
//! The default implementations are:
//! - `VfsSource`: streams pre-loaded audio data from a byte buffer
//! - `IcecastSource`: streams from an HTTP/ICY connection (requires a
//!   pre-connected `NetworkStream`)

use oasis_types::error::{OasisError, Result};

use super::icy::{IcyDemuxer, StreamMetadata, parse_icy_metaint};

/// A chunk of audio data returned by a radio source.
#[derive(Debug)]
pub struct AudioChunk {
    /// Raw audio bytes (MP3 frames, etc.).
    pub data: Vec<u8>,
    /// Updated stream metadata, if any.
    pub metadata: Option<StreamMetadata>,
}

/// Source lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// Source created but not yet producing data.
    Connecting,
    /// Actively producing audio chunks.
    Active,
    /// Source has ended or been disconnected.
    Ended,
    /// An error occurred.
    Error,
}

/// Trait for swappable audio data providers.
pub trait RadioSource {
    /// Poll for the next chunk of audio data.
    ///
    /// Returns `Ok(Some(chunk))` when data is available, `Ok(None)` when
    /// no data is ready yet (non-blocking), or `Err` on failure.
    fn poll(&mut self) -> Result<Option<AudioChunk>>;

    /// Disconnect and clean up.
    fn disconnect(&mut self);

    /// Current source state.
    fn state(&self) -> SourceState;

    /// Human-readable source type name.
    fn source_type(&self) -> &str;

    /// Optional direct stream URL for backends that want to bypass the
    /// `poll`/`feed_data` chunk pipeline (e.g. WASM, where Firefox's
    /// MediaSource doesn't decode `audio/mpeg` and needs to hand the URL
    /// straight to a `<audio>` element). Default `None` means "use the
    /// normal chunk pipeline".
    fn streaming_url(&self) -> Option<&str> {
        None
    }
}

/// A parsed HTTP (or ICY) response head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpHead {
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
pub(crate) fn parse_http_head(buf: &[u8]) -> Option<HttpHead> {
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

/// How long a request write may keep hitting `WouldBlock` before the peer
/// is treated as dead. A request is a few hundred bytes, so a healthy
/// socket accepts it on the first write.
pub(crate) const REQUEST_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Write all of `bytes` to a non-blocking `stream`, retrying `WouldBlock`
/// until `timeout` elapses.
pub(crate) fn write_all_bounded(
    stream: &mut dyn oasis_types::backend::NetworkStream,
    bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    let mut written = 0;
    while written < bytes.len() {
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(OasisError::Backend("request write: peer closed".into())),
            Ok(n) => written += n,
            Err(e) if e.is_would_block() => {
                if std::time::Instant::now() >= deadline {
                    return Err(OasisError::Backend("request write timed out".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            },
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Radio source that streams from a pre-loaded byte buffer.
///
/// Useful for testing and UE5 integration where audio comes from game
/// assets rather than the network.
pub struct VfsSource {
    data: Vec<u8>,
    pos: usize,
    chunk_size: usize,
    state: SourceState,
}

impl VfsSource {
    /// Create a new VFS source from raw audio bytes.
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            pos: 0,
            chunk_size: 4096,
            state: SourceState::Active,
        }
    }

    /// Set the chunk size returned per poll (default 4096).
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }
}

impl RadioSource for VfsSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        if self.state != SourceState::Active {
            return Ok(None);
        }
        if self.pos >= self.data.len() {
            self.state = SourceState::Ended;
            return Ok(None);
        }
        let end = (self.pos + self.chunk_size).min(self.data.len());
        let chunk = AudioChunk {
            data: self.data[self.pos..end].to_vec(),
            metadata: None,
        };
        self.pos = end;
        Ok(Some(chunk))
    }

    fn disconnect(&mut self) {
        self.state = SourceState::Ended;
    }

    fn state(&self) -> SourceState {
        self.state
    }

    fn source_type(&self) -> &str {
        "vfs"
    }
}

/// Radio source that reads from an Icecast/Shoutcast HTTP stream.
///
/// The caller provides a pre-connected `NetworkStream` (from
/// `NetworkBackend::connect()`). The source sends the HTTP GET request
/// with ICY headers and then demuxes audio from metadata.
pub struct IcecastSource {
    stream: Box<dyn oasis_types::backend::NetworkStream>,
    url_path: String,
    host: String,
    state: SourceState,
    demuxer: Option<IcyDemuxer>,
    header_buf: Vec<u8>,
    headers_parsed: bool,
    last_metadata: Option<StreamMetadata>,
    /// `Location` of a 3xx response, kept so
    /// [`connect_icecast`](super::connect::connect_icecast) can follow it
    /// without parsing the error text.
    redirect_location: Option<String>,
    /// Chunk read while waiting for the response head, returned by the next
    /// `poll()` (see [`IcecastSource::push_back_chunk`]).
    pending_chunk: Option<AudioChunk>,
}

impl std::fmt::Debug for IcecastSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcecastSource")
            .field("host", &self.host)
            .field("path", &self.url_path)
            .field("state", &self.state)
            .field("headers_parsed", &self.headers_parsed)
            .finish_non_exhaustive()
    }
}

impl IcecastSource {
    /// Create a new Icecast source from a connected stream.
    ///
    /// `host` is the Host header value, `path` is the URL path
    /// (e.g. "/dronezone-128-mp3").
    pub fn new(
        stream: Box<dyn oasis_types::backend::NetworkStream>,
        host: &str,
        path: &str,
    ) -> Self {
        Self {
            stream,
            url_path: path.to_string(),
            host: host.to_string(),
            state: SourceState::Connecting,
            demuxer: None,
            header_buf: Vec::new(),
            headers_parsed: false,
            last_metadata: None,
            redirect_location: None,
            pending_chunk: None,
        }
    }

    /// `Location` header of a 3xx response, once one has been received.
    pub fn redirect_location(&self) -> Option<&str> {
        self.redirect_location.as_deref()
    }

    /// Whether a 2xx response head has been parsed (audio follows).
    pub fn headers_parsed(&self) -> bool {
        self.headers_parsed
    }

    /// Queue `chunk` to be returned by the next `poll()`, so a caller that
    /// polled past the response head does not lose the first audio bytes.
    pub fn push_back_chunk(&mut self, chunk: AudioChunk) {
        self.pending_chunk = Some(chunk);
    }

    /// Get the last parsed stream metadata.
    pub fn last_metadata(&self) -> Option<&StreamMetadata> {
        self.last_metadata.as_ref()
    }

    /// Send the HTTP GET request with ICY headers.
    fn send_request(&mut self) -> Result<()> {
        let request = format!(
            "GET {} HTTP/1.0\r\nHost: {}\r\nIcy-MetaData: 1\r\n\
             User-Agent: OASIS_OS/0.1\r\nAccept: */*\r\n\r\n",
            self.url_path, self.host
        );
        write_all_bounded(&mut *self.stream, request.as_bytes(), REQUEST_WRITE_TIMEOUT)
            .inspect_err(|_| self.state = SourceState::Error)
    }

    /// Try to parse the response head from the accumulated buffer.
    ///
    /// Returns `None` while the head is still incomplete, `Some(Ok(offset))`
    /// with the body offset for a 2xx response, and `Some(Err(_))` for any
    /// other status (the body of a 404/503 page is not audio). A 3xx with a
    /// `Location` header becomes `redirect:<url>`, like [`ArchiveSource`].
    fn try_parse_headers(&mut self) -> Option<Result<usize>> {
        let head = parse_http_head(&self.header_buf)?;
        let err = |msg: String| Some(Err(OasisError::Backend(msg.into())));
        match head.status {
            Some(200..=299) => {},
            Some(code @ 300..=399) => {
                return match head.header("location") {
                    Some(loc) => {
                        self.redirect_location = Some(loc.to_string());
                        err(format!("redirect:{loc}"))
                    },
                    None => err(format!("HTTP {code}")),
                };
            },
            Some(code) => return err(format!("HTTP {code}")),
            None => return err("malformed HTTP status line".to_string()),
        }
        if let Some(mi) = parse_icy_metaint(&head.text) {
            self.demuxer = Some(IcyDemuxer::new(mi));
        }
        Some(Ok(head.body_offset))
    }
}

impl RadioSource for IcecastSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        if let Some(chunk) = self.pending_chunk.take() {
            return Ok(Some(chunk));
        }
        match self.state {
            SourceState::Connecting => {
                self.send_request()?;
                self.state = SourceState::Active;
                self.headers_parsed = false;
                self.header_buf.clear();
                Ok(None)
            },
            SourceState::Active => {
                let mut buf = [0u8; 4096];
                let n = match self.stream.read(&mut buf) {
                    Ok(0) => {
                        self.state = SourceState::Ended;
                        return Ok(None);
                    },
                    Ok(n) => n,
                    Err(e) if e.is_would_block() => return Ok(None),
                    Err(e) => {
                        self.state = SourceState::Error;
                        return Err(OasisError::Backend(format!("stream read: {e}").into()));
                    },
                };

                if !self.headers_parsed {
                    self.header_buf.extend_from_slice(&buf[..n]);
                    if let Some(parsed) = self.try_parse_headers() {
                        let body_offset = match parsed {
                            Ok(offset) => offset,
                            Err(e) => {
                                self.state = SourceState::Error;
                                let _ = self.stream.close();
                                return Err(e);
                            },
                        };
                        self.headers_parsed = true;
                        let body = self.header_buf[body_offset..].to_vec();
                        self.header_buf.clear();
                        if body.is_empty() {
                            return Ok(None);
                        }
                        return self.process_body(&body);
                    }
                    return Ok(None);
                }

                self.process_body(&buf[..n])
            },
            _ => Ok(None),
        }
    }

    fn disconnect(&mut self) {
        let _ = self.stream.close();
        self.state = SourceState::Ended;
    }

    fn state(&self) -> SourceState {
        self.state
    }

    fn source_type(&self) -> &str {
        "icecast"
    }
}

impl IcecastSource {
    fn process_body(&mut self, data: &[u8]) -> Result<Option<AudioChunk>> {
        if let Some(ref mut demuxer) = self.demuxer {
            let (audio, meta) = demuxer.process(data);
            if let Some(ref m) = meta {
                self.last_metadata = Some(m.clone());
            }
            if audio.is_empty() {
                return Ok(None);
            }
            Ok(Some(AudioChunk {
                data: audio,
                metadata: meta,
            }))
        } else {
            // No ICY metadata -- treat all data as audio.
            Ok(Some(AudioChunk {
                data: data.to_vec(),
                metadata: None,
            }))
        }
    }
}

/// Radio source that downloads an MP3 file from the Internet Archive via HTTP(S).
///
/// Simpler than `IcecastSource` — no ICY metadata demuxing, just HTTP GET
/// and stream body bytes as audio chunks.
pub struct ArchiveSource {
    stream: Box<dyn oasis_types::backend::NetworkStream>,
    path: String,
    host: String,
    state: SourceState,
    header_buf: Vec<u8>,
    headers_parsed: bool,
    content_length: Option<usize>,
    bytes_received: usize,
    track_title: String,
    track_creator: String,
    metadata_sent: bool,
    redirect_url: Option<String>,
    status_code: Option<u16>,
    /// First chunk buffered by `push_back_chunk` so that
    /// `connect_archive_source` can poll past headers without losing data.
    pending_first_chunk: Option<AudioChunk>,
}

impl ArchiveSource {
    /// Create a new archive source from a connected stream.
    ///
    /// `host` is the Host header value (e.g. "archive.org"),
    /// `path` is the HTTP path (e.g. "/download/item/file.mp3").
    /// `title` and `creator` are pre-fetched metadata for display.
    pub fn new(
        stream: Box<dyn oasis_types::backend::NetworkStream>,
        host: &str,
        path: &str,
        title: &str,
        creator: &str,
    ) -> Self {
        Self {
            stream,
            path: path.to_string(),
            host: host.to_string(),
            state: SourceState::Connecting,
            header_buf: Vec::new(),
            headers_parsed: false,
            content_length: None,
            bytes_received: 0,
            track_title: title.to_string(),
            track_creator: creator.to_string(),
            metadata_sent: false,
            redirect_url: None,
            status_code: None,
            pending_first_chunk: None,
        }
    }

    /// Send the HTTP GET request.
    fn send_request(&mut self) -> Result<()> {
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: OASIS_OS/0.1\r\n\
             Connection: close\r\nAccept: */*\r\n\r\n",
            self.path, self.host
        );
        write_all_bounded(&mut *self.stream, request.as_bytes(), REQUEST_WRITE_TIMEOUT)
            .inspect_err(|_| self.state = SourceState::Error)
    }

    /// Push back a chunk so it is returned by the next `poll()` call.
    ///
    /// Used by `connect_archive_source` to preserve the first audio chunk
    /// (and its metadata) that was consumed while waiting for headers.
    pub fn push_back_chunk(&mut self, chunk: AudioChunk) {
        self.pending_first_chunk = Some(chunk);
    }

    /// Try to parse HTTP response headers from the accumulated buffer.
    ///
    /// Searches for `\r\n\r\n` directly in raw bytes to avoid offset
    /// mismatches from lossy UTF-8 conversion.
    fn try_parse_headers(&mut self) -> Option<usize> {
        let head = parse_http_head(&self.header_buf)?;
        self.status_code = head.status;
        self.content_length = head.header("content-length").and_then(|v| v.parse().ok());
        self.redirect_url = head.header("location").map(str::to_string);
        Some(head.body_offset)
    }
}

impl RadioSource for ArchiveSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        // Return any buffered chunk first (pushed back after header parsing).
        if let Some(chunk) = self.pending_first_chunk.take() {
            return Ok(Some(chunk));
        }
        match self.state {
            SourceState::Connecting => {
                self.send_request()?;
                self.state = SourceState::Active;
                self.headers_parsed = false;
                self.header_buf.clear();
                Ok(None)
            },
            SourceState::Active => {
                let mut buf = [0u8; 4096];
                let n = match self.stream.read(&mut buf) {
                    Ok(0) => {
                        self.state = SourceState::Ended;
                        return Ok(None);
                    },
                    Ok(n) => n,
                    Err(e) if e.is_would_block() => return Ok(None),
                    Err(e) => {
                        self.state = SourceState::Error;
                        return Err(OasisError::Backend(format!("stream read: {e}").into()));
                    },
                };

                if !self.headers_parsed {
                    self.header_buf.extend_from_slice(&buf[..n]);
                    if let Some(body_offset) = self.try_parse_headers() {
                        self.headers_parsed = true;

                        // Check for redirect.
                        if let Some(ref url) = self.redirect_url {
                            let url = url.clone();
                            self.state = SourceState::Error;
                            let _ = self.stream.close();
                            return Err(OasisError::Backend(format!("redirect:{url}").into()));
                        }
                        // Check for HTTP error status.
                        if let Some(code) = self.status_code
                            && code >= 400
                        {
                            self.state = SourceState::Error;
                            let _ = self.stream.close();
                            return Err(OasisError::Backend(format!("HTTP {code}").into()));
                        }

                        let body = self.header_buf[body_offset..].to_vec();
                        self.header_buf.clear();
                        if body.is_empty() {
                            return Ok(None);
                        }
                        self.bytes_received += body.len();
                        let metadata = if !self.metadata_sent {
                            self.metadata_sent = true;
                            let title = if self.track_creator.is_empty() {
                                self.track_title.clone()
                            } else {
                                format!("{} - {}", self.track_creator, self.track_title)
                            };
                            Some(super::icy::StreamMetadata { title })
                        } else {
                            None
                        };
                        return Ok(Some(AudioChunk {
                            data: body,
                            metadata,
                        }));
                    }
                    return Ok(None);
                }

                self.bytes_received += n;

                // Check if download is complete.
                if let Some(cl) = self.content_length
                    && self.bytes_received >= cl
                {
                    self.state = SourceState::Ended;
                }

                let metadata = if !self.metadata_sent {
                    self.metadata_sent = true;
                    let title = if self.track_creator.is_empty() {
                        self.track_title.clone()
                    } else {
                        format!("{} - {}", self.track_creator, self.track_title)
                    };
                    Some(super::icy::StreamMetadata { title })
                } else {
                    None
                };

                Ok(Some(AudioChunk {
                    data: buf[..n].to_vec(),
                    metadata,
                }))
            },
            _ => Ok(None),
        }
    }

    fn disconnect(&mut self) {
        let _ = self.stream.close();
        self.state = SourceState::Ended;
    }

    fn state(&self) -> SourceState {
        self.state
    }

    fn source_type(&self) -> &str {
        "archive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut src =
            ArchiveSource::new(stream, "archive.org", "/download/x/y.mp3", "Song", "Artist");

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
    fn archive_source_metadata_on_first_chunk() {
        let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        response.extend_from_slice(&[0xAA; 100]);

        let stream = Box::new(MockStream::new(response));
        let mut src =
            ArchiveSource::new(stream, "archive.org", "/download/x/y.mp3", "Song", "Artist");

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
        let response =
            b"HTTP/1.1 302 Found\r\nLocation: http://cdn.example.com/s.mp3\r\n\r\n".to_vec();
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
}

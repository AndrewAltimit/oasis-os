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
        }
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
        let bytes = request.as_bytes();
        let mut written = 0;
        while written < bytes.len() {
            match self.stream.write(&bytes[written..]) {
                Ok(n) => written += n,
                Err(e) => {
                    self.state = SourceState::Error;
                    return Err(e);
                },
            }
        }
        Ok(())
    }

    /// Try to parse HTTP response headers from the accumulated buffer.
    fn try_parse_headers(&mut self) -> Option<usize> {
        let text = String::from_utf8_lossy(&self.header_buf);
        if let Some(end) = text.find("\r\n\r\n") {
            let header_str = &text[..end];
            let metaint = parse_icy_metaint(header_str);
            // Body starts after \r\n\r\n.
            let body_offset = end + 4;
            if let Some(mi) = metaint {
                self.demuxer = Some(IcyDemuxer::new(mi));
            }
            Some(body_offset)
        } else {
            None
        }
    }
}

impl RadioSource for IcecastSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
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
                    Err(e) => {
                        let msg = format!("{e}");
                        if msg.contains("WouldBlock") || msg.contains("would block") {
                            return Ok(None);
                        }
                        self.state = SourceState::Error;
                        return Err(OasisError::Backend(format!("stream read: {e}")));
                    },
                };

                if !self.headers_parsed {
                    self.header_buf.extend_from_slice(&buf[..n]);
                    if let Some(body_offset) = self.try_parse_headers() {
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
        }
    }

    /// Send the HTTP GET request.
    fn send_request(&mut self) -> Result<()> {
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: OASIS_OS/0.1\r\n\
             Connection: close\r\nAccept: */*\r\n\r\n",
            self.path, self.host
        );
        let bytes = request.as_bytes();
        let mut written = 0;
        while written < bytes.len() {
            match self.stream.write(&bytes[written..]) {
                Ok(n) => written += n,
                Err(e) => {
                    self.state = SourceState::Error;
                    return Err(e);
                },
            }
        }
        Ok(())
    }

    /// Try to parse HTTP response headers from the accumulated buffer.
    fn try_parse_headers(&mut self) -> Option<usize> {
        let text = String::from_utf8_lossy(&self.header_buf);
        if let Some(end) = text.find("\r\n\r\n") {
            let header_str = &text[..end];
            // Parse Content-Length if present.
            for line in header_str.lines() {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("content-length:")
                    && let Some(val) = line.split_once(':').map(|(_, v)| v.trim())
                {
                    self.content_length = val.parse().ok();
                }
            }
            // Check for HTTP redirect (302/301).
            let first_line = header_str.lines().next().unwrap_or("");
            if first_line.contains("301") || first_line.contains("302") {
                for line in header_str.lines() {
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("location:")
                        && let Some(loc) = line.split_once(':').map(|(_, v)| v.trim())
                    {
                        log::info!("HTTP redirect to: {loc}");
                    }
                }
            }
            Some(end + 4)
        } else {
            None
        }
    }
}

impl RadioSource for ArchiveSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
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
                    Err(e) => {
                        let msg = format!("{e}");
                        if msg.contains("WouldBlock") || msg.contains("would block") {
                            return Ok(None);
                        }
                        self.state = SourceState::Error;
                        return Err(OasisError::Backend(format!("stream read: {e}")));
                    },
                };

                if !self.headers_parsed {
                    self.header_buf.extend_from_slice(&buf[..n]);
                    if let Some(body_offset) = self.try_parse_headers() {
                        self.headers_parsed = true;
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
        loop {
            match src.poll().unwrap() {
                Some(chunk) => {
                    total.extend_from_slice(&chunk.data);
                },
                None => break,
            }
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
        loop {
            match src.poll().unwrap() {
                Some(chunk) => {
                    // Subsequent chunks should NOT have metadata.
                    assert!(chunk.metadata.is_none());
                },
                None => break,
            }
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
        loop {
            match src.poll().unwrap() {
                Some(chunk) => received += chunk.data.len(),
                None => break,
            }
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
}

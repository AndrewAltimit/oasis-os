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
}

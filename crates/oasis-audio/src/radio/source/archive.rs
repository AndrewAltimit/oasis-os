//! [`ArchiveSource`]: one Internet Archive MP3 downloaded over HTTP(S).

use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::http::{
    MAX_HEADER_BYTES, get_request, headers_too_large, parse_http_head, read_some, write_request,
};
use super::{AudioChunk, RadioSource, SourceState};
use crate::radio::icy::StreamMetadata;

/// Radio source that downloads an MP3 file from the Internet Archive via HTTP(S).
///
/// Simpler than `IcecastSource` — no ICY metadata demuxing, just HTTP GET
/// and stream body bytes as audio chunks.
pub struct ArchiveSource {
    stream: Box<dyn NetworkStream>,
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
        stream: Box<dyn NetworkStream>,
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
            pending_first_chunk: None,
        }
    }

    /// Send the HTTP GET request.
    fn send_request(&mut self) -> Result<()> {
        get_request(
            &self.path,
            &self.host,
            "1.1",
            "User-Agent: OASIS_OS/0.1\r\nConnection: close\r\nAccept: */*\r\n",
        )
        .and_then(|request| write_request(&mut *self.stream, request.as_bytes()))
        .inspect_err(|_| self.state = SourceState::Error)
    }

    /// Push back a chunk so it is returned by the next `poll()` call.
    ///
    /// Used by `connect_archive_source` to preserve the first audio chunk
    /// (and its metadata) that was consumed while waiting for headers.
    pub fn push_back_chunk(&mut self, chunk: AudioChunk) {
        self.pending_first_chunk = Some(chunk);
    }

    /// Mark the source failed and drop the connection.
    fn fail(&mut self, msg: String) -> Result<Option<AudioChunk>> {
        self.state = SourceState::Error;
        let _ = self.stream.close();
        Err(OasisError::Backend(msg.into()))
    }

    /// The "Creator - Title" metadata, attached to the first audio chunk
    /// only.
    fn take_metadata(&mut self) -> Option<StreamMetadata> {
        if self.metadata_sent {
            return None;
        }
        self.metadata_sent = true;
        let title = if self.track_creator.is_empty() {
            self.track_title.clone()
        } else {
            format!("{} - {}", self.track_creator, self.track_title)
        };
        Some(StreamMetadata { title })
    }

    /// Wrap body bytes as an audio chunk, counting them toward the
    /// `Content-Length` (the source ends once it has all of them).
    fn body_chunk(&mut self, data: Vec<u8>) -> Result<Option<AudioChunk>> {
        if data.is_empty() {
            return Ok(None);
        }
        self.bytes_received += data.len();
        if let Some(cl) = self.content_length
            && self.bytes_received >= cl
        {
            self.state = SourceState::Ended;
        }
        let metadata = self.take_metadata();
        Ok(Some(AudioChunk { data, metadata }))
    }

    /// Accumulate response-head bytes; once the head is complete, check
    /// the status and return the body bytes that followed it.
    ///
    /// A 3xx with a `Location` header fails with `redirect:<url>` (the
    /// caller follows it); any other status of 400 or more fails with
    /// `HTTP <code>`. A `Location` on a 2xx response is not a redirect.
    fn read_head(&mut self, data: &[u8]) -> Result<Option<AudioChunk>> {
        self.header_buf.extend_from_slice(data);
        let Some(head) = parse_http_head(&self.header_buf) else {
            if self.header_buf.len() > MAX_HEADER_BYTES {
                self.state = SourceState::Error;
                let _ = self.stream.close();
                return Err(headers_too_large());
            }
            return Ok(None);
        };
        self.headers_parsed = true;
        match (head.status, head.header("location")) {
            (Some(300..=399), Some(url)) => return self.fail(format!("redirect:{url}")),
            // A 3xx without a `Location` has no audio body to play either.
            (Some(code), _) if code >= 300 => return self.fail(format!("HTTP {code}")),
            _ => {},
        }
        self.content_length = head.header("content-length").and_then(|v| v.parse().ok());
        let body = self.header_buf.split_off(head.body_offset);
        self.header_buf = Vec::new();
        self.body_chunk(body)
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
                let n = match read_some(&mut *self.stream, &mut buf) {
                    Ok(Some(0)) => {
                        self.state = SourceState::Ended;
                        return Ok(None);
                    },
                    Ok(Some(n)) => n,
                    Ok(None) => return Ok(None),
                    Err(e) => {
                        self.state = SourceState::Error;
                        return Err(e);
                    },
                };
                if self.headers_parsed {
                    self.body_chunk(buf[..n].to_vec())
                } else {
                    self.read_head(&buf[..n])
                }
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

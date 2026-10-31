//! [`IcecastSource`]: an Icecast/Shoutcast stream over HTTP with ICY
//! metadata.

use oasis_types::backend::NetworkStream;
use oasis_types::error::{OasisError, Result};

use super::http::{MAX_HEADER_BYTES, headers_too_large, parse_http_head, read_some, write_request};
use super::{AudioChunk, RadioSource, SourceState};
use crate::radio::icy::{IcyDemuxer, StreamMetadata, parse_icy_metaint};

/// Radio source that reads from an Icecast/Shoutcast HTTP stream.
///
/// The caller provides a pre-connected `NetworkStream` (from
/// `NetworkBackend::connect()`). The source sends the HTTP GET request
/// with ICY headers and then demuxes audio from metadata.
pub struct IcecastSource {
    stream: Box<dyn NetworkStream>,
    url_path: String,
    host: String,
    state: SourceState,
    demuxer: Option<IcyDemuxer>,
    header_buf: Vec<u8>,
    headers_parsed: bool,
    last_metadata: Option<StreamMetadata>,
    /// `Location` of a 3xx response, kept so
    /// [`connect_icecast`](crate::radio::connect::connect_icecast) can
    /// follow it without parsing the error text.
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
    pub fn new(stream: Box<dyn NetworkStream>, host: &str, path: &str) -> Self {
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
        write_request(&mut *self.stream, request.as_bytes())
            .inspect_err(|_| self.state = SourceState::Error)
    }

    /// Mark the source failed and drop the connection.
    fn fail(&mut self, err: OasisError) -> Result<Option<AudioChunk>> {
        self.state = SourceState::Error;
        let _ = self.stream.close();
        Err(err)
    }

    /// Try to parse the response head from the accumulated buffer.
    ///
    /// Returns `None` while the head is still incomplete, `Some(Ok(offset))`
    /// with the body offset for a 2xx response, and `Some(Err(_))` for any
    /// other status (the body of a 404/503 page is not audio). A 3xx with a
    /// `Location` header becomes `redirect:<url>`, like
    /// [`ArchiveSource`](super::ArchiveSource).
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

    /// Accumulate response-head bytes; once the head is complete, return
    /// the body bytes that followed it.
    fn read_head(&mut self, data: &[u8]) -> Result<Option<AudioChunk>> {
        self.header_buf.extend_from_slice(data);
        match self.try_parse_headers() {
            Some(Ok(body_offset)) => {
                self.headers_parsed = true;
                let body = self.header_buf.split_off(body_offset);
                self.header_buf = Vec::new();
                if body.is_empty() {
                    return Ok(None);
                }
                self.process_body(&body)
            },
            Some(Err(e)) => self.fail(e),
            None if self.header_buf.len() > MAX_HEADER_BYTES => self.fail(headers_too_large()),
            None => Ok(None),
        }
    }

    /// Split body bytes into audio and ICY metadata.
    fn process_body(&mut self, data: &[u8]) -> Result<Option<AudioChunk>> {
        let Some(demuxer) = self.demuxer.as_mut() else {
            // No ICY metadata -- treat all data as audio.
            return Ok(Some(AudioChunk {
                data: data.to_vec(),
                metadata: None,
            }));
        };
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
                    self.process_body(&buf[..n])
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
        "icecast"
    }
}

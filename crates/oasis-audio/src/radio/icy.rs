//! ICY metadata parser for Icecast/Shoutcast streams.
//!
//! Icecast streams interleave audio data with metadata blocks at intervals
//! specified by the `icy-metaint` response header. This module extracts that
//! interval from headers and demuxes audio from metadata in the raw stream.

/// Metadata extracted from an ICY metadata block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamMetadata {
    /// Current stream title (typically "Artist - Song").
    pub title: String,
}

/// Extract the `icy-metaint` value from HTTP response headers.
///
/// Returns `None` if the header is not present.
pub fn parse_icy_metaint(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("icy-metaint:") {
            let val = line.split_once(':')?.1.trim();
            return val.parse().ok();
        }
    }
    None
}

/// Parse a single ICY metadata block.
///
/// The block starts with a length byte (actual length = byte * 16), followed
/// by that many bytes of metadata text. The text contains semicolon-separated
/// `Key='Value';` pairs. We extract `StreamTitle`.
pub fn parse_icy_metadata(block: &[u8]) -> StreamMetadata {
    if block.is_empty() {
        return StreamMetadata::default();
    }

    // Decode as UTF-8 (lossy -- ICY streams sometimes use Latin-1).
    let text = String::from_utf8_lossy(block);
    let text = text.trim_end_matches('\0');

    let mut meta = StreamMetadata::default();

    // Look for StreamTitle='...';
    if let Some(start) = text.find("StreamTitle='") {
        let after = &text[start + "StreamTitle='".len()..];
        if let Some(end) = after.find("';") {
            meta.title = after[..end].to_string();
        } else {
            // No closing delimiter -- take rest of string.
            meta.title = after.trim_end_matches('\'').to_string();
        }
    }

    meta
}

/// Demuxer that separates audio data from interleaved ICY metadata.
pub struct IcyDemuxer {
    /// Metadata interval in bytes.
    metaint: usize,
    /// Bytes of audio data consumed since last metadata block.
    audio_count: usize,
    /// Whether we're currently reading a metadata block.
    in_meta: bool,
    /// Expected metadata block length (first byte * 16).
    meta_remaining: usize,
    /// Accumulated metadata bytes.
    meta_buf: Vec<u8>,
}

impl IcyDemuxer {
    /// Create a new demuxer with the given metadata interval.
    pub fn new(metaint: usize) -> Self {
        Self {
            metaint,
            audio_count: 0,
            in_meta: false,
            meta_remaining: 0,
            meta_buf: Vec::new(),
        }
    }

    /// Process a chunk of raw stream data.
    ///
    /// Returns `(audio_bytes, Option<metadata>)`. Audio bytes are the pure
    /// audio data with ICY blocks stripped. Metadata is returned whenever a
    /// complete metadata block is parsed.
    pub fn process(&mut self, data: &[u8]) -> (Vec<u8>, Option<StreamMetadata>) {
        let mut audio = Vec::new();
        let mut metadata = None;
        let mut i = 0;

        while i < data.len() {
            if self.in_meta {
                if self.meta_remaining == 0 {
                    // Length byte.
                    let meta_len = data[i] as usize * 16;
                    i += 1;
                    if meta_len == 0 {
                        // Empty metadata block.
                        self.in_meta = false;
                        continue;
                    }
                    self.meta_remaining = meta_len;
                    self.meta_buf.clear();
                } else {
                    // Consume metadata bytes.
                    let take = self.meta_remaining.min(data.len() - i);
                    self.meta_buf.extend_from_slice(&data[i..i + take]);
                    self.meta_remaining -= take;
                    i += take;
                    if self.meta_remaining == 0 {
                        metadata = Some(parse_icy_metadata(&self.meta_buf));
                        self.in_meta = false;
                    }
                }
            } else {
                // Audio data region.
                let remaining_audio = self.metaint - self.audio_count;
                let take = remaining_audio.min(data.len() - i);
                audio.extend_from_slice(&data[i..i + take]);
                self.audio_count += take;
                i += take;
                if self.audio_count >= self.metaint {
                    self.audio_count = 0;
                    self.in_meta = true;
                    self.meta_remaining = 0;
                }
            }
        }

        (audio, metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metaint_present() {
        let headers = "ICY 200 OK\r\nicy-metaint:16000\r\nContent-Type:audio/mpeg\r\n";
        assert_eq!(parse_icy_metaint(headers), Some(16000));
    }

    #[test]
    fn parse_metaint_mixed_case() {
        let headers = "Icy-MetaInt: 8192\r\n";
        assert_eq!(parse_icy_metaint(headers), Some(8192));
    }

    #[test]
    fn parse_metaint_absent() {
        let headers = "HTTP/1.0 200 OK\r\nContent-Type:audio/mpeg\r\n";
        assert_eq!(parse_icy_metaint(headers), None);
    }

    #[test]
    fn parse_metadata_valid() {
        let block = b"StreamTitle='Artist - Song';StreamUrl='';";
        let meta = parse_icy_metadata(block);
        assert_eq!(meta.title, "Artist - Song");
    }

    #[test]
    fn parse_metadata_empty() {
        let meta = parse_icy_metadata(b"");
        assert_eq!(meta.title, "");
    }

    #[test]
    fn parse_metadata_no_title() {
        let block = b"StreamUrl='http://example.com';";
        let meta = parse_icy_metadata(block);
        assert_eq!(meta.title, "");
    }

    #[test]
    fn parse_metadata_null_padded() {
        let mut block = b"StreamTitle='Test';".to_vec();
        block.extend_from_slice(&[0u8; 13]); // Pad to 16*2=32 bytes.
        let meta = parse_icy_metadata(&block);
        assert_eq!(meta.title, "Test");
    }

    #[test]
    fn parse_metadata_utf8_title() {
        let block = "StreamTitle='Café Müsik';".as_bytes();
        let meta = parse_icy_metadata(block);
        assert_eq!(meta.title, "Café Müsik");
    }

    #[test]
    fn demuxer_no_metadata() {
        // Metaint=8, all audio, no metadata trigger.
        let mut demux = IcyDemuxer::new(8);
        let (audio, meta) = demux.process(b"abcde");
        assert_eq!(audio, b"abcde");
        assert!(meta.is_none());
    }

    #[test]
    fn demuxer_with_empty_metadata() {
        // Metaint=4: 4 bytes audio, then 1 byte meta length (0 = empty).
        let mut demux = IcyDemuxer::new(4);
        let mut data = vec![0xAAu8; 4]; // Audio.
        data.push(0); // Meta length byte = 0 (empty block).
        data.extend_from_slice(&[0xBB; 4]); // More audio.
        let (audio, meta) = demux.process(&data);
        assert_eq!(audio.len(), 8);
        assert!(meta.is_none());
    }

    #[test]
    fn demuxer_with_metadata() {
        let mut demux = IcyDemuxer::new(4);
        let mut data = vec![0xAAu8; 4]; // Audio.
        // Metadata: length byte = 2 (2*16=32 bytes of metadata).
        data.push(2);
        let mut meta_block = b"StreamTitle='Hello';".to_vec();
        meta_block.resize(32, 0); // Pad to 32 bytes.
        data.extend_from_slice(&meta_block);
        data.extend_from_slice(&[0xBB; 4]); // More audio.

        let (audio, meta) = demux.process(&data);
        assert_eq!(audio.len(), 8);
        assert!(meta.is_some());
        assert_eq!(meta.unwrap().title, "Hello");
    }

    #[test]
    fn demuxer_split_across_calls() {
        let mut demux = IcyDemuxer::new(4);

        // First call: 2 bytes of audio.
        let (audio1, meta1) = demux.process(&[0xAA; 2]);
        assert_eq!(audio1.len(), 2);
        assert!(meta1.is_none());

        // Second call: 2 more bytes of audio + meta length + meta.
        let mut data = vec![0xAA; 2];
        data.push(1); // Meta length = 1*16 = 16 bytes.
        let mut meta_block = b"StreamTitle='X';".to_vec();
        meta_block.resize(16, 0);
        data.extend_from_slice(&meta_block);

        let (audio2, meta2) = demux.process(&data);
        assert_eq!(audio2.len(), 2);
        assert!(meta2.is_some());
        assert_eq!(meta2.unwrap().title, "X");
    }
}

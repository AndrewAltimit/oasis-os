//! [`VfsSource`]: chunks a pre-loaded byte buffer.

use oasis_types::error::Result;

use super::{AudioChunk, RadioSource, SourceState};

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

    /// Set the chunk size returned per poll (default 4096; 0 is treated
    /// as 1 so polling always makes progress).
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size.max(1);
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

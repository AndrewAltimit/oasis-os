//! Radio source trait and implementations.
//!
//! `RadioSource` abstracts audio data providers for the radio manager.
//! The default implementations are:
//! - `VfsSource`: streams pre-loaded audio data from a byte buffer
//! - `IcecastSource`: streams from an HTTP/ICY connection (requires a
//!   pre-connected `NetworkStream`)
//! - `ArchiveSource`: downloads one Internet Archive MP3 over HTTP(S)

mod archive;
mod http;
mod icecast;
mod vfs;

pub use archive::ArchiveSource;
pub use icecast::IcecastSource;
pub use vfs::VfsSource;

use oasis_types::error::Result;

use super::icy::StreamMetadata;

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

#[cfg(test)]
mod tests;

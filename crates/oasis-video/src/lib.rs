//! Software MP4/H.264+AAC video decode pipeline.
//!
//! Provides a pure-software path for decoding MP4 videos when hardware or
//! browser-native codecs are unavailable.
//!
//! # Features
//!
//! All features are additive; none is on by default.
//!
//! - `demux-lite`: `demux_lite`, a dependency-free MP4 parser (PSP,
//!   seek-point lookup, fuzzing).
//! - `symphonia`: `demux` + `aac` (symphonia MP4 demux and AAC-LC) and an
//!   audio-only `SoftwareVideoDecoder`. Implies `demux-lite`.
//! - `h264`: `h264` (openh264, compiled from C source), so the symphonia
//!   backend decodes video too. Implies `symphonia`.
//! - `ffmpeg`: statically-linked ffmpeg for demux, H.264 and AAC
//!   (Main/High profile, SIMD-optimized).
//!
//! [`SoftwareVideoDecoder`] exists when `symphonia` or `ffmpeg` is on. When
//! both backends are compiled in (e.g. `--all-features`, or one binary
//! enabling `h264` through one crate and `ffmpeg` through another), the
//! ffmpeg backend wins; the symphonia modules stay available as libraries.
//! The [`yuv`] converters and the error/frame types are always available.

#![warn(missing_docs)]

#[cfg(feature = "symphonia")]
mod aac;
#[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
mod decoder;
#[cfg(feature = "symphonia")]
mod demux;
#[cfg(feature = "demux-lite")]
pub mod demux_lite;
#[cfg(feature = "ffmpeg")]
#[allow(clippy::unnecessary_cast)]
mod ffmpeg_decoder;
#[cfg(feature = "h264")]
mod h264;
#[cfg(feature = "symphonia")]
// Compiled (and linted) whenever its dependencies are, even when ffmpeg is
// the selected backend.
#[cfg_attr(feature = "ffmpeg", allow(dead_code))]
mod symphonia_decoder;
pub mod yuv;

use std::io::{Cursor, Read, Seek};

#[cfg(any(feature = "symphonia", feature = "ffmpeg"))]
pub use decoder::SoftwareVideoDecoder;

/// A streaming video source: anything that is `Read + Seek + Send + Sync`.
///
/// Provides optional length and seekability hints for the demuxer.
/// This is a blanket trait — `File`, `Cursor<Vec<u8>>`, and any other
/// `Read + Seek + Send + Sync + 'static` type implements it automatically.
pub trait VideoSource: Read + Seek + Send + Sync {
    /// Whether the source supports seeking. Defaults to `true`.
    fn is_seekable(&self) -> bool {
        true
    }

    /// Total length in bytes, if known.
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl VideoSource for std::fs::File {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.metadata().ok().map(|m| m.len())
    }
}

impl<T: AsRef<[u8]> + Send + Sync> VideoSource for Cursor<T> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.get_ref().as_ref().len() as u64)
    }
}

/// Errors from the video pipeline.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// MP4 container / demuxing error.
    #[error("demux error: {0}")]
    Demux(String),
    /// Codec decode error (H.264 or AAC).
    #[error("decode error: {0}")]
    Decode(String),
    /// No suitable track found.
    #[error("no track: {0}")]
    NoTrack(String),
    /// Decoder couldn't produce a frame within the packet skip limit.
    /// Not a fatal error — caller should continue with audio and retry.
    #[error("decoder skip limit reached")]
    SkipLimit,
}

/// A decoded video frame in RGBA format.
pub struct VideoFrame {
    /// Tightly packed RGBA8 pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Presentation time in seconds from the start of the stream.
    pub timestamp_secs: f64,
}

/// A chunk of decoded audio as interleaved f32 PCM.
pub struct AudioChunk {
    /// Interleaved samples in `[-1.0, 1.0]`, `channels` per frame.
    pub pcm_f32: Vec<f32>,
    /// Channel count.
    pub channels: u16,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Presentation time of the first sample, in seconds.
    pub timestamp_secs: f64,
}

#[cfg(test)]
mod tests;

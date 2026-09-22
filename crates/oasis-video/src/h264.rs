//! H.264 decoder wrapper around the `openh264` crate.

use openh264::OpenH264API;
use openh264::decoder::{Decoder, DecoderConfig, Flush};

use crate::VideoError;

/// Decoded video frame in RGBA format.
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// H.264 NAL unit decoder producing RGBA frames.
pub struct H264Decoder {
    decoder: Decoder,
    /// Total calls to decode (for diagnostics).
    call_count: u64,
    /// Consecutive decode errors (reset on success).
    pub error_streak: u32,
    /// Whether the most recent [`decode`](Self::decode) call failed (as
    /// opposed to succeeding without a displayable picture yet).
    pub last_failed: bool,
}

/// Build an openh264 decoder that never force-flushes its reorder buffer.
///
/// The crate default (`Flush::Flush`) calls `FlushFrame` after every packet
/// that doesn't immediately yield a picture.  On streams with B-frames
/// (Main/High profile -- i.e. nearly every web MP4) that pulls pictures out
/// of the reorder buffer before their references are complete, and after a
/// handful of frames openh264 fails with `dsOutOfMemory` and drops its
/// parameter sets, so every later packet errors with `dsNoParamSets`.  With
/// `NoFlush` pictures come out in display order with a small fixed delay.
fn new_decoder() -> Result<Decoder, VideoError> {
    let config = DecoderConfig::new().flush_after_decode(Flush::NoFlush);
    Decoder::with_api_config(OpenH264API::from_source(), config)
        .map_err(|e| VideoError::Decode(format!("openh264 init: {e}")))
}

impl H264Decoder {
    /// Create a new decoder instance.
    pub fn new() -> Result<Self, VideoError> {
        Ok(Self {
            decoder: new_decoder()?,
            call_count: 0,
            error_streak: 0,
            last_failed: false,
        })
    }

    /// Re-create the internal decoder after unrecoverable errors.
    ///
    /// The fresh decoder has no parameter sets: the next packet fed to it
    /// must be an IDR with SPS/PPS prepended.
    pub fn reinit(&mut self) -> Result<(), VideoError> {
        self.decoder = new_decoder()?;
        self.error_streak = 0;
        self.last_failed = false;
        log::info!("H264: decoder reinitialized");
        Ok(())
    }

    /// Decode an H.264 packet (one or more NAL units).
    ///
    /// Returns `None` if the packet didn't produce a displayable frame
    /// (e.g. SPS/PPS parameter sets, frames held in the reorder buffer,
    /// or a decode error -- see [`last_failed`](Self::last_failed) and
    /// [`error_streak`](Self::error_streak)).
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>, VideoError> {
        self.call_count += 1;
        let n = self.call_count;
        let yuv = match self.decoder.decode(data) {
            Ok(Some(yuv)) => yuv,
            Ok(None) => {
                self.last_failed = false;
                if n <= 10 || n.is_multiple_of(100) {
                    log::debug!("H264: decode #{n} -> None (size={})", data.len());
                }
                return Ok(None);
            },
            Err(e) => {
                self.error_streak += 1;
                self.last_failed = true;
                if n <= 20 || n.is_multiple_of(100) {
                    log::debug!("H264: decode #{n} -> Err({e}) (size={})", data.len());
                }
                return Ok(None);
            },
        };

        self.error_streak = 0;
        self.last_failed = false;
        Ok(Some(yuv_to_frame(&yuv)))
    }

    /// Drain the pictures still held in the reorder buffer (end of stream).
    pub fn flush(&mut self) -> Vec<DecodedFrame> {
        match self.decoder.flush_remaining() {
            Ok(frames) => frames.iter().map(yuv_to_frame).collect(),
            Err(e) => {
                log::debug!("H264: flush failed: {e}");
                Vec::new()
            },
        }
    }
}

/// Convert an openh264 YUV picture to an RGBA frame.
fn yuv_to_frame(yuv: &openh264::decoder::DecodedYUV<'_>) -> DecodedFrame {
    // dimensions_uv() returns (w/2, h/2); full frame is double.
    let (uv_w, uv_h) = yuv.dimensions_uv();
    let w = (uv_w * 2) as u32;
    let h = (uv_h * 2) as u32;

    // Use openh264's optimized YUV→RGBA converter (SIMD when width % 8 == 0).
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    yuv.write_rgba8(&mut rgba);

    DecodedFrame {
        rgba,
        width: w,
        height: h,
    }
}

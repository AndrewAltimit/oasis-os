//! H.264 decoder wrapper around the `openh264` crate.

use openh264::decoder::Decoder;

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
}

impl H264Decoder {
    /// Create a new decoder instance.
    pub fn new() -> Result<Self, VideoError> {
        let decoder =
            Decoder::new().map_err(|e| VideoError::Decode(format!("openh264 init: {e}")))?;
        Ok(Self {
            decoder,
            call_count: 0,
            error_streak: 0,
        })
    }

    /// Re-create the internal decoder after unrecoverable errors.
    pub fn reinit(&mut self) -> Result<(), VideoError> {
        self.decoder =
            Decoder::new().map_err(|e| VideoError::Decode(format!("openh264 reinit: {e}")))?;
        self.error_streak = 0;
        log::info!("H264: decoder reinitialized");
        Ok(())
    }

    /// Decode an H.264 packet (one or more NAL units).
    ///
    /// Returns `None` if the packet didn't produce a displayable frame
    /// (e.g. SPS/PPS parameter sets, B-frames waiting for references,
    /// or corrupted/non-IDR frames after seeking).
    ///
    /// On decode errors, sets `needs_reinit = true`. The caller should
    /// re-send SPS/PPS and skip to the next IDR frame.
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>, VideoError> {
        self.call_count += 1;
        let n = self.call_count;
        let yuv = match self.decoder.decode(data) {
            Ok(Some(yuv)) => yuv,
            Ok(None) => {
                if n <= 10 || n.is_multiple_of(100) {
                    log::debug!("H264: decode #{n} -> None (size={})", data.len());
                }
                return Ok(None);
            },
            Err(e) => {
                self.error_streak += 1;
                if n <= 20 || n.is_multiple_of(100) {
                    log::debug!("H264: decode #{n} -> Err({e}) (size={})", data.len());
                }
                return Ok(None);
            },
        };

        self.error_streak = 0;
        // dimensions_uv() returns (w/2, h/2); full frame is double.
        let (uv_w, uv_h) = yuv.dimensions_uv();
        let w = (uv_w * 2) as u32;
        let h = (uv_h * 2) as u32;

        // Use openh264's optimized YUV→RGBA converter (SIMD when width % 8 == 0).
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        yuv.write_rgba8(&mut rgba);

        Ok(Some(DecodedFrame {
            rgba,
            width: w,
            height: h,
        }))
    }
}

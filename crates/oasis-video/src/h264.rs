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
}

impl H264Decoder {
    /// Create a new decoder instance.
    pub fn new() -> Result<Self, VideoError> {
        let decoder =
            Decoder::new().map_err(|e| VideoError::Decode(format!("openh264 init: {e}")))?;
        Ok(Self { decoder })
    }

    /// Decode an H.264 packet (one or more NAL units).
    ///
    /// Returns `None` if the packet didn't produce a displayable frame
    /// (e.g. SPS/PPS parameter sets, or B-frames waiting for references).
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>, VideoError> {
        let yuv = match self.decoder.decode(data) {
            Ok(Some(yuv)) => yuv,
            Ok(None) => return Ok(None),
            Err(e) => return Err(VideoError::Decode(format!("h264 decode: {e}"))),
        };

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

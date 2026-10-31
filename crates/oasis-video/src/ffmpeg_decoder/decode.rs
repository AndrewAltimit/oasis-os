//! Packet-level decode: demux one packet, decode it, convert the output.

use ffmpeg_next as ffmpeg;
use ffmpeg_next::ffi;
use ffmpeg_next::packet::Mut as PacketMut;

use super::FfmpegDecoder;
use super::codec::create_scaler;
use crate::{AudioChunk, VideoError, VideoFrame};

/// Most decoded video frames held while the caller pulls audio. A 480p
/// RGBA frame is ~1.2 MB, and a caller that only pulls audio (the
/// desktop player's audio-only fallback) would otherwise buffer every
/// frame of the stream; the oldest are dropped instead.
const MAX_BUFFERED_VIDEO_FRAMES: usize = 8;
/// Most decoded audio chunks held while the caller pulls video (~20 s of
/// AAC at 1024 samples / 48 kHz); the oldest are dropped past it.
const MAX_BUFFERED_AUDIO_CHUNKS: usize = 1024;

impl FfmpegDecoder {
    /// Read the next packet from the format context.
    ///
    /// Returns `None` at end-of-stream.
    pub(super) fn read_packet(&mut self) -> Option<ffmpeg::Packet> {
        if self.eof {
            return None;
        }

        let mut pkt = ffmpeg::Packet::empty();
        // SAFETY: `format_ctx` is the open context this decoder owns, and
        // `pkt` is a valid empty packet for ffmpeg to fill.
        let ret = unsafe { ffi::av_read_frame(self.format_ctx, pkt.as_mut_ptr()) };
        if ret < 0 {
            self.eof = true;
            return None;
        }
        Some(pkt)
    }

    /// Drop the oldest buffered video frames past
    /// [`MAX_BUFFERED_VIDEO_FRAMES`] (called on the audio pull path, which
    /// decodes video without consuming it).
    pub(super) fn trim_video_buffer(&mut self) {
        let excess = self
            .video_buffer
            .len()
            .saturating_sub(MAX_BUFFERED_VIDEO_FRAMES);
        if excess > 0 {
            log::debug!("FFmpeg: dropping {excess} unread video frame(s)");
            self.video_buffer.drain(..excess);
        }
    }

    /// Queue a decoded audio chunk for a later audio pull, dropping the
    /// oldest past [`MAX_BUFFERED_AUDIO_CHUNKS`].
    pub(super) fn buffer_audio(&mut self, chunk: AudioChunk) {
        if self.audio_buffer.len() >= MAX_BUFFERED_AUDIO_CHUNKS {
            self.audio_buffer.pop_front();
        }
        self.audio_buffer.push_back(chunk);
    }

    /// Decode a video packet, appending every RGBA frame it yields to
    /// `video_buffer` in decode order.
    ///
    /// Frames drained to make room (EAGAIN on send) are older than the
    /// one received after the retried send; returning the new frame
    /// ahead of them (as this used to) made timestamps jump backwards.
    pub(super) fn decode_video_packet(&mut self, pkt: &ffmpeg::Packet) -> Result<(), VideoError> {
        let decoder = self
            .video_decoder
            .as_mut()
            .ok_or_else(|| VideoError::NoTrack("no video decoder".into()))?;

        // Frames drained during EAGAIN recovery (older than anything the
        // retried send produces).
        let mut drained_frames: Vec<ffmpeg::frame::Video> = Vec::new();

        match decoder.send_packet(pkt) {
            Ok(()) => {},
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::util::error::EAGAIN,
            }) => {
                // Decoder buffer full — drain frames first, then retry.
                let mut decoded = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut decoded).is_ok() {
                    drained_frames.push(decoded);
                    decoded = ffmpeg::frame::Video::empty();
                }
                decoder
                    .send_packet(pkt)
                    .map_err(|e| VideoError::Decode(format!("send video packet: {e}")))?;
            },
            Err(e) => {
                return Err(VideoError::Decode(format!("send video packet: {e}")));
            },
        }

        let mut decoded = ffmpeg::frame::Video::empty();
        let received = match decoder.receive_frame(&mut decoded) {
            Ok(()) => true,
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::util::error::EAGAIN,
            }) => false,
            Err(e) => {
                return Err(VideoError::Decode(format!("receive video frame: {e}")));
            },
        };

        if received {
            drained_frames.push(decoded);
        }
        for frame in &drained_frames {
            if let Some(vf) = self.convert_video_frame(frame)? {
                self.video_buffer.push_back(vf);
            }
        }
        Ok(())
    }

    /// Convert a decoded ffmpeg video frame to an RGBA `VideoFrame`.
    fn convert_video_frame(
        &mut self,
        decoded: &ffmpeg::frame::Video,
    ) -> Result<Option<VideoFrame>, VideoError> {
        let w = decoded.width();
        let h = decoded.height();
        if w == 0 || h == 0 {
            return Ok(None);
        }

        let fmt = decoded.format();
        if fmt == ffmpeg::format::Pixel::None {
            return Ok(None); // skip frames with unresolved pixel format
        }

        // Recreate scaler when dimensions change, or create it lazily on
        // the first frame (handles streams where pixel format wasn't known
        // until the first packet was decoded).
        if self.scaler.is_none() || w != self.video_width || h != self.video_height {
            self.video_width = w;
            self.video_height = h;
            self.scaler = create_scaler(fmt, w, h).ok();
        }

        let scaler = self
            .scaler
            .as_mut()
            .ok_or_else(|| VideoError::Decode("no scaler available".into()))?;

        let mut rgba_frame = ffmpeg::frame::Video::empty();
        scaler
            .run(decoded, &mut rgba_frame)
            .map_err(|e| VideoError::Decode(format!("scaler run: {e}")))?;

        let stride = rgba_frame.stride(0);
        let data = rgba_frame.data(0);
        let row_bytes = (w * 4) as usize;

        let rgba = if stride == row_bytes {
            data[..row_bytes * h as usize].to_vec()
        } else {
            let mut rgba = Vec::with_capacity(row_bytes * h as usize);
            for row in data.chunks(stride).take(h as usize) {
                rgba.extend_from_slice(&row[..row_bytes]);
            }
            rgba
        };

        let ts = decoded.pts().unwrap_or(0);
        Ok(Some(VideoFrame {
            rgba,
            width: w,
            height: h,
            timestamp_secs: ts as f64 * self.video_time_base,
        }))
    }

    /// Decode an audio packet and return PCM f32 chunks.
    pub(super) fn decode_audio_packet(
        &mut self,
        pkt: &ffmpeg::Packet,
    ) -> Result<Option<AudioChunk>, VideoError> {
        let decoder = self
            .audio_decoder
            .as_mut()
            .ok_or_else(|| VideoError::NoTrack("no audio decoder".into()))?;

        decoder
            .send_packet(pkt)
            .map_err(|e| VideoError::Decode(format!("send audio packet: {e}")))?;

        let mut decoded = ffmpeg::frame::Audio::empty();
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {},
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::util::error::EAGAIN,
            }) => {
                return Ok(None);
            },
            Err(e) => {
                return Err(VideoError::Decode(format!("receive audio frame: {e}")));
            },
        }

        let ts = decoded.pts().unwrap_or(0);
        let timestamp_secs = ts as f64 * self.audio_time_base;

        // Resample to f32 interleaved. Without a resampler there is no
        // f32 to hand out (AAC decodes to planar float).
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(None);
        };
        let mut resampled = ffmpeg::frame::Audio::empty();
        resampler
            .run(&decoded, &mut resampled)
            .map_err(|e| VideoError::Decode(format!("resample: {e}")))?;

        let samples = resampled.samples();
        let channels = self.audio_channels as usize;
        if samples == 0 || channels == 0 {
            return Ok(None);
        }

        let data = resampled.data(0);
        let byte_len = samples * channels * 4; // f32 = 4 bytes
        if data.len() < byte_len {
            return Ok(None);
        }

        // The resampler outputs packed native-endian f32.
        let pcm_f32: Vec<f32> = data[..byte_len]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();

        Ok(Some(AudioChunk {
            pcm_f32,
            channels: self.audio_channels,
            sample_rate: self.audio_sample_rate,
            timestamp_secs,
        }))
    }
}

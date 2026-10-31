//! FFmpeg-based decoder for MP4/H.264+AAC.
//!
//! Replaces both the symphonia demuxer and openh264/AAC decoders with ffmpeg's
//! unified pipeline. Statically linked — no runtime dependencies.
//!
//! Layout: `io` (custom AVIO over a [`VideoSource`]), `codec` (decoder /
//! scaler / resampler construction), `decode` (per-packet decode and
//! frame conversion); this module owns the format context and the
//! interleaved video/audio pull API.

mod codec;
mod decode;
mod io;

use std::collections::VecDeque;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::ffi;

use crate::decoder::DecoderBackend;
use crate::{AudioChunk, VideoError, VideoFrame, VideoSource};
use codec::{
    create_audio_decoder, create_resampler, create_scaler, create_video_decoder,
    ffmpeg_error_string, time_base_secs,
};
use io::{IoContext, alloc_custom_io, free_custom_avio, free_io_context};

/// FFmpeg-based MP4/H.264+AAC decoder.
///
/// Handles demuxing, video decoding, audio decoding, and format conversion
/// in a single unified pipeline.
pub struct FfmpegDecoder {
    /// FFmpeg format context.  Its `pb` is our custom AVIO context, which
    /// `avformat_close_input` does not free (`AVFMT_FLAG_CUSTOM_IO`): `Drop`
    /// frees it with [`free_custom_avio`].
    format_ctx: *mut ffi::AVFormatContext,
    /// The AVIO callbacks' `opaque` state, from `Box::into_raw` (a raw
    /// pointer, not a `Box`, so moving the decoder cannot invalidate the
    /// pointer ffmpeg holds).  Freed in `Drop` after the format context.
    io_ctx: *mut IoContext,

    // Video
    video_stream_idx: Option<usize>,
    video_decoder: Option<ffmpeg::decoder::Video>,
    video_time_base: f64,
    video_width: u32,
    video_height: u32,
    scaler: Option<ffmpeg::software::scaling::Context>,

    // Audio
    audio_stream_idx: Option<usize>,
    audio_decoder: Option<ffmpeg::decoder::Audio>,
    audio_time_base: f64,
    audio_sample_rate: u32,
    audio_channels: u16,
    resampler: Option<ffmpeg::software::resampling::Context>,

    /// Buffered audio chunks produced while reading video packets.
    audio_buffer: VecDeque<AudioChunk>,
    /// Decoded video frames not yet returned (decode order; also fed
    /// while reading audio packets).
    video_buffer: VecDeque<VideoFrame>,

    /// Whether we've hit end-of-stream.
    eof: bool,
}

// SAFETY: FfmpegDecoder is used from a single decode thread.
// ffmpeg decoders are not thread-safe, but we never share across threads.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    /// Pick the first decodable video and audio streams and set up their
    /// decoders, scaler and resampler.
    ///
    /// We can't use `ffmpeg::format::input()` with custom I/O, so the raw
    /// context's stream array is inspected directly.
    fn select_streams(&mut self) {
        // SAFETY: format_ctx is valid after successful avformat_find_stream_info.
        let nb_streams = unsafe { (*self.format_ctx).nb_streams } as usize;
        for i in 0..nb_streams {
            // SAFETY: `i < nb_streams`, so the stream pointer is in bounds.
            let stream = unsafe { *(*self.format_ctx).streams.add(i) };
            // SAFETY: stream is valid; codecpar is a non-null pointer set by ffmpeg.
            let codecpar = unsafe { &*(*stream).codecpar };
            // SAFETY: stream pointer is valid within the format context.
            let time_base = time_base_secs(unsafe { (*stream).time_base });

            match codecpar.codec_type {
                ffi::AVMediaType::AVMEDIA_TYPE_VIDEO if self.video_stream_idx.is_none() => {
                    match create_video_decoder(codecpar) {
                        Ok(dec) => {
                            self.video_time_base = time_base;
                            self.video_width = dec.width();
                            self.video_height = dec.height();
                            // YUV -> RGBA scaler; created lazily on the first
                            // frame when the probe left the size unknown.
                            if dec.width() > 0 && dec.height() > 0 {
                                self.scaler =
                                    create_scaler(dec.format(), dec.width(), dec.height()).ok();
                            }
                            log::info!(
                                "FFmpeg: video stream {i}: {}x{} {:?}",
                                dec.width(),
                                dec.height(),
                                codecpar.codec_id
                            );
                            self.video_decoder = Some(dec);
                            self.video_stream_idx = Some(i);
                        },
                        Err(e) => log::warn!("FFmpeg: skip video stream {i}: {e}"),
                    }
                },
                ffi::AVMediaType::AVMEDIA_TYPE_AUDIO if self.audio_stream_idx.is_none() => {
                    match create_audio_decoder(codecpar) {
                        Ok(dec) => {
                            self.audio_time_base = time_base;
                            self.audio_sample_rate = dec.rate();
                            self.audio_channels = dec.channels() as u16;
                            // Convert to f32 interleaved.
                            self.resampler = create_resampler(&dec).ok();
                            log::info!(
                                "FFmpeg: audio stream {i}: {}Hz {}ch",
                                self.audio_sample_rate,
                                self.audio_channels
                            );
                            self.audio_decoder = Some(dec);
                            self.audio_stream_idx = Some(i);
                        },
                        Err(e) => log::warn!("FFmpeg: skip audio stream {i}: {e}"),
                    }
                },
                _ => {},
            }
        }
    }

    /// Whether a video track was found.
    #[cfg(test)]
    pub fn has_video(&self) -> bool {
        self.video_stream_idx.is_some()
    }

    /// Whether an audio track was found.
    #[cfg(test)]
    pub fn has_audio(&self) -> bool {
        self.audio_stream_idx.is_some()
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        // Drop decoders first (they reference codec contexts).
        self.video_decoder.take();
        self.audio_decoder.take();
        self.scaler.take();
        self.resampler.take();

        if !self.format_ctx.is_null() {
            // SAFETY: `format_ctx` is the context `avformat_open_input`
            // opened and nothing freed yet.  With AVFMT_FLAG_CUSTOM_IO,
            // `avformat_close_input` leaves `pb` alone, so it is freed
            // afterwards.
            unsafe {
                let avio = (*self.format_ctx).pb;
                ffi::avformat_close_input(&mut self.format_ctx);
                free_custom_avio(avio);
            }
        }
        // SAFETY: from `alloc_custom_io` in `open_stream`; the AVIO context
        // that called back into it was freed above.
        unsafe { free_io_context(self.io_ctx) };
        self.io_ctx = std::ptr::null_mut();
    }
}

impl DecoderBackend for FfmpegDecoder {
    /// Open a video from a streaming source.
    fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        ffmpeg::init().map_err(|e| VideoError::Demux(format!("ffmpeg init: {e}")))?;

        // On every error path below the pieces built so far are freed in
        // reverse order: the format context, then the AVIO context and its
        // buffer, then the callbacks' state.
        let io = alloc_custom_io(source)?;
        let fail = |msg: String| {
            // SAFETY: called only once no format context references
            // `io.avio` any more; the AVIO goes first, then its state.
            unsafe {
                free_custom_avio(io.avio);
                free_io_context(io.opaque);
            }
            Err(VideoError::Demux(msg))
        };

        // SAFETY: no preconditions; returns null or a fresh context.
        let mut format_ctx = unsafe { ffi::avformat_alloc_context() };
        if format_ctx.is_null() {
            return fail("avformat_alloc_context failed".into());
        }

        // SAFETY: format_ctx is a freshly allocated, valid context; it
        // borrows our AVIO (which it will not free: custom I/O).
        unsafe {
            (*format_ctx).pb = io.avio;
            (*format_ctx).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as i32;
        }

        // SAFETY: `format_ctx` is a valid pre-allocated context with our
        // AVIO in `pb`; a null URL is allowed with custom I/O.  On failure
        // ffmpeg frees the context and nulls `format_ctx`, but leaves the
        // custom `pb` to us.
        let ret = unsafe {
            ffi::avformat_open_input(
                &mut format_ctx,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ret < 0 {
            return fail(format!(
                "avformat_open_input failed: {}",
                ffmpeg_error_string(ret)
            ));
        }

        // Increase analyze duration and probe size so ffmpeg can detect
        // codec parameters (especially pixel format) for streams that
        // require more data to probe (e.g. some archive.org H.264 files).
        // SAFETY: format_ctx is valid after successful avformat_open_input.
        unsafe {
            // 5 seconds in AV_TIME_BASE units.
            (*format_ctx).max_analyze_duration = 5_000_000;
            // 10 MB probe size (default is 5 MB).
            (*format_ctx).probesize = 10 * 1024 * 1024;
        }

        // SAFETY: format_ctx is a valid, opened context.
        let ret = unsafe { ffi::avformat_find_stream_info(format_ctx, std::ptr::null_mut()) };
        if ret < 0 {
            // SAFETY: format_ctx is a valid opened context; closing it
            // leaves the custom AVIO (no longer referenced) to `fail`.
            unsafe { ffi::avformat_close_input(&mut format_ctx) };
            return fail(format!(
                "avformat_find_stream_info failed: {}",
                ffmpeg_error_string(ret)
            ));
        }

        let mut dec = Self {
            format_ctx,
            io_ctx: io.opaque,
            video_stream_idx: None,
            video_decoder: None,
            video_time_base: 0.0,
            video_width: 0,
            video_height: 0,
            scaler: None,
            audio_stream_idx: None,
            audio_decoder: None,
            audio_time_base: 0.0,
            audio_sample_rate: 0,
            audio_channels: 0,
            resampler: None,
            audio_buffer: VecDeque::new(),
            video_buffer: VecDeque::new(),
            eof: false,
        };
        dec.select_streams();
        Ok(dec)
    }

    /// Decode the next video frame, buffering audio packets encountered along the way.
    fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        if self.video_decoder.is_none() {
            return Err(VideoError::NoTrack("no video track".into()));
        }

        let mut packets_read = 0u32;
        loop {
            if let Some(frame) = self.video_buffer.pop_front() {
                return Ok(Some(frame));
            }
            let Some(pkt) = self.read_packet() else {
                log::info!("FFmpeg: next_video_frame EOF after {packets_read} packets");
                return Ok(None);
            };
            packets_read += 1;

            let stream_idx = Some(pkt.stream());
            if stream_idx == self.video_stream_idx {
                // Non-fatal: a bad packet is skipped and decoding goes on.
                if let Err(e) = self.decode_video_packet(&pkt) {
                    log::warn!("FFmpeg: video decode error at pkt {packets_read}: {e}");
                } else if self.video_buffer.is_empty() && packets_read <= 5 {
                    log::debug!("FFmpeg: video pkt {packets_read} -> EAGAIN (need more data)");
                }
            } else if stream_idx == self.audio_stream_idx {
                // Buffer audio while looking for video.
                if let Ok(Some(chunk)) = self.decode_audio_packet(&pkt) {
                    self.buffer_audio(chunk);
                }
            }
            // Packets from other streams are skipped.
        }
    }

    /// Return a buffered audio chunk without reading new packets.
    ///
    /// Used by `drain_audio()` to avoid advancing the stream position.
    /// Only returns audio that was buffered during `next_video_frame()`.
    fn next_buffered_audio(&mut self) -> Option<AudioChunk> {
        self.audio_buffer.pop_front()
    }

    /// Decode the next audio chunk, buffering video frames encountered along the way.
    fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        if self.audio_decoder.is_none() {
            return Err(VideoError::NoTrack("no audio track".into()));
        }

        // Check buffer first.
        if let Some(chunk) = self.audio_buffer.pop_front() {
            return Ok(Some(chunk));
        }

        loop {
            let Some(pkt) = self.read_packet() else {
                return Ok(None);
            };

            let stream_idx = Some(pkt.stream());
            if stream_idx == self.audio_stream_idx {
                if let Some(chunk) = self.decode_audio_packet(&pkt)? {
                    return Ok(Some(chunk));
                }
            } else if stream_idx == self.video_stream_idx {
                // Buffer video while looking for audio (bounded: a caller
                // that only pulls audio must not accumulate every frame).
                let _ = self.decode_video_packet(&pkt);
                self.trim_video_buffer();
            }
        }
    }

    /// Seek to a position in seconds.
    fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        // Convert seconds to AV_TIME_BASE units.
        let ts = (secs * ffi::AV_TIME_BASE as f64) as i64;
        log::info!("FFmpeg: seeking to {secs}s (ts={ts})");

        // SAFETY: `format_ctx` is the open context this decoder owns.
        let ret = unsafe {
            ffi::avformat_seek_file(
                self.format_ctx,
                -1, // default stream
                i64::MIN,
                ts,
                ts,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
        };
        if ret < 0 {
            return Err(VideoError::Demux(format!(
                "seek to {secs}s failed: {}",
                ffmpeg_error_string(ret)
            )));
        }
        log::info!("FFmpeg: seek to {secs}s succeeded");

        // Flush decoder buffers.
        if let Some(dec) = self.video_decoder.as_mut() {
            dec.flush();
        }
        if let Some(dec) = self.audio_decoder.as_mut() {
            dec.flush();
        }

        // Clear buffered data.
        self.audio_buffer.clear();
        self.video_buffer.clear();
        self.eof = false;

        Ok(())
    }

    /// Video dimensions (may be 0x0 if no video track).
    fn video_size(&self) -> (u32, u32) {
        (self.video_width, self.video_height)
    }

    /// Audio sample rate and channel count.
    fn audio_format(&self) -> (u32, u16) {
        (self.audio_sample_rate, self.audio_channels)
    }
}

#[cfg(test)]
mod tests;

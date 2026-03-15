//! FFmpeg-based decoder for MP4/H.264+AAC.
//!
//! Replaces both the symphonia demuxer and openh264/AAC decoders with ffmpeg's
//! unified pipeline. Statically linked — no runtime dependencies.

use std::io::{Read, Seek, SeekFrom};

use ffmpeg_next as ffmpeg;
use ffmpeg_next::ffi;
use ffmpeg_next::packet::Mut as PacketMut;

use crate::{AudioChunk, VideoError, VideoFrame, VideoSource};

/// Custom I/O context wrapping a `Box<dyn VideoSource>` for ffmpeg's AVIO.
struct IoContext {
    source: Box<dyn VideoSource>,
    read_count: u64,
}

/// Read callback for ffmpeg's AVIO context.
///
/// # Safety
/// Called from ffmpeg via function pointer. `opaque` must be a valid
/// `*mut IoContext`.
unsafe extern "C" fn read_packet(
    opaque: *mut std::ffi::c_void,
    buf: *mut u8,
    buf_size: std::ffi::c_int,
) -> std::ffi::c_int {
    // SAFETY: `opaque` is a valid `*mut IoContext` — set during avio_alloc_context.
    let ctx = unsafe { &mut *(opaque as *mut IoContext) };
    // SAFETY: `buf` is a valid ffmpeg-allocated buffer of `buf_size` bytes.
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, buf_size as usize) };
    ctx.read_count += 1;
    match ctx.source.read(slice) {
        Ok(0) => {
            if ctx.read_count <= 20 || ctx.read_count % 1000 == 0 {
                log::debug!(
                    "AVIO: read #{} -> EOF (requested {})",
                    ctx.read_count,
                    buf_size
                );
            }
            ffi::AVERROR_EOF
        },
        Ok(n) => {
            if ctx.read_count <= 10 || ctx.read_count % 5000 == 0 {
                log::trace!(
                    "AVIO: read #{} -> {} bytes (requested {})",
                    ctx.read_count,
                    n,
                    buf_size
                );
            }
            n as std::ffi::c_int
        },
        Err(e) => {
            log::warn!("AVIO: read #{} -> error: {e}", ctx.read_count);
            ffi::AVERROR_EOF
        },
    }
}

/// Seek callback for ffmpeg's AVIO context.
///
/// # Safety
/// Called from ffmpeg via function pointer. `opaque` must be a valid
/// `*mut IoContext`.
unsafe extern "C" fn seek_source(
    opaque: *mut std::ffi::c_void,
    offset: i64,
    whence: std::ffi::c_int,
) -> i64 {
    // SAFETY: `opaque` is a valid `*mut IoContext` — set during avio_alloc_context.
    let ctx = unsafe { &mut *(opaque as *mut IoContext) };

    // AVSEEK_SIZE: ffmpeg is asking for the total size.
    if whence & ffi::AVSEEK_SIZE as i32 != 0 {
        return ctx.source.byte_len().map(|l| l as i64).unwrap_or(-1);
    }

    let seek_from = match whence & 0xFF {
        0 => SeekFrom::Start(offset as u64), // SEEK_SET
        1 => SeekFrom::Current(offset),      // SEEK_CUR
        2 => SeekFrom::End(offset),          // SEEK_END
        _ => return -1,
    };

    match ctx.source.seek(seek_from) {
        Ok(pos) => pos as i64,
        Err(_) => -1,
    }
}

/// FFmpeg-based MP4/H.264+AAC decoder.
///
/// Handles demuxing, video decoding, audio decoding, and format conversion
/// in a single unified pipeline.
pub struct FfmpegDecoder {
    /// FFmpeg format context (owns the AVIO context).
    format_ctx: *mut ffi::AVFormatContext,
    /// The AVIO buffer allocated via `av_malloc` (freed with the context).
    _avio_buffer: *mut u8,
    /// Boxed IoContext kept alive for the AVIO callbacks.
    _io_ctx: Box<IoContext>,

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
    audio_buffer: std::collections::VecDeque<AudioChunk>,
    /// Buffered video frames produced while reading audio packets.
    video_buffer: std::collections::VecDeque<VideoFrame>,

    /// Whether we've hit end-of-stream.
    eof: bool,
}

// SAFETY: FfmpegDecoder is used from a single decode thread.
// ffmpeg decoders are not thread-safe, but we never share across threads.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    /// Open a video from a streaming source.
    pub fn open_stream(source: Box<dyn VideoSource>) -> Result<Self, VideoError> {
        ffmpeg::init().map_err(|e| VideoError::Demux(format!("ffmpeg init: {e}")))?;

        // Set up custom I/O via AVIO context.
        let io_ctx = Box::new(IoContext {
            source,
            read_count: 0,
        });
        let io_ptr = &*io_ctx as *const IoContext as *mut std::ffi::c_void;

        const AVIO_BUF_SIZE: usize = 32 * 1024;

        // SAFETY: av_malloc allocates memory that ffmpeg manages.
        let avio_buffer = unsafe { ffi::av_malloc(AVIO_BUF_SIZE) as *mut u8 };
        if avio_buffer.is_null() {
            return Err(VideoError::Demux("av_malloc failed".into()));
        }

        // SAFETY: Creating AVIO context with our read/seek callbacks.
        let avio_ctx = unsafe {
            ffi::avio_alloc_context(
                avio_buffer,
                AVIO_BUF_SIZE as i32,
                0, // read-only
                io_ptr,
                Some(read_packet),
                None, // no write
                Some(seek_source),
            )
        };
        if avio_ctx.is_null() {
            // SAFETY: Free the buffer we allocated since avio_alloc_context failed.
            // SAFETY: avio_buffer was allocated by av_malloc and not yet owned by an AVIO context.
            unsafe { ffi::av_free(avio_buffer as *mut std::ffi::c_void) };
            return Err(VideoError::Demux("avio_alloc_context failed".into()));
        }

        // SAFETY: Allocate format context and assign our AVIO.
        let format_ctx = unsafe { ffi::avformat_alloc_context() };
        if format_ctx.is_null() {
            // SAFETY: avio_ctx was successfully allocated above; free it on error.
            unsafe { ffi::avio_context_free(&mut (avio_ctx as *mut _)) };
            return Err(VideoError::Demux("avformat_alloc_context failed".into()));
        }

        // SAFETY: format_ctx is a freshly allocated, valid context. Assigning our AVIO.
        unsafe {
            (*format_ctx).pb = avio_ctx;
            // Hint MP4 format.
            (*format_ctx).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as i32;
        }

        // SAFETY: Open the input. NULL filename since we use custom I/O.
        let ret = unsafe {
            ffi::avformat_open_input(
                &mut (format_ctx as *mut _),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ret < 0 {
            return Err(VideoError::Demux(format!(
                "avformat_open_input failed: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // SAFETY: Find stream info.
        let ret = unsafe { ffi::avformat_find_stream_info(format_ctx, std::ptr::null_mut()) };
        if ret < 0 {
            // SAFETY: format_ctx is valid; free on error before returning.
            unsafe { ffi::avformat_close_input(&mut (format_ctx as *mut _)) };
            return Err(VideoError::Demux(format!(
                "avformat_find_stream_info failed: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // Wrap in safe Input context for stream discovery.
        // We can't use ffmpeg::format::input() with custom I/O, so we inspect
        // the raw context directly.
        // SAFETY: format_ctx is valid after successful avformat_find_stream_info.
        let nb_streams = unsafe { (*format_ctx).nb_streams } as usize;

        let mut video_stream_idx = None;
        let mut audio_stream_idx = None;
        let mut video_decoder = None;
        let mut audio_decoder = None;
        let mut video_time_base = 0.0;
        let mut audio_time_base = 0.0;
        let mut video_width = 0u32;
        let mut video_height = 0u32;
        let mut audio_sample_rate = 0u32;
        let mut audio_channels = 0u16;
        let mut scaler = None;
        let mut resampler = None;

        for i in 0..nb_streams {
            // SAFETY: Accessing stream array within bounds.
            let stream = unsafe { *(*format_ctx).streams.add(i) };
            // SAFETY: stream is valid; codecpar is a non-null pointer set by ffmpeg.
            let codecpar = unsafe { &*(*stream).codecpar };

            let codec_type = codecpar.codec_type;

            if codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO && video_stream_idx.is_none() {
                // SAFETY: stream pointer is valid within the format context.
                let tb = unsafe { (*stream).time_base };
                video_time_base = tb.num as f64 / tb.den as f64;

                // Create video decoder.
                match Self::create_video_decoder(codecpar) {
                    Ok(dec) => {
                        video_width = dec.width();
                        video_height = dec.height();

                        // Create scaler for YUV -> RGBA conversion.
                        if video_width > 0 && video_height > 0 {
                            scaler = Self::create_scaler(&dec).ok();
                        }

                        video_decoder = Some(dec);
                        video_stream_idx = Some(i);
                        log::info!(
                            "FFmpeg: video stream {i}: {}x{} {:?}",
                            video_width,
                            video_height,
                            codecpar.codec_id
                        );
                    },
                    Err(e) => log::warn!("FFmpeg: skip video stream {i}: {e}"),
                }
            } else if codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO
                && audio_stream_idx.is_none()
            {
                // SAFETY: stream pointer is valid within the format context.
                let tb = unsafe { (*stream).time_base };
                audio_time_base = tb.num as f64 / tb.den as f64;

                match Self::create_audio_decoder(codecpar) {
                    Ok(dec) => {
                        audio_sample_rate = dec.rate();
                        audio_channels = dec.channels() as u16;

                        // Create resampler to convert to f32 interleaved.
                        resampler = Self::create_resampler(&dec).ok();

                        audio_decoder = Some(dec);
                        audio_stream_idx = Some(i);
                        log::info!(
                            "FFmpeg: audio stream {i}: {}Hz {}ch",
                            audio_sample_rate,
                            audio_channels
                        );
                    },
                    Err(e) => log::warn!("FFmpeg: skip audio stream {i}: {e}"),
                }
            }
        }

        Ok(Self {
            format_ctx,
            _avio_buffer: avio_buffer,
            _io_ctx: io_ctx,
            video_stream_idx,
            video_decoder,
            video_time_base,
            video_width,
            video_height,
            scaler,
            audio_stream_idx,
            audio_decoder,
            audio_time_base,
            audio_sample_rate,
            audio_channels,
            resampler,
            audio_buffer: std::collections::VecDeque::new(),
            video_buffer: std::collections::VecDeque::new(),
            eof: false,
        })
    }

    /// Create a video decoder from codec parameters.
    fn create_video_decoder(
        codecpar: &ffi::AVCodecParameters,
    ) -> Result<ffmpeg::decoder::Video, VideoError> {
        // SAFETY: Find decoder by codec ID.
        let codec = unsafe { ffi::avcodec_find_decoder(codecpar.codec_id) };
        if codec.is_null() {
            return Err(VideoError::Decode(format!(
                "no decoder for codec {:?}",
                codecpar.codec_id
            )));
        }

        // SAFETY: Allocate and configure codec context.
        let mut ctx = unsafe { ffi::avcodec_alloc_context3(codec) };
        if ctx.is_null() {
            return Err(VideoError::Decode("avcodec_alloc_context3 failed".into()));
        }

        // SAFETY: ctx and codecpar are valid pointers allocated above.
        let ret = unsafe { ffi::avcodec_parameters_to_context(ctx, codecpar) };
        if ret < 0 {
            // SAFETY: ctx was allocated by avcodec_alloc_context3; free on error.
            unsafe { ffi::avcodec_free_context(&mut ctx) };
            return Err(VideoError::Decode(format!(
                "avcodec_parameters_to_context: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // SAFETY: ctx is configured, codec is valid; open the decoder.
        let ret = unsafe { ffi::avcodec_open2(ctx, codec, std::ptr::null_mut()) };
        if ret < 0 {
            // SAFETY: ctx was allocated by avcodec_alloc_context3; free on error.
            unsafe { ffi::avcodec_free_context(&mut ctx) };
            return Err(VideoError::Decode(format!(
                "avcodec_open2: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // SAFETY: Wrap the raw context in ffmpeg-next's safe wrapper.
        // The safe wrapper takes ownership and will free it on drop.
        let codec_ctx = unsafe { ffmpeg::codec::context::Context::wrap(ctx, None) };
        codec_ctx
            .decoder()
            .video()
            .map_err(|e| VideoError::Decode(format!("video decoder: {e}")))
    }

    /// Create an audio decoder from codec parameters.
    fn create_audio_decoder(
        codecpar: &ffi::AVCodecParameters,
    ) -> Result<ffmpeg::decoder::Audio, VideoError> {
        // SAFETY: Find decoder by codec ID from ffmpeg's static registry.
        let codec = unsafe { ffi::avcodec_find_decoder(codecpar.codec_id) };
        if codec.is_null() {
            return Err(VideoError::Decode(format!(
                "no decoder for audio codec {:?}",
                codecpar.codec_id
            )));
        }

        // SAFETY: Allocate codec context for the found decoder.
        let mut ctx = unsafe { ffi::avcodec_alloc_context3(codec) };
        if ctx.is_null() {
            return Err(VideoError::Decode("avcodec_alloc_context3 failed".into()));
        }

        // SAFETY: Copy codec parameters into context; ctx and codecpar are valid.
        let ret = unsafe { ffi::avcodec_parameters_to_context(ctx, codecpar) };
        if ret < 0 {
            // SAFETY: Free the context we allocated on error.
            unsafe { ffi::avcodec_free_context(&mut ctx) };
            return Err(VideoError::Decode(format!(
                "avcodec_parameters_to_context: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // SAFETY: Open the codec with the configured context.
        let ret = unsafe { ffi::avcodec_open2(ctx, codec, std::ptr::null_mut()) };
        if ret < 0 {
            // SAFETY: Free the context we allocated on error.
            unsafe { ffi::avcodec_free_context(&mut ctx) };
            return Err(VideoError::Decode(format!(
                "avcodec_open2: {}",
                ffmpeg_error_string(ret)
            )));
        }

        // SAFETY: Wrap raw context in ffmpeg-next's safe wrapper, which takes ownership.
        let codec_ctx = unsafe { ffmpeg::codec::context::Context::wrap(ctx, None) };
        codec_ctx
            .decoder()
            .audio()
            .map_err(|e| VideoError::Decode(format!("audio decoder: {e}")))
    }

    /// Create a pixel format scaler (YUV -> RGBA).
    fn create_scaler(
        dec: &ffmpeg::decoder::Video,
    ) -> Result<ffmpeg::software::scaling::Context, VideoError> {
        ffmpeg::software::scaling::Context::get(
            dec.format(),
            dec.width(),
            dec.height(),
            ffmpeg::format::Pixel::RGBA,
            dec.width(),
            dec.height(),
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| VideoError::Decode(format!("scaler init: {e}")))
    }

    /// Create an audio resampler (any format -> f32 interleaved).
    fn create_resampler(
        dec: &ffmpeg::decoder::Audio,
    ) -> Result<ffmpeg::software::resampling::Context, VideoError> {
        let channel_layout = dec.channel_layout();
        ffmpeg::software::resampling::Context::get(
            dec.format(),
            channel_layout,
            dec.rate(),
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            channel_layout,
            dec.rate(),
        )
        .map_err(|e| VideoError::Decode(format!("resampler init: {e}")))
    }

    /// Read the next packet from the format context.
    ///
    /// Returns `None` at end-of-stream.
    fn read_packet(&mut self) -> Option<ffmpeg::Packet> {
        if self.eof {
            return None;
        }

        let mut pkt = ffmpeg::Packet::empty();
        // SAFETY: Read from our format context.
        let ret = unsafe { ffi::av_read_frame(self.format_ctx, pkt.as_mut_ptr()) };
        if ret < 0 {
            self.eof = true;
            return None;
        }
        Some(pkt)
    }

    /// Decode a video packet and return an RGBA frame.
    fn decode_video_packet(
        &mut self,
        pkt: &ffmpeg::Packet,
    ) -> Result<Option<VideoFrame>, VideoError> {
        let decoder = self
            .video_decoder
            .as_mut()
            .ok_or_else(|| VideoError::NoTrack("no video decoder".into()))?;

        // Collect any frames drained during EAGAIN recovery.
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
                // Retry send after draining.
                decoder
                    .send_packet(pkt)
                    .map_err(|e| VideoError::Decode(format!("send video packet: {e}")))?;
            },
            Err(e) => {
                return Err(VideoError::Decode(format!("send video packet: {e}")));
            },
        }

        // Convert and buffer any drained frames.
        for frame in &drained_frames {
            if let Some(vf) = self.convert_video_frame(frame)? {
                self.video_buffer.push_back(vf);
            }
        }

        // Try to receive a decoded frame.
        let decoder = self
            .video_decoder
            .as_mut()
            .expect("video decoder verified present above");
        let mut decoded = ffmpeg::frame::Video::empty();
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {},
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::util::error::EAGAIN,
            }) => {
                // If we buffered frames during drain, return the first one.
                if let Some(frame) = self.video_buffer.pop_front() {
                    return Ok(Some(frame));
                }
                return Ok(None);
            },
            Err(e) => {
                return Err(VideoError::Decode(format!("receive video frame: {e}")));
            },
        }

        self.convert_video_frame(&decoded)
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

        if w != self.video_width || h != self.video_height {
            self.video_width = w;
            self.video_height = h;
            // Recreate scaler for new dimensions.
            self.scaler = Self::create_scaler(
                self.video_decoder
                    .as_ref()
                    .expect("video decoder verified present"),
            )
            .ok();
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
        let expected_stride = (w * 4) as usize;

        let rgba = if stride == expected_stride {
            data[..expected_stride * h as usize].to_vec()
        } else {
            let mut rgba = Vec::with_capacity(expected_stride * h as usize);
            for row in 0..h as usize {
                let start = row * stride;
                rgba.extend_from_slice(&data[start..start + expected_stride]);
            }
            rgba
        };

        let ts = decoded.pts().unwrap_or(0);
        let timestamp_secs = ts as f64 * self.video_time_base;

        Ok(Some(VideoFrame {
            rgba,
            width: w,
            height: h,
            timestamp_secs,
        }))
    }

    /// Decode an audio packet and return PCM f32 chunks.
    fn decode_audio_packet(
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

        // Resample to f32 interleaved.
        if let Some(resampler) = self.resampler.as_mut() {
            let mut resampled = ffmpeg::frame::Audio::empty();
            // Use the delay-based API to handle buffered samples.
            match resampler.run(&decoded, &mut resampled) {
                Ok(Some(_)) | Ok(None) => {},
                Err(e) => {
                    return Err(VideoError::Decode(format!("resample: {e}")));
                },
            }

            let samples = resampled.samples();
            let channels = self.audio_channels as usize;
            if samples == 0 || channels == 0 {
                return Ok(None);
            }

            let data = resampled.data(0);
            let total_floats = samples * channels;
            let byte_len = total_floats * 4; // f32 = 4 bytes
            if data.len() < byte_len {
                return Ok(None);
            }

            // SAFETY: Reinterpreting f32 bytes. The resampler outputs f32 packed.
            let pcm_f32: Vec<f32> = data[..byte_len]
                .chunks_exact(4)
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                .collect();

            Ok(Some(AudioChunk {
                pcm_f32,
                channels: self.audio_channels,
                sample_rate: self.audio_sample_rate,
                timestamp_secs,
            }))
        } else {
            // No resampler — try to extract f32 directly (unlikely for AAC).
            Ok(None)
        }
    }

    /// Decode the next video frame, buffering audio packets encountered along the way.
    pub fn next_video_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        if self.video_decoder.is_none() {
            return Err(VideoError::NoTrack("no video track".into()));
        }

        // Check buffer first.
        if let Some(frame) = self.video_buffer.pop_front() {
            return Ok(Some(frame));
        }

        let mut packets_read = 0u32;
        loop {
            let pkt = match self.read_packet() {
                Some(p) => p,
                None => {
                    log::info!("FFmpeg: next_video_frame EOF after {packets_read} packets");
                    return Ok(None);
                },
            };
            packets_read += 1;

            let stream_idx = pkt.stream() as usize;

            if Some(stream_idx) == self.video_stream_idx {
                match self.decode_video_packet(&pkt) {
                    Ok(Some(frame)) => return Ok(Some(frame)),
                    Ok(None) => {
                        if packets_read <= 5 {
                            log::debug!(
                                "FFmpeg: video pkt {packets_read} -> EAGAIN (need more data)"
                            );
                        }
                        continue;
                    },
                    Err(e) => {
                        log::warn!("FFmpeg: video decode error at pkt {packets_read}: {e}");
                        // Non-fatal: skip this packet and continue.
                        continue;
                    },
                }
            } else if Some(stream_idx) == self.audio_stream_idx {
                // Buffer audio while looking for video.
                if let Ok(Some(chunk)) = self.decode_audio_packet(&pkt) {
                    self.audio_buffer.push_back(chunk);
                }
            }
            // Skip packets from other streams.
        }
    }

    /// Return a buffered audio chunk without reading new packets.
    ///
    /// Used by `drain_audio()` to avoid advancing the stream position.
    /// Only returns audio that was buffered during `next_video_frame()`.
    pub fn next_buffered_audio(&mut self) -> Option<AudioChunk> {
        self.audio_buffer.pop_front()
    }

    /// Decode the next audio chunk, buffering video frames encountered along the way.
    pub fn next_audio_samples(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        if self.audio_decoder.is_none() {
            return Err(VideoError::NoTrack("no audio track".into()));
        }

        // Check buffer first.
        if let Some(chunk) = self.audio_buffer.pop_front() {
            return Ok(Some(chunk));
        }

        loop {
            let pkt = match self.read_packet() {
                Some(p) => p,
                None => return Ok(None),
            };

            let stream_idx = pkt.stream() as usize;

            if Some(stream_idx) == self.audio_stream_idx {
                match self.decode_audio_packet(&pkt)? {
                    Some(chunk) => return Ok(Some(chunk)),
                    None => continue,
                }
            } else if Some(stream_idx) == self.video_stream_idx {
                // Buffer video while looking for audio.
                if let Ok(Some(frame)) = self.decode_video_packet(&pkt) {
                    self.video_buffer.push_back(frame);
                }
            }
        }
    }

    /// Seek to a position in seconds.
    pub fn seek(&mut self, secs: f64) -> Result<(), VideoError> {
        // Convert seconds to AV_TIME_BASE units.
        let ts = (secs * ffi::AV_TIME_BASE as f64) as i64;
        log::info!("FFmpeg: seeking to {secs}s (ts={ts})");

        // SAFETY: Seek in the format context.
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
    pub fn video_size(&self) -> (u32, u32) {
        (self.video_width, self.video_height)
    }

    /// Audio sample rate and channel count.
    pub fn audio_format(&self) -> (u32, u16) {
        (self.audio_sample_rate, self.audio_channels)
    }

    /// Whether a video track was found.
    pub fn has_video(&self) -> bool {
        self.video_stream_idx.is_some()
    }

    /// Whether an audio track was found.
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

        // SAFETY: Close the format context. This also frees the AVIO context.
        if !self.format_ctx.is_null() {
            unsafe {
                ffi::avformat_close_input(&mut self.format_ctx);
            }
        }
    }
}

/// Convert an ffmpeg error code to a human-readable string.
fn ffmpeg_error_string(errnum: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: av_strerror writes into our buffer.
    unsafe {
        ffi::av_strerror(errnum, buf.as_mut_ptr(), buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

// ---------------------------------------------------------------------------
// Item 77: FFmpeg decoder error path tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::io::Cursor;

    /// Wrap a cursor as a VideoSource via the blanket impl in lib.rs.
    fn cursor_source(data: Vec<u8>) -> Box<dyn VideoSource> {
        Box::new(Cursor::new(data))
    }

    #[test]
    fn open_empty_data_fails() {
        let result = FfmpegDecoder::open_stream(cursor_source(Vec::new()));
        assert!(result.is_err());
    }

    #[test]
    fn open_garbage_data_fails() {
        let result = FfmpegDecoder::open_stream(cursor_source(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert!(result.is_err());
    }

    #[test]
    fn open_single_byte_fails() {
        let result = FfmpegDecoder::open_stream(cursor_source(vec![0xFF]));
        assert!(result.is_err());
    }

    #[test]
    fn open_truncated_ftyp_fails() {
        // Valid ftyp header but truncated content.
        let mut data = Vec::new();
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0; 8]); // truncated
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        assert!(result.is_err());
    }

    #[test]
    fn open_all_zeros_fails() {
        let result = FfmpegDecoder::open_stream(cursor_source(vec![0; 1024]));
        assert!(result.is_err());
    }

    #[test]
    fn open_random_noise_fails() {
        // Pseudorandom-ish data that is definitely not MP4.
        let data: Vec<u8> = (0..2048u16)
            .map(|i| (i.wrapping_mul(31) ^ 0xA5) as u8)
            .collect();
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        assert!(result.is_err());
    }

    #[test]
    fn open_ftyp_only_no_tracks() {
        // Valid ftyp atom but nothing else.
        let mut data = Vec::new();
        let ftyp = b"isom\x00\x00\x00\x00isomavc1";
        let size = (8 + ftyp.len()) as u32;
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(ftyp);
        // This may or may not open successfully depending on ffmpeg's
        // tolerance, but must not panic.
        match FfmpegDecoder::open_stream(cursor_source(data)) {
            Ok(dec) => {
                // If it opens, it shouldn't have useful tracks.
                assert!(!dec.has_video() || !dec.has_audio());
            },
            Err(_) => {
                // Error is acceptable.
            },
        }
    }

    #[test]
    fn video_size_no_tracks() {
        // Open with data that might parse but has no video.
        // We test the accessor directly on a struct.
        // Can't easily construct FfmpegDecoder without ffmpeg, so
        // test that open_stream with garbage produces an error.
        let result = FfmpegDecoder::open_stream(cursor_source(vec![0; 32]));
        assert!(result.is_err());
    }

    #[test]
    fn ffmpeg_error_string_produces_output() {
        // AVERROR_EOF is a well-known error code.
        let msg = ffmpeg_error_string(ffi::AVERROR_EOF);
        assert!(!msg.is_empty());
    }

    #[test]
    fn ffmpeg_error_string_unknown_error() {
        // An unlikely error code should still produce some string.
        let msg = ffmpeg_error_string(-999999);
        assert!(!msg.is_empty());
    }

    #[test]
    fn ffmpeg_error_string_zero() {
        // Error code 0 (success) — ffmpeg should still produce output.
        let msg = ffmpeg_error_string(0);
        // Some versions return empty for 0, some return "Success".
        // Just check no panic.
        let _ = msg;
    }

    #[test]
    fn open_large_garbage_no_oom() {
        // 64KB of garbage should fail quickly without OOM.
        let data = vec![0xAB; 65536];
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        assert!(result.is_err());
    }

    #[test]
    fn open_wav_header_not_mp4() {
        // A WAV-like header should be rejected.
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&1000u32.to_le_bytes());
        data.extend_from_slice(b"WAVE");
        data.extend_from_slice(b"fmt ");
        data.extend_from_slice(&[0; 100]);
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        // ffmpeg may or may not recognize WAV, but it shouldn't crash.
        // The test validates no panic occurs.
        let _ = result;
    }

    #[test]
    fn open_truncated_moov_no_panic() {
        // ftyp + moov header that claims more bytes than available.
        let mut data = Vec::new();
        // ftyp
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0; 16]);
        // moov claiming 10000 bytes
        data.extend_from_slice(&10000u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0; 32]); // only 32 bytes
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        // Must not panic. Error or partial open are both fine.
        let _ = result;
    }

    #[test]
    fn open_repeated_ftyp_no_panic() {
        // Multiple ftyp atoms (malformed but shouldn't crash).
        let mut data = Vec::new();
        for _ in 0..10 {
            data.extend_from_slice(&24u32.to_be_bytes());
            data.extend_from_slice(b"ftyp");
            data.extend_from_slice(&[0; 16]);
        }
        let result = FfmpegDecoder::open_stream(cursor_source(data));
        let _ = result; // just no panic
    }
}

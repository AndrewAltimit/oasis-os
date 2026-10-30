//! Codec, scaler and resampler construction plus small ffmpeg helpers.

use ffmpeg_next as ffmpeg;
use ffmpeg_next::ffi;

use crate::VideoError;

/// Find, configure and open a decoder for `codecpar`, wrapped in
/// ffmpeg-next's owning codec context. `kind` names the stream in errors.
fn open_codec(
    codecpar: &ffi::AVCodecParameters,
    kind: &str,
) -> Result<ffmpeg::codec::context::Context, VideoError> {
    // SAFETY: looks a decoder up in ffmpeg's static registry; no
    // preconditions beyond a codec id.
    let codec = unsafe { ffi::avcodec_find_decoder(codecpar.codec_id) };
    if codec.is_null() {
        return Err(VideoError::Decode(format!(
            "no decoder for {kind} codec {:?}",
            codecpar.codec_id
        )));
    }

    // SAFETY: `codec` is a valid decoder from the registry.
    let mut ctx = unsafe { ffi::avcodec_alloc_context3(codec) };
    if ctx.is_null() {
        return Err(VideoError::Decode("avcodec_alloc_context3 failed".into()));
    }

    // SAFETY: `ctx` was allocated above and `codecpar` is a valid
    // reference to ffmpeg-owned stream parameters.
    let ret = unsafe { ffi::avcodec_parameters_to_context(ctx, codecpar) };
    if ret < 0 {
        // SAFETY: `ctx` came from avcodec_alloc_context3 and is ours.
        unsafe { ffi::avcodec_free_context(&mut ctx) };
        return Err(VideoError::Decode(format!(
            "avcodec_parameters_to_context: {}",
            ffmpeg_error_string(ret)
        )));
    }

    // SAFETY: `ctx` is configured for `codec`; open the decoder.
    let ret = unsafe { ffi::avcodec_open2(ctx, codec, std::ptr::null_mut()) };
    if ret < 0 {
        // SAFETY: `ctx` came from avcodec_alloc_context3 and is ours.
        unsafe { ffi::avcodec_free_context(&mut ctx) };
        return Err(VideoError::Decode(format!(
            "avcodec_open2: {}",
            ffmpeg_error_string(ret)
        )));
    }

    // SAFETY: `ctx` is an open codec context nothing else owns; the safe
    // wrapper takes ownership and frees it on drop.
    Ok(unsafe { ffmpeg::codec::context::Context::wrap(ctx, None) })
}

/// Create a video decoder from codec parameters.
pub(super) fn create_video_decoder(
    codecpar: &ffi::AVCodecParameters,
) -> Result<ffmpeg::decoder::Video, VideoError> {
    open_codec(codecpar, "video")?
        .decoder()
        .video()
        .map_err(|e| VideoError::Decode(format!("video decoder: {e}")))
}

/// Create an audio decoder from codec parameters.
pub(super) fn create_audio_decoder(
    codecpar: &ffi::AVCodecParameters,
) -> Result<ffmpeg::decoder::Audio, VideoError> {
    open_codec(codecpar, "audio")?
        .decoder()
        .audio()
        .map_err(|e| VideoError::Decode(format!("audio decoder: {e}")))
}

/// Create a `w`x`h` pixel format scaler from `src_fmt` to RGBA.
///
/// Returns an error if the pixel format is unspecified (None), which can
/// happen when ffmpeg couldn't fully probe the stream.
pub(super) fn create_scaler(
    src_fmt: ffmpeg::format::Pixel,
    w: u32,
    h: u32,
) -> Result<ffmpeg::software::scaling::Context, VideoError> {
    if src_fmt == ffmpeg::format::Pixel::None {
        return Err(VideoError::Decode(
            "cannot create scaler: pixel format unspecified (probe may need more data)".into(),
        ));
    }
    ffmpeg::software::scaling::Context::get(
        src_fmt,
        w,
        h,
        ffmpeg::format::Pixel::RGBA,
        w,
        h,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| VideoError::Decode(format!("scaler init: {e}")))
}

/// Create an audio resampler (any format -> f32 interleaved).
pub(super) fn create_resampler(
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

/// Seconds per tick of a stream time base (0 for a malformed `x/0` one,
/// which would otherwise turn every timestamp into inf/NaN).
pub(super) fn time_base_secs(tb: ffi::AVRational) -> f64 {
    if tb.den == 0 {
        0.0
    } else {
        f64::from(tb.num) / f64::from(tb.den)
    }
}

/// Convert an ffmpeg error code to a human-readable string.
pub(super) fn ffmpeg_error_string(errnum: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: av_strerror writes at most `buf.len()` bytes (NUL-terminated) into our
    // buffer. `cast()` bridges `c_char` signedness, which differs across targets.
    unsafe {
        ffi::av_strerror(errnum, buf.as_mut_ptr().cast(), buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

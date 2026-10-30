//! Custom AVIO: ffmpeg reads and seeks through a `Box<dyn VideoSource>`.

use std::io::SeekFrom;

use ffmpeg_next::ffi;

use crate::{VideoError, VideoSource};

/// Size of the AVIO read buffer handed to ffmpeg.
const AVIO_BUF_SIZE: usize = 32 * 1024;

/// Custom I/O context wrapping a `Box<dyn VideoSource>` for ffmpeg's AVIO.
pub(super) struct IoContext {
    source: Box<dyn VideoSource>,
    read_count: u64,
}

/// The two halves of a custom AVIO setup. `opaque` is the callbacks'
/// state (from `Box::into_raw`); `avio` references it. Free `avio` (with
/// [`free_custom_avio`]) before `opaque` (with [`free_io_context`]).
pub(super) struct CustomIo {
    pub(super) opaque: *mut IoContext,
    pub(super) avio: *mut ffi::AVIOContext,
}

/// Allocate an AVIO context whose callbacks read from and seek `source`.
///
/// On failure everything allocated so far is freed again.
pub(super) fn alloc_custom_io(source: Box<dyn VideoSource>) -> Result<CustomIo, VideoError> {
    let opaque = Box::into_raw(Box::new(IoContext {
        source,
        read_count: 0,
    }));

    // SAFETY: av_malloc has no preconditions; it returns null or a buffer
    // of AVIO_BUF_SIZE bytes that we hand to the AVIO context.
    let avio_buffer = unsafe { ffi::av_malloc(AVIO_BUF_SIZE) as *mut u8 };
    if avio_buffer.is_null() {
        // SAFETY: `opaque` is from `Box::into_raw` above; nothing uses it.
        unsafe { free_io_context(opaque) };
        return Err(VideoError::Demux("av_malloc failed".into()));
    }

    // SAFETY: `avio_buffer` is an av_malloc'd buffer of AVIO_BUF_SIZE bytes
    // (ownership passes to the context on success); `opaque` is a live
    // `IoContext` that the caller keeps alive until after the context is
    // freed, which is what the callbacks require of it.
    let avio = unsafe {
        ffi::avio_alloc_context(
            avio_buffer,
            AVIO_BUF_SIZE as i32,
            0, // read-only
            opaque.cast(),
            Some(read_packet),
            None, // no write
            Some(seek_source),
        )
    };
    if avio.is_null() {
        // SAFETY: avio_alloc_context failed, so the av_malloc'd buffer is
        // still ours, and nothing references `opaque`.
        unsafe {
            ffi::av_free(avio_buffer.cast());
            free_io_context(opaque);
        }
        return Err(VideoError::Demux("avio_alloc_context failed".into()));
    }
    Ok(CustomIo { opaque, avio })
}

/// Free the callbacks' state.
///
/// # Safety
/// `opaque` is null or came from [`alloc_custom_io`], is freed at most
/// once, and no AVIO context that could call back into it is still live.
pub(super) unsafe fn free_io_context(opaque: *mut IoContext) {
    if !opaque.is_null() {
        // SAFETY: per the contract, `opaque` is an unaliased `Box` pointer.
        drop(unsafe { Box::from_raw(opaque) });
    }
}

/// Free a custom AVIO context and its buffer, which ffmpeg may have
/// replaced since `avio_alloc_context` (so the original pointer must not
/// be freed directly).
///
/// # Safety
/// `avio` is null or a live context from `avio_alloc_context` that no
/// format context references any more.
pub(super) unsafe fn free_custom_avio(mut avio: *mut ffi::AVIOContext) {
    if avio.is_null() {
        return;
    }
    // SAFETY: per the contract `avio` is live and ours; `av_freep` frees
    // `(*avio).buffer` and nulls it, then `avio_context_free` frees the
    // context and nulls `avio`.
    unsafe {
        ffi::av_freep(std::ptr::addr_of_mut!((*avio).buffer).cast());
        ffi::avio_context_free(&mut avio);
    }
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
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, buf_size.max(0) as usize) };
    ctx.read_count += 1;
    match ctx.source.read(slice) {
        Ok(0) => {
            if ctx.read_count <= 20 || ctx.read_count.is_multiple_of(1000) {
                log::debug!(
                    "AVIO: read #{} -> EOF (requested {})",
                    ctx.read_count,
                    buf_size
                );
            }
            ffi::AVERROR_EOF
        },
        Ok(n) => {
            if ctx.read_count <= 10 || ctx.read_count.is_multiple_of(5000) {
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
        // SEEK_SET with a negative offset is invalid, not a huge u64.
        0 => match u64::try_from(offset) {
            Ok(pos) => SeekFrom::Start(pos),
            Err(_) => return -1,
        },
        1 => SeekFrom::Current(offset), // SEEK_CUR
        2 => SeekFrom::End(offset),     // SEEK_END
        _ => return -1,
    };

    match ctx.source.seek(seek_from) {
        Ok(pos) => pos as i64,
        Err(_) => -1,
    }
}

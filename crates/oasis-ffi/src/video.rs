//! Video decode FFI (requires `_video` feature).

#[cfg(feature = "_video")]
use std::os::raw::c_char;

#[cfg(feature = "_video")]
use crate::handle::{OasisInstance, c_str_to_str, ffi_guard, with_instance, with_instance_ref};

/// Background decode thread state.
#[cfg(feature = "_video")]
pub(crate) struct VideoThreadState {
    /// Latest decoded video frame (replaced each iteration).
    latest_frame: std::sync::Arc<std::sync::Mutex<Option<oasis_video::VideoFrame>>>,
    /// Ring buffer of decoded audio chunks (capped at 128).
    audio_buffer:
        std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<oasis_video::AudioChunk>>>,
    /// Flag to signal the decode thread to stop.
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The background decode thread handle.
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Dropping the state (stop, a new `oasis_video_play`, `oasis_destroy`)
/// stops the decode thread and joins it, so no decoder outlives its
/// instance.
#[cfg(feature = "_video")]
impl Drop for VideoThreadState {
    fn drop(&mut self) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(feature = "_video")]
impl VideoThreadState {
    /// Still producing output: the decoder runs, or a decoded frame /
    /// audio chunk is waiting to be polled.
    fn is_playing(&self) -> bool {
        let decoding = self.thread.as_ref().is_some_and(|t| !t.is_finished());
        let frame_pending = self.latest_frame.lock().is_ok_and(|f| f.is_some());
        let audio_pending = self.audio_buffer.lock().is_ok_and(|a| !a.is_empty());
        decoding || frame_pending || audio_pending
    }
}

#[cfg(feature = "_video")]
const AUDIO_RING_CAP: usize = 128;

/// RGBA byte length of a `width` x `height` frame, `None` on overflow.
#[cfg(feature = "_video")]
fn frame_byte_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

/// Background decode loop run on a dedicated thread.
#[cfg(feature = "_video")]
fn decode_loop(
    mut decoder: oasis_video::SoftwareVideoDecoder,
    latest_frame: std::sync::Arc<std::sync::Mutex<Option<oasis_video::VideoFrame>>>,
    audio_buffer: std::sync::Arc<
        std::sync::Mutex<std::collections::VecDeque<oasis_video::AudioChunk>>,
    >,
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Decode one video frame.
        match decoder.next_video_frame() {
            Ok(Some(frame)) => {
                if let Ok(mut slot) = latest_frame.lock() {
                    *slot = Some(frame);
                }
            },
            Ok(None) => break, // end of stream
            Err(e) => {
                log::warn!("video decode error: {e}");
                // Continue -- some frames may fail without being fatal.
            },
        }

        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Decode one audio chunk.
        match decoder.next_audio_samples() {
            Ok(Some(chunk)) => {
                if let Ok(mut buf) = audio_buffer.lock() {
                    if buf.len() >= AUDIO_RING_CAP {
                        buf.pop_front();
                    }
                    buf.push_back(chunk);
                }
            },
            Ok(None) => {},
            Err(e) => {
                log::warn!("audio decode error: {e}");
            },
        }

        // Small sleep to yield CPU when decode is faster than playback.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Stop and join the decode thread (see the `Drop` impl).
#[cfg(feature = "_video")]
pub(crate) fn stop_video_thread(instance: &mut OasisInstance) {
    instance.video_state = None;
}

/// Start software video playback from a file path.
///
/// Opens the file at `path` and spawns a background decode thread.
/// Use `oasis_video_next_frame` and `oasis_video_get_audio` to poll output.
///
/// # Safety
///
/// `handle` must be valid. `path` must be a valid null-terminated C string.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_play(handle: *mut OasisInstance, path: *const c_char) -> i32 {
    ffi_guard("oasis_video_play", -1, || {
        // SAFETY: Caller guarantees pointer is null or a valid C string per function safety contract.
        let Some(path_str) = (unsafe { c_str_to_str(path) }) else {
            return -1;
        };

        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe { with_instance(handle, -1, |instance| video_play_inner(instance, path_str)) }
    })
}

#[cfg(feature = "_video")]
fn video_play_inner(instance: &mut OasisInstance, path_str: &str) -> i32 {
    // Open the new stream before stopping the current one: a bad path
    // must not end playback that was working.
    let file = match std::fs::File::open(path_str) {
        Ok(f) => f,
        Err(e) => {
            log::error!("oasis_video_play: open failed: {e}");
            return -1;
        },
    };

    let decoder = match oasis_video::SoftwareVideoDecoder::open_stream(Box::new(file)) {
        Ok(d) => d,
        Err(e) => {
            log::error!("oasis_video_play: decode init failed: {e}");
            return -1;
        },
    };
    stop_video_thread(instance);

    let latest_frame = std::sync::Arc::new(std::sync::Mutex::new(None));
    let audio_buffer = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::with_capacity(AUDIO_RING_CAP),
    ));
    let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let lf = std::sync::Arc::clone(&latest_frame);
    let ab = std::sync::Arc::clone(&audio_buffer);
    let sf = std::sync::Arc::clone(&stop_flag);

    let thread = match std::thread::Builder::new()
        .name("oasis-video-decode".into())
        .spawn(move || decode_loop(decoder, lf, ab, sf))
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("oasis_video_play: decode thread spawn failed: {e}");
            return -1;
        },
    };

    instance.video_state = Some(VideoThreadState {
        latest_frame,
        audio_buffer,
        stop_flag,
        thread: Some(thread),
    });

    log::info!("oasis_video_play: started {path_str}");
    0
}

/// Stop video playback and join the decode thread.
///
/// # Safety
///
/// `handle` must be valid.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_stop(handle: *mut OasisInstance) {
    ffi_guard("oasis_video_stop", (), || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, (), |instance| {
                stop_video_thread(instance);
                log::info!("oasis_video_stop: stopped");
            });
        }
    })
}

/// Check whether video is currently playing.
///
/// Returns 1 while the decoder runs or decoded output is still waiting to
/// be polled, 0 once the stream ended and everything was drained (or
/// nothing was started).
///
/// # Safety
///
/// `handle` must be valid.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_is_playing(handle: *mut OasisInstance) -> i32 {
    ffi_guard("oasis_video_is_playing", 0, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance_ref(handle, 0, |instance| {
                i32::from(
                    instance
                        .video_state
                        .as_ref()
                        .is_some_and(VideoThreadState::is_playing),
                )
            })
        }
    })
}

/// Poll the latest decoded video frame.
///
/// Takes the most recent frame from the decode thread. Copies RGBA pixels
/// into `buf` and writes dimensions to `out_w`/`out_h`.
///
/// Returns 1 = new frame copied, 0 = no new frame, -1 = error. When `buf`
/// is null or smaller than `w*h*4` bytes, the frame stays queued and its
/// dimensions are still written to `out_w`/`out_h`, so the host can size
/// its buffer and poll again.
///
/// # Safety
///
/// `handle` must be null or valid. `buf` must be null or point to at least
/// `buf_size` writable bytes. `out_w` and `out_h` must be valid pointers
/// (or null to skip).
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_next_frame(
    handle: *mut OasisInstance,
    buf: *mut u8,
    buf_size: u32,
    out_w: *mut u32,
    out_h: *mut u32,
) -> i32 {
    ffi_guard("oasis_video_next_frame", -1, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance_ref(handle, -1, |instance| {
                let Some(state) = &instance.video_state else {
                    return -1;
                };

                let Ok(mut slot) = state.latest_frame.lock() else {
                    return -1;
                };
                let Some(f) = slot.as_ref() else {
                    return 0;
                };
                if !out_w.is_null() {
                    *out_w = f.width;
                }
                if !out_h.is_null() {
                    *out_h = f.height;
                }
                let byte_len = frame_byte_len(f.width, f.height);
                if byte_len != Some(f.rgba.len()) {
                    // A malformed frame can never be delivered: drop it.
                    slot.take();
                    return -1;
                }
                if buf.is_null() || (buf_size as usize) < f.rgba.len() {
                    // Keep the frame for the next poll with a bigger buffer.
                    return -1;
                }
                // SAFETY: `buf` is non-null and the caller guarantees `buf_size`
                // writable bytes; we checked `buf_size >= f.rgba.len()`.
                std::ptr::copy_nonoverlapping(f.rgba.as_ptr(), buf, f.rgba.len());
                slot.take();
                1
            })
        }
    })
}

/// Drain decoded audio samples into a host buffer.
///
/// Copies interleaved f32 PCM from the audio ring buffer into `buf`, up to
/// `max_samples` samples (not bytes -- each sample is 4 bytes).
///
/// Returns: number of samples copied, or -1 on error. At most
/// `i32::MAX` samples are copied per call.
///
/// # Safety
///
/// `handle` must be null or valid. `buf` must point to at least
/// `max_samples * 4` writable bytes.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_get_audio(
    handle: *mut OasisInstance,
    buf: *mut f32,
    max_samples: u32,
) -> i32 {
    ffi_guard("oasis_video_get_audio", -1, || {
        if buf.is_null() {
            return -1;
        }

        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance_ref(handle, -1, |instance| {
                let Some(state) = &instance.video_state else {
                    return -1;
                };

                let mut ring = match state.audio_buffer.lock() {
                    Ok(r) => r,
                    Err(_) => return -1,
                };

                // The count is returned as an i32.
                let max_samples = max_samples.min(i32::MAX as u32);
                let mut written = 0u32;
                let max = max_samples as usize;

                // `front_mut` both checks for and borrows the next chunk, so no
                // "non-empty" invariant has to be re-asserted after a copy.
                while written < max_samples {
                    let Some(chunk) = ring.front_mut() else {
                        break;
                    };

                    let remaining = max - written as usize;
                    let take = chunk.pcm_f32.len().min(remaining);

                    // SAFETY: Caller guarantees `buf` holds `max_samples` f32s, and
                    // `written + take <= max_samples`, so the copy stays in bounds.
                    std::ptr::copy_nonoverlapping(
                        chunk.pcm_f32.as_ptr(),
                        buf.add(written as usize),
                        take,
                    );
                    written += take as u32;

                    if take == chunk.pcm_f32.len() {
                        // Whole chunk consumed.
                        ring.pop_front();
                    } else {
                        // Partial copy -- drain the consumed samples and stop.
                        chunk.pcm_f32.drain(..take);
                        break;
                    }
                }

                written as i32
            })
        }
    })
}

#[cfg(all(test, feature = "_video"))]
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use super::*;

    /// Install a finished (thread-less) video state holding `frame`.
    fn with_frame(handle: *mut OasisInstance, frame: oasis_video::VideoFrame) {
        unsafe {
            with_instance(handle, (), |inst| {
                inst.video_state = Some(VideoThreadState {
                    latest_frame: std::sync::Arc::new(std::sync::Mutex::new(Some(frame))),
                    audio_buffer: std::sync::Arc::default(),
                    stop_flag: std::sync::Arc::default(),
                    thread: None,
                });
            });
        }
    }

    fn create() -> *mut OasisInstance {
        let null = std::ptr::null();
        let h = unsafe { crate::oasis_create(64, 64, null, null, null) };
        assert!(!h.is_null());
        h
    }

    #[test]
    fn small_buffer_keeps_frame_and_reports_size() {
        let h = create();
        with_frame(
            h,
            oasis_video::VideoFrame {
                rgba: vec![7; 2 * 3 * 4],
                width: 2,
                height: 3,
                timestamp_secs: 0.0,
            },
        );
        let (mut w, mut ht) = (0u32, 0u32);
        let mut small = [0u8; 4];
        let r = unsafe { oasis_video_next_frame(h, small.as_mut_ptr(), 4, &mut w, &mut ht) };
        assert_eq!(r, -1);
        assert_eq!((w, ht), (2, 3), "dimensions reported for resizing");
        assert_eq!(
            unsafe { oasis_video_is_playing(h) },
            1,
            "frame still queued"
        );

        let mut buf = vec![0u8; (w * ht * 4) as usize];
        let r = unsafe {
            oasis_video_next_frame(h, buf.as_mut_ptr(), buf.len() as u32, &mut w, &mut ht)
        };
        assert_eq!(r, 1);
        assert!(buf.iter().all(|b| *b == 7));
        // Drained and no decoder running: no longer playing.
        assert_eq!(unsafe { oasis_video_is_playing(h) }, 0);
        unsafe { crate::oasis_destroy(h) };
    }

    #[test]
    fn mismatched_frame_is_dropped() {
        let h = create();
        with_frame(
            h,
            oasis_video::VideoFrame {
                rgba: vec![0; 3],
                width: 2,
                height: 2,
                timestamp_secs: 0.0,
            },
        );
        let mut buf = vec![0u8; 64];
        let null = std::ptr::null_mut();
        let r = unsafe { oasis_video_next_frame(h, buf.as_mut_ptr(), 64, null, null) };
        assert_eq!(r, -1);
        let r = unsafe { oasis_video_next_frame(h, buf.as_mut_ptr(), 64, null, null) };
        assert_eq!(r, 0, "the malformed frame is not offered again");
        unsafe { crate::oasis_destroy(h) };
    }

    #[test]
    fn failed_play_keeps_current_playback() {
        let h = create();
        with_frame(
            h,
            oasis_video::VideoFrame {
                rgba: vec![1; 4],
                width: 1,
                height: 1,
                timestamp_secs: 0.0,
            },
        );
        let bad = std::ffi::CString::new("/no/such/video.mp4").unwrap();
        assert_eq!(unsafe { oasis_video_play(h, bad.as_ptr()) }, -1);
        assert_eq!(unsafe { oasis_video_is_playing(h) }, 1, "still playing");
        unsafe { crate::oasis_destroy(h) };
    }

    #[test]
    fn frame_byte_len_checks_overflow() {
        assert_eq!(frame_byte_len(2, 3), Some(24));
        // (2^32 - 1)^2 * 4 overflows even a 64-bit usize.
        assert_eq!(frame_byte_len(u32::MAX, u32::MAX), None);
    }
}

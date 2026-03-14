//! Video decode FFI (requires `_video` feature).

#[cfg(feature = "_video")]
use std::os::raw::c_char;

#[cfg(feature = "_video")]
use crate::handle::{OasisInstance, c_str_to_str, with_instance, with_instance_ref};

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

#[cfg(feature = "_video")]
const AUDIO_RING_CAP: usize = 128;

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

/// Internal helper to stop and join the decode thread.
#[cfg(feature = "_video")]
pub(crate) fn stop_video_thread(instance: &mut OasisInstance) {
    if let Some(mut state) = instance.video_state.take() {
        state
            .stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = state.thread.take() {
            let _ = thread.join();
        }
    }
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
    // SAFETY: Caller guarantees pointer is null or a valid C string per function safety contract.
    let Some(path_str) = (unsafe { c_str_to_str(path) }) else {
        return -1;
    };

    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe { with_instance(handle, -1, |instance| video_play_inner(instance, path_str)) }
}

#[cfg(feature = "_video")]
fn video_play_inner(instance: &mut OasisInstance, path_str: &str) -> i32 {
    // Stop any existing playback.
    stop_video_thread(instance);

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

    let latest_frame = std::sync::Arc::new(std::sync::Mutex::new(None));
    let audio_buffer = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::with_capacity(AUDIO_RING_CAP),
    ));
    let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let lf = std::sync::Arc::clone(&latest_frame);
    let ab = std::sync::Arc::clone(&audio_buffer);
    let sf = std::sync::Arc::clone(&stop_flag);

    let thread = std::thread::Builder::new()
        .name("oasis-video-decode".into())
        .spawn(move || decode_loop(decoder, lf, ab, sf))
        .ok();

    instance.video_state = Some(VideoThreadState {
        latest_frame,
        audio_buffer,
        stop_flag,
        thread,
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
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            stop_video_thread(instance);
            log::info!("oasis_video_stop: stopped");
        });
    }
}

/// Check whether video is currently playing.
///
/// Returns 1 if playing, 0 if not.
///
/// # Safety
///
/// `handle` must be valid.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_is_playing(handle: *mut OasisInstance) -> i32 {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance_ref(handle, 0, |instance| {
            i32::from(instance.video_state.is_some())
        })
    }
}

/// Poll the latest decoded video frame.
///
/// Takes the most recent frame from the decode thread. Copies RGBA pixels
/// into `buf` and writes dimensions to `out_w`/`out_h`.
///
/// Returns: 1 = new frame copied, 0 = no new frame, -1 = error.
///
/// # Safety
///
/// `handle` must be valid. `buf` must point to at least `buf_size` bytes.
/// `buf_size` must be >= `w*h*4` for the decoded frame dimensions.
/// `out_w` and `out_h` must be valid pointers (or null to skip).
///
/// Returns 1 on success, 0 if no frame available, -1 on error (including
/// if the destination buffer is too small for the decoded frame).
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_next_frame(
    handle: *mut OasisInstance,
    buf: *mut u8,
    buf_size: u32,
    out_w: *mut u32,
    out_h: *mut u32,
) -> i32 {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance_ref(handle, -1, |instance| {
            let Some(state) = &instance.video_state else {
                return -1;
            };

            let frame = match state.latest_frame.lock() {
                Ok(mut slot) => slot.take(),
                Err(_) => return -1,
            };

            match frame {
                Some(f) => {
                    let byte_len = (f.width * f.height * 4) as usize;
                    if buf.is_null() || f.rgba.len() != byte_len || (buf_size as usize) < byte_len {
                        return -1;
                    }
                    // SAFETY: Caller provides buf_size; we verified buf_size >= byte_len
                    // and byte_len == f.rgba.len(), so the copy is within bounds.
                    std::ptr::copy_nonoverlapping(f.rgba.as_ptr(), buf, byte_len);
                    if !out_w.is_null() {
                        *out_w = f.width;
                    }
                    if !out_h.is_null() {
                        *out_h = f.height;
                    }
                    1
                },
                None => 0,
            }
        })
    }
}

/// Drain decoded audio samples into a host buffer.
///
/// Copies interleaved f32 PCM from the audio ring buffer into `buf`, up to
/// `max_samples` samples (not bytes -- each sample is 4 bytes).
///
/// Returns: number of samples copied, or -1 on error.
///
/// # Safety
///
/// `handle` must be valid. `buf` must point to at least `max_samples * 4`
/// bytes.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_get_audio(
    handle: *mut OasisInstance,
    buf: *mut f32,
    max_samples: u32,
) -> i32 {
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

            let mut written = 0u32;
            let max = max_samples as usize;

            while written < max_samples {
                let chunk = match ring.front() {
                    Some(c) => c,
                    None => break,
                };

                let remaining = max - written as usize;
                let available = chunk.pcm_f32.len();

                if available <= remaining {
                    // Copy entire chunk.
                    // SAFETY: Caller guarantees destination buffer has sufficient space.
                    std::ptr::copy_nonoverlapping(
                        chunk.pcm_f32.as_ptr(),
                        buf.add(written as usize),
                        available,
                    );
                    written += available as u32;
                    ring.pop_front();
                } else {
                    // Partial copy -- take what fits and drain consumed samples.
                    // SAFETY: Caller guarantees destination buffer has sufficient space.
                    std::ptr::copy_nonoverlapping(
                        chunk.pcm_f32.as_ptr(),
                        buf.add(written as usize),
                        remaining,
                    );
                    written += remaining as u32;
                    // Remove consumed samples from the front of this chunk.
                    let chunk = ring
                        .front_mut()
                        .expect("ring buffer non-empty after front() check");
                    chunk.pcm_f32.drain(..remaining);
                    break;
                }
            }

            written as i32
        })
    }
}

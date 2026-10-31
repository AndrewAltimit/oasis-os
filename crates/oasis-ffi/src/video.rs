//! Video decode FFI (requires `_video` feature).
//!
//! `oasis_video_play` spawns a decode thread that runs at the media's own
//! pace: each frame is published when the wall clock reaches its
//! presentation time, and audio is decoded at most [`AUDIO_LEAD_SECS`]
//! ahead of it. (It used to decode as fast as the CPU allowed, so the
//! bounded audio ring overflowed and dropped chunks before a host polling
//! once per frame could take them, and frames flashed past.)

#[cfg(feature = "_video")]
use std::os::raw::c_char;
#[cfg(feature = "_video")]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(feature = "_video")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "_video")]
use std::time::{Duration, Instant};

#[cfg(feature = "_video")]
use oasis_video::{AudioChunk, VideoError, VideoFrame};

#[cfg(feature = "_video")]
use crate::handle::{OasisInstance, c_str_to_str, ffi_guard, with_instance, with_instance_ref};

/// Most decoded audio chunks held for the host. Reached only when the host
/// stops draining (decode runs [`AUDIO_LEAD_SECS`] ahead, a few dozen
/// chunks); then the oldest chunk is dropped.
#[cfg(feature = "_video")]
const AUDIO_RING_CAP: usize = 128;

/// How far ahead of the wall clock audio is decoded: enough to ride out a
/// host frame hitch, far below [`AUDIO_RING_CAP`] chunks.
#[cfg(feature = "_video")]
const AUDIO_LEAD_SECS: f64 = 0.5;

/// Longest single sleep of the decode thread, bounding stop latency.
#[cfg(feature = "_video")]
const MAX_SLEEP: Duration = Duration::from_millis(5);

/// Consecutive decode errors after which a stream is treated as ended.
#[cfg(feature = "_video")]
const MAX_CONSECUTIVE_ERRORS: u32 = 64;

/// Sample rate and channel count of the decoded audio, published by the
/// decode thread (0 = not known yet / no audio track).
#[cfg(feature = "_video")]
#[derive(Default)]
pub(crate) struct AudioFormat {
    sample_rate: AtomicU32,
    channels: AtomicU32,
}

#[cfg(feature = "_video")]
impl AudioFormat {
    fn set(&self, sample_rate: u32, channels: u16) {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.channels.store(u32::from(channels), Ordering::Relaxed);
    }

    fn get(&self) -> (u32, u32) {
        (
            self.sample_rate.load(Ordering::Relaxed),
            self.channels.load(Ordering::Relaxed),
        )
    }
}

/// What the decode thread shares with the exports.
#[cfg(feature = "_video")]
#[derive(Clone, Default)]
pub(crate) struct VideoShared {
    /// Latest due video frame (replaced as newer ones fall due).
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    /// Decoded audio chunks waiting for `oasis_video_get_audio`.
    audio_buffer: Arc<Mutex<std::collections::VecDeque<AudioChunk>>>,
    /// Set to make the decode thread exit.
    stop_flag: Arc<AtomicBool>,
    /// Format of the chunks in `audio_buffer`.
    audio_format: Arc<AudioFormat>,
}

/// Background decode thread state.
#[cfg(feature = "_video")]
pub(crate) struct VideoThreadState {
    shared: VideoShared,
    /// The background decode thread handle.
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Dropping the state (stop, a new `oasis_video_play`, `oasis_destroy`)
/// stops the decode thread and joins it, so no decoder outlives its
/// instance.
#[cfg(feature = "_video")]
impl Drop for VideoThreadState {
    fn drop(&mut self) {
        self.shared.stop_flag.store(true, Ordering::Relaxed);
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
        let frame_pending = self
            .shared
            .latest_frame
            .lock()
            .is_ok_and(|f| f.is_some());
        let audio_pending = self
            .shared
            .audio_buffer
            .lock()
            .is_ok_and(|a| !a.is_empty());
        decoding || frame_pending || audio_pending
    }
}

/// RGBA byte length of a `width` x `height` frame, `None` on overflow.
#[cfg(feature = "_video")]
fn frame_byte_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

/// The two decode streams the loop pulls from (a trait so tests can drive
/// the pacing with synthetic timestamps).
#[cfg(feature = "_video")]
pub(crate) trait DecodeSource: Send {
    fn next_video(&mut self) -> Result<Option<VideoFrame>, VideoError>;
    fn next_audio(&mut self) -> Result<Option<AudioChunk>, VideoError>;
}

#[cfg(feature = "_video")]
impl DecodeSource for oasis_video::SoftwareVideoDecoder {
    fn next_video(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        self.next_video_frame()
    }

    fn next_audio(&mut self) -> Result<Option<AudioChunk>, VideoError> {
        self.next_audio_samples()
    }
}

/// Per-stream end-of-stream / error bookkeeping for [`decode_loop`].
#[cfg(feature = "_video")]
#[derive(Default)]
struct StreamState {
    done: bool,
    errors: u32,
}

#[cfg(feature = "_video")]
impl StreamState {
    /// Fold one decode result in; `Some` = a decoded item.
    fn take<T>(&mut self, what: &str, result: Result<Option<T>, VideoError>) -> Option<T> {
        match result {
            Ok(Some(item)) => {
                self.errors = 0;
                return Some(item);
            },
            // End of stream, or the file has no such track.
            Ok(None) | Err(VideoError::NoTrack(_)) => self.done = true,
            // Not fatal: the decoder retries on the next call.
            Err(VideoError::SkipLimit) => {},
            Err(e) => {
                log::warn!("{what} decode error: {e}");
                self.errors += 1;
                if self.errors >= MAX_CONSECUTIVE_ERRORS {
                    log::warn!("{what}: {MAX_CONSECUTIVE_ERRORS} errors in a row, giving up");
                    self.done = true;
                }
            },
        }
        None
    }
}

/// Background decode loop run on a dedicated thread, paced to the wall
/// clock (see the module docs). Media time is measured from the first
/// timestamp either stream produces, which plays at the moment it decodes.
#[cfg(feature = "_video")]
pub(crate) fn decode_loop(mut source: impl DecodeSource, shared: VideoShared) {
    let start = Instant::now();
    // Media timestamp shown at `start`.
    let mut base: Option<f64> = None;
    // The next frame, decoded but not yet due.
    let mut pending: Option<VideoFrame> = None;
    // Timestamp of the newest decoded audio chunk.
    let mut audio_ts = f64::NEG_INFINITY;
    let mut video = StreamState::default();
    let mut audio = StreamState::default();

    while !shared.stop_flag.load(Ordering::Relaxed) {
        let now = start.elapsed().as_secs_f64();

        if pending.is_none() && !video.done {
            pending = video.take("video", source.next_video());
            if let Some(f) = &pending {
                base.get_or_insert(f.timestamp_secs);
            }
        }
        // Seconds until the pending frame is due (<= 0: due now).
        let due_in = match (&pending, base) {
            (Some(f), Some(b)) => (f.timestamp_secs - b) - now,
            _ => 0.0,
        };
        if pending.is_some()
            && due_in <= 0.0
            && let Ok(mut slot) = shared.latest_frame.lock()
        {
            *slot = pending.take();
        }

        while !audio.done
            && base.is_none_or(|b| audio_ts - b <= now + AUDIO_LEAD_SECS)
            && !shared.stop_flag.load(Ordering::Relaxed)
        {
            let Some(chunk) = audio.take("audio", source.next_audio()) else {
                break;
            };
            base.get_or_insert(chunk.timestamp_secs);
            audio_ts = chunk.timestamp_secs;
            shared.audio_format.set(chunk.sample_rate, chunk.channels);
            if let Ok(mut ring) = shared.audio_buffer.lock() {
                if ring.len() >= AUDIO_RING_CAP {
                    log::debug!("video audio: host not draining, dropping a chunk");
                    ring.pop_front();
                }
                ring.push_back(chunk);
            }
        }

        if video.done && audio.done && pending.is_none() {
            break;
        }
        // Wait for the pending frame, or (video over / not due yet) idle
        // briefly while audio stays ahead; decode the next frame at once
        // when the last one was just published.
        let sleep = if pending.is_some() {
            Duration::from_secs_f64(due_in.max(0.0)).min(MAX_SLEEP)
        } else if video.done {
            MAX_SLEEP
        } else {
            Duration::ZERO
        };
        if !sleep.is_zero() {
            std::thread::sleep(sleep);
        }
    }
}

/// Stop and join the decode thread (see the `Drop` impl).
#[cfg(feature = "_video")]
pub(crate) fn stop_video_thread(instance: &mut OasisInstance) {
    instance.video_state = None;
}

/// Start software video playback from a file path.
///
/// Opens the file at `path` and spawns a background decode thread that
/// plays at the media's pace (frames fall due at their timestamps, audio
/// is decoded half a second ahead). Use `oasis_video_next_frame` and
/// `oasis_video_get_audio` to poll output, and `oasis_video_audio_format`
/// for the PCM layout.
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

    let shared = VideoShared::default();
    let (rate, channels) = decoder.audio_format();
    shared.audio_format.set(rate, channels);
    let thread_shared = shared.clone();
    let thread = match std::thread::Builder::new()
        .name("oasis-video-decode".into())
        .spawn(move || decode_loop(decoder, thread_shared))
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("oasis_video_play: decode thread spawn failed: {e}");
            return -1;
        },
    };

    instance.video_state = Some(VideoThreadState {
        shared,
        thread: Some(thread),
    });

    log::info!("oasis_video_play: started {path_str}");
    0
}

/// Report the PCM layout `oasis_video_get_audio` delivers: the sample rate
/// in Hz and the channel count (samples are interleaved per channel).
///
/// Returns 0 and writes both values when playback is active and the format
/// is known; -1 otherwise (null or busy handle, nothing playing, or a file
/// without an audio track), writing nothing. Either out-pointer may be null.
///
/// # Safety
///
/// `handle` must be null or valid. `out_rate` and `out_channels` must be
/// null or valid for a `u32` write.
#[cfg(feature = "_video")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_video_audio_format(
    handle: *mut OasisInstance,
    out_rate: *mut u32,
    out_channels: *mut u32,
) -> i32 {
    ffi_guard("oasis_video_audio_format", -1, || {
        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        let format = unsafe {
            with_instance_ref(handle, None, |instance| {
                instance
                    .video_state
                    .as_ref()
                    .map(|s| s.shared.audio_format.get())
            })
        };
        let Some((rate, channels)) = format.filter(|&(r, c)| r > 0 && c > 0) else {
            return -1;
        };
        // SAFETY: the caller guarantees each pointer is null or valid for a
        // `u32` write; null ones are skipped.
        unsafe {
            if let Some(out) = out_rate.as_mut() {
                *out = rate;
            }
            if let Some(out) = out_channels.as_mut() {
                *out = channels;
            }
        }
        0
    })
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

                let Ok(mut slot) = state.shared.latest_frame.lock() else {
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

                let mut ring = match state.shared.audio_buffer.lock() {
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
                let shared = VideoShared::default();
                *shared.latest_frame.lock().unwrap() = Some(frame);
                inst.video_state = Some(VideoThreadState {
                    shared,
                    thread: None,
                });
            });
        }
    }

    /// Synthetic decoder: video frames and 10 ms audio chunks at the given
    /// timestamps, then end of stream (or `NoTrack` for an empty list).
    struct Fake {
        frames: std::collections::VecDeque<f64>,
        audio: std::collections::VecDeque<f64>,
        has_video: bool,
    }

    impl Fake {
        fn new(frames: &[f64], audio: impl IntoIterator<Item = f64>) -> Self {
            Self {
                frames: frames.iter().copied().collect(),
                audio: audio.into_iter().collect(),
                has_video: !frames.is_empty(),
            }
        }
    }

    impl DecodeSource for Fake {
        fn next_video(&mut self) -> Result<Option<VideoFrame>, VideoError> {
            if !self.has_video {
                return Err(VideoError::NoTrack("video".into()));
            }
            Ok(self.frames.pop_front().map(|t| VideoFrame {
                rgba: vec![0; 4],
                width: 1,
                height: 1,
                timestamp_secs: t,
            }))
        }

        fn next_audio(&mut self) -> Result<Option<AudioChunk>, VideoError> {
            Ok(self.audio.pop_front().map(|t| AudioChunk {
                pcm_f32: vec![0.0; 882],
                channels: 2,
                sample_rate: 44_100,
                timestamp_secs: t,
            }))
        }
    }

    #[test]
    fn decode_is_paced_to_frame_timestamps() {
        let shared = VideoShared::default();
        let start = Instant::now();
        // Timestamps start at 5 s: media time is relative to the first one.
        decode_loop(
            Fake::new(&[5.0, 5.1, 5.2, 5.3], (0..30).map(|i| 5.0 + f64::from(i) * 0.01)),
            shared.clone(),
        );
        let elapsed = start.elapsed().as_secs_f64();
        assert!(elapsed >= 0.29, "finished after {elapsed:.3}s, frames ran ahead");
        assert!(elapsed < 3.0, "finished after {elapsed:.3}s");
        let last = shared.latest_frame.lock().unwrap().take().unwrap();
        assert_eq!(last.timestamp_secs, 5.3);
        assert_eq!(shared.audio_buffer.lock().unwrap().len(), 30, "no chunk dropped");
        assert_eq!(shared.audio_format.get(), (44_100, 2));
    }

    #[test]
    fn audio_is_decoded_only_a_bounded_lead_ahead() {
        let shared = VideoShared::default();
        let worker = shared.clone();
        // Audio-only, 10 s of 10 ms chunks: far more than the ring holds.
        let thread = std::thread::spawn(move || {
            decode_loop(Fake::new(&[], (0..1000).map(|i| f64::from(i) * 0.01)), worker)
        });
        std::thread::sleep(Duration::from_millis(100));
        let queued = shared.audio_buffer.lock().unwrap().len();
        shared.stop_flag.store(true, Ordering::Relaxed);
        thread.join().unwrap();
        // ~0.1 s elapsed + 0.5 s lead = ~60 chunks; never the 128-chunk cap.
        assert!(queued > 40, "only {queued} chunks decoded");
        assert!(queued < AUDIO_RING_CAP, "{queued} chunks: decode ran ahead");
    }

    #[test]
    fn audio_format_reports_active_stream() {
        let h = create();
        let (mut rate, mut ch) = (0u32, 0u32);
        assert_eq!(unsafe { oasis_video_audio_format(h, &mut rate, &mut ch) }, -1);
        unsafe {
            with_instance(h, (), |inst| {
                let shared = VideoShared::default();
                shared.audio_format.set(48_000, 2);
                inst.video_state = Some(VideoThreadState {
                    shared,
                    thread: None,
                });
            });
        }
        assert_eq!(unsafe { oasis_video_audio_format(h, &mut rate, &mut ch) }, 0);
        assert_eq!((rate, ch), (48_000, 2));
        let null = std::ptr::null_mut();
        assert_eq!(unsafe { oasis_video_audio_format(h, null, null) }, 0);
        unsafe { crate::oasis_destroy(h) };
        assert_eq!(unsafe { oasis_video_audio_format(std::ptr::null_mut(), &mut rate, &mut ch) }, -1);
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

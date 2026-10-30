//! Streaming playback: pre-demuxed H.264 access units from the I/O thread
//! are decoded on the Media Engine and handed to the main thread.

use core::sync::atomic::Ordering;

use super::decoder::NalDecoder;
use super::sps::parse_sps_info;
use super::{
    AUDIO_ONLY, AudioCmd, DECODER_READY, ME_LEAKED, STREAM_REQUESTED, StreamFrame, VIDEO_CMD_QUEUE,
    VIDEO_DECODE_COUNT, VIDEO_ERROR_COUNT, VIDEO_FRAME_QUEUE, VIDEO_FRAMES_DROPPED,
    VIDEO_FRAMES_PROCESSED, VIDEO_FRAMES_PUSHED, VIDEO_HEIGHT, VIDEO_NOPIC_COUNT, VIDEO_PLAYING,
    VIDEO_STATE, VIDEO_STREAM_QUEUE, VIDEO_WIDTH, VLOG_ENABLED, VSTATE_AUDIO_ONLY, VSTATE_DECODING,
    VSTATE_IDLE, VSTATE_ME_LEAKED, VSTATE_WAITING_KEYFRAME, VideoCmd, delay_us, drain_queues,
    now_us, reset_stats, send_audio_cmd, vlog, vlog_force, wait_stream_frame,
};

/// Wait (up to 2 s) for the I/O thread to stop feeding frames, so the
/// decoder is never dropped while a download is still pushing into it.
fn wait_for_io_stop(log_progress: bool) {
    for i in 0..200u32 {
        if crate::threading::is_download_stopped() {
            if log_progress {
                vlog_force("[VIDEO] STOP: I/O stopped");
            }
            return;
        }
        if log_progress && i % 50 == 49 {
            vlog_force(&format!(
                "[VIDEO] STOP: still waiting for I/O ({}ms)",
                (i + 1) * 10
            ));
        }
        delay_us(10_000);
    }
    if log_progress {
        vlog_force("[VIDEO] STOP: I/O timeout, proceeding");
    }
}

/// Wait for the first keyframe (~30 s). Non-keyframes before it are
/// dropped: the decoder cannot start without SPS/PPS.
///
/// `Err(shutdown)` if a Stop/Shutdown command arrived meanwhile.
fn wait_for_keyframe() -> Result<Option<StreamFrame>, bool> {
    // 3000 × 10 ms. Long enough for: old download abort (2 s) + TLS
    // handshake (5-10 s) + moov buffering (1-3 s) + first keyframe.
    for _ in 0..3000 {
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop()
            && matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown)
        {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            send_audio_cmd(AudioCmd::VideoAudioStop);
            return Err(matches!(cmd, VideoCmd::Shutdown));
        }

        if let Some(frame) = VIDEO_STREAM_QUEUE.pop() {
            if frame.is_keyframe {
                vlog_force(&format!(
                    "[VIDEO] GOT KEYFRAME: sz={} ts={:.2}",
                    frame.data.len(),
                    frame.timestamp_secs,
                ));
                return Ok(Some(frame));
            }
            vlog_force(&format!("[VIDEO] skip non-kf: sz={}", frame.data.len()));
        }

        // Wait for I/O thread to signal a new frame (10ms timeout).
        wait_stream_frame(10_000);
    }
    Ok(None)
}

/// Streaming playback: receive pre-demuxed H.264 frames from I/O thread
/// and decode them via sceMpeg.
///
/// Returns `true` if Shutdown was received (caller should exit thread).
pub(super) fn play_stream() -> bool {
    // Wait for previous decoder cleanup to complete before starting
    // a new stream. sceMpegDelete + sceMpegFinish need time for
    // kernel-side ME cleanup; starting sceMpegCreate too soon corrupts state.
    if !DECODER_READY.load(Ordering::Acquire) {
        vlog("[VIDEO] play_stream: waiting for previous decoder cleanup...");
        // 100 × 10ms = 1 second max wait
        for _ in 0..100 {
            if DECODER_READY.load(Ordering::Acquire) {
                break;
            }
            delay_us(10_000);
        }
        if !DECODER_READY.load(Ordering::Acquire) {
            vlog("[VIDEO] play_stream: cleanup timeout, proceeding anyway");
        } else {
            vlog("[VIDEO] play_stream: previous decoder cleanup done");
        }
    }

    vlog("[VIDEO] play_stream: starting streaming decode");
    reset_stats();
    VIDEO_STATE.store(VSTATE_WAITING_KEYFRAME, Ordering::Relaxed);

    // Drain stale commands that may have been queued during moov buffering.
    while let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
        if matches!(cmd, VideoCmd::Shutdown) {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
            vlog("[VIDEO] play_stream: shutdown during drain");
            return true;
        }
        vlog("[VIDEO] play_stream: drained stale command");
    }

    // We need the first keyframe to extract SPS and get video dimensions
    // before initializing the decoder. Drop stale frames from the previous
    // stream first (the I/O thread may have pushed some between
    // drain_stream_only exiting and this play_stream starting).
    while VIDEO_STREAM_QUEUE.pop().is_some() {}
    vlog("[VIDEO] play_stream: waiting for first keyframe...");
    let first_frame = match wait_for_keyframe() {
        Ok(Some(f)) => f,
        Ok(None) => {
            vlog("[VIDEO] play_stream: no keyframe received, audio-only");
            return drain_stream_only();
        },
        Err(shutdown) => return shutdown,
    };

    // Parse SPS from the first keyframe to get video dimensions.
    let (vid_w, vid_h) = first_frame
        .avcc_sps
        .as_deref()
        .and_then(parse_sps_info)
        .map_or((480, 272), |i| (i.width, i.height));
    VIDEO_WIDTH.store(vid_w, Ordering::Relaxed);
    VIDEO_HEIGHT.store(vid_h, Ordering::Relaxed);
    vlog(&format!(
        "[VIDEO] play_stream: SPS dimensions = {vid_w}x{vid_h}"
    ));

    // Audio-only mode: skip video decode entirely.
    if AUDIO_ONLY.load(Ordering::Relaxed) {
        vlog("[VIDEO] audio-only mode, skipping video decode");
        VIDEO_STATE.store(VSTATE_AUDIO_ONLY, Ordering::Relaxed);
        return drain_stream_only();
    }

    // ME was leaked from a previous deadlock — can't reinit until reboot.
    if ME_LEAKED.load(Ordering::Relaxed) {
        vlog("[VIDEO] ME leaked from prior deadlock, audio-only until reboot");
        VIDEO_STATE.store(VSTATE_ME_LEAKED, Ordering::Relaxed);
        return drain_stream_only();
    }

    // NAL direct path (mpeg_vsh370.prx): works for ≤480p content.
    // >480p content deadlocks sceMpegAvcDecode (mode 5 firmware bug), so
    // it plays audio-only.
    if vid_w > 480 || vid_h > 480 {
        vlog_force(&format!(
            "[VIDEO] {vid_w}x{vid_h} is >480p — audio-only \
             (mode 5 deadlocks ME firmware)"
        ));
        VIDEO_STATE.store(VSTATE_AUDIO_ONLY, Ordering::Relaxed);
        return drain_stream_only();
    }

    let mut nal_dec = match NalDecoder::try_init(&first_frame) {
        Ok(dec) => {
            vlog("[VIDEO] NAL decoder initialized OK");
            dec
        },
        Err(e) => {
            vlog(&format!("[VIDEO] NAL decoder failed: {e}, audio-only"));
            return drain_stream_only();
        },
    };

    // Decoder created — mark not ready for new streams until cleanup.
    DECODER_READY.store(false, Ordering::Release);
    VIDEO_STATE.store(VSTATE_DECODING, Ordering::Relaxed);

    let mut start_us = now_us();
    let mut decode_count = 0u32;
    let mut frames_processed = 0u32;
    let mut error_count = 0u32;
    let mut no_pic_count = 0u32;
    let mut wait_count = 0u32;

    // Decode the first keyframe.
    match nal_dec.decode(&first_frame.data, first_frame.nal_prefix_size) {
        Ok(Some(decoded)) => {
            decode_count += 1;
            push_frame_with_retry(decoded);
            vlog("[VIDEO] play_stream: first frame decoded!");
        },
        Ok(None) => no_pic_count += 1,
        Err(()) => error_count += 1,
    }
    frames_processed += 1;

    loop {
        // Check for stop/shutdown commands.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop()
            && matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown)
        {
            VLOG_ENABLED.store(true, Ordering::Relaxed);
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
            drain_queues(true);
            send_audio_cmd(AudioCmd::VideoAudioStop);
            vlog_force(&format!(
                "[VIDEO] STOP: proc={frames_processed} \
                 dec={decode_count} err={error_count} nopic={no_pic_count}"
            ));
            // The I/O thread may be blocked in a network read; let it
            // quiesce before the decoder goes away.
            vlog_force("[VIDEO] STOP: waiting for I/O thread...");
            wait_for_io_stop(true);
            drop(nal_dec);
            // Brief ME cooldown before the next stream may start.
            delay_us(100_000);
            DECODER_READY.store(true, Ordering::Release);
            vlog_force("[VIDEO] STOP: cleanup complete");
            return matches!(cmd, VideoCmd::Shutdown);
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            break;
        }

        // Pop next pre-demuxed H.264 frame from stream queue.
        let Some(frame) = VIDEO_STREAM_QUEUE.pop() else {
            wait_count += 1;
            // Log when stuck waiting (every 30 waits ≈ 1 second).
            if wait_count % 30 == 0 {
                let step = psp::mpeg::DECODE_STEP.load(Ordering::Relaxed);
                vlog_force(&format!(
                    "[VIDEO] waiting: w={wait_count} proc={frames_processed} \
                     playing={} step={step}",
                    VIDEO_PLAYING.load(Ordering::Relaxed),
                ));
            }
            // No frame available — wait for I/O thread signal
            // (33ms timeout matches ~30fps frame interval).
            wait_stream_frame(33_000);
            continue;
        };

        // Wait (max 60 ms) for output queue space BEFORE decoding. ME
        // decode takes ~50ms per frame; without this we decode frames that
        // are then dropped because the queue is full, wasting ~30% of ME
        // time.
        for _ in 0..20 {
            if VIDEO_FRAME_QUEUE.len() < VIDEO_FRAME_QUEUE.capacity() as u32 {
                break;
            }
            delay_us(3_000);
        }

        match nal_dec.decode(&frame.data, frame.nal_prefix_size) {
            Ok(Some(decoded)) => {
                decode_count += 1;
                VIDEO_DECODE_COUNT.store(decode_count, Ordering::Relaxed);

                // Suppress verbose logging after first frame.
                if decode_count == 1 {
                    VLOG_ENABLED.store(false, Ordering::Relaxed);
                }

                // Frame pacing via PTS with drift correction.
                let pts_us = (frame.timestamp_secs * 1_000_000.0) as u64;
                let now = now_us();
                let elapsed = now.wrapping_sub(start_us);
                if pts_us > elapsed {
                    let wait = (pts_us - elapsed) as u32;
                    if wait < 100_000 {
                        delay_us(wait);
                    }
                } else if elapsed > pts_us + 200_000 {
                    // Decoder >200ms behind — reset baseline to
                    // prevent permanent fast-forward.
                    start_us = now.wrapping_sub(pts_us);
                }

                // Queue space was checked before decode. Push
                // should succeed; if not, drop gracefully.
                if VIDEO_FRAME_QUEUE.push(decoded).is_ok() {
                    VIDEO_FRAMES_PUSHED.fetch_add(1, Ordering::Relaxed);
                } else {
                    VIDEO_FRAMES_DROPPED.fetch_add(1, Ordering::Relaxed);
                }
            },
            Ok(None) => {
                no_pic_count += 1;
                VIDEO_NOPIC_COUNT.store(no_pic_count, Ordering::Relaxed);
            },
            Err(()) => {
                error_count += 1;
                VIDEO_ERROR_COUNT.store(error_count, Ordering::Relaxed);
                // If the watchdog unblocked a stuck decode, the ME
                // is in a bad state. Switch to audio-only immediately.
                if error_count >= 3 {
                    vlog_force(&format!(
                        "[VIDEO] {error_count} decode errors, ME likely unblocked \
                         by watchdog. Switching to audio-only \
                         (dec={decode_count} total={frames_processed})"
                    ));
                    // LEAK the decoder — do NOT call sceMpegDelete/Finish
                    // while the ME is in a bad state (causes crash).
                    // The ~2MB DDR allocation and mpeg data are lost until
                    // the next cold reboot, but the EBOOT stays alive.
                    core::mem::forget(nal_dec);
                    ME_LEAKED.store(true, Ordering::Release);
                    VIDEO_STATE.store(VSTATE_ME_LEAKED, Ordering::Relaxed);
                    // ME is leaked — no cleanup needed, mark ready.
                    DECODER_READY.store(true, Ordering::Release);
                    return drain_stream_only();
                }
            },
        }
        frames_processed += 1;
        VIDEO_FRAMES_PROCESSED.store(frames_processed, Ordering::Relaxed);

        // Periodic diagnostic (unconditional) — every 50 frames
        // to reduce ~5-20ms per-write I/O overhead.
        if frames_processed % 50 == 0 {
            vlog_force(&format!(
                "[VIDEO] #{frames_processed}: dec={decode_count} \
                 err={error_count} nopic={no_pic_count}"
            ));
        }
    }

    VLOG_ENABLED.store(true, Ordering::Relaxed);
    VIDEO_STATE.store(VSTATE_IDLE, Ordering::Relaxed);
    vlog(&format!(
        "[VIDEO] play_stream ended: proc={frames_processed} \
         dec={decode_count} err={error_count} nopic={no_pic_count}"
    ));
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    // Same ordering as the Stop path: I/O quiesced, then decoder dropped,
    // then a short ME cooldown.
    wait_for_io_stop(false);
    drop(nal_dec);
    delay_us(50_000);
    DECODER_READY.store(true, Ordering::Release);
    vlog("[VIDEO] decoder cleanup done");
    false
}

/// Push a decoded frame to the output queue, retrying with brief sleeps.
/// Returns true if the frame was pushed, false if dropped.
fn push_frame_with_retry(frame: super::DecodedFrame) -> bool {
    let mut item = frame;
    for _ in 0..6 {
        match VIDEO_FRAME_QUEUE.push(item) {
            Ok(()) => return true,
            Err(ret) => {
                item = ret;
                // Yield to the main thread.
                delay_us(3_000);
            },
        }
    }
    // Last-ditch: drop oldest frame, push newer one.
    let _ = VIDEO_FRAME_QUEUE.pop();
    VIDEO_FRAME_QUEUE.push(item).is_ok()
}

/// Drain the stream queue without decoding (audio-only fallback).
/// Keeps the thread alive to handle Stop/Shutdown commands.
fn drain_stream_only() -> bool {
    loop {
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop()
            && matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown)
        {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            drain_queues(true);
            send_audio_cmd(AudioCmd::VideoAudioStop);
            return matches!(cmd, VideoCmd::Shutdown);
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            return false;
        }

        // If a new stream was requested, exit drain so the main loop
        // can pick up STREAM_REQUESTED and start a new play_stream().
        if STREAM_REQUESTED.load(Ordering::Relaxed) {
            return false;
        }

        // Drain frames to prevent queue backup.
        while VIDEO_STREAM_QUEUE.pop().is_some() {}

        delay_us(50_000);
    }
}

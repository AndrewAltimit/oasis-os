//! MP4 parsing and stream demux helpers for video downloads: moov atom
//! detection, sample table traversal, and interleaved sample extraction.

use super::{AUDIO_QUEUE, AudioCmd, io_log_verbose};
use crate::video::{StreamFrame, try_push_stream_frame};

// ---------------------------------------------------------------------------
// MP4 box parsing
// ---------------------------------------------------------------------------

/// Parse MP4 box headers from the first bytes of a download to find where
/// the moov atom ends.  Returns `Some(moov_offset + moov_size)` for
/// faststart files (moov before mdat), or `None` if moov wasn't found.
pub(super) fn find_moov_end(header_bytes: &[u8]) -> Option<u64> {
    let mut pos = 0usize;
    while pos + 8 <= header_bytes.len() {
        let size = u32::from_be_bytes([
            header_bytes[pos],
            header_bytes[pos + 1],
            header_bytes[pos + 2],
            header_bytes[pos + 3],
        ]) as u64;
        let box_type = &header_bytes[pos + 4..pos + 8];

        if box_type == b"moov" {
            if size == 0 {
                return None; // extends to EOF, can't determine end
            }
            return Some(pos as u64 + size);
        }

        // 64-bit extended size
        if size == 1 {
            if pos + 16 > header_bytes.len() {
                break;
            }
            let big = u64::from_be_bytes([
                header_bytes[pos + 8],
                header_bytes[pos + 9],
                header_bytes[pos + 10],
                header_bytes[pos + 11],
                header_bytes[pos + 12],
                header_bytes[pos + 13],
                header_bytes[pos + 14],
                header_bytes[pos + 15],
            ]);
            pos += big as usize;
        } else if size == 0 {
            break; // box extends to EOF
        } else {
            pos += size as usize;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Stream demux helpers
// ---------------------------------------------------------------------------

/// Determine the next sample to extract (lowest file offset among pending
/// video and audio samples).
pub(super) fn next_sample_target(
    v_idx: usize,
    a_idx: usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) -> Option<(u64, u32, bool)> {
    let v_next = video_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(v_idx));
    let a_next = audio_track
        .as_ref()
        .and_then(|t| t.sample_offset_size(a_idx));

    match (v_next, a_next) {
        (Some((vo, vs)), Some((ao, a_s))) => {
            if vo <= ao {
                Some((vo, vs, true))
            } else {
                Some((ao, a_s, false))
            }
        },
        (Some((vo, vs)), None) => Some((vo, vs, true)),
        (None, Some((ao, a_s))) => Some((ao, a_s, false)),
        (None, None) => None,
    }
}

/// Process a chunk of HTTP data, extracting complete audio AND video
/// samples and pushing them to the respective decode threads.
///
/// Video samples are converted from AVCC to Annex B format and pushed
/// to the video thread via `VIDEO_STREAM_QUEUE`. Audio samples are
/// pushed to the audio thread via `AUDIO_QUEUE`. Both use backpressure
/// to throttle the I/O thread when consumers can't keep up.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_stream_chunk(
    chunk: &[u8],
    http_pos: &mut u64,
    have_target: &mut bool,
    sample_offset: &mut u64,
    sample_size: &mut u32,
    sample_is_video: &mut bool,
    sample_data: &mut Vec<u8>,
    video_sample_data: &mut Vec<u8>,
    v_idx: &mut usize,
    a_idx: &mut usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
    audio_track: &Option<oasis_video::demux_lite::TrackInfo>,
) {
    let mut chunk_pos = 0usize;

    while chunk_pos < chunk.len() {
        // Find next sample target if we don't have one.
        if !*have_target {
            match next_sample_target(*v_idx, *a_idx, video_track, audio_track) {
                Some((off, sz, is_v)) => {
                    *sample_offset = off;
                    *sample_size = sz;
                    *sample_is_video = is_v;
                    *have_target = true;
                    if is_v {
                        video_sample_data.clear();
                    } else {
                        sample_data.clear();
                    }
                },
                None => {
                    // All samples extracted; skip remaining data.
                    *http_pos += (chunk.len() - chunk_pos) as u64;
                    return;
                },
            }
        }

        // Skip bytes before sample start.
        if *http_pos < *sample_offset {
            let skip = core::cmp::min(
                (*sample_offset - *http_pos) as usize,
                chunk.len() - chunk_pos,
            );
            chunk_pos += skip;
            *http_pos += skip as u64;
            if *http_pos < *sample_offset {
                return; // need more data to reach sample
            }
        }

        if *sample_is_video {
            // Buffer video sample data for ME H.264 decode.
            let remaining = *sample_size as usize - video_sample_data.len();
            let available = chunk.len() - chunk_pos;
            let take = core::cmp::min(remaining, available);
            video_sample_data.extend_from_slice(&chunk[chunk_pos..chunk_pos + take]);
            chunk_pos += take;
            *http_pos += take as u64;

            if video_sample_data.len() == *sample_size as usize {
                // Convert AVCC→Annex B and push to video thread.
                let raw_data = core::mem::take(video_sample_data);
                push_video_sample(&raw_data, *v_idx, video_track);
                *v_idx += 1;
                *have_target = false;
            }
        } else {
            // Buffer audio sample data.
            let remaining = *sample_size as usize - sample_data.len();
            let available = chunk.len() - chunk_pos;
            let take = core::cmp::min(remaining, available);
            sample_data.extend_from_slice(&chunk[chunk_pos..chunk_pos + take]);
            chunk_pos += take;
            *http_pos += take as u64;

            if sample_data.len() == *sample_size as usize {
                let data = core::mem::take(sample_data);
                // Blocking push with backpressure: retry until the audio
                // queue has space, sleeping 8ms between attempts. This
                // throttles the I/O thread to match the audio decode rate
                // (~23ms per AAC frame at 44.1kHz/1024 samples). A 2ms
                // sleep caused ~11 futile wakeups per frame; 8ms reduces
                // this to ~3, freeing CPU for the audio decode thread.
                let mut cmd = AudioCmd::VideoAudioAac { data };
                loop {
                    match AUDIO_QUEUE.push(cmd) {
                        Ok(()) => break,
                        Err(returned) => {
                            cmd = returned;
                            // Check if playback was stopped to avoid
                            // deadlocking the I/O thread.
                            if !crate::video::is_video_playing() {
                                break;
                            }
                            // SAFETY: sceKernelDelayThread sleeps thread.
                            unsafe {
                                psp::sys::sceKernelDelayThread(8_000);
                            }
                        },
                    }
                }
                *a_idx += 1;
                *have_target = false;
                sample_data.clear();
            }
        }
    }
}

/// Convert a raw AVCC video sample to Annex B and push it to the video
/// decode thread via `VIDEO_STREAM_QUEUE` with backpressure.
fn push_video_sample(
    raw_data: &[u8],
    v_idx: usize,
    video_track: &Option<oasis_video::demux_lite::TrackInfo>,
) {
    let Some(vt) = video_track.as_ref() else {
        return;
    };

    let is_keyframe = vt.sample_is_keyframe(v_idx);
    let timestamp_secs = vt.sample_timestamp(v_idx);

    // Convert AVCC to Annex B (prepends SPS/PPS on keyframes).
    let annex_b = match vt.avcc.as_ref() {
        Some(avcc) => match oasis_video::demux_lite::avcc_to_annex_b(raw_data, avcc, is_keyframe)
        {
            Ok(data) => data,
            Err(e) => {
                io_log_verbose(&format!(
                    "[IO-DL] AVCC→Annex B error at v_idx={v_idx}: {e}"
                ));
                return;
            },
        },
        None => {
            // No AVCC config — assume data is already Annex B.
            raw_data.to_vec()
        },
    };

    // Include raw AVCC data + SPS/PPS for the NAL decoder.
    let (raw_avcc, prefix_size, avcc_sps, avcc_pps) = match vt.avcc.as_ref() {
        Some(avcc) => (
            Some(raw_data.to_vec()),
            avcc.nal_length_size as u8,
            Some(avcc.sps.clone()),
            Some(avcc.pps.clone()),
        ),
        None => (None, 4, None, None),
    };

    // Blocking push with backpressure (same pattern as audio queue).
    let mut frame = StreamFrame {
        data: annex_b,
        raw_avcc,
        nal_prefix_size: prefix_size,
        avcc_sps,
        avcc_pps,
        timestamp_secs,
        is_keyframe,
    };
    loop {
        match try_push_stream_frame(frame) {
            Ok(()) => break,
            Err(returned) => {
                frame = returned;
                if !crate::video::is_video_playing() {
                    break;
                }
                // SAFETY: sceKernelDelayThread sleeps the current thread.
                // Sleep 8ms — video frames arrive less frequently than
                // audio (~30fps vs ~43fps), so backpressure is rare.
                unsafe {
                    psp::sys::sceKernelDelayThread(8_000);
                }
            },
        }
    }
}

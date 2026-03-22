//! Video download handler: streaming MP4 download with moov parsing,
//! HTTP redirect handling, and TLS fallback.

use core::sync::atomic::Ordering;

use super::tls_http::TlsHttpReader;
use super::video_dl_http::{HttpDataSource, http_open_with_redirect};
use super::video_dl_parse::{find_moov_end, process_stream_chunk};
use super::{
    AudioCmd, DOWNLOAD_CANCEL, IO_RESP_QUEUE, IoResponse, io_log, io_log_verbose, send_audio_cmd,
    set_streaming_active,
};

// ---------------------------------------------------------------------------
// Video download handler
// ---------------------------------------------------------------------------

/// Streaming video download: buffers moov atom in memory, parses MP4 track
/// tables, then extracts and pushes demuxed samples directly to the video
/// and audio threads as HTTP data arrives. No disk I/O.
///
/// Supports both HTTP (via sceHttp) and HTTPS (via raw TCP + embedded-tls).
pub(super) fn handle_video_download(url: String, _dest: String, tag: u32) {
    use oasis_video::demux_lite::Mp4Lite;

    io_log(&format!("[IO-DL] starting stream: {url}"));

    // Clear any previous cancellation flag.
    DOWNLOAD_CANCEL.store(false, Ordering::Release);

    // Check connectivity without showing a dialog (must not call
    // ensure_net_init_pub from background thread -- freezes EBOOT).
    if !psp::net::is_connected() {
        let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
            tag,
            msg: "not connected to WiFi".to_string(),
        });
        return;
    }

    // Try sceHttp first (HTTP port 80), fall back to raw TCP + TLS 1.3
    // (HTTPS port 443) if sceHttp fails. sceHttp's internal connection
    // pool corrupts after an aborted partial download, causing 0x80431079
    // on subsequent requests. The TLS path uses independent raw sockets.
    let http_url = if url.starts_with("https://") {
        url.replacen("https://", "http://", 1)
    } else {
        url.clone()
    };

    // SAFETY: All sceHttp calls use IDs returned by prior creation.
    let (mut source, content_length) = match unsafe { http_open_with_redirect(&http_url) } {
        Ok((req_id, conn_id, cl)) => {
            io_log("[IO-DL] sceHttp OK");
            (HttpDataSource::SceHttp { req_id, conn_id }, cl)
        },
        Err((msg, https_redirect)) => {
            // Try TLS fallback. First try archive.org HTTPS
            // (may redirect to a different, reachable CDN node),
            // then try CDN HTTPS directly as a last resort.
            let origin_tls = if url.starts_with("http://") {
                url.replacen("http://", "https://", 1)
            } else {
                url.clone()
            };

            // Build candidate list: origin first, then CDN if different.
            let mut tls_candidates: Vec<String> = vec![origin_tls];
            if let Some(cdn_url) = &https_redirect {
                if !tls_candidates.contains(cdn_url) {
                    tls_candidates.push(cdn_url.clone());
                }
            }

            let mut last_err = msg.clone();
            let mut found = None;
            for (i, tls_url) in tls_candidates.iter().enumerate() {
                io_log(&format!(
                    "[IO-DL] sceHttp failed ({msg}), trying TLS \
                         #{} to {tls_url}...",
                    i + 1
                ));
                match TlsHttpReader::open(tls_url) {
                    Ok((reader, cl)) => {
                        io_log(&format!("[IO-DL] TLS fallback #{} OK, len={cl}", i + 1));
                        found = Some((reader, cl));
                        break;
                    },
                    Err(e) => {
                        io_log(&format!("[IO-DL] TLS fallback #{} failed: {e}", i + 1));
                        last_err = e;
                    },
                }
            }

            match found {
                Some((reader, cl)) => (HttpDataSource::Tls(reader), cl),
                None => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                        tag,
                        msg: format!("HTTP: {msg}; TLS: {last_err}"),
                    });
                    return;
                },
            }
        },
    };

    let total = if content_length > 0 {
        Some(content_length)
    } else {
        None
    };
    io_log(&format!("[IO-DL] content-length={content_length}"));

    // Phase 1: buffer data until moov atom is fully received.
    let mut moov_buf: Vec<u8> = Vec::new();
    let mut moov_end: Option<u64> = None;
    // 32KB read buffer: reduces sceHttp/TLS syscall overhead and better
    // utilizes PSP's 16-32KB TCP receive buffer per read.
    let mut buf = [0u8; 32768];
    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    loop {
        // Check for cancellation (user pressed Circle during download).
        if DOWNLOAD_CANCEL.load(Ordering::Acquire) {
            io_log("[IO-DL] cancelled during moov buffering");
            source.cleanup();
            return;
        }

        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase1): {n:#x}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("read: {n:#x}"),
            });
            return;
        }
        if n == 0 {
            break; // EOF during moov buffering
        }

        moov_buf.extend_from_slice(&buf[..n as usize]);
        downloaded += n as u64;

        // Report progress.
        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }

        // Try to find moov end from headers once we have enough.
        if moov_end.is_none() && moov_buf.len() >= 32 {
            moov_end = find_moov_end(&moov_buf);
            if let Some(end) = moov_end {
                io_log(&format!("[IO-DL] moov ends at byte {end}"));
            }
        }

        // Check if we've buffered past moov end.
        if let Some(end) = moov_end {
            if downloaded >= end {
                io_log(&format!(
                    "[IO-DL] moov fully buffered ({downloaded} bytes, \
                     moov_end={end})"
                ));
                break;
            }
        }

        // Safety limit: if moov hasn't been found after 8MB, abort.
        if moov_buf.len() > 8 * 1024 * 1024 {
            io_log("[IO-DL] moov not found in first 8MB, aborting");
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: String::from("moov atom not found (non-faststart?)"),
            });
            return;
        }
    }

    // Parse moov using Mp4Lite with a Cursor over the buffered data.
    io_log(&format!(
        "[IO-DL] parsing moov ({} bytes buffered)...",
        moov_buf.len()
    ));

    let cursor = std::io::Cursor::new(&moov_buf);
    let mp4 = match Mp4Lite::open(cursor) {
        Ok(m) => m,
        Err(e) => {
            io_log(&format!("[IO-DL] Mp4Lite parse failed: {e}"));
            source.cleanup();
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoError {
                tag,
                msg: format!("MP4 parse: {e}"),
            });
            return;
        },
    };

    let video_track = mp4.video_track_info().cloned();
    let audio_track = mp4.audio_track_info().cloned();
    drop(mp4);

    let v_count = video_track.as_ref().map_or(0, |t| t.sample_count());
    let a_count = audio_track.as_ref().map_or(0, |t| t.sample_count());
    io_log(&format!(
        "[IO-DL] parsed: {v_count} video, {a_count} audio samples"
    ));

    // Send AAC config to audio thread before any frames arrive.
    if let Some(ref at) = audio_track {
        if let Some(ref aac) = at.aac_config {
            send_audio_cmd(AudioCmd::VideoAudioAacConfig {
                sample_rate: aac.sample_rate,
                channels: aac.channels,
            });
            io_log(&format!(
                "[IO-DL] AAC config: rate={}, ch={}",
                aac.sample_rate, aac.channels
            ));
        }
    }

    io_log("[IO-DL] setting video_playing...");

    // Pre-arm the playing flag BEFORE sending StreamStart to avoid a
    // race: the I/O thread checks is_video_playing() in the phase-2
    // loop, but the video thread may not have processed the command yet.
    crate::video::set_video_playing(true);
    crate::video::request_stream_start();

    io_log("[IO-DL] sending VideoStreamReady...");

    // Notify main thread that streaming playback has begun.
    let _ = IO_RESP_QUEUE.push(IoResponse::VideoStreamReady {
        tag,
        path: String::new(),
        content_length: content_length as u32,
    });

    // Phase 2: stream mdat samples from HTTP(S).
    let mut v_idx = 0usize;
    let mut a_idx = 0usize;
    let mut http_pos: u64;

    // Pre-allocate sample buffers with capacity based on the largest
    // sample sizes to avoid per-sample reallocation during streaming.
    let max_audio_sample = audio_track
        .as_ref()
        .map(|t| t.max_sample_size())
        .unwrap_or(0) as usize;
    let max_video_sample = video_track
        .as_ref()
        .map(|t| t.max_sample_size())
        .unwrap_or(0) as usize;
    let mut sample_data: Vec<u8> = Vec::with_capacity(max_audio_sample.max(4096));
    let mut video_sample_data: Vec<u8> =
        Vec::with_capacity(max_video_sample.max(16384));
    let mut sample_offset: u64 = 0;
    let mut sample_size: u32 = 0;
    let mut sample_is_video = false;
    let mut have_target = false;

    let moov_end_off = moov_end.unwrap_or(downloaded);
    let leftover_start = moov_end_off as usize;
    let leftover = if leftover_start < moov_buf.len() {
        &moov_buf[leftover_start..]
    } else {
        &[]
    };
    http_pos = moov_end_off;

    io_log(&format!(
        "[IO-DL] leftover={} bytes, moov_end_off={moov_end_off}",
        leftover.len()
    ));

    if !leftover.is_empty() {
        process_stream_chunk(
            leftover,
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut video_sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );
        io_log(&format!("[IO-DL] leftover processed: v={v_idx} a={a_idx}"));
    }

    io_log("[IO-DL] dropping moov_buf...");
    drop(moov_buf);
    io_log("[IO-DL] entering phase 2 loop");

    // Suppress verbose logging during streaming to avoid Memory Stick
    // I/O stalls on the I/O thread (~5-20ms per log write).
    set_streaming_active(true);

    let mut loop_iter = 0u32;
    loop {
        if !crate::video::is_video_playing() || DOWNLOAD_CANCEL.load(Ordering::Acquire) {
            io_log("[IO-DL] playback stopped, ending stream");
            break;
        }

        if loop_iter < 3 {
            io_log_verbose(&format!("[IO-DL] phase2 read #{loop_iter}..."));
        }
        let n = source.read_data(&mut buf);
        if n < 0 {
            io_log(&format!("[IO-DL] read error (phase2): {n:#x}"));
            break;
        }
        if n == 0 {
            break; // EOF
        }
        if loop_iter < 3 {
            io_log_verbose(&format!(
                "[IO-DL] phase2 read #{loop_iter}: {n} bytes"
            ));
        }

        downloaded += n as u64;
        loop_iter += 1;

        process_stream_chunk(
            &buf[..n as usize],
            &mut http_pos,
            &mut have_target,
            &mut sample_offset,
            &mut sample_size,
            &mut sample_is_video,
            &mut sample_data,
            &mut video_sample_data,
            &mut v_idx,
            &mut a_idx,
            &video_track,
            &audio_track,
        );

        if downloaded - last_progress >= 65536 {
            let _ = IO_RESP_QUEUE.push(IoResponse::VideoProgress {
                tag,
                bytes: downloaded,
                total,
            });
            last_progress = downloaded;
        }
    }

    set_streaming_active(false);
    source.cleanup();

    io_log(&format!(
        "[IO-DL] stream complete: {downloaded} bytes, \
         {v_idx}/{v_count} video, {a_idx}/{a_count} audio"
    ));

    crate::video::set_video_playing(false);
    send_audio_cmd(AudioCmd::VideoAudioStop);
}

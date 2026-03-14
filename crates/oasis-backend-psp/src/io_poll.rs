//! I/O response polling (extracted from main loop body).
//!
//! Drains async responses from the I/O worker thread and dispatches them
//! to the appropriate app state structs.

use oasis_backend_psp::{AudioCmd, IoResponse, PspBackend};

use crate::app_states::*;
use crate::types::RadioStatus;
use crate::views;

/// Poll all pending I/O responses and update app state accordingly.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_io_responses(
    io: &oasis_backend_psp::threading::IoHandle,
    audio: &oasis_backend_psp::AudioHandle,
    backend: &mut PspBackend,
    term: &mut TerminalState,
    pv: &mut PhotoViewerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    dbg_log: &dyn Fn(&str),
) {
    while let Some(resp) = io.try_recv() {
        match resp {
            IoResponse::TextureReady {
                path: _,
                width,
                height,
                rgba,
            } => {
                if pv.loading {
                    if let Some(old) = pv.tex.take() {
                        backend.destroy_texture_inner(old);
                    }
                    pv.tex = backend.load_texture_inner(width, height, &rgba);
                    pv.img_w = width;
                    pv.img_h = height;
                    pv.viewing = true;
                    pv.loading = false;
                }
            },
            IoResponse::Error { path, msg } => {
                dbg_log(&format!("[IO] error: {} - {}", path, msg));
                term.lines.push(format!("I/O error: {} - {}", path, msg));
                pv.loading = false;
                if br.loading {
                    br.loading = false;
                    br.status_msg = format!("Error: {}", msg);
                }
            },
            IoResponse::FileReady { .. } => {},
            IoResponse::HttpDone {
                tag,
                status_code,
                body,
            } => {
                if tag == 0xBEEF {
                    let html = String::from_utf8_lossy(&body);
                    let text = views::strip_html(&html);
                    br.content_lines = views::wrap_text(&text, 58);
                    br.scroll = 0;
                    br.loading = false;
                    br.status_msg = format!("HTTP {} - {} bytes", status_code, body.len());
                } else if (tag & 0xFF00) == 0xAA00 {
                    // Legacy TV Guide tag -- no longer used.
                    let _ = (tag, body);
                } else {
                    let preview = String::from_utf8_lossy(&body[..body.len().min(256)]);
                    term.lines.push(format!(
                        "HTTP {status_code} ({} bytes): {preview}",
                        body.len(),
                    ));
                }
            },
            IoResponse::TvCatalogReady { ch_idx, episodes } => {
                dbg_log(&format!(
                    "[TV] catalog ready ch={ch_idx} episodes={}",
                    episodes.len()
                ));
                if ch_idx < tv.channels.len() {
                    let ch = &tv.channels[ch_idx];
                    let catalog = tv.catalogs[ch_idx].get_or_insert_with(|| {
                        oasis_core::apps::tv_guide::ChannelCatalog::new(ch.number)
                    });
                    if !episodes.is_empty() {
                        catalog.add_episodes(episodes);
                    }
                }
            },
            IoResponse::RadioConnected {
                fd,
                icy_metaint,
                initial_data,
            } => {
                radio.status = RadioStatus::Buffering;
                audio.send(AudioCmd::RadioStreamFromFd {
                    fd,
                    icy_metaint,
                    initial_data,
                });
            },
            IoResponse::RadioError { msg } => {
                radio.status = RadioStatus::Error;
                radio.error_msg = msg;
            },
            IoResponse::VideoProgress {
                tag: _,
                bytes,
                total,
            } => {
                if let Some(t) = total {
                    if t > 0 {
                        tv.download_progress = bytes as f32 / t as f32;
                    }
                }
            },
            IoResponse::VideoReady { tag: _, path } => {
                tv.downloading = false;
                tv.download_progress = 1.0;
                oasis_backend_psp::video::send_video_cmd(
                    oasis_backend_psp::video::VideoCmd::Play { path, seek_secs: 0 },
                );
            },
            IoResponse::VideoStreamReady { tag: _, .. } => {
                tv.downloading = false;
                tv.download_progress = 1.0;
            },
            IoResponse::VideoError { tag: _, msg } => {
                tv.downloading = false;
                tv.error_msg = format!("Download: {msg}");
                tv.tuned = None;
            },
        }
    }
}

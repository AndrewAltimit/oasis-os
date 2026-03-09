//! Classic full-screen view renderers.
//!
//! List-view functions (file manager, photo browser, music browser, browser,
//! radio stations, TV channels) have been migrated to `views_sdi.rs`.  The
//! direct-rendering functions here are retained for playback overlays that
//! use animated content (visualizer bars, progress bars, video textures).
#![allow(dead_code)]

use oasis_backend_psp::{AudioHandle, Color, FileEntry, PspBackend, SCREEN_WIDTH, TextureId};

use crate::chrome::draw_view_header;
use crate::theme::*;
use crate::types::RADIO_STATIONS;

// ---------------------------------------------------------------------------
// Terminal rendering (classic full-screen)
// ---------------------------------------------------------------------------

pub(crate) fn draw_terminal(
    backend: &mut PspBackend,
    lines: &[String],
    input: &str,
    scroll_back: usize,
) {
    let bg = Color::rgba(0, 0, 0, 180);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    // Compute visible window: scroll_back=0 means show the latest lines.
    let end = lines.len().saturating_sub(scroll_back);
    let start = end.saturating_sub(MAX_OUTPUT_LINES);
    for (i, line) in lines[start..end].iter().enumerate() {
        let y = CONTENT_TOP as i32 + 4 + i as i32 * 9;
        if y > TERM_INPUT_Y - 12 {
            break;
        }
        backend.draw_text_inner(line, 4, y, 8, Color::rgb(0, 255, 0));
    }

    // Scroll indicator when not at bottom.
    if scroll_back > 0 {
        let indicator = format!("-- scroll: +{} --", scroll_back);
        backend.draw_text_inner(
            &indicator,
            SCREEN_WIDTH as i32 - indicator.len() as i32 * 8 - 4,
            TERM_INPUT_Y - 10,
            8,
            Color::rgb(180, 180, 0),
        );
    }

    let prompt = format!("> {}_", input);
    backend.draw_text_inner(&prompt, 4, TERM_INPUT_Y, 8, Color::rgb(0, 255, 0));
}

// ---------------------------------------------------------------------------
// File manager rendering (classic full-screen)
// ---------------------------------------------------------------------------

pub(crate) fn draw_file_manager_dual(
    backend: &mut PspBackend,
    path_l: &str,
    entries_l: &[FileEntry],
    selected_l: usize,
    scroll_l: usize,
    path_r: &str,
    entries_r: &[FileEntry],
    selected_r: usize,
    scroll_r: usize,
    active_panel: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    // Header with both panel paths.
    let header = if active_panel == 0 {
        format!("[L] {}  |  {}", path_l, path_r)
    } else {
        format!("{}  |  [R] {}", path_l, path_r)
    };
    draw_view_header(
        backend,
        "FILE MGR",
        Color::rgb(100, 200, 255),
        Some(&header),
    );

    // Vertical divider.
    let half_w = SCREEN_WIDTH / 2;
    let div_x = half_w as i32;
    backend.fill_rect_inner(
        div_x,
        CONTENT_TOP as i32 + 12,
        1,
        CONTENT_H - 12,
        Color::rgba(100, 200, 255, 80),
    );

    // Active panel indicator (bright line at top of active panel).
    let indicator_x = if active_panel == 0 { 0 } else { div_x + 1 };
    let indicator_w = if active_panel == 0 {
        half_w - 1
    } else {
        half_w
    };
    backend.fill_rect_inner(
        indicator_x,
        CONTENT_TOP as i32 + 12,
        indicator_w,
        1,
        Color::rgb(100, 200, 255),
    );

    // Draw each panel.
    let panels: [(&[FileEntry], usize, usize, i32, u32, bool); 2] = [
        (
            entries_l,
            selected_l,
            scroll_l,
            0,
            half_w - 1,
            active_panel == 0,
        ),
        (
            entries_r,
            selected_r,
            scroll_r,
            div_x + 1,
            half_w,
            active_panel == 1,
        ),
    ];
    // Half the visible rows since panels are narrower but same height.
    let panel_rows = FM_VISIBLE_ROWS;

    for &(entries, selected, scroll, px, pw, is_active) in &panels {
        if entries.is_empty() {
            backend.draw_text_inner("(empty)", px + 4, FM_START_Y, 8, Color::rgb(140, 140, 140));
            continue;
        }

        let end = (scroll + panel_rows).min(entries.len());
        for i in scroll..end {
            let entry = &entries[i];
            let row = (i - scroll) as i32;
            let y = FM_START_Y + row * FM_ROW_H;

            if i == selected && is_active {
                backend.fill_rect_inner(
                    px,
                    y - 1,
                    pw,
                    FM_ROW_H as u32,
                    Color::rgba(80, 120, 200, 100),
                );
            }

            let (prefix, prefix_clr) = if entry.is_dir {
                ("[D]", Color::rgb(255, 220, 80))
            } else {
                ("[F]", Color::rgb(180, 180, 180))
            };
            backend.draw_text_inner(prefix, px + 2, y, 8, prefix_clr);

            let name_color = if entry.is_dir {
                Color::rgb(120, 220, 255)
            } else {
                Color::WHITE
            };
            // Max chars for half-width panel (~28 chars at 8px each).
            let max_name_chars = ((pw as i32 - 32) / CHAR_W).max(4) as usize;
            let display_name = if entry.name.len() > max_name_chars {
                let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
                format!("{}..", truncated)
            } else {
                entry.name.clone()
            };
            backend.draw_text_inner(&display_name, px + 28, y, 8, name_color);
        }

        // Scroll indicator per panel.
        if entries.len() > panel_rows {
            let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
            let track_h = CONTENT_H as i32 - 16;
            let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
            let dot_x = px + pw as i32 - 4;
            backend.fill_rect_inner(dot_x, dot_y, 3, 8, Color::rgba(255, 255, 255, 120));
        }
    }
}

// ---------------------------------------------------------------------------
// Photo viewer rendering (classic full-screen)
// ---------------------------------------------------------------------------

pub(crate) fn draw_photo_browser(
    backend: &mut PspBackend,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(
        backend,
        "PHOTO VIEWER",
        Color::rgb(100, 149, 237),
        Some(path),
    );

    if entries.is_empty() {
        backend.draw_text_inner(
            "No images found (.jpg/.jpeg)",
            8,
            FM_START_Y,
            8,
            Color::rgb(140, 140, 140),
        );
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(80, 120, 200, 100),
            );
        }

        let (prefix, prefix_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[I]", Color::rgb(100, 200, 255))
        };
        backend.draw_text_inner(prefix, 4, y, 8, prefix_clr);

        let name_color = if entry.is_dir {
            Color::rgb(120, 220, 255)
        } else {
            Color::WHITE
        };
        let max_name_chars = 44;
        let display_name = if entry.name.len() > max_name_chars {
            let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
            format!("{}..", truncated)
        } else {
            entry.name.clone()
        };
        backend.draw_text_inner(&display_name, 32, y, 8, name_color);

        if !entry.is_dir {
            let size_str = oasis_backend_psp::format_size(entry.size);
            let size_x = 480 - (size_str.len() as i32 * 8) - 4;
            backend.draw_text_inner(&size_str, size_x, y, 8, Color::rgb(180, 180, 180));
        }
    }

    // Scrollbar.
    if entries.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

pub(crate) fn draw_photo_view(
    backend: &mut PspBackend,
    tex: Option<TextureId>,
    img_w: u32,
    img_h: u32,
) {
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, Color::BLACK);

    if let Some(t) = tex {
        let max_w = SCREEN_WIDTH;
        let max_h = CONTENT_H;
        let scale_w = max_w as f32 / img_w as f32;
        let scale_h = max_h as f32 / img_h as f32;
        let scale = if scale_w < scale_h { scale_w } else { scale_h };
        let draw_w = (img_w as f32 * scale) as u32;
        let draw_h = (img_h as f32 * scale) as u32;
        let draw_x = ((max_w - draw_w) / 2) as i32;
        let draw_y = CONTENT_TOP as i32 + ((max_h - draw_h) / 2) as i32;

        backend.blit_inner(t, draw_x, draw_y, draw_w, draw_h);
    } else {
        backend.draw_text_inner("Failed to load image", 160, 130, 8, Color::rgb(255, 80, 80));
    }
}

// ---------------------------------------------------------------------------
// Music player rendering (classic full-screen, threaded audio)
// ---------------------------------------------------------------------------

pub(crate) fn draw_music_browser(
    backend: &mut PspBackend,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "MUSIC PLAYER", Color::rgb(205, 92, 92), Some(path));

    if entries.is_empty() {
        backend.draw_text_inner(
            "No MP3 files found",
            8,
            FM_START_Y,
            8,
            Color::rgb(140, 140, 140),
        );
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(200, 80, 80, 100),
            );
        }

        let (prefix, prefix_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[M]", Color::rgb(205, 92, 92))
        };
        backend.draw_text_inner(prefix, 4, y, 8, prefix_clr);

        let name_color = if entry.is_dir {
            Color::rgb(120, 220, 255)
        } else {
            Color::WHITE
        };
        let max_name_chars = 44;
        let display_name = if entry.name.len() > max_name_chars {
            let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
            format!("{}..", truncated)
        } else {
            entry.name.clone()
        };
        backend.draw_text_inner(&display_name, 32, y, 8, name_color);

        if !entry.is_dir {
            let size_str = oasis_backend_psp::format_size(entry.size);
            let size_x = 480 - (size_str.len() as i32 * 8) - 4;
            backend.draw_text_inner(&size_str, size_x, y, 8, Color::rgb(180, 180, 180));
        }
    }

    // Scrollbar.
    if entries.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

/// Draw the now-playing music player UI (using threaded AudioHandle).
pub(crate) fn draw_music_player_threaded(
    backend: &mut PspBackend,
    file_name: &str,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;
    let title_color = Color::rgb(255, 200, 200);
    let info_color = Color::rgb(180, 180, 180);

    // Now-playing visualizer above album art.
    draw_now_playing_visualizer(backend, audio, viz_frame);

    // Album art placeholder.
    let art_size: u32 = 70;
    let art_x = cx - art_size as i32 / 2;
    let art_y = CONTENT_TOP as i32 + 44;
    backend.fill_rect_inner(art_x, art_y, art_size, art_size, Color::rgb(205, 92, 92));
    backend.fill_rect_inner(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        Color::rgb(60, 30, 30),
    );
    backend.draw_text_inner("MP3", art_x + 22, art_y + 28, 8, Color::rgb(205, 92, 92));

    // Track name.
    let max_chars = 50;
    let display_name = if file_name.len() > max_chars {
        let truncated: String = file_name.chars().take(max_chars - 2).collect();
        format!("{}..", truncated)
    } else {
        file_name.to_string()
    };
    let name_x = cx - (display_name.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        &display_name,
        name_x,
        art_y + art_size as i32 + 8,
        8,
        title_color,
    );

    // Format info from atomic state.
    let info = format!(
        "{}Hz  {}kbps  {}ch",
        audio.sample_rate(),
        audio.bitrate(),
        audio.channels(),
    );
    let info_x = cx - (info.len() as i32 * 8) / 2;
    backend.draw_text_inner(&info, info_x, art_y + art_size as i32 + 20, 8, info_color);

    // Progress bar.
    let pos = audio.position_ms();
    let dur = audio.duration_ms();
    let bar_w: u32 = 260;
    let bar_x = cx - bar_w as i32 / 2;
    let bar_y = art_y + art_size as i32 + 32;

    // Track bar outline.
    backend.fill_rect_inner(bar_x, bar_y, bar_w, 4, Color::rgba(80, 80, 80, 180));
    // Fill.
    if dur > 0 {
        let fill = ((bar_w as u64 * pos) / dur).min(bar_w as u64) as u32;
        if fill > 0 {
            backend.fill_rect_inner(bar_x, bar_y, fill, 4, Color::rgb(205, 92, 92));
        }
    }
    // Time labels.
    let pos_s = (pos / 1000) as u32;
    let dur_s = (dur / 1000) as u32;
    let time_str = format!(
        "{}:{:02} / {}:{:02}",
        pos_s / 60,
        pos_s % 60,
        dur_s / 60,
        dur_s % 60,
    );
    let time_x = cx - (time_str.len() as i32 * 8) / 2;
    backend.draw_text_inner(&time_str, time_x, bar_y + 6, 8, info_color);

    let status = if audio.is_paused() {
        "PAUSED"
    } else {
        "PLAYING"
    };
    let status_clr = if audio.is_paused() {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(120, 255, 120)
    };
    let status_x = cx - (status.len() as i32 * 8) / 2;
    backend.draw_text_inner(status, status_x, bar_y + 20, 8, status_clr);
}

/// Draw a larger visualizer for the now-playing music player view.
pub(crate) fn draw_now_playing_visualizer(
    backend: &mut PspBackend,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let bar_count: i32 = 20;
    let bar_w: i32 = 6;
    let bar_gap: i32 = 2;
    let max_h: i32 = 30;
    let min_h: i32 = 2;
    let total_w = bar_count * (bar_w + bar_gap) - bar_gap;
    let viz_x = (SCREEN_WIDTH as i32 - total_w) / 2;
    let viz_base_y = CONTENT_TOP as i32 + 40;
    let playing = (audio.is_playing() && !audio.is_paused())
        || (audio.is_radio_streaming() && !audio.is_radio_buffering());

    for i in 0..bar_count {
        let bar_h = if playing {
            let t = viz_frame as f32 * 0.12;
            let freq1 = 0.7 + (i as f32) * 0.25;
            let freq2 = 1.4 + (i as f32) * 0.15;
            let phase = (i as f32) * 1.1;
            let val =
                libm::sinf(t * freq1 + phase) * 0.6 + libm::sinf(t * freq2 + phase * 0.7) * 0.4;
            let norm = (val + 1.0) * 0.5;
            min_h + ((max_h - min_h) as f32 * norm) as i32
        } else {
            min_h
        };
        let bx = viz_x + i * (bar_w + bar_gap);
        let by = viz_base_y - bar_h;
        let r = (120 + ((i * 4) as u8).min(40)) as u8;
        let b = (160 + ((i * 3) as u8).min(30)) as u8;
        let bar_clr = Color::rgba(r, 60, b, 200);
        backend.fill_rect_inner(bx, by, bar_w as u32, bar_h as u32, bar_clr);
        if bar_h > 2 {
            backend.fill_rect_inner(bx, by, bar_w as u32, 1, VIZ_BAR_PEAK);
        }
    }
}

// ---------------------------------------------------------------------------
// Browser rendering (classic full-screen)
// ---------------------------------------------------------------------------

/// Strip HTML tags and decode common entities.
pub(crate) fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut i = 0;
    let bytes = html.as_bytes();

    while i < bytes.len() {
        if in_script {
            // Look for </script>.
            if i + 8 < bytes.len() {
                let window: &[u8] = &bytes[i..i + 9];
                let lower: Vec<u8> = window.iter().map(|b| b.to_ascii_lowercase()).collect();
                if lower == b"</script>" {
                    in_script = false;
                    i += 9;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if in_style {
            if i + 7 < bytes.len() {
                let window: &[u8] = &bytes[i..i + 8];
                let lower: Vec<u8> = window.iter().map(|b| b.to_ascii_lowercase()).collect();
                if lower == b"</style>" {
                    in_style = false;
                    i += 8;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'<' {
            // Check for <script or <style.
            if i + 7 < bytes.len() {
                let peek: Vec<u8> = bytes[i + 1..i + 7]
                    .iter()
                    .map(|b| b.to_ascii_lowercase())
                    .collect();
                if peek == b"script" {
                    in_script = true;
                    in_tag = true;
                    i += 1;
                    continue;
                }
                if peek.starts_with(b"style") {
                    in_style = true;
                    in_tag = true;
                    i += 1;
                    continue;
                }
            }
            in_tag = true;
            // Insert newline for block elements.
            if i + 2 < bytes.len() {
                let next = bytes[i + 1].to_ascii_lowercase();
                if next == b'p'
                    || next == b'h'
                    || (next == b'b'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'r')
                    || (next == b'd'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'i')
                    || (next == b'l'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'i')
                {
                    out.push('\n');
                }
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if in_tag {
            i += 1;
            continue;
        }
        // Decode entities.
        if bytes[i] == b'&' {
            if i + 4 < bytes.len() && &bytes[i..i + 4] == b"&lt;" {
                out.push('<');
                i += 4;
                continue;
            }
            if i + 4 < bytes.len() && &bytes[i..i + 4] == b"&gt;" {
                out.push('>');
                i += 4;
                continue;
            }
            if i + 5 < bytes.len() && &bytes[i..i + 5] == b"&amp;" {
                out.push('&');
                i += 5;
                continue;
            }
            if i + 6 < bytes.len() && &bytes[i..i + 6] == b"&nbsp;" {
                out.push(' ');
                i += 6;
                continue;
            }
            if i + 6 < bytes.len() && &bytes[i..i + 6] == b"&quot;" {
                out.push('"');
                i += 6;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Word-wrap text to `max_chars` columns.
pub(crate) fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            if lines.last().map_or(true, |l: &String| !l.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut line = String::new();
        for word in &words {
            if line.is_empty() {
                line = word.to_string();
            } else if line.len() + 1 + word.len() <= max_chars {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

pub(crate) fn draw_browser_view(
    backend: &mut PspBackend,
    url: &str,
    lines: &[String],
    scroll: usize,
    status_msg: &str,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "BROWSER", Color::rgb(50, 120, 200), None);

    // URL bar.
    let url_y = CONTENT_TOP as i32 + 3;
    let url_x = 4 + 7 * CHAR_W + 8;
    let display_url = if url.len() > 45 {
        let trunc: String = url.chars().take(43).collect();
        format!("{}..", trunc)
    } else {
        url.to_string()
    };
    backend.draw_text_inner(&display_url, url_x, url_y, 8, Color::rgb(120, 180, 255));

    // Status line.
    backend.draw_text_inner(status_msg, 4, FM_START_Y - 1, 8, Color::rgb(160, 160, 160));

    // Content area.
    let text_start_y = FM_START_Y + 10;
    let visible_rows = ((BOTTOMBAR_Y - HINT_Y_OFFSET - text_start_y) / 9) as usize;
    let end = (scroll + visible_rows).min(lines.len());
    for i in scroll..end {
        let row = (i - scroll) as i32;
        let y = text_start_y + row * 9;
        backend.draw_text_inner(&lines[i], 4, y, 8, Color::rgb(220, 220, 220));
    }

    // Scroll indicator.
    if lines.len() > visible_rows && !lines.is_empty() {
        let ratio = scroll as f32 / (lines.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 30;
        let dot_y = text_start_y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

// ---------------------------------------------------------------------------
// Radio rendering (classic full-screen)
// ---------------------------------------------------------------------------

pub(crate) fn draw_radio_stations(backend: &mut PspBackend, selected: usize, scroll: usize) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "RADIO", Color::rgb(255, 140, 60), None);

    if RADIO_STATIONS.is_empty() {
        backend.draw_text_inner("No stations", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(RADIO_STATIONS.len());
    for i in scroll..end {
        let station = &RADIO_STATIONS[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(255, 140, 60, 100),
            );
        }

        // Radio icon.
        backend.draw_text_inner("[R]", 4, y, 8, Color::rgb(255, 140, 60));

        // Station name.
        backend.draw_text_inner(station.name, 32, y, 8, Color::WHITE);

        // Genre.
        let genre_x = 230;
        backend.draw_text_inner(station.genre, genre_x, y, 8, Color::rgb(160, 160, 160));

        // Bitrate.
        let br_str = format!("{}k", station.bitrate);
        let br_x = 480 - (br_str.len() as i32 * 8) - 4;
        backend.draw_text_inner(&br_str, br_x, y, 8, Color::rgb(140, 140, 140));
    }

    // Scrollbar.
    if RADIO_STATIONS.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (RADIO_STATIONS.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

pub(crate) fn draw_radio_playing(
    backend: &mut PspBackend,
    station_name: &str,
    now_playing: &str,
    is_buffering: bool,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;

    // Visualizer (reuse music player's).
    draw_now_playing_visualizer(backend, audio, viz_frame);

    // Radio icon placeholder.
    let art_size: u32 = 70;
    let art_x = cx - art_size as i32 / 2;
    let art_y = CONTENT_TOP as i32 + 44;
    backend.fill_rect_inner(art_x, art_y, art_size, art_size, Color::rgb(255, 140, 60));
    backend.fill_rect_inner(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        Color::rgb(60, 40, 15),
    );
    backend.draw_text_inner("RADIO", art_x + 12, art_y + 28, 8, Color::rgb(255, 140, 60));

    // Station name.
    let max_chars = 50;
    let display_name = if station_name.len() > max_chars {
        let trunc: String = station_name.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        station_name.to_string()
    };
    let name_x = cx - (display_name.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        &display_name,
        name_x,
        art_y + art_size as i32 + 8,
        8,
        Color::rgb(255, 200, 150),
    );

    // Now playing (ICY metadata).
    if !now_playing.is_empty() {
        let np_display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let np_x = cx - (np_display.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &np_display,
            np_x,
            art_y + art_size as i32 + 20,
            8,
            Color::rgb(180, 180, 180),
        );
    }

    // Status.
    let status = if is_buffering {
        "BUFFERING"
    } else {
        "STREAMING"
    };
    let status_clr = if is_buffering {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(120, 255, 120)
    };
    let status_x = cx - (status.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        status,
        status_x,
        art_y + art_size as i32 + 36,
        8,
        status_clr,
    );
}

pub(crate) fn draw_radio_error(backend: &mut PspBackend, error_msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "RADIO", Color::rgb(255, 140, 60), None);

    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;

    backend.draw_text_inner(
        "Connection Error",
        cx - 8 * 8,
        cy - 12,
        8,
        Color::rgb(255, 80, 80),
    );

    let max_chars = 55;
    let display_msg = if error_msg.len() > max_chars {
        let trunc: String = error_msg.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        error_msg.to_string()
    };
    let msg_x = cx - (display_msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(&display_msg, msg_x, cy + 4, 8, Color::rgb(200, 200, 200));

    backend.draw_text_inner(
        "Press X to retry or O to go back",
        cx - 16 * 8,
        cy + 20,
        8,
        Color::rgb(140, 140, 140),
    );
}

// ---------------------------------------------------------------------------
// TV Guide drawing functions
// ---------------------------------------------------------------------------

/// Draw the TV Guide channel list (browsing mode).
pub(crate) fn draw_tv_channels(
    backend: &mut PspBackend,
    channels: &[oasis_core::apps::tv_guide::Channel],
    catalogs: &[Option<oasis_core::apps::tv_guide::ChannelCatalog>],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

    if channels.is_empty() {
        backend.draw_text_inner("No channels", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(channels.len());
    for i in scroll..end {
        let ch = &channels[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(0, 100, 200, 100),
            );
        }

        // Channel number.
        let num_str = format!("{:2}", ch.number);
        backend.draw_text_inner(&num_str, 4, y, 8, Color::rgb(0, 160, 255));
        // Call sign.
        backend.draw_text_inner(&ch.call_sign, 28, y, 8, Color::WHITE);
        // Channel name.
        let name_x = 80;
        let max_name = 25;
        let display_name = if ch.name.len() > max_name {
            let trunc: String = ch.name.chars().take(max_name - 2).collect();
            format!("{}..", trunc)
        } else {
            ch.name.clone()
        };
        backend.draw_text_inner(&display_name, name_x, y, 8, Color::rgb(200, 200, 200));
        // Status indicator (loaded / loading).
        let status_x = 380;
        if i < catalogs.len() {
            if let Some(cat) = &catalogs[i] {
                let ep_str = format!("{}ep", cat.episodes.len());
                backend.draw_text_inner(&ep_str, status_x, y, 8, Color::rgb(120, 200, 120));
            } else {
                backend.draw_text_inner("...", status_x, y, 8, Color::rgb(180, 180, 80));
            }
        }
        // Genre.
        let genre_x = 430;
        let genre_display = if ch.genre.len() > 6 {
            let trunc: String = ch.genre.chars().take(5).collect();
            format!("{}", trunc)
        } else {
            ch.genre.clone()
        };
        backend.draw_text_inner(&genre_display, genre_x, y, 8, Color::rgb(140, 140, 140));
    }

    // Scrollbar.
    if channels.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (channels.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

/// Draw the TV Guide "now playing" / downloading view.
pub(crate) fn draw_tv_playing(
    backend: &mut PspBackend,
    now_playing: &str,
    downloading: bool,
    progress: f32,
    preview_tex: Option<TextureId>,
    error_msg: &str,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;

    if downloading {
        // Download progress view.
        draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

        let pct = (progress * 100.0) as u32;
        let status = format!("Downloading... {}%", pct);
        let status_x = cx - (status.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &status,
            status_x,
            CONTENT_TOP as i32 + 60,
            8,
            Color::rgb(255, 200, 80),
        );

        // Progress bar.
        let bar_w: u32 = 300;
        let bar_h: u32 = 8;
        let bar_x = cx - bar_w as i32 / 2;
        let bar_y = CONTENT_TOP as i32 + 80;
        backend.fill_rect_inner(bar_x, bar_y, bar_w, bar_h, Color::rgba(40, 40, 60, 200));
        let fill_w = (bar_w as f32 * progress) as u32;
        if fill_w > 0 {
            backend.fill_rect_inner(bar_x, bar_y, fill_w, bar_h, Color::rgb(0, 160, 255));
        }

        // Episode title.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_x = cx - (display.len() as i32 * 8) / 2;
        backend.draw_text_inner(&display, title_x, bar_y + 20, 8, Color::rgb(180, 180, 180));
    } else if let Some(tex) = preview_tex {
        // Video playing -- show the decoded frame.
        // Scale to fit within the content area while preserving aspect ratio.
        let max_w = SCREEN_WIDTH;
        let max_h = CONTENT_H;
        backend.blit_inner(tex, 0, CONTENT_TOP as i32, max_w, max_h);

        // LIVE indicator.
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 48,
            CONTENT_TOP as i32 + 4,
            44,
            12,
            Color::rgba(200, 0, 0, 200),
        );
        backend.draw_text_inner(
            "LIVE",
            SCREEN_WIDTH as i32 - 40,
            CONTENT_TOP as i32 + 6,
            8,
            Color::WHITE,
        );

        // Title overlay at bottom.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_y = BOTTOMBAR_Y - 14;
        backend.fill_rect_inner(0, title_y - 2, SCREEN_WIDTH, 12, Color::rgba(0, 0, 0, 160));
        backend.draw_text_inner(&display, 4, title_y, 8, Color::WHITE);
    } else {
        // No video frame yet but not downloading -- audio only or ended.
        draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

        let status = if !error_msg.is_empty() {
            error_msg
        } else {
            "Playing audio..."
        };
        let status_x = cx - (status.len() as i32 * 8) / 2;
        let status_clr = if error_msg.is_empty() {
            Color::rgb(120, 255, 120)
        } else {
            Color::rgb(255, 80, 80)
        };
        backend.draw_text_inner(status, status_x, CONTENT_TOP as i32 + 80, 8, status_clr);

        // Episode title.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_x = cx - (display.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &display,
            title_x,
            CONTENT_TOP as i32 + 100,
            8,
            Color::rgb(180, 180, 180),
        );
    }
}

/// Draw TV Guide error screen.
pub(crate) fn draw_tv_error(backend: &mut PspBackend, error_msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;

    backend.draw_text_inner("Error", cx - 2 * 8, cy - 12, 8, Color::rgb(255, 80, 80));

    let max_chars = 55;
    let display_msg = if error_msg.len() > max_chars {
        let trunc: String = error_msg.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        error_msg.to_string()
    };
    let msg_x = cx - (display_msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(&display_msg, msg_x, cy + 4, 8, Color::rgb(200, 200, 200));

    backend.draw_text_inner(
        "Press X to retry or O to go back",
        cx - 16 * 8,
        cy + 20,
        8,
        Color::rgb(140, 140, 140),
    );
}

//! Music Player PSP-style window rendering.
//!
//! The stock PSP Music app has three things on screen while a track
//! plays: a big "album-art" region on the left, the track title +
//! metadata on the right, and a transport row (▶ ⏸ ⏮ ⏭) along the
//! bottom with a progress bar. We approximate that layout with plain
//! `fill_rect` / `draw_text` calls — no album art in the VFS yet, so
//! the art region is a coloured tile with a big musical-note glyph.
//!
//! All colors come from [`MusicColors`] (skin + `[app_themes.music_player]`)
//! and all font sizes from the active theme. The progress bar reflects
//! the host-reported playback position ([`BrowsingApp::playback`]) and is
//! hidden while no position is known.

use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use crate::BrowsingApp;
use crate::palette::MusicColors;

pub fn draw(
    app: &BrowsingApp,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
) -> Result<()> {
    let colors = MusicColors::from_theme(at);
    let font_heading = at.font_heading;
    let font_body = at.font_body;
    let font_hint = at.font_hint;

    backend.fill_rect(cx, cy, cw, ch, colors.bg)?;
    backend.fill_rect(cx, cy, cw, 2, colors.header_rule)?;

    let pad = 12i32;
    let art_size = (ch.saturating_sub(80)).min(cw / 2).max(80);
    let art_x = cx + pad;
    let art_y = cy + pad;

    // Album-art placeholder: outer frame, accent ring, inner fill, ♫ glyph.
    backend.fill_rect(art_x, art_y, art_size, art_size, colors.art_frame)?;
    backend.fill_rect(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        colors.art_ring,
    )?;
    backend.fill_rect(
        art_x + 6,
        art_y + 6,
        art_size - 12,
        art_size - 12,
        colors.art_fill,
    )?;
    let note = "\u{266B}"; // ♫
    // The glyph scales with the tile (it is artwork, not text), floored
    // at the theme heading size.
    let note_size = ((art_size / 3).min(u16::MAX as u32) as u16).max(font_heading);
    let note_w = backend.measure_text(note, note_size) as i32;
    let note_h = backend.measure_text_height(note_size) as i32;
    let note_x = art_x + (art_size as i32 - note_w) / 2;
    let note_y = art_y + (art_size as i32 - note_h) / 2;
    backend.draw_text(note, note_x, note_y, note_size, colors.art_glyph)?;

    // Right column: title, filename, metadata.
    let info_x = art_x + art_size as i32 + 16;
    let info_w = (cx + cw as i32 - info_x - pad).max(0) as u32;
    let mut row_y = art_y;
    let line_gap = 4;

    let (title, duration, size_bytes) = app.track_info();
    let title = title.unwrap_or("Unknown Track");
    backend.draw_text_ellipsis(
        "Now Playing",
        info_x,
        row_y,
        font_hint,
        colors.label,
        info_w,
    )?;
    row_y += backend.measure_text_height(font_hint) as i32 + line_gap;
    backend.draw_text_ellipsis(title, info_x, row_y, font_heading, colors.title, info_w)?;
    row_y += backend.measure_text_height(font_heading) as i32 + line_gap;

    let file_path = app.content.viewing_file.as_deref().unwrap_or("");
    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
    let body_h = backend.measure_text_height(font_body) as i32 + line_gap;
    backend.draw_text_ellipsis(file_name, info_x, row_y, font_body, colors.meta, info_w)?;
    row_y += body_h + line_gap * 2;

    if let Some(d) = duration {
        let line = format!("Duration: {d}");
        backend.draw_text_ellipsis(&line, info_x, row_y, font_body, colors.meta, info_w)?;
        row_y += body_h;
    }
    if let Some(bytes) = size_bytes {
        let line = format!("Size: {} KB", bytes / 1024);
        backend.draw_text_ellipsis(&line, info_x, row_y, font_body, colors.meta, info_w)?;
        row_y += body_h;
    }
    if app.shuffle() {
        backend.draw_text_ellipsis(
            "Shuffle: ON",
            info_x,
            row_y,
            font_body,
            colors.shuffle_on,
            info_w,
        )?;
    }

    // Transport row along the bottom.
    let transport_h = 48u32;
    let transport_y = cy + ch as i32 - transport_h as i32 - pad;
    let inner_w = cw.saturating_sub(pad as u32 * 2);
    draw_transport(
        backend,
        &colors,
        at,
        cx + pad,
        transport_y,
        inner_w,
        transport_h,
    )?;

    // Progress bar just above transport — only when the host has
    // reported a real position for this track.
    if let Some((pos_ms, dur_ms)) = app.playback() {
        let bar_y = transport_y - 14;
        let readout = format!("{} / {}", format_ms(pos_ms), format_ms(dur_ms));
        let readout_w = backend.measure_text(&readout, font_hint);
        let bar_w = inner_w.saturating_sub(readout_w + 8);
        backend.fill_rect(cx + pad, bar_y, bar_w, 4, colors.progress_track)?;
        let fill_w = progress_width(pos_ms, dur_ms, bar_w);
        if fill_w > 0 {
            backend.fill_rect(cx + pad, bar_y, fill_w, 4, colors.progress_fill)?;
        }
        let text_h = backend.measure_text_height(font_hint) as i32;
        backend.draw_text(
            &readout,
            cx + pad + inner_w as i32 - readout_w as i32,
            bar_y + 2 - text_h / 2,
            font_hint,
            colors.progress_text,
        )?;
    }

    Ok(())
}

/// Width of the filled progress portion for `pos_ms` of `dur_ms` on a
/// `bar_w`-pixel bar (clamped to the bar).
pub(crate) fn progress_width(pos_ms: u64, dur_ms: u64, bar_w: u32) -> u32 {
    if dur_ms == 0 {
        return 0;
    }
    let frac = pos_ms.min(dur_ms) as f64 / dur_ms as f64;
    (bar_w as f64 * frac).round() as u32
}

/// `m:ss` (or `h:mm:ss`) readout for a millisecond count.
fn format_ms(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn draw_transport(
    backend: &mut dyn SdiBackend,
    colors: &MusicColors,
    at: &ActiveTheme,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> Result<()> {
    backend.fill_rect(x, y, w, h, colors.transport_bg)?;
    backend.fill_rect(x, y, w, 1, colors.transport_rule)?;

    // Five transport buttons: prev, rewind, play/pause, ff, next.
    let buttons = [
        "\u{25C0}\u{25C0}",
        "\u{25C0}",
        "\u{25B6}",
        "\u{25B6}",
        "\u{25B6}\u{25B6}",
    ];
    let n = buttons.len() as u32;
    let slot_w = w / n;
    for (i, label) in buttons.iter().enumerate() {
        let bx = x + (i as u32 * slot_w) as i32;
        // Play/Pause (center) gets highlighted.
        let is_primary = i == 2;
        let (bg, fg, text_size) = if is_primary {
            (
                colors.button_primary_bg,
                colors.button_primary_text,
                at.font_heading,
            )
        } else {
            (colors.button_bg, colors.button_text, at.font_body)
        };
        backend.fill_rect(bx + 4, y + 8, slot_w.saturating_sub(8), h - 16, bg)?;
        let tw = backend.measure_text(label, text_size) as i32;
        let th = backend.measure_text_height(text_size) as i32;
        let tx = bx + (slot_w as i32 - tw) / 2;
        let ty = y + (h as i32 - th) / 2;
        backend.draw_text(label, tx, ty, text_size, fg)?;
    }
    Ok(())
}

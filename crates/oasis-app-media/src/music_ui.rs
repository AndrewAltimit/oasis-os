//! Music Player PSP-style window rendering.
//!
//! The stock PSP Music app has three things on screen while a track
//! plays: a big "album-art" region on the left, the track title +
//! metadata on the right, and a transport row (▶ ⏸ ⏮ ⏭) along the
//! bottom with a progress bar. We approximate that layout with plain
//! `fill_rect` / `draw_text` calls — no album art in the VFS yet, so
//! the art region is a coloured tile with a big musical-note glyph.

use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use crate::BrowsingApp;

pub fn draw(
    app: &BrowsingApp,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    backend: &mut dyn SdiBackend,
    _at: &ActiveTheme,
) -> Result<()> {
    // Background: a soft gradient-ish dark blue band, stock PSP look.
    backend.fill_rect(cx, cy, cw, ch, Color::rgb(20, 24, 40))?;
    backend.fill_rect(cx, cy, cw, 2, Color::rgb(40, 60, 110))?;

    let pad = 12i32;
    let art_size = (ch.saturating_sub(80)).min(cw / 2).max(80);
    let art_x = cx + pad;
    let art_y = cy + pad;

    // Album-art placeholder: outer border, accent fill, big ♪ glyph.
    backend.fill_rect(art_x, art_y, art_size, art_size, Color::rgb(60, 70, 110))?;
    backend.fill_rect(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        Color::rgb(90, 120, 180),
    )?;
    backend.fill_rect(
        art_x + 6,
        art_y + 6,
        art_size - 12,
        art_size - 12,
        Color::rgb(50, 65, 105),
    )?;
    let note = "\u{266B}"; // ♫
    let note_size: u16 = ((art_size / 3).max(16).min(u16::MAX as u32)) as u16;
    let note_x = art_x + (art_size as i32 - note_size as i32) / 2;
    let note_y = art_y + (art_size as i32 - note_size as i32) / 2;
    backend.draw_text(note, note_x, note_y, note_size, Color::rgb(255, 255, 255))?;

    // Right column: title, album (= filename), duration.
    let info_x = art_x + art_size as i32 + 16;
    let info_w = (cx + cw as i32 - info_x - pad).max(0) as u32;
    let info_y = art_y;

    let (title, duration, size_bytes) = app.track_info();
    let title = title.unwrap_or("Unknown Track");
    backend.draw_text("Now Playing", info_x, info_y, 10, Color::rgb(160, 180, 220))?;
    let title_clamped = clamp_text(title, info_w, 14);
    backend.draw_text(
        &title_clamped,
        info_x,
        info_y + 16,
        18,
        Color::rgb(255, 255, 255),
    )?;

    // Filename (subtitle) line.
    let file_path = app.content.viewing_file.as_deref().unwrap_or("");
    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
    backend.draw_text(
        &clamp_text(file_name, info_w, 8),
        info_x,
        info_y + 40,
        12,
        Color::rgb(180, 190, 210),
    )?;

    // Metadata rows.
    let mut row_y = info_y + 64;
    if let Some(d) = duration {
        backend.draw_text(
            &format!("Duration: {d}"),
            info_x,
            row_y,
            11,
            Color::rgb(180, 190, 210),
        )?;
        row_y += 16;
    }
    if let Some(bytes) = size_bytes {
        backend.draw_text(
            &format!("Size: {} KB", bytes / 1024),
            info_x,
            row_y,
            11,
            Color::rgb(180, 190, 210),
        )?;
        row_y += 16;
    }
    if app.shuffle() {
        backend.draw_text("Shuffle: ON", info_x, row_y, 11, Color::rgb(130, 200, 150))?;
    }

    // Transport row along the bottom.
    let transport_h = 48u32;
    let transport_y = cy + ch as i32 - transport_h as i32 - pad;
    draw_transport(
        backend,
        cx + pad,
        transport_y,
        cw - pad as u32 * 2,
        transport_h,
    )?;

    // Progress bar just above transport.
    let bar_y = transport_y - 14;
    let bar_w = cw - pad as u32 * 2;
    backend.fill_rect(cx + pad, bar_y, bar_w, 4, Color::rgb(60, 70, 100))?;
    // Static progress indicator: we don't poll the audio backend from
    // the app (pure data, no backend handle), so show a subtle
    // "playing" animation tied to the slideshow_timer-style frame
    // counter if we had one. Keep the bar static for now at 10%.
    backend.fill_rect(cx + pad, bar_y, bar_w / 10, 4, Color::rgb(120, 170, 240))?;

    Ok(())
}

fn draw_transport(backend: &mut dyn SdiBackend, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
    backend.fill_rect(x, y, w, h, Color::rgb(28, 34, 54))?;
    backend.fill_rect(x, y, w, 1, Color::rgb(80, 100, 150))?;

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
        let bg = if is_primary {
            Color::rgb(90, 130, 220)
        } else {
            Color::rgb(40, 50, 80)
        };
        backend.fill_rect(bx + 4, y + 8, slot_w - 8, h - 16, bg)?;
        let text_size = if is_primary { 18 } else { 14 };
        let tx = bx + (slot_w as i32 / 2) - (text_size as i32 / 2);
        let ty = y + (h as i32 / 2) - (text_size as i32 / 2);
        backend.draw_text(label, tx, ty, text_size, Color::rgb(255, 255, 255))?;
    }
    Ok(())
}

fn clamp_text(s: &str, pixel_w: u32, approx_char_w: u32) -> String {
    let max_chars = (pixel_w / approx_char_w.max(1)) as usize;
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let stop = max_chars.saturating_sub(1);
    s.chars()
        .take(stop)
        .chain(std::iter::once('\u{2026}'))
        .collect()
}

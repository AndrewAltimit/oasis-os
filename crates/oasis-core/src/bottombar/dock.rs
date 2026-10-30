//! Decorative PSIX-style media dock (`bottombar_style = "media_dock"`).

use std::fmt::Write as _;

use super::classic::PAGE_DOT_NAMES;
use super::{BarGeometry, BottomBar, place_label, sync_url_plate, text_px, url_offset};
use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_border, ensure_pill, ensure_rounded_fill, hide_bezel};
use crate::sdi::helpers::{hide_indexed, hide_objects};
use crate::skin::SkinFeatures;

/// Media-dock transport pill names.
const DOCK_BTN_NAMES: [&str; 3] = ["bar_dock_prev", "bar_dock_play", "bar_dock_next"];
/// Media-dock progress/volume track + fill names.
const DOCK_TRACK_NAMES: [&str; 4] = [
    "bar_dock_progress_track",
    "bar_dock_progress_fill",
    "bar_dock_vol_track",
    "bar_dock_vol_fill",
];
/// Scanline-rect prefixes for the transport-glyph triangles. Each hosts up
/// to `DOCK_GLYPH_ROWS` 1px scanlines (`{prefix}0`, `{prefix}1`, …): prev is
/// two left triangles, play one right triangle, next two right triangles.
const DOCK_GLYPH_PREFIXES: [&str; 5] = [
    "bar_dock_prev_a_",
    "bar_dock_prev_b_",
    "bar_dock_play_g_",
    "bar_dock_next_a_",
    "bar_dock_next_b_",
];
/// Max scanlines reserved per transport-glyph triangle.
const DOCK_GLYPH_ROWS: usize = 9;

/// Hide every media-dock SDI object (transport pills, glyph scanlines, and
/// progress/volume tracks). A no-op when the objects don't exist, so it is
/// safe to call from the classic footer path.
pub(super) fn hide_dock(sdi: &mut SdiRegistry) {
    hide_objects(sdi, &DOCK_BTN_NAMES);
    hide_objects(sdi, &DOCK_TRACK_NAMES);
    for prefix in DOCK_GLYPH_PREFIXES {
        hide_indexed(sdi, prefix, DOCK_GLYPH_ROWS);
    }
}

/// Draw one transport-glyph triangle as a stack of 1px scanline rects.
///
/// `rows` scanlines form a triangle whose apex points right (`point_right`)
/// or left. The widest scanline sits at the vertical center. Scanlines
/// from `rows` up to `DOCK_GLYPH_ROWS` are hidden (a smaller bar after a
/// skin swap must not leave the old, taller glyph's tail behind).
fn dock_triangle(
    sdi: &mut SdiRegistry,
    prefix: &str,
    (gx, gy): (i32, i32),
    rows: usize,
    point_right: bool,
    color: Color,
) {
    let half = (rows as i32 - 1) / 2;
    let max_w = half + 1;
    // Runs every frame on dock skins: reuse one name buffer across rows.
    let mut name = String::with_capacity(prefix.len() + 2);
    for r in 0..DOCK_GLYPH_ROWS {
        name.clear();
        let _ = write!(name, "{prefix}{r}");
        if r >= rows {
            hide_objects(sdi, &[&name]);
            continue;
        }
        let dc = (r as i32 - half).abs();
        let w = (max_w - dc).max(1);
        let x = if point_right { gx } else { gx + (max_w - w) };
        ensure_border(sdi, &name, x, gy + r as i32, w as u32, 1, color);
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.z = 902; // above the pill fill (901)
        }
    }
}

impl BottomBar {
    /// Decorative PSIX-style media dock.
    ///
    /// Draws transport pills (prev / play / next) with primitive triangle
    /// glyphs, a progress track + fill, a volume track + fill, and keeps the
    /// right-aligned media tab strip + USB indicator + clock. Progress and
    /// volume use static placeholder levels — the dock carries no audio
    /// binding.
    pub(super) fn update_sdi_dock(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &SkinFeatures,
        geo: &BarGeometry,
    ) {
        // The dock omits the page dots (the URL plate is kept: PSIX's
        // footer leads with its site plate at the left edge).
        hide_objects(sdi, &PAGE_DOT_NAMES);

        // Right side: clock (optional), then media tabs, then USB.
        let right_edge = self.sync_clock(sdi, at, features, geo);

        // Media tab strip is forced on in dock mode (regardless of
        // `show_media_tabs`) so the dock always carries the AUDIO/VIDEO/
        // IMAGE/FILE selector. The classic chrome bezel is hidden: dock
        // skins bake their own tab-shelf chrome into the bar texture.
        let tabs_x = self.draw_media_tabs(sdi, at, geo, right_edge);
        hide_bezel(sdi, "bar_tab_bezel");

        // USB indicator to the left of the tab strip.
        let usb_x = tabs_x - 8 - text_px("USB", at.font_small);
        place_label(
            sdi,
            at,
            "bar_usb",
            (usb_x, geo.text_y),
            "USB",
            at.bar.usb_color,
        );

        // Left side: URL plate (PSIX site plate) followed by transport
        // pills. The plate reuses the classic footer's objects so switching
        // bottombar styles never leaks a stale copy.
        let url_offset = url_offset(at, features);
        let left = match sync_url_plate(sdi, at, url_offset, geo) {
            Some(end) => end + 6,
            None => url_offset + 8,
        };
        let pills_end = draw_transport(sdi, at, geo, left + 4);
        draw_tracks(sdi, at, geo, pills_end + 6, usb_x - 12);
    }
}

/// Compact transport pills (~1/3 bar height) with their triangle glyphs,
/// starting at `x`: PSIX's transports are small chrome squares, not
/// full-height buttons. Returns the x just past the last pill's gap.
fn draw_transport(sdi: &mut SdiRegistry, at: &ActiveTheme, geo: &BarGeometry, x: i32) -> i32 {
    let pill_h = (geo.h as i32 / 3).max(10);
    let pill_w = pill_h + 6;
    let gap = 4;
    let by = geo.y + (geo.h as i32 - pill_h) / 2;
    // Glyph triangle geometry (odd row count for a symmetric apex).
    let rows = ((pill_h / 2) | 1).clamp(3, DOCK_GLYPH_ROWS as i32) as usize;
    let max_w = (rows as i32 - 1) / 2 + 1;
    let gy = by + (pill_h - rows as i32) / 2;
    let glyph = at.bar.dock_button_glyph;
    // Two triangles side by side are this wide (1px gap between them).
    let pair_w = 2 * max_w + 1;

    let mut bx = x;
    for (i, name) in DOCK_BTN_NAMES.iter().enumerate() {
        ensure_pill(
            sdi,
            name,
            bx,
            by,
            pill_w as u32,
            pill_h as u32,
            at.bar.dock_button_fill,
            glyph,
        );
        let pair_x = bx + (pill_w - pair_w) / 2;
        match i {
            // prev: two left-pointing triangles.
            0 => {
                dock_triangle(
                    sdi,
                    DOCK_GLYPH_PREFIXES[0],
                    (pair_x, gy),
                    rows,
                    false,
                    glyph,
                );
                let second = (pair_x + max_w + 1, gy);
                dock_triangle(sdi, DOCK_GLYPH_PREFIXES[1], second, rows, false, glyph);
            },
            // play: one right-pointing triangle.
            1 => {
                let gx = bx + (pill_w - max_w) / 2;
                dock_triangle(sdi, DOCK_GLYPH_PREFIXES[2], (gx, gy), rows, true, glyph);
            },
            // next: two right-pointing triangles.
            _ => {
                dock_triangle(sdi, DOCK_GLYPH_PREFIXES[3], (pair_x, gy), rows, true, glyph);
                let second = (pair_x + max_w + 1, gy);
                dock_triangle(sdi, DOCK_GLYPH_PREFIXES[4], second, rows, true, glyph);
            },
        }
        bx += pill_w + gap;
    }
    bx
}

/// Progress + volume tracks filling `track_x..region_end` (between the
/// transport pills and the USB indicator). Static placeholder levels.
fn draw_tracks(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    geo: &BarGeometry,
    track_x: i32,
    region_end: i32,
) {
    let track_h = 4u32;
    let track_y = geo.y + (geo.h as i32 - track_h as i32) / 2;
    let avail = (region_end - track_x).max(24);
    let prog_w = (avail * 3 / 5).max(16) as u32;
    let vol_x = track_x + prog_w as i32 + 10;
    let vol_w = (region_end - vol_x).clamp(12, avail) as u32;

    let tracks = [
        (
            "bar_dock_progress_track",
            track_x,
            prog_w,
            at.bar.dock_progress_track,
        ),
        // 40% placeholder progress.
        (
            "bar_dock_progress_fill",
            track_x,
            (prog_w * 2 / 5).max(1),
            at.bar.dock_progress_fill,
        ),
        ("bar_dock_vol_track", vol_x, vol_w, at.bar.dock_vol_track),
        // 70% placeholder volume.
        (
            "bar_dock_vol_fill",
            vol_x,
            (vol_w * 7 / 10).max(1),
            at.bar.dock_vol_fill,
        ),
    ];
    for (name, x, w, color) in tracks {
        ensure_rounded_fill(sdi, name, x, track_y, w, track_h, color, 2);
    }
    // Draw fills above their tracks.
    for name in ["bar_dock_progress_fill", "bar_dock_vol_fill"] {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.z = 902;
        }
    }
}

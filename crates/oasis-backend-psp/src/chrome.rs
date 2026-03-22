//! Dashboard + shell chrome rendering (status bar, bottom bar, icons).
#![allow(dead_code)]

use oasis_backend_psp::{AudioHandle, Color, PspBackend, SCREEN_WIDTH, WindowManager};

use crate::theme::*;
use crate::types::{APPS, AppEntry};

use oasis_backend_psp::StatusBarInfo;
use oasis_backend_psp::SystemInfo;

// ---------------------------------------------------------------------------
// Dashboard rendering
// ---------------------------------------------------------------------------

pub(crate) fn draw_dashboard(
    backend: &mut PspBackend,
    selected: usize,
    page: usize,
    viz_frame: u32,
) {
    let page_start = page * ICONS_PER_PAGE;
    let page_end = (page_start + ICONS_PER_PAGE).min(APPS.len());
    let page_count = page_end - page_start;

    for i in 0..page_count {
        let app = &APPS[page_start + i];
        let col = (i % GRID_COLS) as i32;
        let row = (i / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;

        draw_icon(backend, app, ix, iy);

        // Label below icon with drop shadow.
        let label_y = iy + ICON_H as i32 + ICON_LABEL_PAD;
        let text_width = (app.title.len() as i32) * CHAR_W;
        let label_x = cell_x + (CELL_W - text_width) / 2;
        backend.draw_text_inner(app.title, label_x + 1, label_y + 1, 8, LABEL_SHADOW);
        backend.draw_text_inner(app.title, label_x, label_y, 8, LABEL_CLR);
    }

    // Icon selection is now cursor-based (no grid selector box).
}

/// Draw a PSIX document-style icon with 6 layers:
/// shadow, outline, body, stripe, fold, app graphic.
fn draw_icon(backend: &mut PspBackend, app: &AppEntry, ix: i32, iy: i32) {
    backend.fill_rect_inner(ix + 2, iy + 3, ICON_W + 2, ICON_H + 1, SHADOW_CLR);
    backend.fill_rect_inner(ix - 1, iy - 1, ICON_W + 2, ICON_H + 2, OUTLINE_CLR);
    backend.fill_rect_inner(ix, iy, ICON_W, ICON_H, BODY_CLR);
    backend.fill_rect_inner(ix, iy, ICON_W - ICON_FOLD_SIZE, ICON_STRIPE_H, app.color);
    backend.fill_rect_inner(
        ix + ICON_W as i32 - ICON_FOLD_SIZE as i32,
        iy,
        ICON_FOLD_SIZE,
        ICON_FOLD_SIZE,
        FOLD_CLR,
    );

    let gfx_w = ICON_W - 2 * ICON_GFX_PAD;
    let gx = ix + ICON_GFX_PAD as i32;
    let gy = iy + ICON_STRIPE_H as i32 + 3;
    let c = app.color;
    let gfx_color = Color::rgba(
        c.r.saturating_add(30),
        c.g.saturating_add(10),
        c.b.saturating_add(30),
        200,
    );
    backend.fill_rect_inner(gx, gy, gfx_w, ICON_GFX_H, gfx_color);

    // Per-app mini-graphic symbol.
    draw_icon_graphic(backend, app.id, gx, gy, gfx_w, ICON_GFX_H);
}

/// Draw a recognizable per-app symbol inside the icon graphic area.
fn draw_icon_graphic(backend: &mut PspBackend, app_id: &str, gx: i32, gy: i32, gw: u32, gh: u32) {
    let s = ICON_SYM_CLR;
    let cx = gx + gw as i32 / 2;
    let cy = gy + gh as i32 / 2;

    match app_id {
        "filemgr" => {
            // Folder: body rect + tab on top-left.
            backend.fill_rect_inner(cx - 6, cy - 2, 12, 6, s);
            backend.fill_rect_inner(cx - 6, cy - 4, 5, 2, s);
        },
        "settings" => {
            // Gear: cross pattern.
            backend.fill_rect_inner(cx - 4, cy - 1, 8, 2, s);
            backend.fill_rect_inner(cx - 1, cy - 4, 2, 8, s);
            backend.fill_rect_inner(cx - 3, cy - 3, 2, 2, s);
            backend.fill_rect_inner(cx + 1, cy - 3, 2, 2, s);
            backend.fill_rect_inner(cx - 3, cy + 1, 2, 2, s);
        },
        "network" => {
            // WiFi arcs: 3 horizontal bars widening bottom-up.
            backend.fill_rect_inner(cx - 1, cy + 1, 3, 2, s);
            backend.fill_rect_inner(cx - 4, cy - 1, 9, 2, s);
            backend.fill_rect_inner(cx - 6, cy - 3, 13, 2, s);
        },
        "terminal" => {
            // >_ prompt text.
            backend.draw_text_inner(">_", cx - 8, cy - 3, 8, s);
        },
        "music" => {
            // Music note: stem + filled head.
            backend.fill_rect_inner(cx + 1, cy - 4, 2, 8, s);
            backend.fill_rect_inner(cx - 2, cy + 1, 4, 3, s);
        },
        "photos" => {
            // Mountain/landscape: stepped pyramid.
            backend.fill_rect_inner(cx - 6, cy + 1, 13, 2, s);
            backend.fill_rect_inner(cx - 4, cy - 1, 9, 2, s);
            backend.fill_rect_inner(cx - 1, cy - 3, 3, 2, s);
        },
        "packages" => {
            // Box/crate: outlined rect + cross divider.
            backend.fill_rect_inner(cx - 5, cy - 4, 11, 1, s);
            backend.fill_rect_inner(cx - 5, cy + 3, 11, 1, s);
            backend.fill_rect_inner(cx - 5, cy - 4, 1, 8, s);
            backend.fill_rect_inner(cx + 5, cy - 4, 1, 8, s);
            backend.fill_rect_inner(cx, cy - 4, 1, 8, s);
        },
        "sysmon" => {
            // Bar chart: 3 vertical bars at different heights.
            backend.fill_rect_inner(cx - 5, cy, 3, 4, s);
            backend.fill_rect_inner(cx - 1, cy - 2, 3, 6, s);
            backend.fill_rect_inner(cx + 3, cy - 4, 3, 8, s);
        },
        "browser" => {
            // Globe: circle outline approximation.
            backend.fill_rect_inner(cx - 5, cy - 1, 11, 2, s);
            backend.fill_rect_inner(cx - 1, cy - 5, 2, 10, s);
            backend.fill_rect_inner(cx - 4, cy - 4, 1, 8, s);
            backend.fill_rect_inner(cx + 4, cy - 4, 1, 8, s);
            backend.fill_rect_inner(cx - 3, cy - 5, 7, 1, s);
            backend.fill_rect_inner(cx - 3, cy + 4, 7, 1, s);
        },
        "radio" => {
            // Radio waves: antenna dot + arcs.
            backend.fill_rect_inner(cx - 1, cy + 1, 3, 2, s);
            backend.fill_rect_inner(cx - 3, cy - 1, 2, 3, s);
            backend.fill_rect_inner(cx + 2, cy - 1, 2, 3, s);
            backend.fill_rect_inner(cx - 5, cy - 3, 2, 5, s);
            backend.fill_rect_inner(cx + 4, cy - 3, 2, 5, s);
        },
        _ => {},
    }
}

// ---------------------------------------------------------------------------
// Status bar rendering
// ---------------------------------------------------------------------------

pub(crate) fn draw_status_bar(
    backend: &mut PspBackend,
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
) {
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, STATUSBAR_H, STATUSBAR_BG);
    // Gradient simulation: highlight strips at top.
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 20));
    backend.fill_rect_inner(0, 1, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 10));
    backend.fill_rect_inner(0, STATUSBAR_H as i32 - 1, SCREEN_WIDTH, 1, SEPARATOR);

    // -- Left side: battery percentage + charging bolt + WiFi + CPU MHz --

    // Battery percentage (color-coded).
    let bat_label = if status.battery_percent >= 0 {
        format!("{}%", status.battery_percent)
    } else if status.ac_power {
        String::from("AC")
    } else {
        String::from("---")
    };
    let bat_color = if status.battery_charging || status.ac_power {
        BATTERY_CLR
    } else if status.battery_percent < 20 {
        Color::rgb(255, 80, 80)
    } else {
        BATTERY_CLR
    };
    backend.draw_text_inner(&bat_label, 6, 5, 8, bat_color);
    let bat_w = bat_label.len() as i32 * CHAR_W;

    // Charging bolt indicator (Z shape) when battery is charging.
    let mut next_x = 6 + bat_w + 4;
    if status.battery_charging {
        let bolt_clr = Color::rgb(255, 220, 60);
        backend.fill_rect_inner(next_x + 1, 5, 3, 2, bolt_clr);
        backend.fill_rect_inner(next_x, 7, 3, 2, bolt_clr);
        backend.fill_rect_inner(next_x - 1, 9, 3, 2, bolt_clr);
        next_x += 7;
    }

    // WiFi indicator square.
    let wifi_x = next_x;
    if status.wifi_on {
        backend.fill_rect_inner(wifi_x, 7, 5, 5, Color::rgb(100, 200, 255));
    } else {
        let off = Color::rgb(100, 100, 100);
        backend.fill_rect_inner(wifi_x, 7, 5, 1, off);
        backend.fill_rect_inner(wifi_x, 11, 5, 1, off);
        backend.fill_rect_inner(wifi_x, 7, 1, 5, off);
        backend.fill_rect_inner(wifi_x + 4, 7, 1, 5, off);
    }

    // CPU MHz with filled-square indicator.
    let mhz_x = wifi_x + 8;
    backend.fill_rect_inner(mhz_x, 7, 5, 5, Color::WHITE);
    let mhz_label = format!("{} MHZ", sysinfo.cpu_mhz);
    backend.draw_text_inner(&mhz_label, mhz_x + 8, 5, 8, Color::WHITE);

    // -- Right side: time + day-of-week + full date --
    let date_label = format!(
        "{:02}:{:02} {} {} {}, {}",
        status.hour,
        status.minute,
        status.day_of_week,
        status.month_name(),
        status.day,
        status.year,
    );
    let date_w = date_label.len() as i32 * CHAR_W;
    let date_x = SCREEN_WIDTH as i32 - date_w - 6;
    backend.draw_text_inner(&date_label, date_x, 5, 8, Color::WHITE);
}

// ---------------------------------------------------------------------------
// Bottom bar rendering
// ---------------------------------------------------------------------------

pub(crate) fn draw_bottom_bar(
    backend: &mut PspBackend,
    audio: &AudioHandle,
    viz_frame: u32,
    status: &StatusBarInfo,
    url_text: &str,
    desktop_wm: Option<&WindowManager>,
) {
    // Full 32px bottom bar background with gradient simulation.
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, BOTTOMBAR_H, BAR_BG);
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, 1, SEPARATOR);
    backend.fill_rect_inner(
        0,
        BOTTOMBAR_Y + 1,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 15),
    );

    // -- Upper row (y=BOTTOM_UPPER_Y, 16px): URL bezel | Visualizer --

    // URL chrome bezel (left, 140px).
    let url_bx = 2i32;
    let url_bw = 140u32;
    let ubz_y = BOTTOM_UPPER_Y + 1;
    let ubz_h = BOTTOM_UPPER_H - 2;
    draw_chrome_bezel(backend, url_bx, ubz_y, url_bw, ubz_h);
    // Truncate URL text to fit bezel (max 16 chars).
    let max_url = 16;
    let display_url = if url_text.len() > max_url {
        &url_text[..url_text.floor_char_boundary(max_url)]
    } else {
        url_text
    };
    backend.draw_text_inner(display_url, 6, BOTTOM_UPPER_Y + 4, 8, URL_CLR);

    // Visualizer (center of upper row).
    draw_visualizer(backend, audio, viz_frame);

    // -- Lower row (y=BOTTOM_LOWER_Y, 16px) --
    backend.fill_rect_inner(
        0,
        BOTTOM_LOWER_Y,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 20),
    );

    if let Some(wm) = desktop_wm {
        // Desktop mode: show window tab buttons in lower row.
        draw_desktop_taskbar_row(backend, wm);
    } else {
        // Classic mode: transport | USB | battery bar.
        backend.draw_text_inner("<L", 4, BOTTOM_LOWER_Y + 4, 8, L_HINT_CLR);
        draw_transport_controls(backend, audio);
        backend.draw_text_inner("USB", 250, BOTTOM_LOWER_Y + 4, 8, USB_CLR);
        draw_battery_bar(backend, status);
        backend.draw_text_inner(
            "R>",
            SCREEN_WIDTH as i32 - R_HINT_W,
            BOTTOM_LOWER_Y + 4,
            8,
            R_HINT_CLR,
        );
    }
}

/// Draw animated music visualizer bars in center of upper bottom row.
fn draw_visualizer(backend: &mut PspBackend, audio: &AudioHandle, viz_frame: u32) {
    let total_viz_w = VIZ_BAR_COUNT * (VIZ_BAR_W + VIZ_BAR_GAP) - VIZ_BAR_GAP;
    let viz_x = (SCREEN_WIDTH as i32 - total_viz_w) / 2;
    let viz_base_y = BOTTOM_UPPER_Y + BOTTOM_UPPER_H as i32 - 2;
    let playing = audio.is_playing() && !audio.is_paused();

    for i in 0..VIZ_BAR_COUNT {
        let bar_h = if playing {
            // Composite waveform: two sine waves per bar.
            let t = viz_frame as f32 * 0.12;
            let freq1 = 0.7 + (i as f32) * 0.3;
            let freq2 = 1.4 + (i as f32) * 0.15;
            let phase = (i as f32) * 1.1;
            let val =
                libm::sinf(t * freq1 + phase) * 0.6 + libm::sinf(t * freq2 + phase * 0.7) * 0.4;
            let norm = (val + 1.0) * 0.5;
            VIZ_BAR_MIN_H + ((VIZ_BAR_MAX_H - VIZ_BAR_MIN_H) as f32 * norm) as i32
        } else {
            VIZ_BAR_MIN_H
        };
        let bx = viz_x + i * (VIZ_BAR_W + VIZ_BAR_GAP);
        let by = viz_base_y - bar_h;
        // Per-bar color tint for visual interest.
        let r = (120 + ((i * 4) as u8).min(40)) as u8;
        let b = (160 + ((i * 3) as u8).min(30)) as u8;
        let bar_clr = Color::rgba(r, 60, b, 200);
        backend.fill_rect_inner(bx, by, VIZ_BAR_W as u32, bar_h as u32, bar_clr);
        // Peak highlight (top 1px).
        if bar_h > 1 {
            backend.fill_rect_inner(bx, by, VIZ_BAR_W as u32, 1, VIZ_BAR_PEAK);
        }
    }
}

/// Draw transport controls in the lower bottom row.
fn draw_transport_controls(backend: &mut PspBackend, audio: &AudioHandle) {
    let y = BOTTOM_LOWER_Y + 4;
    let mut tx = 36i32;
    let playing = audio.is_playing();
    let paused = audio.is_paused();

    // Rewind.
    backend.draw_text_inner("<<", tx, y, 8, TRANSPORT_CLR);
    tx += 20;

    // Pause (two 2x8 bars, highlighted green when paused).
    let pause_clr = if playing && paused {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.fill_rect_inner(tx, y, 2, 8, pause_clr);
    backend.fill_rect_inner(tx + 4, y, 2, 8, pause_clr);
    tx += 12;

    // Play arrow (highlighted green when playing and not paused).
    let play_clr = if playing && !paused {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.draw_text_inner(">", tx, y, 8, play_clr);
    tx += 14;

    // Forward.
    backend.draw_text_inner(">>", tx, y, 8, TRANSPORT_CLR);
    tx += 20;

    // Stop (6x6 filled square, highlighted green when stopped).
    let stop_clr = if !playing {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.fill_rect_inner(tx, y + 1, 6, 6, stop_clr);
}

/// Draw horizontal battery bar in the lower bottom row.
fn draw_battery_bar(backend: &mut PspBackend, status: &StatusBarInfo) {
    let bar_x = 310i32;
    let bar_y = BOTTOM_LOWER_Y + 4;
    let bar_w = 60u32;
    let bar_h = 8u32;

    // Outline.
    backend.fill_rect_inner(bar_x, bar_y, bar_w, 1, Color::rgba(200, 200, 200, 140));
    backend.fill_rect_inner(
        bar_x,
        bar_y + bar_h as i32 - 1,
        bar_w,
        1,
        Color::rgba(200, 200, 200, 140),
    );
    backend.fill_rect_inner(bar_x, bar_y, 1, bar_h, Color::rgba(200, 200, 200, 140));
    backend.fill_rect_inner(
        bar_x + bar_w as i32 - 1,
        bar_y,
        1,
        bar_h,
        Color::rgba(200, 200, 200, 140),
    );

    // Dark bg fill.
    backend.fill_rect_inner(
        bar_x + 1,
        bar_y + 1,
        bar_w - 2,
        bar_h - 2,
        Color::rgba(20, 20, 20, 180),
    );

    // Battery nub on right side.
    backend.fill_rect_inner(
        bar_x + bar_w as i32,
        bar_y + 2,
        2,
        4,
        Color::rgba(200, 200, 200, 140),
    );

    // Colored fill proportional to battery_percent.
    let pct = if status.battery_percent >= 0 {
        status.battery_percent.min(100) as u32
    } else {
        0
    };
    let fill_w = ((bar_w - 2) * pct) / 100;
    if fill_w > 0 {
        let fill_clr = if pct >= 50 {
            Color::rgb(120, 255, 120)
        } else if pct >= 20 {
            Color::rgb(255, 200, 80)
        } else {
            Color::rgb(255, 80, 80)
        };
        backend.fill_rect_inner(bar_x + 1, bar_y + 1, fill_w, bar_h - 2, fill_clr);
    }
}

/// Draw a chrome/metallic bezel (fill + 4 corner-trimmed edges).
pub(crate) fn draw_chrome_bezel(backend: &mut PspBackend, x: i32, y: i32, w: u32, h: u32) {
    backend.fill_rect_inner(x, y, w, h, BEZEL_FILL);
    // Top/bottom edges trimmed 1px each side for pseudo-rounded corners.
    backend.fill_rect_inner(x + 1, y, w - 2, 1, BEZEL_TOP);
    backend.fill_rect_inner(x + 1, y + h as i32 - 1, w - 2, 1, BEZEL_BOTTOM);
    // Left/right edges trimmed 1px each end.
    backend.fill_rect_inner(x, y + 1, 1, h - 2, BEZEL_LEFT);
    backend.fill_rect_inner(x + w as i32 - 1, y + 1, 1, h - 2, BEZEL_RIGHT);
}

// ---------------------------------------------------------------------------
// Shared UI helpers (button hints, view headers)
// ---------------------------------------------------------------------------

/// Draw contextual button hints at the bottom of the content area.
pub(crate) fn draw_button_hints(backend: &mut PspBackend, hints: &[(&str, &str)]) {
    let y = BOTTOMBAR_Y - HINT_Y_OFFSET;
    backend.fill_rect_inner(0, y, SCREEN_WIDTH, HINT_Y_OFFSET as u32, HINT_BG);
    let mut x = 6i32;
    for (btn, label) in hints {
        backend.draw_text_inner(btn, x, y + 1, 8, HINT_BTN_CLR);
        x += btn.len() as i32 * 8 + 2;
        backend.draw_text_inner(label, x, y + 1, 8, HINT_TEXT_CLR);
        x += label.len() as i32 * 8 + 10;
    }
}

/// Draw a consistent view header with colored title and optional path.
pub(crate) fn draw_view_header(
    backend: &mut PspBackend,
    title: &str,
    title_clr: Color,
    path: Option<&str>,
) {
    backend.draw_text_inner(title, 4, CONTENT_TOP as i32 + 3, 8, title_clr);
    if let Some(p) = path {
        let path_x = 4 + title.len() as i32 * 8 + 8;
        backend.draw_text_inner(
            p,
            path_x,
            CONTENT_TOP as i32 + 3,
            8,
            Color::rgb(160, 160, 160),
        );
    }
    backend.fill_rect_inner(
        0,
        FM_START_Y - 2,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 40),
    );
}

/// Draw desktop window tabs in the bottom bar lower row.
fn draw_desktop_taskbar_row(backend: &mut PspBackend, wm: &WindowManager) {
    let y = BOTTOM_LOWER_Y + 2;

    // L hint.
    backend.draw_text_inner("<L", 4, BOTTOM_LOWER_Y + 4, 8, L_HINT_CLR);

    let active_id = wm.active_window();
    let mut tx = 24i32;

    for app in APPS {
        if wm.get_window(app.id).is_some() {
            let is_active = active_id == Some(app.id);
            let label_clr = if is_active {
                Color::WHITE
            } else {
                Color::rgb(160, 160, 160)
            };
            if is_active {
                let label_w = (app.title.len() as i32 * 8 + 8) as u32;
                backend.fill_rect_inner(tx - 2, y, label_w, 12, Color::rgba(60, 90, 160, 140));
            }
            backend.draw_text_inner(app.title, tx + 2, y + 1, 8, label_clr);
            tx += app.title.len() as i32 * 8 + 12;
        }
    }

    // R hint.
    backend.draw_text_inner(
        "R>",
        SCREEN_WIDTH as i32 - R_HINT_W,
        BOTTOM_LOWER_Y + 4,
        8,
        R_HINT_CLR,
    );
}

//! Desktop mode helpers and windowed content renderers.

use oasis_backend_psp::{
    AudioHandle, Color, FileEntry, PspBackend, SCREEN_WIDTH, SdiBackend, SdiRegistry,
    StatusBarInfo, SystemInfo, TextureId, WindowConfig, WindowManager, WindowType, WmEvent,
};

use crate::theme::*;
use crate::types::APPS;

/// Check if coordinates are over a dashboard icon, returning the global index.
pub(crate) fn hit_test_dashboard_icon(x: i32, y: i32, page: usize) -> Option<usize> {
    let page_start = page * ICONS_PER_PAGE;
    let page_end = (page_start + ICONS_PER_PAGE).min(APPS.len());
    for i in 0..(page_end - page_start) {
        let col = (i % GRID_COLS) as i32;
        let row = (i / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;
        if x >= ix
            && x < ix + ICON_W as i32
            && y >= iy
            && y < iy + ICON_H as i32 + ICON_LABEL_PAD + 10
        {
            return Some(page_start + i);
        }
    }
    None
}

/// Open an app as a floating window (or focus if already open).
///
/// When `kiosk` is true the window is created and immediately enters
/// fullscreen kiosk mode (fills screen, no decorations).
pub(crate) fn open_app_window(
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    app_id: &str,
    title: &str,
    kiosk: bool,
) {
    if wm.get_window(app_id).is_some() {
        let _ = wm.focus_window(app_id, sdi);
        if kiosk {
            let _ = wm.enter_fullscreen(app_id, sdi);
            hide_kiosk_content_bg(sdi, app_id);
        }
        return;
    }
    let config = WindowConfig {
        id: app_id.to_string(),
        title: title.to_string(),
        x: None,
        y: Some(STATUSBAR_H as i32 + 2),
        width: 300,
        height: 180,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    let _ = wm.create_window(&config, sdi);
    if kiosk {
        let _ = wm.enter_fullscreen(app_id, sdi);
        hide_kiosk_content_bg(sdi, app_id);
    }
}

/// Hide the WM content background SDI object for a kiosk window.
///
/// In kiosk mode the PSP renders app content directly via `render_classic`,
/// so the WM's dark content background would cause an unwanted tint.
fn hide_kiosk_content_bg(sdi: &mut SdiRegistry, app_id: &str) {
    let mut buf = [0u8; 64];
    let id_bytes = app_id.as_bytes();
    let suffix = b".content";
    let total = id_bytes.len() + suffix.len();
    if total <= buf.len() {
        buf[..id_bytes.len()].copy_from_slice(id_bytes);
        buf[id_bytes.len()..total].copy_from_slice(suffix);
        // SAFETY: both parts are valid UTF-8.
        let name = unsafe { core::str::from_utf8_unchecked(&buf[..total]) };
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

/// Handle WM events (window closed, desktop click opens apps, etc.).
pub(crate) fn handle_wm_event(
    event: &WmEvent,
    term_lines: &mut Vec<String>,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    page: usize,
) {
    match event {
        WmEvent::WindowClosed(id) => {
            term_lines.push(format!("[WM] Window closed: {}", id));
        },
        WmEvent::ContentClick(id, lx, ly) => {
            term_lines.push(format!("[WM] Click in {}: ({}, {})", id, lx, ly));
        },
        WmEvent::DesktopClick(x, y) => {
            if let Some(idx) = hit_test_dashboard_icon(*x, *y, page) {
                if idx < APPS.len() {
                    open_app_window(wm, sdi, APPS[idx].id, APPS[idx].title, false);
                }
            }
        },
        _ => {},
    }
}

// ---------------------------------------------------------------------------
// Windowed content renderers (for draw_with_clips callback)
// ---------------------------------------------------------------------------

pub(crate) fn draw_terminal_windowed(
    lines: &[String],
    input: &str,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    let bg = Color::rgba(0, 0, 0, 200);
    be.fill_rect(cx, cy, cw, ch, bg)?;

    let max_lines = (ch as usize) / 9;
    let visible_start = if lines.len() > max_lines {
        lines.len() - max_lines
    } else {
        0
    };
    for (i, line) in lines[visible_start..].iter().enumerate() {
        let y = cy + 2 + i as i32 * 9;
        if y > cy + ch as i32 - 14 {
            break;
        }
        be.draw_text(line, cx + 2, y, 8, Color::rgb(0, 255, 0))?;
    }

    let prompt = format!("> {}_", input);
    be.draw_text(
        &prompt,
        cx + 2,
        cy + ch as i32 - 12,
        8,
        Color::rgb(0, 255, 0),
    )?;
    Ok(())
}

pub(crate) fn draw_filemgr_windowed(
    path_l: &str,
    entries_l: &[FileEntry],
    selected_l: usize,
    scroll_l: usize,
    path_r: &str,
    entries_r: &[FileEntry],
    selected_r: usize,
    scroll_r: usize,
    active_panel: usize,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 200))?;

    let half_w = cw / 2;
    let div_x = cx + half_w as i32;

    // Panel path headers.
    let l_clr = if active_panel == 0 {
        Color::rgb(100, 200, 255)
    } else {
        Color::rgb(140, 140, 140)
    };
    let r_clr = if active_panel == 1 {
        Color::rgb(100, 200, 255)
    } else {
        Color::rgb(140, 140, 140)
    };
    be.draw_text(path_l, cx + 2, cy + 2, 8, l_clr)?;
    be.draw_text(path_r, div_x + 2, cy + 2, 8, r_clr)?;

    // Vertical divider.
    be.fill_rect(div_x, cy + 12, 1, ch - 12, Color::rgba(100, 200, 255, 80))?;

    // Draw each panel.
    let panels: [(&[FileEntry], usize, usize, i32, u32, bool); 2] = [
        (
            entries_l,
            selected_l,
            scroll_l,
            cx,
            half_w - 1,
            active_panel == 0,
        ),
        (
            entries_r,
            selected_r,
            scroll_r,
            div_x + 1,
            cw - half_w,
            active_panel == 1,
        ),
    ];
    let max_rows = ((ch as i32 - 14) / FM_ROW_H) as usize;

    for &(entries, selected, scroll, px, _pw, is_active) in &panels {
        let end = (scroll + max_rows).min(entries.len());
        for i in scroll..end {
            let entry = &entries[i];
            let row = (i - scroll) as i32;
            let y = cy + 14 + row * FM_ROW_H;
            if i == selected && is_active {
                be.fill_rect(
                    px,
                    y - 1,
                    half_w,
                    FM_ROW_H as u32,
                    Color::rgba(80, 120, 200, 100),
                )?;
            }
            let (prefix, clr) = if entry.is_dir {
                ("[D]", Color::rgb(255, 220, 80))
            } else {
                ("[F]", Color::rgb(180, 180, 180))
            };
            be.draw_text(prefix, px + 2, y, 8, clr)?;
            let name_clr = if entry.is_dir {
                Color::rgb(120, 220, 255)
            } else {
                Color::WHITE
            };
            be.draw_text(&entry.name, px + 28, y, 8, name_clr)?;
        }
    }
    Ok(())
}

pub(crate) fn draw_photos_windowed(
    tex: Option<TextureId>,
    img_w: u32,
    img_h: u32,
    viewing: bool,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::BLACK)?;
    if viewing {
        if let Some(t) = tex {
            if img_w == 0 || img_h == 0 {
                return Ok(());
            }
            let scale_w = cw as f32 / img_w as f32;
            let scale_h = ch as f32 / img_h as f32;
            let scale = if scale_w < scale_h { scale_w } else { scale_h };
            let dw = (img_w as f32 * scale) as u32;
            let dh = (img_h as f32 * scale) as u32;
            let dx = cx + ((cw - dw) / 2) as i32;
            let dy = cy + ((ch - dh) / 2) as i32;
            be.blit(t, dx, dy, dw, dh)?;
        }
    } else {
        be.draw_text(
            "Select photo from browser",
            cx + 4,
            cy + 4,
            8,
            Color::rgb(160, 160, 160),
        )?;
    }
    Ok(())
}

pub(crate) fn draw_music_windowed(
    file_name: &str,
    audio: &AudioHandle,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 210))?;

    if audio.is_playing() {
        let center_x = cx + cw as i32 / 2;
        be.draw_text(file_name, cx + 4, cy + 4, 8, Color::rgb(255, 200, 200))?;
        let mut buf = [0u8; 64];
        let info = stack_fmt(
            &mut buf,
            format_args!(
                "{}Hz {}kbps {}ch",
                audio.sample_rate(),
                audio.bitrate(),
                audio.channels()
            ),
        );
        let info_x = center_x - (info.len() as i32 * 8) / 2;
        be.draw_text(info, info_x, cy + 18, 8, Color::rgb(180, 180, 180))?;
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
        let status_x = center_x - (status.len() as i32 * 8) / 2;
        be.draw_text(status, status_x, cy + ch as i32 / 2, 8, status_clr)?;
    } else {
        be.draw_text(
            "No track loaded",
            cx + 4,
            cy + 4,
            8,
            Color::rgb(160, 160, 160),
        )?;
    }
    Ok(())
}

pub(crate) fn draw_settings_windowed(
    clock_mhz: i32,
    bus_mhz: i32,
    vol_info: Option<(usize, usize)>,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 20, 10, 210))?;
    be.draw_text("SETTINGS", cx + 4, cy + 2, 8, Color::rgb(60, 179, 113))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let val = Color::WHITE;
    let mut y = cy + 16;
    let vx = cx + 110;
    let mut buf = [0u8; 64];

    be.draw_text("CPU Clock:", cx + 4, y, 8, lbl)?;
    be.draw_text(
        stack_fmt(&mut buf, format_args!("{} MHz", clock_mhz)),
        vx,
        y,
        8,
        val,
    )?;
    y += 10;

    be.draw_text("Bus Clock:", cx + 4, y, 8, lbl)?;
    be.draw_text(
        stack_fmt(&mut buf, format_args!("{} MHz", bus_mhz)),
        vx,
        y,
        8,
        val,
    )?;
    y += 10;

    let profile = match clock_mhz {
        333 => "Max Performance",
        266 => "Balanced",
        222 => "Power Save",
        _ => "Custom",
    };
    be.draw_text("Profile:", cx + 4, y, 8, lbl)?;
    be.draw_text(profile, vx, y, 8, val)?;
    y += 10;

    be.draw_text("Display:", cx + 4, y, 8, lbl)?;
    be.draw_text("480x272 RGBA8888", vx, y, 8, val)?;
    y += 10;

    if let Some((total, remaining)) = vol_info {
        let used_kb = (total - remaining) / 1024;
        let total_kb = total / 1024;
        be.draw_text("Tex Cache:", cx + 4, y, 8, lbl)?;
        be.draw_text(
            stack_fmt(&mut buf, format_args!("{}/{} KB", used_kb, total_kb)),
            vx,
            y,
            8,
            val,
        )?;
    } else {
        be.draw_text("Tex Cache:", cx + 4, y, 8, lbl)?;
        be.draw_text("N/A (PSP-1000)", vx, y, 8, Color::rgb(140, 140, 140))?;
    }

    Ok(())
}

pub(crate) fn draw_network_windowed(
    status: &StatusBarInfo,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(15, 12, 0, 210))?;
    be.draw_text("NETWORK", cx + 4, cy + 2, 8, Color::rgb(218, 165, 32))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 16;
    let vx = cx + 110;

    let (wifi_str, wifi_clr) = if status.wifi_on {
        ("ON", Color::rgb(100, 200, 255))
    } else {
        ("OFF", Color::rgb(255, 100, 100))
    };
    be.draw_text("WiFi Switch:", cx + 4, y, 8, lbl)?;
    be.draw_text(wifi_str, vx, y, 8, wifi_clr)?;
    y += 10;

    let (usb_str, usb_clr) = if status.usb_connected {
        ("Connected", Color::rgb(120, 255, 120))
    } else {
        ("Disconnected", Color::rgb(160, 160, 160))
    };
    be.draw_text("USB Cable:", cx + 4, y, 8, lbl)?;
    be.draw_text(usb_str, vx, y, 8, usb_clr)?;
    y += 10;

    let (ac_str, ac_clr) = if status.ac_power {
        ("Connected", Color::rgb(120, 255, 120))
    } else {
        ("Battery", Color::rgb(200, 200, 200))
    };
    be.draw_text("AC Power:", cx + 4, y, 8, lbl)?;
    be.draw_text(ac_str, vx, y, 8, ac_clr)?;
    y += 10;

    if status.battery_percent >= 0 {
        let mut buf = [0u8; 64];
        be.draw_text("Battery:", cx + 4, y, 8, lbl)?;
        be.draw_text(
            stack_fmt(&mut buf, format_args!("{}%", status.battery_percent)),
            vx,
            y,
            8,
            Color::WHITE,
        )?;
    }

    Ok(())
}

pub(crate) fn draw_sysmon_windowed(
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
    fps: f32,
    free_kb: i32,
    max_blk_kb: i32,
    vol_info: Option<(usize, usize)>,
    usb_active: bool,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 10, 20, 210))?;
    be.draw_text(
        "SYSTEM MONITOR",
        cx + 4,
        cy + 2,
        8,
        Color::rgb(60, 179, 113),
    )?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(140, 140, 140);
    let val = Color::WHITE;
    let mut y = cy + 16;
    let vx = cx + 100;

    // Use a stack buffer to avoid heap allocations from format!() every frame.
    let mut buf = [0u8; 64];

    let fps_clr = if fps >= 55.0 {
        Color::rgb(120, 255, 120)
    } else if fps >= 30.0 {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(255, 80, 80)
    };
    be.draw_text("FPS:", cx + 4, y, 8, lbl)?;
    let s = stack_fmt(&mut buf, format_args!("{:.1}", fps));
    be.draw_text(s, vx, y, 8, fps_clr)?;
    y += 11;

    be.draw_text("CPU/Bus/ME:", cx + 4, y, 8, lbl)?;
    let s = stack_fmt(
        &mut buf,
        format_args!("{}/{}/{}", sysinfo.cpu_mhz, sysinfo.bus_mhz, sysinfo.me_mhz),
    );
    be.draw_text(s, vx, y, 8, val)?;
    y += 11;

    be.draw_text("Free RAM:", cx + 4, y, 8, lbl)?;
    let s = stack_fmt(&mut buf, format_args!("{} KB", free_kb));
    be.draw_text(s, vx, y, 8, val)?;
    y += 11;

    be.draw_text("Max Block:", cx + 4, y, 8, lbl)?;
    let s = stack_fmt(&mut buf, format_args!("{} KB", max_blk_kb));
    be.draw_text(s, vx, y, 8, val)?;
    y += 11;

    if let Some((total, remaining)) = vol_info {
        let used_kb = (total - remaining) / 1024;
        let total_kb = total / 1024;
        be.draw_text("Tex VRAM:", cx + 4, y, 8, lbl)?;
        let s = stack_fmt(&mut buf, format_args!("{}/{} KB", used_kb, total_kb));
        be.draw_text(s, vx, y, 8, val)?;
        y += 11;
    }

    let bat_clr = if status.battery_charging || status.battery_percent >= 50 {
        Color::rgb(120, 255, 120)
    } else if status.battery_percent >= 20 {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(255, 80, 80)
    };
    be.draw_text("Battery:", cx + 4, y, 8, lbl)?;
    let bat_s = if status.battery_percent >= 0 {
        if status.battery_charging {
            stack_fmt(&mut buf, format_args!("{}% CHG", status.battery_percent))
        } else {
            stack_fmt(&mut buf, format_args!("{}%", status.battery_percent))
        }
    } else if status.ac_power {
        "AC"
    } else {
        "N/A"
    };
    be.draw_text(bat_s, vx, y, 8, bat_clr)?;
    y += 11;

    let wifi_str = if status.wifi_on { "ON" } else { "OFF" };
    let usb_str = if usb_active {
        "STORAGE"
    } else if status.usb_connected {
        "CONN"
    } else {
        "---"
    };
    be.draw_text("WiFi:", cx + 4, y, 8, lbl)?;
    be.draw_text(wifi_str, vx, y, 8, val)?;
    be.draw_text("USB:", cx + 150, y, 8, lbl)?;
    be.draw_text(usb_str, cx + 190, y, 8, val)?;

    Ok(())
}

pub(crate) fn draw_browser_windowed(
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(5, 10, 25, 210))?;
    be.draw_text("BROWSER", cx + 4, cy + 2, 8, Color::rgb(50, 120, 200))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 20;
    be.draw_text("Web browser for PSP.", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("Press Start for fullscreen.", cx + 4, y, 8, lbl)?;
    Ok(())
}

pub(crate) fn draw_browser_windowed_with_url(
    url: &str,
    status: &str,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(5, 10, 25, 220))?;
    be.draw_text("BROWSER", cx + 4, cy + 2, 8, Color::rgb(50, 120, 200))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let hi = Color::rgb(120, 200, 255);
    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 18;

    // Truncate URL to fit window width.
    let max_chars = (cw as usize / 5).min(url.len());
    let display_url = if url.len() > max_chars {
        let start = url.floor_char_boundary(url.len() - max_chars);
        &url[start..]
    } else {
        url
    };
    be.draw_text(display_url, cx + 4, y, 8, hi)?;
    y += 14;

    if !status.is_empty() {
        be.draw_text(status, cx + 4, y, 8, lbl)?;
        y += 14;
    }

    be.draw_text("Start = fullscreen", cx + 4, y, 8, lbl)?;
    Ok(())
}

pub(crate) fn draw_packages_windowed(
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(5, 10, 20, 210))?;
    be.draw_text("PACKAGE MGR", cx + 4, cy + 2, 8, Color::rgb(70, 130, 180))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 20;
    be.draw_text("Manage homebrew packages.", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("Use Terminal commands:", cx + 4, y, 8, lbl)?;
    y += 12;
    be.draw_text("  pkg list", cx + 4, y, 8, Color::rgb(120, 200, 255))?;
    y += 12;
    be.draw_text(
        "  pkg install <name>",
        cx + 4,
        y,
        8,
        Color::rgb(120, 200, 255),
    )?;
    Ok(())
}

pub(crate) fn draw_radio_windowed(
    _audio: &AudioHandle,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(20, 10, 0, 210))?;
    be.draw_text("RADIO", cx + 4, cy + 2, 8, Color::rgb(255, 140, 60))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let hi = Color::rgb(120, 200, 255);
    let mut y = cy + 20;

    be.draw_text(
        "Internet Radio Streaming",
        cx + 4,
        y,
        8,
        Color::rgb(255, 200, 80),
    )?;
    y += 14;
    be.draw_text("Stations: SomaFM (8 presets)", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("In-game: L+R+Start to open", cx + 4, y, 8, hi)?;
    y += 12;
    be.draw_text("overlay and toggle radio.", cx + 4, y, 8, hi)?;
    y += 14;
    be.draw_text("Requires WiFi connection.", cx + 4, y, 8, lbl)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// TV Guide (windowed)
// ---------------------------------------------------------------------------

pub(crate) fn draw_tvguide_windowed(
    channels: &[oasis_core::apps::tv_guide::Channel],
    catalogs: &[Option<oasis_core::apps::tv_guide::ChannelCatalog>],
    selected: usize,
    scroll: usize,
    now_playing: &str,
    tuned: bool,
    downloading: bool,
    download_progress: f32,
    error_msg: &str,
    preview_tex: Option<TextureId>,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    // Fullscreen video: skip ALL UI drawing — only blit video + title.
    // This eliminates flickering from UI elements rendered before the blit.
    if tuned {
        if let Some(tex) = preview_tex {
            be.blit(tex, 0, 0, 480, 272)?;

            // Title overlay at bottom of screen.
            let max_chars = ((480i32 - 12) / 6).max(6) as usize;
            let display = truncate_str(now_playing, max_chars);
            let title_y = 272 - 14;
            be.fill_rect(0, title_y - 2, 480, 14, Color::rgba(0, 0, 0, 160))?;
            let tx = (240 - (display.len() as i32 * 4)).max(4);
            be.draw_text(&display, tx, title_y, 8, Color::WHITE)?;
            return Ok(());
        }
    }

    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 10, 30, 220))?;
    be.draw_text("TV GUIDE", cx + 4, cy + 2, 8, Color::rgb(80, 200, 255))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let hi = Color::rgb(120, 200, 255);
    let sel_bg = Color::rgba(80, 200, 255, 60);

    if tuned {
        let mid_x = cx + cw as i32 / 2;
        let mid_y = cy + ch as i32 / 2;

        if downloading {
            // Download progress.
            let mut fbuf = [0u8; 64];
            let pct = (download_progress * 100.0) as u32;
            let status = stack_fmt(&mut fbuf, format_args!("Downloading... {}%", pct));
            let sx = (mid_x - (status.len() as i32 * 4)).max(cx + 4);
            be.draw_text(status, sx, mid_y - 20, 8, Color::rgb(255, 200, 80))?;

            // Progress bar.
            let bar_w = (cw as i32 - 20).max(40) as u32;
            let bar_x = cx + 10;
            let bar_y = mid_y - 4;
            be.fill_rect(bar_x, bar_y, bar_w, 6, Color::rgba(40, 40, 60, 200))?;
            let fill = (bar_w as f32 * download_progress) as u32;
            if fill > 0 {
                be.fill_rect(bar_x, bar_y, fill, 6, Color::rgb(0, 160, 255))?;
            }

            // Episode title below bar.
            let max_chars = ((cw as i32 - 12) / 6).max(6) as usize;
            let display = truncate_str(now_playing, max_chars);
            let tx = (mid_x - (display.len() as i32 * 4)).max(cx + 4);
            be.draw_text(&display, tx, bar_y + 14, 8, lbl)?;
        } else {
            // Audio playing / idle.
            let status = if !error_msg.is_empty() {
                error_msg
            } else {
                "Playing audio..."
            };
            let status_clr = if error_msg.is_empty() {
                Color::rgb(120, 255, 120)
            } else {
                Color::rgb(255, 80, 80)
            };
            let sx = (mid_x - (status.len() as i32 * 4)).max(cx + 4);
            be.draw_text(status, sx, mid_y - 14, 8, status_clr)?;

            // Episode title — truncate to fit window width with margin.
            let max_chars = ((cw as i32 - 12) / 6).max(6) as usize;
            let display = truncate_str(now_playing, max_chars);
            let tx = (mid_x - (display.len() as i32 * 4)).max(cx + 4);
            be.draw_text(&display, tx, mid_y + 4, 8, lbl)?;
        }
        return Ok(());
    }

    let mut y = cy + 16;

    if channels.is_empty() {
        be.draw_text("Loading channels...", cx + 4, y, 8, lbl)?;
        return Ok(());
    }

    // Draw channel list.
    let row_h = 12i32;
    let max_rows = ((ch as i32 - 18) / row_h).max(1) as usize;
    let end = (scroll + max_rows).min(channels.len());

    for i in scroll..end {
        let row = (i - scroll) as i32;
        let ry = y + row * row_h;

        // Highlight selected row.
        if i == selected {
            be.fill_rect(cx, ry - 1, cw, row_h as u32, sel_bg)?;
        }

        let c = &channels[i];
        let num_col = Color::rgb(200, 200, 200);
        let name_col = if i == selected { hi } else { Color::rgb(220, 220, 220) };

        // Channel number.
        let mut fbuf = [0u8; 64];
        let num_str = stack_fmt(&mut fbuf, format_args!("{:2}", c.number));
        be.draw_text(num_str, cx + 4, ry, 8, num_col)?;

        // Channel name.
        let max_name_w = cw as i32 - 80;
        let name = if c.name.len() as i32 * 6 > max_name_w {
            let chars = (max_name_w / 6 - 2).max(3) as usize;
            let t: String = c.name.chars().take(chars).collect();
            t + ".."
        } else {
            c.name.clone()
        };
        be.draw_text(&name, cx + 30, ry, 8, name_col)?;

        // Episode count.
        if i < catalogs.len() {
            if let Some(cat) = &catalogs[i] {
                let ep_str = stack_fmt(&mut fbuf, format_args!("{}ep", cat.episodes.len()));
                let ep_x = cx + cw as i32 - 30;
                be.draw_text(ep_str, ep_x, ry, 8, Color::rgb(120, 200, 120))?;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Loading indicator
// ---------------------------------------------------------------------------

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() > max {
        let t: String = s.chars().take(max.saturating_sub(2)).collect();
        t + ".."
    } else {
        s.to_string()
    }
}

/// Format into a stack buffer, returning a `&str`. Avoids heap allocation.
fn stack_fmt<'a>(buf: &'a mut [u8; 64], args: core::fmt::Arguments<'_>) -> &'a str {
    use core::fmt::Write;

    struct BufWriter<'b> {
        buf: &'b mut [u8],
        pos: usize,
    }
    impl core::fmt::Write for BufWriter<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let avail = self.buf.len() - self.pos;
            // Avoid splitting a multi-byte UTF-8 character at the boundary.
            let len = s.floor_char_boundary(bytes.len().min(avail));
            self.buf[self.pos..self.pos + len].copy_from_slice(&bytes[..len]);
            self.pos += len;
            Ok(())
        }
    }
    let mut w = BufWriter {
        buf: &mut buf[..],
        pos: 0,
    };
    let _ = w.write_fmt(args);
    let n = w.pos;
    core::str::from_utf8(&buf[..n]).unwrap_or("???")
}

pub(crate) fn draw_loading_indicator(backend: &mut PspBackend, msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);
    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;
    let text_x = cx - (msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(msg, text_x, cy, 8, Color::rgb(200, 200, 200));
}

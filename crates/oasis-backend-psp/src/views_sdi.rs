//! SDI-based classic view renderers.
//!
//! Each view creates a set of named [`SdiObject`]s in the registry, updates
//! their properties every frame, and hides them when switching away.  The
//! actual drawing is handled by [`SdiRegistry::draw_base_layer`].
//!
//! Object names are prefixed per view (`radio_`, `tv_`, `photo_`, `browser_`,
//! `music_`, `fm_`) to avoid collisions.

use core::fmt::Write;

use oasis_backend_psp::{Color, FileEntry, SCREEN_WIDTH, TextureId};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;
use crate::types::{ClassicView, RADIO_STATIONS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum rows visible in a list view.
const LIST_ROWS: usize = FM_VISIBLE_ROWS;
/// Row height for list items.
const ROW_H: i32 = FM_ROW_H;
/// Y start for list content (below header).
const LIST_Y: i32 = FM_START_Y;

// ---------------------------------------------------------------------------
// Reusable name buffer (avoids per-frame heap allocations on PSP)
// ---------------------------------------------------------------------------

/// Format an SDI object name into a reusable buffer, returning `&str`.
/// The buffer is cleared and rewritten on each call, so the returned
/// reference is only valid until the next `sdi_key!` call on the same buffer.
macro_rules! sdi_key {
    ($buf:expr, $($arg:tt)*) => {{
        $buf.clear();
        write!($buf, $($arg)*).unwrap();
        $buf.as_str()
    }};
}

// ---------------------------------------------------------------------------
// View lifecycle helpers
// ---------------------------------------------------------------------------

/// Ensure an object exists in the registry, creating it if necessary.
fn ensure(sdi: &mut SdiRegistry, name: &str) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
}

/// Only update `obj.text` when the value actually changed, avoiding a heap
/// allocation + drop on every frame for static content.
fn set_text(slot: &mut Option<String>, value: &str) {
    match slot {
        Some(existing) if existing == value => {},
        _ => *slot = Some(value.to_owned()),
    }
}

/// Hide all SDI objects whose name starts with `prefix`.
fn hide_prefixed(sdi: &mut SdiRegistry, prefix: &str) {
    let names: Vec<String> = sdi
        .names()
        .filter(|n| n.starts_with(prefix))
        .map(|n| n.to_string())
        .collect();
    for name in &names {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

/// All view prefixes.
const VIEW_PREFIXES: &[&str] = &["radio_", "tv_", "photo_", "browser_", "music_", "fm_"];

/// Hide all view objects (called on view transition).
pub(crate) fn hide_all(sdi: &mut SdiRegistry) {
    for prefix in VIEW_PREFIXES {
        hide_prefixed(sdi, prefix);
    }
}

// ---------------------------------------------------------------------------
// Shared: list-view background + header
// ---------------------------------------------------------------------------

/// Set up the common background + header objects for a list view.
fn setup_list_bg(sdi: &mut SdiRegistry, prefix: &str) {
    let bg_name = format!("{prefix}bg");
    let hdr_name = format!("{prefix}hdr");
    let hdr_sub = format!("{prefix}hdr_sub");
    ensure(sdi, &bg_name);
    ensure(sdi, &hdr_name);
    ensure(sdi, &hdr_sub);

    // Pre-create row objects.
    for i in 0..LIST_ROWS {
        let row_bg = format!("{prefix}row_bg_{i}");
        let row_icon = format!("{prefix}row_icon_{i}");
        let row_name = format!("{prefix}row_name_{i}");
        let row_extra = format!("{prefix}row_extra_{i}");
        let row_extra2 = format!("{prefix}row_extra2_{i}");
        ensure(sdi, &row_bg);
        ensure(sdi, &row_icon);
        ensure(sdi, &row_name);
        ensure(sdi, &row_extra);
        ensure(sdi, &row_extra2);
    }

    // Scrollbar.
    let scroll_name = format!("{prefix}scroll");
    ensure(sdi, &scroll_name);
}

/// Update common list-view background + header.
fn update_list_bg(
    sdi: &mut SdiRegistry,
    buf: &mut String,
    prefix: &str,
    title: &str,
    accent: Color,
    at: &ActiveTheme,
    subtitle: Option<&str>,
) {
    let name = sdi_key!(buf, "{prefix}bg");
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = 0;
        obj.y = CONTENT_TOP as i32;
        obj.w = SCREEN_WIDTH;
        obj.h = CONTENT_H;
        obj.color = at.app.bg;
        obj.alpha = 200;
        obj.z = 100;
        obj.visible = true;
    }

    let name = sdi_key!(buf, "{prefix}hdr");
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = 4;
        obj.y = CONTENT_TOP as i32 + 2;
        obj.w = 0;
        obj.h = 0;
        set_text(&mut obj.text, title);
        obj.font_size = 8;
        obj.text_color = accent;
        obj.z = 101;
        obj.visible = true;
    }

    let name = sdi_key!(buf, "{prefix}hdr_sub");
    if let Ok(obj) = sdi.get_mut(name) {
        if let Some(sub) = subtitle {
            let max = 45;
            let display = if sub.len() > max {
                let t: String = sub.chars().take(max - 2).collect();
                format!("{t}..")
            } else {
                sub.to_string()
            };
            obj.x = 4 + (title.len() as i32 + 2) * CHAR_W;
            obj.y = CONTENT_TOP as i32 + 2;
            set_text(&mut obj.text, &display);
            obj.font_size = 8;
            obj.text_color = at.app.dim_text;
            obj.z = 101;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

/// Update scrollbar indicator for a list view.
fn update_scrollbar(
    sdi: &mut SdiRegistry,
    buf: &mut String,
    prefix: &str,
    selected: usize,
    total: usize,
    at: &ActiveTheme,
) {
    let name = sdi_key!(buf, "{prefix}scroll");
    if let Ok(obj) = sdi.get_mut(name) {
        if total > LIST_ROWS {
            let ratio = selected as f32 / (total - 1).max(1) as f32;
            let track_h = CONTENT_H as i32 - 16;
            let dot_y = LIST_Y + (ratio * track_h as f32) as i32;
            obj.x = SCREEN_WIDTH as i32 - 4;
            obj.y = dot_y;
            obj.w = 3;
            obj.h = 8;
            obj.color = at.scrollbar.thumb_color;
            obj.z = 105;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

/// Generic helper: update a row in a list view.
#[allow(clippy::too_many_arguments)]
fn update_list_row(
    sdi: &mut SdiRegistry,
    buf: &mut String,
    prefix: &str,
    row_idx: usize,
    is_selected: bool,
    icon_text: &str,
    icon_color: Color,
    name_text: &str,
    name_color: Color,
    extra_text: Option<(&str, Color, i32)>,
    extra2_text: Option<(&str, Color, i32)>,
    highlight_color: Color,
    at: &ActiveTheme,
) {
    let y = LIST_Y + row_idx as i32 * ROW_H;

    // Row highlight.
    let name = sdi_key!(buf, "{prefix}row_bg_{row_idx}");
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = 0;
        obj.y = y - 1;
        obj.w = SCREEN_WIDTH;
        obj.h = ROW_H as u32;
        obj.color = highlight_color;
        obj.z = 102;
        obj.visible = is_selected;
    }

    // Icon / prefix.
    let name = sdi_key!(buf, "{prefix}row_icon_{row_idx}");
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = 4;
        obj.y = y;
        set_text(&mut obj.text, icon_text);
        obj.font_size = 8;
        obj.text_color = icon_color;
        obj.z = 103;
        obj.visible = true;
    }

    // Name text.
    let name = sdi_key!(buf, "{prefix}row_name_{row_idx}");
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = 32;
        obj.y = y;
        set_text(&mut obj.text, name_text);
        obj.font_size = 8;
        obj.text_color = name_color;
        obj.z = 103;
        obj.visible = true;
    }

    // Extra column (genre, size, etc).
    let name = sdi_key!(buf, "{prefix}row_extra_{row_idx}");
    if let Ok(obj) = sdi.get_mut(name) {
        if let Some((text, color, x)) = extra_text {
            obj.x = x;
            obj.y = y;
            set_text(&mut obj.text, text);
            obj.font_size = 8;
            obj.text_color = color;
            obj.z = 103;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }

    // Second extra column.
    let name = sdi_key!(buf, "{prefix}row_extra2_{row_idx}");
    if let Ok(obj) = sdi.get_mut(name) {
        if let Some((text, color, x)) = extra2_text {
            obj.x = x;
            obj.y = y;
            set_text(&mut obj.text, text);
            obj.font_size = 8;
            obj.text_color = color;
            obj.z = 103;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }

    let _ = at; // used by callers for derived colors
}

/// Hide unused rows beyond `visible_count`.
fn hide_unused_rows(sdi: &mut SdiRegistry, buf: &mut String, prefix: &str, visible_count: usize) {
    for i in visible_count..LIST_ROWS {
        for suffix in &[
            "row_bg_",
            "row_icon_",
            "row_name_",
            "row_extra_",
            "row_extra2_",
        ] {
            let name = sdi_key!(buf, "{prefix}{suffix}{i}");
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
    }
}

// ===========================================================================
// Radio Stations (list view)
// ===========================================================================

/// Create SDI objects for the radio stations list.
pub(crate) fn setup_radio(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "radio_");
}

/// Update SDI objects for the radio stations list each frame.
pub(crate) fn update_radio(
    sdi: &mut SdiRegistry,
    selected: usize,
    scroll: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(sdi, &mut buf, "radio_", "RADIO", accent, at, None);

    let end = (scroll + LIST_ROWS).min(RADIO_STATIONS.len());
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let station = &RADIO_STATIONS[i];
        let br_str = format!("{}k", station.bitrate);
        let br_x = 480 - (br_str.len() as i32 * CHAR_W) - 4;
        update_list_row(
            sdi,
            &mut buf,
            "radio_",
            row,
            i == selected,
            "[R]",
            accent,
            station.name,
            at.app.text,
            Some((station.genre, at.app.dim_text, 230)),
            Some((&br_str, at.app.dim_text, br_x)),
            oasis_core::color::with_alpha(accent, 100),
            at,
        );
    }
    hide_unused_rows(sdi, &mut buf, "radio_", visible);
    update_scrollbar(sdi, &mut buf, "radio_", selected, RADIO_STATIONS.len(), at);
}

// ===========================================================================
// TV Guide Channels (list view)
// ===========================================================================

pub(crate) fn setup_tv_channels(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "tv_");
}

pub(crate) fn update_tv_channels(
    sdi: &mut SdiRegistry,
    channels: &[oasis_core::apps::tv_guide::Channel],
    catalogs: &[Option<oasis_core::apps::tv_guide::ChannelCatalog>],
    selected: usize,
    scroll: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(sdi, &mut buf, "tv_", "TV GUIDE", accent, at, None);

    let end = (scroll + LIST_ROWS).min(channels.len());
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let ch = &channels[i];
        let num_str = format!("{:2}", ch.number);
        let max_name = 25;
        let display_name = if ch.name.len() > max_name {
            let t: String = ch.name.chars().take(max_name - 2).collect();
            format!("{t}..")
        } else {
            ch.name.clone()
        };
        // Status indicator.
        let status = if i < catalogs.len() {
            if let Some(cat) = &catalogs[i] {
                Some((
                    format!("{}ep", cat.episodes.len()),
                    Color::rgb(120, 200, 120),
                    380,
                ))
            } else {
                Some(("...".to_string(), Color::rgb(180, 180, 80), 380))
            }
        } else {
            None
        };
        let genre_display = if ch.genre.len() > 6 {
            let t: String = ch.genre.chars().take(5).collect();
            t
        } else {
            ch.genre.clone()
        };

        // For TV, override icon/name positions.
        let y = LIST_Y + row as i32 * ROW_H;
        let highlight = oasis_core::color::with_alpha(accent, 100);

        // Row highlight.
        let name = sdi_key!(buf, "tv_row_bg_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = 0;
            obj.y = y - 1;
            obj.w = SCREEN_WIDTH;
            obj.h = ROW_H as u32;
            obj.color = highlight;
            obj.z = 102;
            obj.visible = i == selected;
        }
        // Channel number.
        let name = sdi_key!(buf, "tv_row_icon_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = 4;
            obj.y = y;
            set_text(&mut obj.text, &num_str);
            obj.font_size = 8;
            obj.text_color = accent;
            obj.z = 103;
            obj.visible = true;
        }
        // Call sign + name combined.
        let name = sdi_key!(buf, "tv_row_name_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = 28;
            obj.y = y;
            let combined = format!("{:<6} {}", ch.call_sign, display_name);
            set_text(&mut obj.text, &combined);
            obj.font_size = 8;
            obj.text_color = at.app.text;
            obj.z = 103;
            obj.visible = true;
        }
        // Status.
        let name = sdi_key!(buf, "tv_row_extra_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            if let Some((text, color, x)) = &status {
                obj.x = *x;
                obj.y = y;
                set_text(&mut obj.text, text);
                obj.font_size = 8;
                obj.text_color = *color;
                obj.z = 103;
                obj.visible = true;
            } else {
                obj.visible = false;
            }
        }
        // Genre.
        let name = sdi_key!(buf, "tv_row_extra2_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = 430;
            obj.y = y;
            set_text(&mut obj.text, &genre_display);
            obj.font_size = 8;
            obj.text_color = at.app.dim_text;
            obj.z = 103;
            obj.visible = true;
        }
    }
    hide_unused_rows(sdi, &mut buf, "tv_", visible);
    update_scrollbar(sdi, &mut buf, "tv_", selected, channels.len(), at);
}

// ===========================================================================
// Photo Browser (list view)
// ===========================================================================

pub(crate) fn setup_photo_browser(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "photo_");
}

pub(crate) fn update_photo_browser(
    sdi: &mut SdiRegistry,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(
        sdi,
        &mut buf,
        "photo_",
        "PHOTO VIEWER",
        accent,
        at,
        Some(path),
    );

    let end = (scroll + LIST_ROWS).min(entries.len());
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let entry = &entries[i];
        let (icon, icon_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[I]", accent)
        };
        let name_color = if entry.is_dir { accent } else { at.app.text };
        let max_name = 44;
        let display = if entry.name.len() > max_name {
            let t: String = entry.name.chars().take(max_name - 2).collect();
            format!("{t}..")
        } else {
            entry.name.clone()
        };
        let size_info = if !entry.is_dir {
            let s = oasis_backend_psp::format_size(entry.size);
            let x = 480 - (s.len() as i32 * CHAR_W) - 4;
            Some((s, at.app.dim_text, x))
        } else {
            None
        };
        update_list_row(
            sdi,
            &mut buf,
            "photo_",
            row,
            i == selected,
            icon,
            icon_clr,
            &display,
            name_color,
            size_info.as_ref().map(|(s, c, x)| (s.as_str(), *c, *x)),
            None,
            oasis_core::color::with_alpha(accent, 100),
            at,
        );
    }
    hide_unused_rows(sdi, &mut buf, "photo_", visible);
    update_scrollbar(sdi, &mut buf, "photo_", selected, entries.len(), at);
}

// ===========================================================================
// Photo full-screen view
// ===========================================================================

pub(crate) fn setup_photo_view(sdi: &mut SdiRegistry) {
    ensure(sdi, "photo_view_bg");
    ensure(sdi, "photo_view_img");
    ensure(sdi, "photo_view_err");
}

pub(crate) fn update_photo_view(
    sdi: &mut SdiRegistry,
    tex: Option<TextureId>,
    img_w: u32,
    img_h: u32,
) {
    if let Ok(obj) = sdi.get_mut("photo_view_bg") {
        obj.x = 0;
        obj.y = CONTENT_TOP as i32;
        obj.w = SCREEN_WIDTH;
        obj.h = CONTENT_H;
        obj.color = Color::BLACK;
        obj.z = 100;
        obj.visible = true;
    }

    if let Ok(obj) = sdi.get_mut("photo_view_img") {
        if let Some(t) = tex {
            let max_w = SCREEN_WIDTH;
            let max_h = CONTENT_H;
            let scale_w = max_w as f32 / img_w.max(1) as f32;
            let scale_h = max_h as f32 / img_h.max(1) as f32;
            let scale = if scale_w < scale_h { scale_w } else { scale_h };
            let draw_w = (img_w as f32 * scale) as u32;
            let draw_h = (img_h as f32 * scale) as u32;
            obj.x = ((max_w - draw_w) / 2) as i32;
            obj.y = CONTENT_TOP as i32 + ((max_h - draw_h) / 2) as i32;
            obj.w = draw_w;
            obj.h = draw_h;
            obj.texture = Some(t);
            obj.z = 101;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }

    if let Ok(obj) = sdi.get_mut("photo_view_err") {
        if tex.is_none() {
            obj.x = 160;
            obj.y = 130;
            set_text(&mut obj.text, "Failed to load image");
            obj.font_size = 8;
            obj.text_color = Color::rgb(255, 80, 80);
            obj.z = 102;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

// ===========================================================================
// Music Browser (list view)
// ===========================================================================

pub(crate) fn setup_music_browser(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "music_");
}

pub(crate) fn update_music_browser(
    sdi: &mut SdiRegistry,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(
        sdi,
        &mut buf,
        "music_",
        "MUSIC PLAYER",
        accent,
        at,
        Some(path),
    );

    let end = (scroll + LIST_ROWS).min(entries.len());
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let entry = &entries[i];
        let (icon, icon_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[M]", accent)
        };
        let name_color = if entry.is_dir { accent } else { at.app.text };
        let max_name = 44;
        let display = if entry.name.len() > max_name {
            let t: String = entry.name.chars().take(max_name - 2).collect();
            format!("{t}..")
        } else {
            entry.name.clone()
        };
        let size_info = if !entry.is_dir {
            let s = oasis_backend_psp::format_size(entry.size);
            let x = 480 - (s.len() as i32 * CHAR_W) - 4;
            Some((s, at.app.dim_text, x))
        } else {
            None
        };
        update_list_row(
            sdi,
            &mut buf,
            "music_",
            row,
            i == selected,
            icon,
            icon_clr,
            &display,
            name_color,
            size_info.as_ref().map(|(s, c, x)| (s.as_str(), *c, *x)),
            None,
            oasis_core::color::with_alpha(accent, 100),
            at,
        );
    }
    hide_unused_rows(sdi, &mut buf, "music_", visible);
    update_scrollbar(sdi, &mut buf, "music_", selected, entries.len(), at);
}

// ===========================================================================
// Browser (list view with URL bar)
// ===========================================================================

pub(crate) fn setup_browser(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "browser_");
    ensure(sdi, "browser_url");
    ensure(sdi, "browser_status");
    // Extra content rows for the text-heavy browser view.
    for i in LIST_ROWS..40 {
        ensure(sdi, &format!("browser_row_name_{i}"));
    }
}

pub(crate) fn update_browser(
    sdi: &mut SdiRegistry,
    url: &str,
    lines: &[String],
    scroll: usize,
    status_msg: &str,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(sdi, &mut buf, "browser_", "BROWSER", accent, at, None);

    // URL bar.
    if let Ok(obj) = sdi.get_mut("browser_url") {
        let display_url = if url.len() > 45 {
            let t: String = url.chars().take(43).collect();
            format!("{t}..")
        } else {
            url.to_string()
        };
        obj.x = 4 + 7 * CHAR_W + 8;
        obj.y = CONTENT_TOP as i32 + 3;
        set_text(&mut obj.text, &display_url);
        obj.font_size = 8;
        obj.text_color = accent;
        obj.z = 101;
        obj.visible = true;
    }

    // Status line.
    if let Ok(obj) = sdi.get_mut("browser_status") {
        obj.x = 4;
        obj.y = LIST_Y - 1;
        set_text(&mut obj.text, status_msg);
        obj.font_size = 8;
        obj.text_color = at.app.dim_text;
        obj.z = 101;
        obj.visible = true;
    }

    // Content lines (browser text uses 9px line height, more rows than FM).
    let text_start_y = LIST_Y + 10;
    let visible_rows = ((BOTTOMBAR_Y - HINT_Y_OFFSET - text_start_y) / 9) as usize;
    let end = (scroll + visible_rows).min(lines.len());

    // Use the list row name objects for content lines.
    for row in 0..visible_rows.min(40) {
        let name = sdi_key!(buf, "browser_row_name_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            if scroll + row < end {
                let y = text_start_y + row as i32 * 9;
                obj.x = 4;
                obj.y = y;
                set_text(&mut obj.text, &lines[scroll + row]);
                obj.font_size = 8;
                obj.text_color = at.app.text;
                obj.z = 103;
                obj.visible = true;
            } else {
                obj.visible = false;
            }
        }
    }

    // Scrollbar.
    let scroll_name = "browser_scroll";
    if let Ok(obj) = sdi.get_mut(scroll_name) {
        if lines.len() > visible_rows && !lines.is_empty() {
            let ratio = scroll as f32 / (lines.len() - 1).max(1) as f32;
            let track_h = CONTENT_H as i32 - 30;
            let dot_y = text_start_y + (ratio * track_h as f32) as i32;
            obj.x = SCREEN_WIDTH as i32 - 4;
            obj.y = dot_y;
            obj.w = 3;
            obj.h = 8;
            obj.color = at.scrollbar.thumb_color;
            obj.z = 105;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

// ===========================================================================
// File Manager (dual-panel list view)
// ===========================================================================

pub(crate) fn setup_file_manager(sdi: &mut SdiRegistry) {
    // Left panel.
    setup_list_bg(sdi, "fm_l_");
    // Right panel.
    setup_list_bg(sdi, "fm_r_");
    // Shared elements.
    ensure(sdi, "fm_bg");
    ensure(sdi, "fm_hdr");
    ensure(sdi, "fm_divider");
    ensure(sdi, "fm_indicator");
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_file_manager(
    sdi: &mut SdiRegistry,
    path_l: &str,
    entries_l: &[FileEntry],
    selected_l: usize,
    scroll_l: usize,
    path_r: &str,
    entries_r: &[FileEntry],
    selected_r: usize,
    scroll_r: usize,
    active_panel: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    let half_w = SCREEN_WIDTH / 2;

    // Background.
    if let Ok(obj) = sdi.get_mut("fm_bg") {
        obj.x = 0;
        obj.y = CONTENT_TOP as i32;
        obj.w = SCREEN_WIDTH;
        obj.h = CONTENT_H;
        obj.color = at.app.bg;
        obj.alpha = 200;
        obj.z = 100;
        obj.visible = true;
    }

    // Header.
    let header = if active_panel == 0 {
        format!("[L] {}  |  {}", path_l, path_r)
    } else {
        format!("{}  |  [R] {}", path_l, path_r)
    };
    if let Ok(obj) = sdi.get_mut("fm_hdr") {
        obj.x = 4;
        obj.y = CONTENT_TOP as i32 + 2;
        let fm_title = format!("FILE MGR  {}", header);
        set_text(&mut obj.text, &fm_title);
        obj.font_size = 8;
        obj.text_color = accent;
        obj.z = 101;
        obj.visible = true;
    }

    // Vertical divider.
    if let Ok(obj) = sdi.get_mut("fm_divider") {
        obj.x = half_w as i32;
        obj.y = CONTENT_TOP as i32 + 12;
        obj.w = 1;
        obj.h = CONTENT_H - 12;
        obj.color = oasis_core::color::with_alpha(accent, 80);
        obj.z = 104;
        obj.visible = true;
    }

    // Active panel indicator line.
    if let Ok(obj) = sdi.get_mut("fm_indicator") {
        obj.x = if active_panel == 0 {
            0
        } else {
            half_w as i32 + 1
        };
        obj.y = CONTENT_TOP as i32 + 12;
        obj.w = if active_panel == 0 {
            half_w - 1
        } else {
            half_w
        };
        obj.h = 1;
        obj.color = accent;
        obj.z = 104;
        obj.visible = true;
    }

    // Left panel entries.
    update_fm_panel(
        sdi,
        &mut buf,
        "fm_l_",
        entries_l,
        selected_l,
        scroll_l,
        0,
        half_w - 1,
        active_panel == 0,
        at,
    );
    // Right panel entries.
    update_fm_panel(
        sdi,
        &mut buf,
        "fm_r_",
        entries_r,
        selected_r,
        scroll_r,
        half_w as i32 + 1,
        half_w,
        active_panel == 1,
        at,
    );
}

#[allow(clippy::too_many_arguments)]
fn update_fm_panel(
    sdi: &mut SdiRegistry,
    buf: &mut String,
    prefix: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
    panel_x: i32,
    panel_w: u32,
    is_active: bool,
    at: &ActiveTheme,
) {
    let accent = at.app.selected_text;
    let end = (scroll + LIST_ROWS).min(entries.len());
    let visible = end - scroll;
    let max_name_chars = ((panel_w as i32 - 32) / CHAR_W).max(4) as usize;

    for row in 0..visible {
        let i = scroll + row;
        let entry = &entries[i];
        let y = LIST_Y + row as i32 * ROW_H;
        let (icon, icon_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[F]", at.app.dim_text)
        };
        let name_color = if entry.is_dir { accent } else { at.app.text };
        let display = if entry.name.len() > max_name_chars {
            let t: String = entry.name.chars().take(max_name_chars - 2).collect();
            format!("{t}..")
        } else {
            entry.name.clone()
        };

        // Row highlight.
        let name = sdi_key!(buf, "{prefix}row_bg_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = panel_x;
            obj.y = y - 1;
            obj.w = panel_w;
            obj.h = ROW_H as u32;
            obj.color = oasis_core::color::with_alpha(accent, 100);
            obj.z = 102;
            obj.visible = i == selected && is_active;
        }
        let name = sdi_key!(buf, "{prefix}row_icon_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = panel_x + 2;
            obj.y = y;
            set_text(&mut obj.text, icon);
            obj.font_size = 8;
            obj.text_color = icon_clr;
            obj.z = 103;
            obj.visible = true;
        }
        let name = sdi_key!(buf, "{prefix}row_name_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = panel_x + 28;
            obj.y = y;
            set_text(&mut obj.text, &display);
            obj.font_size = 8;
            obj.text_color = name_color;
            obj.z = 103;
            obj.visible = true;
        }
        // Extra/extra2 not used for file manager panels.
        let name = sdi_key!(buf, "{prefix}row_extra_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
        let name = sdi_key!(buf, "{prefix}row_extra2_{row}");
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
    hide_unused_rows(sdi, buf, prefix, visible);

    // Scrollbar.
    let name = sdi_key!(buf, "{prefix}scroll");
    if let Ok(obj) = sdi.get_mut(name) {
        if entries.len() > LIST_ROWS {
            let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
            let track_h = CONTENT_H as i32 - 16;
            let dot_y = LIST_Y + (ratio * track_h as f32) as i32;
            obj.x = panel_x + panel_w as i32 - 4;
            obj.y = dot_y;
            obj.w = 3;
            obj.h = 8;
            obj.color = at.scrollbar.thumb_color;
            obj.z = 105;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

// ===========================================================================
// Setup dispatcher (called on view transition)
// ===========================================================================

/// Set up SDI objects for the given view.  Idempotent -- safe to call every
/// time a view is entered.
pub(crate) fn setup_view(sdi: &mut SdiRegistry, view: ClassicView) {
    match view {
        ClassicView::Radio => setup_radio(sdi),
        ClassicView::TvGuide => setup_tv_channels(sdi),
        ClassicView::PhotoViewer => {
            setup_photo_browser(sdi);
            setup_photo_view(sdi);
        },
        ClassicView::MusicPlayer => setup_music_browser(sdi),
        ClassicView::Browser => setup_browser(sdi),
        ClassicView::FileManager => setup_file_manager(sdi),
        // Dashboard and Terminal already have their own SDI setup.
        ClassicView::Dashboard | ClassicView::Terminal => {},
    }
}

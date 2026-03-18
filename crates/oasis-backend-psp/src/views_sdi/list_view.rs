//! Shared list-view background, header, scrollbar, and row helpers.

use oasis_backend_psp::{Color, SCREEN_WIDTH};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::helpers::{ensure, sdi_key, set_text};

/// Maximum rows visible in a list view.
pub(crate) const LIST_ROWS: usize = FM_VISIBLE_ROWS;
/// Row height for list items.
pub(crate) const ROW_H: i32 = FM_ROW_H;
/// Y start for list content (below header).
pub(crate) const LIST_Y: i32 = FM_START_Y;

/// Set up the common background + header objects for a list view.
pub(crate) fn setup_list_bg(sdi: &mut SdiRegistry, prefix: &str) {
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
pub(crate) fn update_list_bg(
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
pub(crate) fn update_scrollbar(
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
pub(crate) fn update_list_row(
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
pub(crate) fn hide_unused_rows(
    sdi: &mut SdiRegistry,
    buf: &mut String,
    prefix: &str,
    visible_count: usize,
) {
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

//! SDI view: File Manager (dual-panel list view).

use oasis_backend_psp::{Color, FileEntry, SCREEN_WIDTH};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::helpers::{ensure, set_text, sdi_key};
use super::list_view::{hide_unused_rows, setup_list_bg, LIST_ROWS, LIST_Y, ROW_H};

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
pub(crate) fn update_fm_panel(
    sdi: &mut SdiRegistry,
    mut buf: &mut String,
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

//! SDI view: Photo Browser + Photo full-screen view.

use oasis_backend_psp::{Color, FileEntry, TextureId, SCREEN_WIDTH};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::helpers::{ensure, set_text};
use super::list_view::{
    hide_unused_rows, setup_list_bg, update_list_bg, update_list_row, update_scrollbar, LIST_ROWS,
};

// ---- Photo Browser (list view) ----

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

// ---- Photo full-screen view ----

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

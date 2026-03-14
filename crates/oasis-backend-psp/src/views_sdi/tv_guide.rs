//! SDI view: TV Guide Channels (list view).

use oasis_backend_psp::{Color, SCREEN_WIDTH};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::helpers::{set_text, sdi_key};
use super::list_view::{
    hide_unused_rows, setup_list_bg, update_list_bg, update_scrollbar, LIST_ROWS, LIST_Y, ROW_H,
};

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

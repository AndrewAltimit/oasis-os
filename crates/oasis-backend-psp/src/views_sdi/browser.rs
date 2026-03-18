//! SDI view: Browser (list view with URL bar).

use oasis_backend_psp::SCREEN_WIDTH;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::helpers::{ensure, sdi_key, set_text};
use super::list_view::{LIST_ROWS, LIST_Y, setup_list_bg, update_list_bg};

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

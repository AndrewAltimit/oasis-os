//! SDI view: Music Browser (list view).

use oasis_backend_psp::{Color, FileEntry};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;

use super::list_view::{
    hide_unused_rows, setup_list_bg, update_list_bg, update_list_row, update_scrollbar, LIST_ROWS,
};

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

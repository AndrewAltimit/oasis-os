//! SDI view: Settings (theme picker).
//!
//! Resolution is fixed at 480x272 on PSP, so the only setting exposed is
//! the active theme. Mirrors the desktop Settings app's "Display" panel
//! but without the resolution preset list.

use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::skins::PspSkinPreset;

use super::list_view::{
    LIST_ROWS, hide_unused_rows, setup_list_bg, update_list_bg, update_list_row, update_scrollbar,
};

/// Create SDI objects for the Settings theme list.
pub(crate) fn setup_settings(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "settings_");
}

/// Update SDI objects for the Settings theme list each frame.
///
/// `current` is the currently active preset; the row matching it gets a
/// `[*]` marker so the user can see what's applied (vs. just hovered).
pub(crate) fn update_settings(
    sdi: &mut SdiRegistry,
    selected: usize,
    scroll: usize,
    current: PspSkinPreset,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(
        sdi,
        &mut buf,
        "settings_",
        "THEME",
        accent,
        at,
        Some("X = Apply, O = Back"),
    );

    let total = PspSkinPreset::ALL.len();
    let end = (scroll + LIST_ROWS).min(total);
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let preset = PspSkinPreset::ALL[i];
        let is_active = preset == current;
        let icon = if is_active { "[*]" } else { "[ ]" };
        update_list_row(
            sdi,
            &mut buf,
            "settings_",
            row,
            i == selected,
            icon,
            accent,
            preset.name(),
            at.app.text,
            None,
            None,
            oasis_core::color::with_alpha(accent, 100),
            at,
        );
    }
    hide_unused_rows(sdi, &mut buf, "settings_", visible);
    update_scrollbar(sdi, &mut buf, "settings_", selected, total, at);
}

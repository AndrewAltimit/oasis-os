//! Top-level navigation tabs (APPS / MODS / NET) below the status bar.

use oasis_types::backend::TextureId;

use super::text_px;
use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_pill, ensure_text, hide_objects};

/// Top-level tabs (cycled with L trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopTab {
    /// Main dashboard (app grid).
    Apps,
    /// Module / plugin manager.
    Mods,
    /// Network status.
    Net,
}

impl TopTab {
    /// Cycle to the next tab.
    pub fn next(self) -> Self {
        match self {
            Self::Apps => Self::Mods,
            Self::Mods => Self::Net,
            Self::Net => Self::Apps,
        }
    }

    /// Display label for the tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => "APPS",
            Self::Mods => "MODS",
            Self::Net => "NET",
        }
    }

    /// All tabs in order.
    pub const ALL: &[TopTab] = &[TopTab::Apps, TopTab::Mods, TopTab::Net];
}

/// Pre-built SDI object names for the top tab row (avoids per-frame
/// `format!` allocations — indices match `TopTab::ALL`).
const TAB_TEXT_NAMES: [&str; 3] = ["bar_tab_0", "bar_tab_1", "bar_tab_2"];
const TAB_BG_NAMES: [&str; 3] = ["bar_tab_bg_0", "bar_tab_bg_1", "bar_tab_bg_2"];
/// Legacy 4-edge tab border objects (older skins may still define them in
/// layout.toml; kept hidden every frame).
const TAB_EDGE_NAMES: [[&str; 4]; 3] = [
    [
        "bar_tab_bt_0",
        "bar_tab_bb_0",
        "bar_tab_bl_0",
        "bar_tab_br_0",
    ],
    [
        "bar_tab_bt_1",
        "bar_tab_bb_1",
        "bar_tab_bl_1",
        "bar_tab_br_1",
    ],
    [
        "bar_tab_bt_2",
        "bar_tab_bb_2",
        "bar_tab_bl_2",
        "bar_tab_br_2",
    ],
];

/// Pill textures for the tab row (from the skin's `bar_overrides`).
#[derive(Debug, Clone, Copy)]
pub(super) struct TabTextures {
    pub active: Option<TextureId>,
    pub inactive: Option<TextureId>,
}

/// Synchronize the tab row's SDI objects (`visible = false` hides the row).
pub(super) fn update_tab_row(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    visible: bool,
    active_tab: TopTab,
    textures: TabTextures,
) {
    let font_small = at.font_small;
    let tab_y = at.statusbar_height as i32;
    for (i, tab) in TopTab::ALL.iter().enumerate() {
        let name = TAB_TEXT_NAMES[i];
        let bg_name = TAB_BG_NAMES[i];

        if !visible {
            hide_objects(sdi, &[name, bg_name]);
            continue;
        }

        let x = at.tab_start_x + (i as i32) * (at.tab_w + at.tab_gap);
        let tw = at.tab_w as u32;
        let th = at.tab_h as u32;
        let is_active = *tab == active_tab;

        // Hide legacy 4-edge border objects.
        hide_objects(sdi, &TAB_EDGE_NAMES[i]);

        // Single pill tab background (replaces fill + 4 borders). Inactive
        // tabs use the skin's (usually transparent) fill and a dim stroke.
        let (fill, stroke) = if is_active {
            (at.bar.tab_active_fill, at.bar.tab_active_stroke)
        } else {
            (at.bar.tab_inactive_fill, at.bar.tab_inactive_stroke)
        };
        ensure_pill(sdi, bg_name, x, tab_y, tw, th, fill, stroke);

        // Tab pill texture slot: shaped tab chrome from the skin's
        // `tab_texture_active` / `tab_texture_inactive` assets. Assigned
        // every frame so tab switches swap textures; a missing state
        // texture clears back to the plain pill fill. Skins without
        // either slot are left alone (a layout.toml `texture =` on the
        // pill object must not be clobbered).
        if (textures.active.is_some() || textures.inactive.is_some())
            && let Ok(obj) = sdi.get_mut(bg_name)
        {
            obj.texture = if is_active {
                textures.active
            } else {
                textures.inactive
            };
        }

        // Tab text (centered in tab).
        let tx = x + (at.tab_w - text_px(tab.label(), font_small)) / 2;
        let tab_text_y = tab_y + (at.tab_h - font_small as i32) / 2;
        ensure_text(
            sdi,
            name,
            tx.max(x + 2),
            tab_text_y,
            font_small,
            at.bar.tab_text_inactive,
        );
        if let Ok(obj) = sdi.get_mut(name) {
            obj.set_text(tab.label());
            obj.text_color = if is_active {
                at.bar.tab_text_active
            } else {
                at.bar.tab_text_inactive
            };
        }
    }
}

/// Hide every tab-row SDI object (including the legacy edge borders).
pub(super) fn hide_tab_row(sdi: &mut SdiRegistry) {
    hide_objects(sdi, &TAB_TEXT_NAMES);
    hide_objects(sdi, &TAB_BG_NAMES);
    for edges in &TAB_EDGE_NAMES {
        hide_objects(sdi, edges);
    }
}

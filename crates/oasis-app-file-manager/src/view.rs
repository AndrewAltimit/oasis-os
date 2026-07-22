//! Rendering and SDI scene-update logic for the File Manager app.
//!
//! Both presentation modes (`ViewMode::Dual` and `ViewMode::Explorer`)
//! have a windowed direct-draw path (`draw_windowed_*`) and an SDI
//! pooled-object path (`update_sdi_*`) here. Hit-testing helpers
//! (`tree_hit_test`, `grid_hit_test`, `compute_explorer_geom`) live
//! alongside the renderer that defines the geometry, since callers in
//! [`crate::commands`] must agree byte-for-byte on the layout.

use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{BatchRect, BatchText, Color, SdiBackend};
use oasis_ui::flex;
use oasis_ui::menu_bar::{MenuBar, MenuEntry, MenuStyle};

use crate::colors::FileManagerColors;
use crate::model::{EntryKind, TreeEntry, parse_entry, truncate_label};
use crate::state::FileManagerApp;

/// Pixel height reserved for the file-manager menu bar (matches the
/// dimensions used by Notepad's `MenuBar` rendering).
pub(crate) const FM_MENU_H: u32 = 18;
/// Pixel height of the address bar in Explorer view.
pub(crate) const FM_ADDR_H: u32 = 18;
/// Pixel height of the status strip in Explorer view.
pub(crate) const FM_STATUS_H: u32 = 14;

/// Maximum number of icon tiles allocated as SDI objects in Explorer view.
pub(crate) const MAX_TILES: usize = 48;
/// Maximum number of folder-tree lines.
pub(crate) const MAX_TREE_LINES: usize = 16;

/// Maximum number of menu labels rendered to SDI (matches `MenuBar::menus`
/// length used by the file manager).
const FM_MENU_MAX_LABELS: usize = 6;
/// Maximum number of dropdown rows pooled as SDI objects.
const FM_DROPDOWN_MAX_ROWS: usize = 8;

// Re-exported for use in `lib.rs` tests that need to mirror the renderer's
// hit-test geometry exactly.
pub(crate) use private::{
    Box2d, TextStyle, compute_explorer_geom, draw_outline, ensure_rect, ensure_text, grid_hit_test,
    hide_dual_panel_sdi, hide_explorer_sdi, hide_menu_sdi, outline_rect, push_icon, tree_hit_test,
    update_menu_bar_sdi, update_menu_dropdown_sdi,
};

mod private {
    use super::*;

    /// Geometry for the Explorer view, computed from a content rect.
    pub(crate) struct ExplorerGeom {
        pub menu_x: i32,
        pub menu_y: i32,
        pub menu_w: u32,
        pub menu_h: u32,
        pub addr_y: i32,
        pub addr_h: u32,
        pub body_y: i32,
        pub body_h: u32,
        pub tree_x: i32,
        pub tree_w: u32,
        pub grid_x: i32,
        pub grid_w: u32,
        pub status_y: i32,
        pub status_h: u32,
        pub tile_w: u32,
        pub tile_h: u32,
        pub icon_w: u32,
        pub icon_h: u32,
        pub cols: usize,
        pub rows: usize,
    }

    pub(crate) fn compute_explorer_geom(cx: i32, cy: i32, cw: u32, ch: u32) -> ExplorerGeom {
        let menu_h = FM_MENU_H;
        let addr_h = FM_ADDR_H;
        let status_h = FM_STATUS_H;
        let pad = 4i32;

        let menu_x = cx;
        let menu_y = cy;
        let menu_w = cw;
        let addr_y = menu_y + menu_h as i32;
        let body_y = addr_y + addr_h as i32 + pad;
        let status_y = cy + ch as i32 - status_h as i32;
        let body_h = (status_y - body_y).max(20) as u32;

        // Tree pane: ~28% width, clamped to keep both panes usable.
        let tree_w_target = ((cw as f32) * 0.28) as u32;
        let tree_w = tree_w_target.clamp(80, 200).min(cw.saturating_sub(120));
        let tree_x = cx + pad;

        let grid_x = cx + tree_w as i32 + pad;
        let grid_w = (cw as i32 - tree_w as i32 - pad * 2).max(60) as u32;

        let tile_w = 64u32.min(grid_w / 2).max(48);
        let tile_h = 56u32;
        let icon_w = 28u32;
        let icon_h = 24u32;

        let cols = ((grid_w.saturating_sub(8) / tile_w) as usize).max(1);
        let rows = ((body_h.saturating_sub(8) / tile_h) as usize).max(1);

        ExplorerGeom {
            menu_x,
            menu_y,
            menu_w,
            menu_h,
            addr_y,
            addr_h,
            body_y,
            body_h,
            tree_x,
            tree_w,
            grid_x,
            grid_w,
            status_y,
            status_h,
            tile_w,
            tile_h,
            icon_w,
            icon_h,
            cols,
            rows,
        }
    }

    /// Hit-test the folder-tree pane in Explorer view. Returns the absolute
    /// path of the clicked row, if any. `font_hint` must match the value used
    /// by the renderer so the row metric is identical.
    pub(crate) fn tree_hit_test(
        g: &ExplorerGeom,
        lx: i32,
        ly: i32,
        tree_entries: &[TreeEntry],
        font_hint: u16,
    ) -> Option<String> {
        if lx < g.tree_x || lx >= g.tree_x + g.tree_w as i32 {
            return None;
        }
        if ly < g.body_y + 4 || ly >= g.body_y + g.body_h as i32 - 4 {
            return None;
        }
        let line_h = (font_hint as i32 + 2).max(11);
        let row = ((ly - (g.body_y + 4)) / line_h) as usize;
        tree_entries.get(row).map(|e| e.path.clone())
    }

    /// Hit-test the icon grid. Returns the absolute index into `lines` (i.e.
    /// `panel.scroll + tile_index`) when the click lands on a populated tile.
    pub(crate) fn grid_hit_test(
        g: &ExplorerGeom,
        lx: i32,
        ly: i32,
        lines: &[String],
        scroll: usize,
    ) -> Option<usize> {
        if lx < g.grid_x + 2 || lx >= g.grid_x + g.grid_w as i32 - 2 {
            return None;
        }
        if ly < g.body_y + 2 || ly >= g.body_y + g.body_h as i32 - 2 {
            return None;
        }
        let col = ((lx - (g.grid_x + 4)) / g.tile_w.max(1) as i32).max(0) as usize;
        let row = ((ly - (g.body_y + 4)) / g.tile_h.max(1) as i32).max(0) as usize;
        if col >= g.cols || row >= g.rows {
            return None;
        }
        let idx = row * g.cols.max(1) + col;
        let abs = scroll + idx;
        if abs >= lines.len() {
            return None;
        }
        Some(abs)
    }

    /// Rectangle in screen coordinates, used by Explorer-view SDI helpers.
    #[derive(Clone, Copy)]
    pub(crate) struct Box2d {
        pub x: i32,
        pub y: i32,
        pub w: u32,
        pub h: u32,
    }

    pub(crate) fn ensure_rect(sdi: &mut SdiRegistry, name: &str, b: Box2d, color: Color, z: i32) {
        if !sdi.contains(name) {
            sdi.create(name);
        }
        if let Ok(obj) = sdi.get_mut(name) {
            obj.x = b.x;
            obj.y = b.y;
            obj.w = b.w;
            obj.h = b.h;
            obj.color = color;
            obj.text = None;
            obj.stroke_width = None;
            obj.stroke_color = None;
            obj.visible = true;
            obj.z = z;
        }
    }

    /// Compact text style used by `ensure_text` to keep the helper's argument
    /// count under clippy's `too_many_arguments` limit.
    #[derive(Clone, Copy)]
    pub(crate) struct TextStyle {
        pub font_size: u16,
        pub color: Color,
        pub z: i32,
    }

    pub(crate) fn ensure_text(
        sdi: &mut SdiRegistry,
        name: &str,
        text: &str,
        x: i32,
        y: i32,
        style: TextStyle,
    ) {
        if !sdi.contains(name) {
            sdi.create(name);
        }
        if let Ok(obj) = sdi.get_mut(name) {
            obj.text = Some(text.to_string());
            obj.x = x;
            obj.y = y;
            obj.font_size = style.font_size;
            obj.text_color = style.color;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = style.z;
        }
    }

    pub(crate) fn outline_rect(sdi: &mut SdiRegistry, base: &str, b: Box2d, color: Color, z: i32) {
        let Box2d { x, y, w, h } = b;
        ensure_rect(sdi, &format!("{base}_t"), Box2d { x, y, w, h: 1 }, color, z);
        ensure_rect(
            sdi,
            &format!("{base}_b"),
            Box2d {
                x,
                y: y + h as i32 - 1,
                w,
                h: 1,
            },
            color,
            z,
        );
        ensure_rect(sdi, &format!("{base}_l"), Box2d { x, y, w: 1, h }, color, z);
        ensure_rect(
            sdi,
            &format!("{base}_r"),
            Box2d {
                x: x + w as i32 - 1,
                y,
                w: 1,
                h,
            },
            color,
            z,
        );
    }

    pub(crate) fn draw_outline(
        backend: &mut dyn SdiBackend,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    ) -> oasis_types::error::Result<()> {
        backend.fill_rect(x, y, w, 1, color)?;
        backend.fill_rect(x, y + h as i32 - 1, w, 1, color)?;
        backend.fill_rect(x, y, 1, h, color)?;
        backend.fill_rect(x + w as i32 - 1, y, 1, h, color)?;
        Ok(())
    }

    /// Push a 1-pixel-wide rectangle outline into a batch. Mirror of
    /// [`draw_outline`] for callers that build a `Vec<BatchRect>` and
    /// submit it via [`SdiBatch::submit_rect_batch`].
    pub(crate) fn push_outline(
        batch: &mut Vec<BatchRect>,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    ) {
        batch.push(BatchRect {
            x,
            y,
            w,
            h: 1,
            color,
        });
        batch.push(BatchRect {
            x,
            y: y + h as i32 - 1,
            w,
            h: 1,
            color,
        });
        batch.push(BatchRect {
            x,
            y,
            w: 1,
            h,
            color,
        });
        batch.push(BatchRect {
            x: x + w as i32 - 1,
            y,
            w: 1,
            h,
            color,
        });
    }

    /// Push the rectangles for an Explorer-view file/folder icon into a
    /// batch. The Explorer-view tile loop submits all per-tile geometry
    /// via `SdiBatch::submit_rect_batch` so the WASM backend can collapse
    /// ~336 individual `fill_rect` round-trips into a single one.
    pub(crate) fn push_icon(
        batch: &mut Vec<BatchRect>,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        kind: EntryKind,
        colors: &FileManagerColors,
    ) {
        match kind {
            EntryKind::Dir | EntryKind::ParentDir => {
                batch.push(BatchRect {
                    x,
                    y,
                    w,
                    h,
                    color: colors.folder_icon,
                });
                batch.push(BatchRect {
                    x,
                    y,
                    w: w / 2,
                    h: 4,
                    color: colors.folder_icon_tab,
                });
            },
            EntryKind::File => {
                batch.push(BatchRect {
                    x,
                    y,
                    w,
                    h,
                    color: colors.file_icon,
                });
                let fold = 6u32;
                batch.push(BatchRect {
                    x: x + w as i32 - fold as i32,
                    y,
                    w: fold,
                    h: fold,
                    color: colors.file_icon_fold,
                });
            },
        }
        push_outline(batch, x, y, w, h, colors.divider);
    }

    /// Hide all Explorer-view SDI objects.
    pub(crate) fn hide_explorer_sdi(sdi: &mut SdiRegistry) {
        let fixed = [
            "app_xp_addr_bg",
            "app_xp_addr_label",
            "app_xp_addr_field",
            "app_xp_addr_o_t",
            "app_xp_addr_o_b",
            "app_xp_addr_o_l",
            "app_xp_addr_o_r",
            "app_xp_addr_text",
            "app_xp_tree_bg",
            "app_xp_tree_o_t",
            "app_xp_tree_o_b",
            "app_xp_tree_o_l",
            "app_xp_tree_o_r",
            "app_xp_tree_sel",
            "app_xp_grid_bg",
            "app_xp_grid_o_t",
            "app_xp_grid_o_b",
            "app_xp_grid_o_l",
            "app_xp_grid_o_r",
            "app_xp_status_bg",
            "app_xp_status_text",
            "app_xp_status_hint",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..MAX_TREE_LINES {
            if let Ok(obj) = sdi.get_mut(&format!("app_xp_tree_l_{i}")) {
                obj.visible = false;
            }
        }
        for i in 0..MAX_TILES {
            for prefix in [
                "app_xp_t_sel_",
                "app_xp_t_body_",
                "app_xp_t_accent_",
                "app_xp_t_lbl_",
            ] {
                if let Ok(obj) = sdi.get_mut(&format!("{prefix}{i}")) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Render the menu bar to SDI by mirroring `MenuBar::draw_bar` over named
    /// scene-graph objects. Pools labels/highlight rects per slot so we can
    /// hide unused ones each frame instead of churning the registry.
    pub(crate) fn update_menu_bar_sdi(
        sdi: &mut SdiRegistry,
        bar: &MenuBar,
        bar_x: i32,
        bar_y: i32,
        bar_w: u32,
        bar_h: u32,
        style: &MenuStyle,
    ) {
        ensure_rect(
            sdi,
            "app_fm_menubar_bg",
            Box2d {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
            },
            style.bar_bg,
            108,
        );
        ensure_rect(
            sdi,
            "app_fm_menubar_border",
            Box2d {
                x: bar_x,
                y: bar_y + bar_h as i32 - 1,
                w: bar_w,
                h: 1,
            },
            style.bar_border,
            109,
        );

        let mut cursor = 6i32;
        for i in 0..FM_MENU_MAX_LABELS {
            let hot_name = format!("app_fm_menubar_hot_{i}");
            let label_name = format!("app_fm_menubar_lbl_{i}");
            let Some(menu) = bar.menus.get(i) else {
                if let Ok(obj) = sdi.get_mut(&hot_name) {
                    obj.visible = false;
                }
                if let Ok(obj) = sdi.get_mut(&label_name) {
                    obj.visible = false;
                }
                continue;
            };
            let label_w = menu.label.chars().count() as i32 * 7 + 16;
            let is_open = bar.open == Some(i);
            if !sdi.contains(&hot_name) {
                sdi.create(&hot_name);
            }
            if let Ok(obj) = sdi.get_mut(&hot_name) {
                obj.x = bar_x + cursor;
                obj.y = bar_y + 2;
                obj.w = label_w as u32;
                obj.h = bar_h.saturating_sub(4);
                obj.color = style.label_hot_bg;
                obj.visible = is_open;
                obj.text = None;
                obj.stroke_width = None;
                obj.z = 109;
            }
            let text_color = if is_open {
                style.label_hot_text
            } else {
                style.label_text
            };
            ensure_text(
                sdi,
                &label_name,
                &menu.label,
                bar_x + cursor + 8,
                bar_y + (bar_h as i32 - style.font_size as i32) / 2,
                TextStyle {
                    font_size: style.font_size,
                    color: text_color,
                    z: 110,
                },
            );
            cursor += label_w;
        }
    }

    /// Render the open menu's drop-down to SDI; hides pooled objects when no
    /// menu is open.
    pub(crate) fn update_menu_dropdown_sdi(
        sdi: &mut SdiRegistry,
        bar: &MenuBar,
        bar_x: i32,
        bar_y: i32,
        bar_h: u32,
        style: &MenuStyle,
    ) {
        for name in [
            "app_fm_dd_bg",
            "app_fm_dd_border_l_t",
            "app_fm_dd_border_l_l",
            "app_fm_dd_border_d_b",
            "app_fm_dd_border_d_r",
        ] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..FM_DROPDOWN_MAX_ROWS {
            for kind in ["hot", "text", "shortcut", "sep"] {
                if let Ok(obj) = sdi.get_mut(&format!("app_fm_dd_{kind}_{i}")) {
                    obj.visible = false;
                }
            }
        }
        let Some(idx) = bar.open else {
            return;
        };
        let Some(menu) = bar.menus.get(idx) else {
            return;
        };

        // Anchor x to the open label.
        let mut label_x = 6i32;
        for prev in 0..idx {
            label_x += bar.menus[prev].label.chars().count() as i32 * 7 + 16;
        }
        let dd_x = bar_x + label_x;
        let dd_y = bar_y + bar_h as i32;
        let (dd_w, dd_h) = bar.dropdown_dimensions(menu);

        ensure_rect(
            sdi,
            "app_fm_dd_bg",
            Box2d {
                x: dd_x,
                y: dd_y,
                w: dd_w,
                h: dd_h,
            },
            style.dropdown_bg,
            150,
        );
        ensure_rect(
            sdi,
            "app_fm_dd_border_l_t",
            Box2d {
                x: dd_x,
                y: dd_y,
                w: dd_w,
                h: 1,
            },
            style.dropdown_border_light,
            151,
        );
        ensure_rect(
            sdi,
            "app_fm_dd_border_l_l",
            Box2d {
                x: dd_x,
                y: dd_y,
                w: 1,
                h: dd_h,
            },
            style.dropdown_border_light,
            151,
        );
        ensure_rect(
            sdi,
            "app_fm_dd_border_d_b",
            Box2d {
                x: dd_x,
                y: dd_y + dd_h as i32 - 1,
                w: dd_w,
                h: 1,
            },
            style.dropdown_border_dark,
            151,
        );
        ensure_rect(
            sdi,
            "app_fm_dd_border_d_r",
            Box2d {
                x: dd_x + dd_w as i32 - 1,
                y: dd_y,
                w: 1,
                h: dd_h,
            },
            style.dropdown_border_dark,
            151,
        );

        let mut item_y = dd_y + 4;
        for (i, entry) in menu.entries.iter().enumerate().take(FM_DROPDOWN_MAX_ROWS) {
            match entry {
                MenuEntry::Action {
                    label,
                    shortcut,
                    enabled,
                    ..
                } => {
                    let hot = bar.hovered_item == Some(i) && *enabled;
                    let hot_name = format!("app_fm_dd_hot_{i}");
                    if !sdi.contains(&hot_name) {
                        sdi.create(&hot_name);
                    }
                    if let Ok(obj) = sdi.get_mut(&hot_name) {
                        obj.x = dd_x + 2;
                        obj.y = item_y;
                        obj.w = dd_w.saturating_sub(4);
                        obj.h = 20;
                        obj.color = style.item_hot_bg;
                        obj.text = None;
                        obj.stroke_width = None;
                        obj.visible = hot;
                        obj.z = 152;
                    }
                    let color = if !*enabled {
                        style.item_disabled_text
                    } else if hot {
                        style.item_hot_text
                    } else {
                        style.item_text
                    };
                    ensure_text(
                        sdi,
                        &format!("app_fm_dd_text_{i}"),
                        label,
                        dd_x + 22,
                        item_y + 4,
                        TextStyle {
                            font_size: style.font_size,
                            color,
                            z: 153,
                        },
                    );
                    if let Some(sc) = shortcut {
                        let sc_w = sc.chars().count() as i32 * 7;
                        ensure_text(
                            sdi,
                            &format!("app_fm_dd_shortcut_{i}"),
                            sc,
                            dd_x + dd_w as i32 - sc_w - 22,
                            item_y + 4,
                            TextStyle {
                                font_size: style.font_size,
                                color,
                                z: 153,
                            },
                        );
                    }
                    item_y += 20;
                },
                MenuEntry::Separator => {
                    ensure_rect(
                        sdi,
                        &format!("app_fm_dd_sep_{i}"),
                        Box2d {
                            x: dd_x + 4,
                            y: item_y + 3,
                            w: dd_w - 8,
                            h: 1,
                        },
                        style.separator,
                        152,
                    );
                    item_y += 6;
                },
            }
        }
    }

    /// Hide all menu-bar SDI objects (called when the file manager is hidden).
    pub(crate) fn hide_menu_sdi(sdi: &mut SdiRegistry) {
        for name in ["app_fm_menubar_bg", "app_fm_menubar_border"] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..FM_MENU_MAX_LABELS {
            for kind in ["hot", "lbl"] {
                if let Ok(obj) = sdi.get_mut(&format!("app_fm_menubar_{kind}_{i}")) {
                    obj.visible = false;
                }
            }
        }
        for name in [
            "app_fm_dd_bg",
            "app_fm_dd_border_l_t",
            "app_fm_dd_border_l_l",
            "app_fm_dd_border_d_b",
            "app_fm_dd_border_d_r",
        ] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..FM_DROPDOWN_MAX_ROWS {
            for kind in ["hot", "text", "shortcut", "sep"] {
                if let Ok(obj) = sdi.get_mut(&format!("app_fm_dd_{kind}_{i}")) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Hide dual-panel SDI objects (used when switching to Explorer view).
    pub(crate) fn hide_dual_panel_sdi(sdi: &mut SdiRegistry) {
        if let Ok(obj) = sdi.get_mut("app_divider") {
            obj.visible = false;
        }
        for i in 0..100 {
            let lp = format!("app_lp_line_{i}");
            if !sdi.contains(&lp) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&lp) {
                obj.visible = false;
            }
            if let Ok(obj) = sdi.get_mut(&format!("app_rp_line_{i}")) {
                obj.visible = false;
            }
        }
    }
}

impl FileManagerApp {
    /// Draw dual-panel layout to backend (windowed mode).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_windowed_dual(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = FileManagerColors::from_theme(at);
        let half_w = (cw / 2).saturating_sub(1);
        let divider_x = cx + half_w as i32;

        // Context header row: panel paths only. The WM titlebar already
        // shows "File Manager", so the app title is not repeated here. The
        // row keeps `title_bar_height` so `handle_click`'s dual-mode hit
        // math is unchanged.
        let header = format!(
            "[L: {}]  [R: {}]",
            self.panels[0].browse_dir, self.panels[1].browse_dir,
        );
        backend.draw_text(&header, cx + 4, cy + 2, 12, colors.dim_text)?;
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            colors.divider,
        )?;

        // Menu bar below the title bar.
        let title_h = at.app.title_bar_height as i32;
        let menu_y = cy + title_h;
        let menu_style = MenuStyle::from_theme(&at.ui_theme);
        self.menu
            .draw_bar(backend, cx, menu_y, cw, FM_MENU_H, &menu_style)?;

        // Vertical divider below the menu strip.
        let content_y = menu_y + FM_MENU_H as i32;
        let content_h = ch.saturating_sub(title_h as u32 + FM_MENU_H + 14);
        backend.fill_rect(divider_x, content_y, 1, content_h, colors.divider)?;

        // Draw each panel.
        let line_h = at.terminal_line_height.max(12) as i32;
        let max_lines = ((content_h as i32) / line_h).max(0) as usize;
        for (pi, panel) in self.panels.iter().enumerate() {
            let px = if pi == 0 { cx } else { divider_x + 1 };
            let pw = if pi == 0 { half_w } else { cw - half_w - 1 };
            let is_active = pi == self.active_panel;

            if is_active {
                backend.fill_rect(px, content_y, pw, 1, colors.selected_text)?;
            }

            let visible = panel
                .lines
                .len()
                .saturating_sub(panel.scroll)
                .min(max_lines);
            for i in 0..visible {
                let line_idx = panel.scroll + i;
                let line = &panel.lines[line_idx];
                let prefix = if is_active && i == panel.cursor {
                    "> "
                } else {
                    "  "
                };
                let max_chars = (pw as usize / 8).saturating_sub(2);
                let display = if line.len() > max_chars {
                    &line[..line.floor_char_boundary(max_chars)]
                } else {
                    line.as_str()
                };
                let text = format!("{prefix}{display}");
                let text_color = if is_active && i == panel.cursor {
                    colors.selected_text
                } else {
                    colors.text
                };
                let y = content_y + 2 + i as i32 * line_h;
                backend.draw_text(&text, px + 2, y, 12, text_color)?;
            }
        }

        let scroll_y = cy + ch as i32 - 14;
        backend.draw_text(
            "L/R=panel  \u{25b3}=del  \u{25a1}=mkdir  View>Grid  Cancel=back",
            cx + 4,
            scroll_y,
            10,
            colors.dim_text,
        )?;

        // Drop-down floats above the rest of the windowed content.
        if self.menu.is_open() {
            self.menu
                .draw_dropdown(backend, cx, menu_y, FM_MENU_H, &menu_style)?;
        }

        Ok(())
    }

    /// Render dual-panel to SDI objects.
    pub(crate) fn update_sdi_dual(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let colors = FileManagerColors::from_theme(at);
        // Title with both panel paths.
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            obj.text = Some(format!(
                "File Manager  [L: {}]  [R: {}]",
                self.panels[0].browse_dir, self.panels[1].browse_dir,
            ));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = colors.title_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        // Responsive dual-panel geometry. Reserve a row below the title
        // bar for the menu strip so it sits between title and panels.
        let title_h = at.app.title_bar_height;
        let menu_y = title_h as i32;
        let content_y = (title_h + FM_MENU_H + 4) as i32;
        let half_w = at.screen_w / 2;
        let panel_pad = 8u32;
        let divider_x = half_w as i32;
        let left_x = 8i32;
        let left_w = half_w - panel_pad - left_x as u32;
        let right_x = divider_x + panel_pad as i32;
        let right_w = at.screen_w - right_x as u32 - panel_pad;
        let divider_h = at
            .screen_h
            .saturating_sub(title_h + FM_MENU_H + at.statusbar_height + at.bottombar_height);
        let usable_h = at
            .screen_h
            .saturating_sub(title_h + FM_MENU_H + at.statusbar_height + at.bottombar_height + 14);
        let panel_visible = (usable_h / at.terminal_line_height.max(1)).max(1) as usize;

        let menu_style = MenuStyle::from_theme(&at.ui_theme);
        update_menu_bar_sdi(
            sdi,
            &self.menu,
            0,
            menu_y,
            at.screen_w,
            FM_MENU_H,
            &menu_style,
        );

        // Vertical divider.
        if !sdi.contains("app_divider") {
            sdi.create("app_divider");
        }
        if let Ok(obj) = sdi.get_mut("app_divider") {
            obj.x = divider_x;
            obj.y = content_y - 2;
            obj.w = 1;
            obj.h = divider_h;
            obj.color = colors.divider;
            obj.visible = true;
            obj.z = 102;
        }

        // Left panel lines.
        let lp_rects = flex::vertical_list(
            left_x,
            content_y,
            left_w,
            at.terminal_line_height,
            0,
            panel_visible,
        );
        for (i, rect) in lp_rects.iter().enumerate() {
            let name = format!("app_lp_line_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let p = &self.panels[0];
                let line_idx = p.scroll + i;
                let is_active = self.active_panel == 0;
                if line_idx < p.lines.len() {
                    obj.text = Some(p.lines[line_idx].clone());
                    obj.visible = true;
                } else {
                    obj.text = None;
                    obj.visible = false;
                }
                obj.x = rect.x + 6;
                obj.y = rect.y;
                obj.font_size = at.font_body;
                obj.text_color = if is_active && i == p.cursor {
                    colors.selected_text
                } else {
                    colors.text
                };
                obj.w = 0;
                obj.h = 0;
                obj.z = 102;
            }
        }

        // Right panel lines.
        let rp_rects = flex::vertical_list(
            right_x,
            content_y,
            right_w,
            at.terminal_line_height,
            0,
            panel_visible,
        );
        for (i, rect) in rp_rects.iter().enumerate() {
            let name = format!("app_rp_line_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let p = &self.panels[1];
                let line_idx = p.scroll + i;
                let is_active = self.active_panel == 1;
                if line_idx < p.lines.len() {
                    obj.text = Some(p.lines[line_idx].clone());
                    obj.visible = true;
                } else {
                    obj.text = None;
                    obj.visible = false;
                }
                obj.x = rect.x + 6;
                obj.y = rect.y;
                obj.font_size = at.font_body;
                obj.text_color = if is_active && i == p.cursor {
                    colors.selected_text
                } else {
                    colors.text
                };
                obj.w = 0;
                obj.h = 0;
                obj.z = 102;
            }
        }

        // Scroll indicator.
        if !sdi.contains("app_scroll") {
            sdi.create("app_scroll");
        }
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            obj.text =
                Some("L/R=panel  \u{25b3}=del  \u{25a1}=mkdir  View>Grid  Cancel=back".to_string());
            obj.x = 8;
            obj.y = at.screen_h as i32 - 14;
            obj.font_size = at.font_hint;
            obj.text_color = colors.dim_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        // Drop-down overlay (when an open menu exists).
        update_menu_dropdown_sdi(sdi, &self.menu, 0, menu_y, FM_MENU_H, &menu_style);

        // Hide single-panel lines.
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }

    /// Render Explorer view to SDI. Updates `explorer_cols`/`rows` cache.
    pub(crate) fn update_sdi_explorer(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let colors = FileManagerColors::from_theme(at);
        let panel = self.panels[self.active_panel].clone();
        // Title text reused from chrome.
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            obj.text = Some(format!("File Manager  -  {}", panel.browse_dir));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = colors.title_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }

        let title_h = at.app.title_bar_height as i32;
        let body_top = title_h;
        let body_h = at
            .screen_h
            .saturating_sub(at.app.title_bar_height + at.statusbar_height + at.bottombar_height);
        let g = compute_explorer_geom(0, body_top, at.screen_w, body_h);
        self.explorer_cols.set(g.cols);
        self.explorer_visible_rows.set(g.rows);
        self.cached_font_hint.set(at.font_hint);
        self.cached_system_bars
            .set(at.statusbar_height + at.bottombar_height);

        let dark = colors.divider;

        // Menu bar (shared MenuBar widget; click hits routed via App::handle_click).
        let menu_style = MenuStyle::from_theme(&at.ui_theme);
        update_menu_bar_sdi(
            sdi,
            &self.menu,
            g.menu_x,
            g.menu_y,
            g.menu_w,
            g.menu_h,
            &menu_style,
        );

        // Address bar.
        ensure_rect(
            sdi,
            "app_xp_addr_bg",
            Box2d {
                x: g.menu_x,
                y: g.addr_y,
                w: g.menu_w,
                h: g.addr_h,
            },
            colors.bg,
            104,
        );
        ensure_text(
            sdi,
            "app_xp_addr_label",
            "Address:",
            g.menu_x + 4,
            g.addr_y + 2,
            TextStyle {
                font_size: at.font_hint,
                color: colors.dim_text,
                z: 105,
            },
        );
        let addr_field_x = g.menu_x + 56;
        let addr_field_w = g.menu_w.saturating_sub(60);
        let addr_field = Box2d {
            x: addr_field_x,
            y: g.addr_y + 1,
            w: addr_field_w,
            h: g.addr_h - 2,
        };
        ensure_rect(sdi, "app_xp_addr_field", addr_field, colors.pane_bg, 105);
        outline_rect(sdi, "app_xp_addr_o", addr_field, dark, 106);
        ensure_text(
            sdi,
            "app_xp_addr_text",
            &panel.browse_dir,
            addr_field_x + 4,
            g.addr_y + 3,
            TextStyle {
                font_size: at.font_hint,
                color: colors.pane_text,
                z: 107,
            },
        );

        // Tree pane (sunken white pane).
        let tree_box = Box2d {
            x: g.tree_x,
            y: g.body_y,
            w: g.tree_w,
            h: g.body_h,
        };
        ensure_rect(sdi, "app_xp_tree_bg", tree_box, colors.pane_bg, 104);
        outline_rect(sdi, "app_xp_tree_o", tree_box, dark, 105);

        // Tree contents.
        let tree_entries = &panel.tree_entries;
        let tree_line_h = (at.font_hint as i32 + 2).max(11);
        for i in 0..MAX_TREE_LINES {
            let name = format!("app_xp_tree_l_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                if let Some(entry) = tree_entries.get(i) {
                    let y = g.body_y + 4 + i as i32 * tree_line_h;
                    if y + tree_line_h > g.body_y + g.body_h as i32 - 4 {
                        obj.visible = false;
                        continue;
                    }
                    let indent = entry.depth as i32 * 8;
                    obj.text = Some(entry.label.clone());
                    obj.x = g.tree_x + 4 + indent;
                    obj.y = y + 1;
                    obj.font_size = at.font_hint;
                    obj.text_color = if entry.is_current {
                        colors.selected_text
                    } else {
                        colors.pane_text
                    };
                    obj.w = 0;
                    obj.h = 0;
                    obj.visible = true;
                    obj.z = 106;
                } else {
                    obj.visible = false;
                }
            }
        }
        // Tree current-row highlight.
        if let Some(idx) = tree_entries.iter().position(|e| e.is_current) {
            let y = g.body_y + 4 + idx as i32 * tree_line_h;
            if y + tree_line_h <= g.body_y + g.body_h as i32 - 4 {
                ensure_rect(
                    sdi,
                    "app_xp_tree_sel",
                    Box2d {
                        x: g.tree_x + 2,
                        y,
                        w: g.tree_w.saturating_sub(4),
                        h: tree_line_h as u32,
                    },
                    colors.selected_bg,
                    105,
                );
            } else if let Ok(obj) = sdi.get_mut("app_xp_tree_sel") {
                obj.visible = false;
            }
        } else if let Ok(obj) = sdi.get_mut("app_xp_tree_sel") {
            obj.visible = false;
        }

        // Grid pane (sunken white).
        let grid_box = Box2d {
            x: g.grid_x,
            y: g.body_y,
            w: g.grid_w,
            h: g.body_h,
        };
        ensure_rect(sdi, "app_xp_grid_bg", grid_box, colors.pane_bg, 104);
        outline_rect(sdi, "app_xp_grid_o", grid_box, dark, 105);

        // Tiles.
        let count_visible = (g.cols * g.rows).min(MAX_TILES);
        for i in 0..MAX_TILES {
            let abs_idx = panel.scroll + i;
            let line = if i < count_visible {
                panel.lines.get(abs_idx)
            } else {
                None
            };
            let row = i.checked_div(g.cols).unwrap_or(0);
            let col = i.checked_rem(g.cols).unwrap_or(0);
            let tx = g.grid_x + 4 + col as i32 * g.tile_w as i32;
            let ty = g.body_y + 4 + row as i32 * g.tile_h as i32;
            let tile_visible = line.is_some()
                && tx + g.tile_w as i32 <= g.grid_x + g.grid_w as i32 - 2
                && ty + g.tile_h as i32 <= g.body_y + g.body_h as i32 - 2;
            let is_selected = abs_idx == panel.scroll + panel.cursor;

            // Selection background.
            let sel_name = format!("app_xp_t_sel_{i}");
            if !sdi.contains(&sel_name) {
                sdi.create(&sel_name);
            }
            if let Ok(obj) = sdi.get_mut(&sel_name) {
                obj.x = tx;
                obj.y = ty;
                obj.w = g.tile_w;
                obj.h = g.tile_h;
                obj.color = colors.selected_bg;
                obj.visible = tile_visible && is_selected;
                obj.z = 106;
            }

            let (name, kind) = if let Some(l) = line {
                parse_entry(l)
            } else {
                (String::new(), EntryKind::File)
            };

            // Icon body.
            let icon_x = tx + (g.tile_w as i32 - g.icon_w as i32) / 2;
            let icon_y = ty + 4;
            let body_color = match kind {
                EntryKind::Dir | EntryKind::ParentDir => colors.folder_icon,
                EntryKind::File => colors.file_icon,
            };
            let body_name = format!("app_xp_t_body_{i}");
            if !sdi.contains(&body_name) {
                sdi.create(&body_name);
            }
            if let Ok(obj) = sdi.get_mut(&body_name) {
                obj.x = icon_x;
                obj.y = icon_y;
                obj.w = g.icon_w;
                obj.h = g.icon_h;
                obj.color = body_color;
                obj.stroke_width = Some(1);
                obj.stroke_color = Some(dark);
                obj.visible = tile_visible;
                obj.z = 107;
            }

            // Icon accent (folder tab or page fold).
            let accent_name = format!("app_xp_t_accent_{i}");
            if !sdi.contains(&accent_name) {
                sdi.create(&accent_name);
            }
            if let Ok(obj) = sdi.get_mut(&accent_name) {
                match kind {
                    EntryKind::Dir | EntryKind::ParentDir => {
                        obj.x = icon_x;
                        obj.y = icon_y;
                        obj.w = g.icon_w / 2;
                        obj.h = 4;
                        obj.color = colors.folder_icon_tab;
                    },
                    EntryKind::File => {
                        let fold = 6u32;
                        obj.x = icon_x + g.icon_w as i32 - fold as i32;
                        obj.y = icon_y;
                        obj.w = fold;
                        obj.h = fold;
                        obj.color = colors.file_icon_fold;
                    },
                }
                obj.stroke_width = None;
                obj.stroke_color = None;
                obj.visible = tile_visible;
                obj.z = 108;
            }

            // Label.
            let label_name = format!("app_xp_t_lbl_{i}");
            if !sdi.contains(&label_name) {
                sdi.create(&label_name);
            }
            if let Ok(obj) = sdi.get_mut(&label_name) {
                let max_chars = (g.tile_w as usize / 7).max(4);
                obj.text = Some(truncate_label(&name, max_chars));
                obj.x = tx + 2;
                obj.y = icon_y + g.icon_h as i32 + 2;
                obj.font_size = at.font_hint;
                obj.text_color = if is_selected {
                    colors.selected_text
                } else {
                    colors.pane_text
                };
                obj.w = 0;
                obj.h = 0;
                obj.visible = tile_visible;
                obj.z = 108;
            }
        }

        // Status bar.
        let count = panel.lines.iter().filter(|l| l.trim() != "..").count();
        ensure_rect(
            sdi,
            "app_xp_status_bg",
            Box2d {
                x: g.menu_x,
                y: g.status_y,
                w: g.menu_w,
                h: g.status_h,
            },
            colors.status_bg,
            104,
        );
        let status_style = TextStyle {
            font_size: at.font_hint,
            color: colors.status_text,
            z: 105,
        };
        ensure_text(
            sdi,
            "app_xp_status_text",
            &format!("{count} object(s)"),
            g.menu_x + 4,
            g.status_y + 1,
            status_style,
        );
        ensure_text(
            sdi,
            "app_xp_status_hint",
            "View>List  Cancel=back",
            g.menu_x + g.menu_w as i32 - 180,
            g.status_y + 1,
            status_style,
        );

        // Hide the scroll/divider used by other modes.
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut("app_divider") {
            obj.visible = false;
        }

        // Drop-down overlay last so it floats above the icon grid.
        update_menu_dropdown_sdi(sdi, &self.menu, g.menu_x, g.menu_y, g.menu_h, &menu_style);
    }

    /// Direct-draw Explorer view in windowed mode.
    pub(crate) fn draw_windowed_explorer(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = FileManagerColors::from_theme(at);
        let panel = self.active();
        // No inner title bar -- the WM titlebar already shows "File Manager"
        // and the address bar below shows the current path.
        let g = compute_explorer_geom(cx, cy, cw, ch);
        self.explorer_cols.set(g.cols);
        self.explorer_visible_rows.set(g.rows);
        self.cached_font_hint.set(at.font_hint);
        self.cached_system_bars
            .set(at.statusbar_height + at.bottombar_height);

        let dark = colors.divider;

        // Menu bar via the shared MenuBar widget.
        let menu_style = MenuStyle::from_theme(&at.ui_theme);
        self.menu
            .draw_bar(backend, g.menu_x, g.menu_y, g.menu_w, g.menu_h, &menu_style)?;

        // Address bar.
        backend.fill_rect(g.menu_x, g.addr_y, g.menu_w, g.addr_h, colors.bg)?;
        backend.draw_text(
            "Address:",
            g.menu_x + 4,
            g.addr_y + 2,
            at.font_hint,
            colors.dim_text,
        )?;
        let addr_field_x = g.menu_x + 56;
        let addr_field_w = g.menu_w.saturating_sub(60);
        backend.fill_rect(
            addr_field_x,
            g.addr_y + 1,
            addr_field_w,
            g.addr_h - 2,
            colors.pane_bg,
        )?;
        draw_outline(
            backend,
            addr_field_x,
            g.addr_y + 1,
            addr_field_w,
            g.addr_h - 2,
            dark,
        )?;
        backend.draw_text(
            &panel.browse_dir,
            addr_field_x + 4,
            g.addr_y + 3,
            at.font_hint,
            colors.pane_text,
        )?;

        // Tree pane.
        backend.fill_rect(g.tree_x, g.body_y, g.tree_w, g.body_h, colors.pane_bg)?;
        draw_outline(backend, g.tree_x, g.body_y, g.tree_w, g.body_h, dark)?;
        let tree_entries = &panel.tree_entries;
        let tree_line_h = (at.font_hint as i32 + 2).max(11);
        for (i, entry) in tree_entries.iter().enumerate() {
            let y = g.body_y + 4 + i as i32 * tree_line_h;
            if y + tree_line_h > g.body_y + g.body_h as i32 - 4 {
                break;
            }
            if entry.is_current {
                backend.fill_rect(
                    g.tree_x + 2,
                    y,
                    g.tree_w.saturating_sub(4),
                    tree_line_h as u32,
                    colors.selected_bg,
                )?;
            }
            let indent = entry.depth as i32 * 8;
            let color = if entry.is_current {
                colors.selected_text
            } else {
                colors.pane_text
            };
            backend.draw_text(
                &entry.label,
                g.tree_x + 4 + indent,
                y + 1,
                at.font_hint,
                color,
            )?;
        }

        // Grid pane. The tile loop produces ~7 rects + 1 text per tile
        // (icon body, fold, 4-side outline, optional selection, label).
        // For the 48-tile maximum that's ~336 rects + 48 texts per frame.
        // Collecting them into batches collapses the per-item wasm-bindgen
        // cost into one round-trip per batch on the WASM backend; native
        // backends fall through to the per-item default.
        backend.fill_rect(g.grid_x, g.body_y, g.grid_w, g.body_h, colors.pane_bg)?;
        draw_outline(backend, g.grid_x, g.body_y, g.grid_w, g.body_h, dark)?;
        let count_visible = g.cols * g.rows;
        let mut tile_rects: Vec<BatchRect> = Vec::with_capacity(count_visible * 7);
        // Labels are owned `String`s held here so the `BatchText<'_>`
        // entries can borrow them with a stable lifetime.
        let mut tile_labels: Vec<(i32, i32, Color, String)> = Vec::with_capacity(count_visible);
        for i in 0..count_visible {
            let abs_idx = panel.scroll + i;
            let Some(line) = panel.lines.get(abs_idx) else {
                break;
            };
            let row = i / g.cols.max(1);
            let col = i % g.cols.max(1);
            let tx = g.grid_x + 4 + col as i32 * g.tile_w as i32;
            let ty = g.body_y + 4 + row as i32 * g.tile_h as i32;
            if tx + g.tile_w as i32 > g.grid_x + g.grid_w as i32 - 2
                || ty + g.tile_h as i32 > g.body_y + g.body_h as i32 - 2
            {
                break;
            }
            let is_selected = abs_idx == panel.scroll + panel.cursor;
            if is_selected {
                tile_rects.push(BatchRect {
                    x: tx,
                    y: ty,
                    w: g.tile_w,
                    h: g.tile_h,
                    color: colors.selected_bg,
                });
            }
            let (name, kind) = parse_entry(line);
            let icon_x = tx + (g.tile_w as i32 - g.icon_w as i32) / 2;
            let icon_y = ty + 4;
            push_icon(
                &mut tile_rects,
                icon_x,
                icon_y,
                g.icon_w,
                g.icon_h,
                kind,
                &colors,
            );
            let max_chars = (g.tile_w as usize / 7).max(4);
            let label = truncate_label(&name, max_chars);
            let label_color = if is_selected {
                colors.selected_text
            } else {
                colors.pane_text
            };
            tile_labels.push((tx + 2, icon_y + g.icon_h as i32 + 2, label_color, label));
        }
        if !tile_rects.is_empty() {
            backend.submit_rect_batch(&tile_rects)?;
        }
        if !tile_labels.is_empty() {
            let text_batch: Vec<BatchText<'_>> = tile_labels
                .iter()
                .map(|(x, y, color, label)| BatchText {
                    text: label.as_str(),
                    x: *x,
                    y: *y,
                    color: *color,
                })
                .collect();
            backend.submit_text_batch(&text_batch, at.font_hint, false, false)?;
        }

        // Status.
        let count = panel.lines.iter().filter(|l| l.trim() != "..").count();
        backend.fill_rect(g.menu_x, g.status_y, g.menu_w, g.status_h, colors.status_bg)?;
        backend.draw_text(
            &format!("{count} object(s)"),
            g.menu_x + 4,
            g.status_y + 1,
            at.font_hint,
            colors.status_text,
        )?;
        backend.draw_text(
            "View>List  Cancel=back",
            g.menu_x + g.menu_w as i32 - 180,
            g.status_y + 1,
            at.font_hint,
            colors.status_text,
        )?;

        // Drop-down floats above the rest of the windowed content.
        if self.menu.is_open() {
            self.menu
                .draw_dropdown(backend, g.menu_x, g.menu_y, g.menu_h, &menu_style)?;
        }

        Ok(())
    }
}

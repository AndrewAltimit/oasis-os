//! Rendering for the File Manager app.
//!
//! Both presentation modes (`ViewMode::Dual` and `ViewMode::Explorer`)
//! draw directly to the backend (`draw_windowed_*`). Hit-testing helpers
//! (`tree_hit_test`, `grid_hit_test`, `compute_explorer_geom`) live
//! alongside the renderer that defines the geometry, since callers in
//! [`crate::commands`] must agree byte-for-byte on the layout.

use oasis_skin::ActiveTheme;
use oasis_types::backend::{BatchRect, BatchText, Color, SdiBackend};
use oasis_ui::menu_bar::MenuStyle;

use crate::colors::FileManagerColors;
use crate::model::{EntryKind, TreeEntry, truncate_label};
use crate::state::FileManagerApp;

/// Pixel height reserved for the file-manager menu bar (matches the
/// dimensions used by Notepad's `MenuBar` rendering).
pub(crate) const FM_MENU_H: u32 = 18;
/// Pixel height of the address bar in Explorer view.
pub(crate) const FM_ADDR_H: u32 = 18;
/// Pixel height of the status strip in Explorer view.
pub(crate) const FM_STATUS_H: u32 = 14;

// Re-exported for use in `lib.rs` tests that need to mirror the renderer's
// hit-test geometry exactly.
pub(crate) use private::{
    Box2d, compute_explorer_geom, draw_outline, grid_hit_test, push_icon, tree_hit_test,
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

    /// Rectangle in content coordinates, used by the Explorer-view helpers.
    #[derive(Clone, Copy)]
    pub(crate) struct Box2d {
        pub x: i32,
        pub y: i32,
        pub w: u32,
        pub h: u32,
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
}

/// Default hint line under the dual-panel view.
const DUAL_HINT: &str = "L/R=panel  \u{25b3}=del  \u{25a1}=new  F2=rename  Start=copy/paste";

impl FileManagerApp {
    /// The last operation's feedback, or `default` when there is none.
    pub(crate) fn status_or(&self, default: &str) -> String {
        self.status.clone().unwrap_or_else(|| default.to_string())
    }

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
            let pw = if pi == 0 {
                half_w
            } else {
                cw.saturating_sub(half_w + 1)
            };
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
            &self.status_or(DUAL_HINT),
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
        // Grid metrics for navigation / hit-tests were recorded by
        // `App::layout` from the same geometry.
        let g = compute_explorer_geom(cx, cy, cw, ch);

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
            // Informational rows (`(empty directory)`) draw as a file.
            let (name, kind) = match panel.entry(abs_idx) {
                Some(e) => (e.name.clone(), e.kind),
                None => (line.trim().to_string(), EntryKind::File),
            };
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
            &self.status_or(&format!("{count} object(s)")),
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

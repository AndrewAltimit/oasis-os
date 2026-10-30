//! Dashboard configuration derived from the skin's feature gates.

use crate::active_theme::ActiveTheme;
use crate::skin::SkinFeatures;
use crate::ui::flex::GridLayout;
use crate::ui::layout::Padding;

/// Minimum free/column-layout cell width (px). Wide enough that every
/// word of every built-in app title fits a label line (widest: 66px,
/// "Calculator" at the 8px bitmap font) plus the label gutters.
const FREE_CELL_MIN_W: u32 = 70;

/// Dashboard configuration derived from the skin's feature gates.
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    /// Grid columns (d-pad Up/Down move by this many icons).
    pub grid_cols: u32,
    /// Grid rows.
    pub grid_rows: u32,
    /// Icons per page (pages are `ceil(apps / icons_per_page)`).
    pub icons_per_page: u32,
    /// Grid area origin and cell size (pixels).
    pub grid_x: i32,
    /// Grid area top edge (pixels).
    pub grid_y: i32,
    /// Grid cell width (pixels).
    pub cell_w: u32,
    /// Grid cell height (pixels).
    pub cell_h: u32,
    /// Cursor highlight size offset (drawn slightly larger than the icon).
    pub cursor_pad: i32,
    /// Grid layout helper for computing cell positions.
    pub grid_layout: GridLayout,
    /// Total grid area width (for `GridLayout::cell_rect`).
    pub grid_w: u32,
    /// Total grid area height (for `GridLayout::cell_rect`).
    pub grid_h: u32,
    /// Page slide animation duration in frames.
    pub page_slide_duration: u32,
    /// Press flash duration in frames (0 = disabled).
    pub press_flash_duration: u32,
    /// Free (desktop-style) icon layout: per-icon positions with column
    /// auto-flow instead of uniform grid cells.
    pub free_layout: bool,
    /// Fixed cell width in free mode (grid cells stretch to fill instead).
    pub free_cell_w: u32,
    /// Fixed cell height in free mode.
    pub free_cell_h: u32,
    /// Cap free/column auto-flow to this many columns (`None` = as many as
    /// fit the grid width). Set to `Some(1)` by `"column"` layout.
    pub free_cols: Option<u32>,
    /// Left-align icons (and their labels) within the cell instead of
    /// centering. Enabled by the `"column"` layout for a sparse PSIX-style
    /// left column.
    pub left_align_icons: bool,
    /// Snap dragged icons to the virtual grid on drop (free mode).
    pub snap_to_grid: bool,
    /// Page change style: "slide" (default) or "fade" (no icon slide; the
    /// shell overlays a fade transition instead).
    pub page_style: String,
}

impl DashboardConfig {
    /// Create a config from skin features and screen dimensions.
    /// Uses PSIX-style layout: icons on the left side with generous spacing.
    pub fn from_features(features: &SkinFeatures, at: &ActiveTheme) -> Self {
        // Saturating throughout: a skin whose bars (or padding) exceed the
        // screen, or that asks for a 0x0 grid, gets an empty grid instead
        // of an arithmetic overflow / division by zero.
        let cols = features.grid_cols.max(1);
        let rows = features.grid_rows.max(1);
        let content_top = at.statusbar_height.saturating_add(at.tab_row_height);
        let content_h = at
            .screen_h
            .saturating_sub(content_top)
            .saturating_sub(at.bottombar_height);
        let grid_padding_x = at.grid_padding_x;
        let grid_padding_y = at.grid_padding_y;
        let grid_x = grid_padding_x as i32;
        let grid_y =
            i32::try_from(content_top.saturating_add(grid_padding_y as u32)).unwrap_or(i32::MAX);
        let grid_w = at.screen_w.saturating_sub(2 * grid_padding_x as u32);
        let grid_h = content_h.saturating_sub(2 * grid_padding_y as u32);

        // Size cells to fill available space evenly.
        let cell_w = grid_w / cols;
        let cell_h = grid_h / rows;

        let grid_layout = GridLayout::new(cols).with_padding(Padding::ZERO);

        // `"column"` layout: sparse PSIX-style single left column. Same
        // interaction semantics as `"free"`, but auto-flow is capped to
        // `free_icon_cols` columns (default 1) and icons hug the left edge.
        let is_column = features.icon_layout == "column";
        let free_layout = features.icon_layout == "free" || is_column;

        // Free-mode cells are a fixed size derived from the theme's icon
        // dimensions (2x leaves room for a two-line label) instead of
        // stretching to fill the grid area like grid cells do. The width
        // floor keeps neighbouring labels apart: labels are clipped to
        // their cell, so a cell narrower than a title word ellipsizes it.
        let free_cell_w = (at.icon_width * 2).max(FREE_CELL_MIN_W);
        let free_cell_h = if is_column {
            // Taller pitch so the column reads sparse: icon + two label
            // lines + a comfortable gap. Scales with the theme's icon size
            // so a column skin with small icons still packs several rows.
            (at.icon_height + at.icon_label_pad as u32 + (at.font_small as u32 + 1) * 2 + 8).max(48)
        } else {
            (at.icon_height * 2).max(48)
        };

        // Column mode caps auto-flow to `free_icon_cols` columns, defaulting
        // to a single column. Free mode only honours an explicit cap.
        let free_cols = if is_column {
            Some(features.free_icon_cols.unwrap_or(1))
        } else {
            features.free_icon_cols
        };

        Self {
            grid_cols: cols,
            grid_rows: rows,
            icons_per_page: features.icons_per_page,
            grid_x,
            grid_y,
            cell_w,
            cell_h,
            cursor_pad: at.cursor_pad,
            grid_layout,
            grid_w,
            grid_h,
            page_slide_duration: at.page_slide_duration,
            press_flash_duration: at.press_flash_duration,
            free_layout,
            free_cell_w,
            free_cell_h,
            free_cols,
            left_align_icons: is_column,
            snap_to_grid: features.snap_to_grid,
            page_style: at.transition_page_style.clone(),
        }
    }
}

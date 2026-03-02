//! Tiling window layout engine for the window manager.
//!
//! Provides automatic window arrangement using several layout algorithms:
//! master-stack, grid, columns, rows, and monocle. The [`TilingManager`]
//! tracks which layout is active, per-window floating overrides, and
//! computes tile geometries for a given set of window IDs.

use std::collections::HashSet;

use crate::window::Geometry;

// ── Layout enum ─────────────────────────────────────────────────────

/// Available tiling layout algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilingLayout {
    /// One master pane on the left, stack on the right.
    MasterStack,
    /// Equal-sized grid (rows x columns).
    Grid,
    /// Vertical columns of equal width.
    Columns,
    /// Horizontal rows of equal height.
    Rows,
    /// Monocle: each window fills the full area, only active visible.
    Monocle,
}

/// Cycle to the next layout in deterministic order.
///
/// The order is: `MasterStack` -> `Grid` -> `Columns` -> `Rows` ->
/// `Monocle` -> `MasterStack`.
pub fn cycle_layout(current: TilingLayout) -> TilingLayout {
    match current {
        TilingLayout::MasterStack => TilingLayout::Grid,
        TilingLayout::Grid => TilingLayout::Columns,
        TilingLayout::Columns => TilingLayout::Rows,
        TilingLayout::Rows => TilingLayout::Monocle,
        TilingLayout::Monocle => TilingLayout::MasterStack,
    }
}

// ── Config ──────────────────────────────────────────────────────────

/// Configuration for the tiling layout engine.
#[derive(Debug, Clone)]
pub struct TilingConfig {
    /// Gap between windows in pixels.
    pub gap: u32,
    /// Outer margin from screen edges.
    pub margin: u32,
    /// Master pane ratio (0.0 to 1.0) for `MasterStack` layout.
    pub master_ratio: f32,
    /// Minimum window width.
    pub min_width: u32,
    /// Minimum window height.
    pub min_height: u32,
}

impl Default for TilingConfig {
    fn default() -> Self {
        Self {
            gap: 4,
            margin: 4,
            master_ratio: 0.55,
            min_width: 80,
            min_height: 60,
        }
    }
}

/// Adjust `master_ratio` by `delta`, clamping to `0.1..=0.9`.
pub fn adjust_master_ratio(config: &mut TilingConfig, delta: f32) {
    config.master_ratio = (config.master_ratio + delta).clamp(0.1, 0.9);
}

// ── Tile geometry ───────────────────────────────────────────────────

/// Result of a tiling layout computation for a single window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileGeometry {
    /// The window this geometry belongs to.
    pub window_id: String,
    /// Computed position and size.
    pub geometry: Geometry,
}

// Geometry doesn't derive PartialEq/Eq upstream, so we add a local
// impl so TileGeometry can derive them for tests.
impl PartialEq for Geometry {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y && self.w == other.w && self.h == other.h
    }
}

impl Eq for Geometry {}

// ── Tiling manager ──────────────────────────────────────────────────

/// Manages tiling layout for a set of windows.
///
/// The manager holds the active layout algorithm, the configuration
/// (gaps, margins, master ratio), and a set of window IDs that are
/// forced floating (excluded from tiling).
pub struct TilingManager {
    layout: TilingLayout,
    config: TilingConfig,
    enabled: bool,
    /// Window IDs that are forced floating (excluded from tiling).
    floating_overrides: HashSet<String>,
}

impl TilingManager {
    /// Create a new tiling manager with default `MasterStack` layout.
    pub fn new() -> Self {
        Self {
            layout: TilingLayout::MasterStack,
            config: TilingConfig::default(),
            enabled: true,
            floating_overrides: HashSet::new(),
        }
    }

    /// Create a new tiling manager with the given layout.
    pub fn with_layout(layout: TilingLayout) -> Self {
        Self {
            layout,
            config: TilingConfig::default(),
            enabled: true,
            floating_overrides: HashSet::new(),
        }
    }

    /// Set the active layout algorithm.
    pub fn set_layout(&mut self, layout: TilingLayout) {
        self.layout = layout;
    }

    /// Return the active layout algorithm.
    pub fn layout(&self) -> TilingLayout {
        self.layout
    }

    /// Enable or disable tiling.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Whether tiling is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Toggle the enabled state, returning the new value.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Borrow the tiling configuration.
    pub fn config(&self) -> &TilingConfig {
        &self.config
    }

    /// Mutably borrow the tiling configuration.
    pub fn config_mut(&mut self) -> &mut TilingConfig {
        &mut self.config
    }

    /// Mark or unmark a window as floating (excluded from tiling).
    pub fn set_floating(&mut self, window_id: &str, floating: bool) {
        if floating {
            self.floating_overrides.insert(window_id.to_string());
        } else {
            self.floating_overrides.remove(window_id);
        }
    }

    /// Whether a window is marked as floating.
    pub fn is_floating(&self, window_id: &str) -> bool {
        self.floating_overrides.contains(window_id)
    }

    /// Compute tile geometries for the given window IDs.
    ///
    /// Windows in the floating-override set are silently skipped.
    /// The `area_w` and `area_h` describe the total tiling area
    /// (typically the desktop content area minus bars).
    ///
    /// Returns one [`TileGeometry`] per non-floating window.
    pub fn compute_layout(
        &self,
        window_ids: &[&str],
        area_w: u32,
        area_h: u32,
    ) -> Vec<TileGeometry> {
        // Filter out floating windows.
        let tiled: Vec<&str> = window_ids
            .iter()
            .filter(|id| !self.floating_overrides.contains(**id))
            .copied()
            .collect();

        if tiled.is_empty() {
            return Vec::new();
        }

        match self.layout {
            TilingLayout::MasterStack => self.layout_master_stack(&tiled, area_w, area_h),
            TilingLayout::Grid => self.layout_grid(&tiled, area_w, area_h),
            TilingLayout::Columns => self.layout_columns(&tiled, area_w, area_h),
            TilingLayout::Rows => self.layout_rows(&tiled, area_w, area_h),
            TilingLayout::Monocle => self.layout_monocle(&tiled, area_w, area_h),
        }
    }

    // ── Private layout helpers ──────────────────────────────────────

    /// Usable area after subtracting outer margins from both sides.
    fn usable_area(&self, area_w: u32, area_h: u32) -> (i32, i32, u32, u32) {
        let m = self.config.margin;
        let x = m as i32;
        let y = m as i32;
        let w = area_w.saturating_sub(m * 2);
        let h = area_h.saturating_sub(m * 2);
        (x, y, w, h)
    }

    /// Enforce minimum size constraints on a geometry.
    fn clamp_size(&self, mut g: Geometry) -> Geometry {
        if g.w < self.config.min_width {
            g.w = self.config.min_width;
        }
        if g.h < self.config.min_height {
            g.h = self.config.min_height;
        }
        g
    }

    fn layout_master_stack(&self, ids: &[&str], area_w: u32, area_h: u32) -> Vec<TileGeometry> {
        let (ux, uy, uw, uh) = self.usable_area(area_w, area_h);
        let n = ids.len();
        let gap = self.config.gap;

        if n == 1 {
            // Single window fills the entire usable area.
            let g = self.clamp_size(Geometry {
                x: ux,
                y: uy,
                w: uw,
                h: uh,
            });
            return vec![TileGeometry {
                window_id: ids[0].to_string(),
                geometry: g,
            }];
        }

        // Master pane on the left.
        let master_w_raw = (uw as f32 * self.config.master_ratio) as u32;
        let master_w = master_w_raw.saturating_sub(gap / 2);
        let stack_w = uw
            .saturating_sub(master_w_raw)
            .saturating_sub(gap - gap / 2);

        let mut result = Vec::with_capacity(n);

        // Master.
        let master_g = self.clamp_size(Geometry {
            x: ux,
            y: uy,
            w: master_w,
            h: uh,
        });
        result.push(TileGeometry {
            window_id: ids[0].to_string(),
            geometry: master_g,
        });

        // Stack (right side).
        let stack_count = (n - 1) as u32;
        let total_gaps = gap * stack_count.saturating_sub(1);
        let per_h = uh.saturating_sub(total_gaps) / stack_count;
        let stack_x = ux + master_w_raw as i32 + gap as i32 - (gap / 2) as i32;

        for (i, id) in ids.iter().enumerate().skip(1) {
            let idx = (i - 1) as u32;
            let sy = uy + (per_h + gap) as i32 * idx as i32;
            // Last window absorbs rounding remainder.
            let sh = if idx == stack_count - 1 {
                (uy + uh as i32 - sy) as u32
            } else {
                per_h
            };
            let g = self.clamp_size(Geometry {
                x: stack_x,
                y: sy,
                w: stack_w,
                h: sh,
            });
            result.push(TileGeometry {
                window_id: id.to_string(),
                geometry: g,
            });
        }

        result
    }

    fn layout_grid(&self, ids: &[&str], area_w: u32, area_h: u32) -> Vec<TileGeometry> {
        let (ux, uy, uw, uh) = self.usable_area(area_w, area_h);
        let n = ids.len() as u32;
        let gap = self.config.gap;

        // Compute grid dimensions.
        let cols = ceil_sqrt(n);
        let rows = n.div_ceil(cols);

        let total_gap_x = gap * cols.saturating_sub(1);
        let total_gap_y = gap * rows.saturating_sub(1);
        let cell_w = uw.saturating_sub(total_gap_x) / cols;
        let cell_h = uh.saturating_sub(total_gap_y) / rows;

        let mut result = Vec::with_capacity(ids.len());

        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let col = idx % cols;
            let row = idx / cols;

            let cx = ux + (cell_w + gap) as i32 * col as i32;
            let cy = uy + (cell_h + gap) as i32 * row as i32;

            // Last column absorbs horizontal remainder.
            let w = if col == cols - 1 {
                (ux + uw as i32 - cx) as u32
            } else {
                cell_w
            };
            // Last row absorbs vertical remainder.
            let h = if row == rows - 1 {
                (uy + uh as i32 - cy) as u32
            } else {
                cell_h
            };

            let g = self.clamp_size(Geometry { x: cx, y: cy, w, h });
            result.push(TileGeometry {
                window_id: id.to_string(),
                geometry: g,
            });
        }

        result
    }

    fn layout_columns(&self, ids: &[&str], area_w: u32, area_h: u32) -> Vec<TileGeometry> {
        let (ux, uy, uw, uh) = self.usable_area(area_w, area_h);
        let n = ids.len() as u32;
        let gap = self.config.gap;

        let total_gaps = gap * n.saturating_sub(1);
        let col_w = uw.saturating_sub(total_gaps) / n;

        let mut result = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let cx = ux + (col_w + gap) as i32 * idx as i32;
            // Last column absorbs rounding remainder.
            let w = if idx == n - 1 {
                (ux + uw as i32 - cx) as u32
            } else {
                col_w
            };
            let g = self.clamp_size(Geometry {
                x: cx,
                y: uy,
                w,
                h: uh,
            });
            result.push(TileGeometry {
                window_id: id.to_string(),
                geometry: g,
            });
        }

        result
    }

    fn layout_rows(&self, ids: &[&str], area_w: u32, area_h: u32) -> Vec<TileGeometry> {
        let (ux, uy, uw, uh) = self.usable_area(area_w, area_h);
        let n = ids.len() as u32;
        let gap = self.config.gap;

        let total_gaps = gap * n.saturating_sub(1);
        let row_h = uh.saturating_sub(total_gaps) / n;

        let mut result = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let ry = uy + (row_h + gap) as i32 * idx as i32;
            // Last row absorbs rounding remainder.
            let h = if idx == n - 1 {
                (uy + uh as i32 - ry) as u32
            } else {
                row_h
            };
            let g = self.clamp_size(Geometry {
                x: ux,
                y: ry,
                w: uw,
                h,
            });
            result.push(TileGeometry {
                window_id: id.to_string(),
                geometry: g,
            });
        }

        result
    }

    fn layout_monocle(&self, ids: &[&str], area_w: u32, area_h: u32) -> Vec<TileGeometry> {
        let (ux, uy, uw, uh) = self.usable_area(area_w, area_h);
        let g = self.clamp_size(Geometry {
            x: ux,
            y: uy,
            w: uw,
            h: uh,
        });
        ids.iter()
            .map(|id| TileGeometry {
                window_id: id.to_string(),
                geometry: g,
            })
            .collect()
    }
}

impl Default for TilingManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Integer ceiling of the square root of `n`.
fn ceil_sqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let s = (n as f64).sqrt().ceil() as u32;
    // Guard against floating-point edge cases.
    if s > 0 && (s - 1) * (s - 1) >= n {
        s - 1
    } else {
        s
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand for building a zero-gap, zero-margin manager.
    fn no_gap_manager(layout: TilingLayout) -> TilingManager {
        let mut mgr = TilingManager::with_layout(layout);
        mgr.config_mut().gap = 0;
        mgr.config_mut().margin = 0;
        mgr.config_mut().min_width = 1;
        mgr.config_mut().min_height = 1;
        mgr
    }

    // ── MasterStack ─────────────────────────────────────────────────

    #[test]
    fn master_stack_single_window_fills_area() {
        let mgr = no_gap_manager(TilingLayout::MasterStack);
        let tiles = mgr.compute_layout(&["a"], 800, 600);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].window_id, "a");
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            }
        );
    }

    #[test]
    fn master_stack_two_windows() {
        let mut mgr = no_gap_manager(TilingLayout::MasterStack);
        mgr.config_mut().master_ratio = 0.5;
        let tiles = mgr.compute_layout(&["m", "s"], 800, 600);
        assert_eq!(tiles.len(), 2);
        // Master takes left half.
        assert_eq!(tiles[0].window_id, "m");
        assert_eq!(tiles[0].geometry.x, 0);
        assert_eq!(tiles[0].geometry.w, 400);
        assert_eq!(tiles[0].geometry.h, 600);
        // Stack takes right half.
        assert_eq!(tiles[1].window_id, "s");
        assert_eq!(tiles[1].geometry.w, 400);
        assert_eq!(tiles[1].geometry.h, 600);
    }

    #[test]
    fn master_stack_five_windows() {
        let mut mgr = no_gap_manager(TilingLayout::MasterStack);
        mgr.config_mut().master_ratio = 0.5;
        let ids: Vec<&str> = vec!["m", "a", "b", "c", "d"];
        let tiles = mgr.compute_layout(&ids, 800, 400);
        assert_eq!(tiles.len(), 5);
        // Master.
        assert_eq!(tiles[0].window_id, "m");
        assert_eq!(tiles[0].geometry.w, 400);
        assert_eq!(tiles[0].geometry.h, 400);
        // Stack: 4 windows split 400px vertically -> 100px each.
        for t in &tiles[1..] {
            assert_eq!(t.geometry.w, 400);
            assert!(t.geometry.h >= 100);
        }
    }

    // ── Grid ────────────────────────────────────────────────────────

    #[test]
    fn grid_four_windows_2x2() {
        let mgr = no_gap_manager(TilingLayout::Grid);
        let tiles = mgr.compute_layout(&["a", "b", "c", "d"], 800, 600);
        assert_eq!(tiles.len(), 4);
        // ceil(sqrt(4)) = 2 cols, 2 rows.
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 400,
                h: 300,
            }
        );
        assert_eq!(
            tiles[1].geometry,
            Geometry {
                x: 400,
                y: 0,
                w: 400,
                h: 300,
            }
        );
        assert_eq!(
            tiles[2].geometry,
            Geometry {
                x: 0,
                y: 300,
                w: 400,
                h: 300,
            }
        );
        assert_eq!(
            tiles[3].geometry,
            Geometry {
                x: 400,
                y: 300,
                w: 400,
                h: 300,
            }
        );
    }

    #[test]
    fn grid_three_windows() {
        let mgr = no_gap_manager(TilingLayout::Grid);
        let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
        assert_eq!(tiles.len(), 3);
        // ceil(sqrt(3)) = 2 cols, ceil(3/2) = 2 rows.
        // Row 0: a(col0), b(col1). Row 1: c(col0).
        assert_eq!(tiles[0].geometry.x, 0);
        assert_eq!(tiles[1].geometry.x, 400);
        assert_eq!(tiles[2].geometry.x, 0);
        assert_eq!(tiles[2].geometry.y, 300);
    }

    #[test]
    fn grid_single_window_fills_area() {
        let mgr = no_gap_manager(TilingLayout::Grid);
        let tiles = mgr.compute_layout(&["x"], 480, 272);
        assert_eq!(tiles.len(), 1);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 480,
                h: 272,
            }
        );
    }

    // ── Columns ─────────────────────────────────────────────────────

    #[test]
    fn columns_three_windows() {
        let mgr = no_gap_manager(TilingLayout::Columns);
        let tiles = mgr.compute_layout(&["a", "b", "c"], 900, 600);
        assert_eq!(tiles.len(), 3);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 300,
                h: 600,
            }
        );
        assert_eq!(
            tiles[1].geometry,
            Geometry {
                x: 300,
                y: 0,
                w: 300,
                h: 600,
            }
        );
        assert_eq!(
            tiles[2].geometry,
            Geometry {
                x: 600,
                y: 0,
                w: 300,
                h: 600,
            }
        );
    }

    #[test]
    fn columns_single_fills_area() {
        let mgr = no_gap_manager(TilingLayout::Columns);
        let tiles = mgr.compute_layout(&["a"], 480, 272);
        assert_eq!(tiles.len(), 1);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 480,
                h: 272,
            }
        );
    }

    // ── Rows ────────────────────────────────────────────────────────

    #[test]
    fn rows_three_windows() {
        let mgr = no_gap_manager(TilingLayout::Rows);
        let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
        assert_eq!(tiles.len(), 3);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 800,
                h: 200,
            }
        );
        assert_eq!(
            tiles[1].geometry,
            Geometry {
                x: 0,
                y: 200,
                w: 800,
                h: 200,
            }
        );
        assert_eq!(
            tiles[2].geometry,
            Geometry {
                x: 0,
                y: 400,
                w: 800,
                h: 200,
            }
        );
    }

    #[test]
    fn rows_single_fills_area() {
        let mgr = no_gap_manager(TilingLayout::Rows);
        let tiles = mgr.compute_layout(&["a"], 480, 272);
        assert_eq!(tiles.len(), 1);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 480,
                h: 272,
            }
        );
    }

    // ── Monocle ─────────────────────────────────────────────────────

    #[test]
    fn monocle_all_same_geometry() {
        let mgr = no_gap_manager(TilingLayout::Monocle);
        let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
        assert_eq!(tiles.len(), 3);
        let expected = Geometry {
            x: 0,
            y: 0,
            w: 800,
            h: 600,
        };
        for t in &tiles {
            assert_eq!(t.geometry, expected);
        }
    }

    #[test]
    fn monocle_single_window() {
        let mgr = no_gap_manager(TilingLayout::Monocle);
        let tiles = mgr.compute_layout(&["only"], 480, 272);
        assert_eq!(tiles.len(), 1);
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 480,
                h: 272,
            }
        );
    }

    // ── Gaps and margins ────────────────────────────────────────────

    #[test]
    fn gap_spacing_columns() {
        let mut mgr = TilingManager::with_layout(TilingLayout::Columns);
        mgr.config_mut().gap = 10;
        mgr.config_mut().margin = 0;
        mgr.config_mut().min_width = 1;
        mgr.config_mut().min_height = 1;
        let tiles = mgr.compute_layout(&["a", "b"], 810, 600);
        assert_eq!(tiles.len(), 2);
        // (810 - 10 gap) / 2 = 400 each.
        assert_eq!(tiles[0].geometry.x, 0);
        assert_eq!(tiles[0].geometry.w, 400);
        // Second column starts at 400 + 10 = 410.
        assert_eq!(tiles[1].geometry.x, 410);
        assert_eq!(tiles[1].geometry.w, 400);
    }

    #[test]
    fn margin_from_edges() {
        let mut mgr = TilingManager::with_layout(TilingLayout::Monocle);
        mgr.config_mut().gap = 0;
        mgr.config_mut().margin = 20;
        mgr.config_mut().min_width = 1;
        mgr.config_mut().min_height = 1;
        let tiles = mgr.compute_layout(&["a"], 800, 600);
        assert_eq!(tiles.len(), 1);
        // 800 - 40 margin = 760, starts at x=20.
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 20,
                y: 20,
                w: 760,
                h: 560,
            }
        );
    }

    #[test]
    fn gap_and_margin_rows() {
        let mut mgr = TilingManager::with_layout(TilingLayout::Rows);
        mgr.config_mut().gap = 8;
        mgr.config_mut().margin = 10;
        mgr.config_mut().min_width = 1;
        mgr.config_mut().min_height = 1;
        let tiles = mgr.compute_layout(&["a", "b"], 400, 228);
        // Usable: 380 x 208. Row height = (208 - 8) / 2 = 100 each.
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].geometry.x, 10);
        assert_eq!(tiles[0].geometry.y, 10);
        assert_eq!(tiles[0].geometry.w, 380);
        assert_eq!(tiles[0].geometry.h, 100);
        assert_eq!(tiles[1].geometry.x, 10);
        assert_eq!(tiles[1].geometry.y, 118); // 10 + 100 + 8
        assert_eq!(tiles[1].geometry.w, 380);
        assert_eq!(tiles[1].geometry.h, 100); // remainder absorbed
    }

    // ── Empty window list ───────────────────────────────────────────

    #[test]
    fn empty_window_list() {
        let mgr = TilingManager::new();
        let tiles = mgr.compute_layout(&[], 800, 600);
        assert!(tiles.is_empty());
    }

    // ── Floating overrides ──────────────────────────────────────────

    #[test]
    fn floating_override_excludes_window() {
        let mut mgr = no_gap_manager(TilingLayout::Columns);
        mgr.set_floating("b", true);
        let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
        assert_eq!(tiles.len(), 2);
        let ids: Vec<&str> = tiles.iter().map(|t| t.window_id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(!ids.contains(&"b"));
        assert!(ids.contains(&"c"));
    }

    #[test]
    fn floating_override_can_be_removed() {
        let mut mgr = no_gap_manager(TilingLayout::Columns);
        mgr.set_floating("b", true);
        assert!(mgr.is_floating("b"));
        mgr.set_floating("b", false);
        assert!(!mgr.is_floating("b"));
        let tiles = mgr.compute_layout(&["a", "b"], 800, 600);
        assert_eq!(tiles.len(), 2);
    }

    #[test]
    fn all_windows_floating_returns_empty() {
        let mut mgr = no_gap_manager(TilingLayout::Rows);
        mgr.set_floating("a", true);
        mgr.set_floating("b", true);
        let tiles = mgr.compute_layout(&["a", "b"], 800, 600);
        assert!(tiles.is_empty());
    }

    // ── Toggle enabled/disabled ─────────────────────────────────────

    #[test]
    fn toggle_enabled() {
        let mut mgr = TilingManager::new();
        assert!(mgr.is_enabled());
        let new_state = mgr.toggle();
        assert!(!new_state);
        assert!(!mgr.is_enabled());
        let new_state = mgr.toggle();
        assert!(new_state);
        assert!(mgr.is_enabled());
    }

    #[test]
    fn set_enabled() {
        let mut mgr = TilingManager::new();
        mgr.set_enabled(false);
        assert!(!mgr.is_enabled());
        mgr.set_enabled(true);
        assert!(mgr.is_enabled());
    }

    // ── cycle_layout ────────────────────────────────────────────────

    #[test]
    fn cycle_layout_full_cycle() {
        let l = TilingLayout::MasterStack;
        let l = cycle_layout(l);
        assert_eq!(l, TilingLayout::Grid);
        let l = cycle_layout(l);
        assert_eq!(l, TilingLayout::Columns);
        let l = cycle_layout(l);
        assert_eq!(l, TilingLayout::Rows);
        let l = cycle_layout(l);
        assert_eq!(l, TilingLayout::Monocle);
        let l = cycle_layout(l);
        assert_eq!(l, TilingLayout::MasterStack);
    }

    // ── adjust_master_ratio ─────────────────────────────────────────

    #[test]
    fn adjust_master_ratio_clamp_high() {
        let mut cfg = TilingConfig::default();
        cfg.master_ratio = 0.85;
        adjust_master_ratio(&mut cfg, 0.2);
        assert!((cfg.master_ratio - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn adjust_master_ratio_clamp_low() {
        let mut cfg = TilingConfig::default();
        cfg.master_ratio = 0.15;
        adjust_master_ratio(&mut cfg, -0.1);
        assert!((cfg.master_ratio - 0.1).abs() < 0.01);
    }

    #[test]
    fn adjust_master_ratio_normal() {
        let mut cfg = TilingConfig::default();
        cfg.master_ratio = 0.5;
        adjust_master_ratio(&mut cfg, 0.05);
        assert!((cfg.master_ratio - 0.55).abs() < f32::EPSILON);
    }

    // ── Minimum size enforcement ────────────────────────────────────

    #[test]
    fn minimum_size_enforced_columns() {
        let mut mgr = TilingManager::with_layout(TilingLayout::Columns);
        mgr.config_mut().gap = 0;
        mgr.config_mut().margin = 0;
        mgr.config_mut().min_width = 200;
        mgr.config_mut().min_height = 100;
        // 10 columns in 400px -> 40px each, below min_width of 200.
        let ids: Vec<&str> = (0..10).map(|_| "w").collect();
        let tiles = mgr.compute_layout(&ids, 400, 50);
        for t in &tiles {
            assert!(t.geometry.w >= 200);
            assert!(t.geometry.h >= 100);
        }
    }

    #[test]
    fn minimum_size_enforced_rows() {
        let mut mgr = TilingManager::with_layout(TilingLayout::Rows);
        mgr.config_mut().gap = 0;
        mgr.config_mut().margin = 0;
        mgr.config_mut().min_width = 80;
        mgr.config_mut().min_height = 60;
        // 20 rows in 100px -> 5px each, below min_height of 60.
        let ids: Vec<&str> = (0..20).map(|_| "w").collect();
        let tiles = mgr.compute_layout(&ids, 50, 100);
        for t in &tiles {
            assert!(t.geometry.w >= 80);
            assert!(t.geometry.h >= 60);
        }
    }

    // ── Config mutation ─────────────────────────────────────────────

    #[test]
    fn config_mutation() {
        let mut mgr = TilingManager::new();
        mgr.config_mut().gap = 16;
        mgr.config_mut().margin = 8;
        mgr.config_mut().master_ratio = 0.6;
        assert_eq!(mgr.config().gap, 16);
        assert_eq!(mgr.config().margin, 8);
        assert!((mgr.config().master_ratio - 0.6).abs() < f32::EPSILON);
    }

    // ── Default trait ───────────────────────────────────────────────

    #[test]
    fn default_manager_is_master_stack_enabled() {
        let mgr = TilingManager::default();
        assert_eq!(mgr.layout(), TilingLayout::MasterStack);
        assert!(mgr.is_enabled());
    }

    // ── with_layout constructor ─────────────────────────────────────

    #[test]
    fn with_layout_sets_layout() {
        let mgr = TilingManager::with_layout(TilingLayout::Grid);
        assert_eq!(mgr.layout(), TilingLayout::Grid);
    }

    // ── set_layout ──────────────────────────────────────────────────

    #[test]
    fn set_layout_changes_layout() {
        let mut mgr = TilingManager::new();
        mgr.set_layout(TilingLayout::Monocle);
        assert_eq!(mgr.layout(), TilingLayout::Monocle);
    }

    // ── ceil_sqrt helper ────────────────────────────────────────────

    #[test]
    fn ceil_sqrt_values() {
        assert_eq!(ceil_sqrt(0), 0);
        assert_eq!(ceil_sqrt(1), 1);
        assert_eq!(ceil_sqrt(2), 2);
        assert_eq!(ceil_sqrt(3), 2);
        assert_eq!(ceil_sqrt(4), 2);
        assert_eq!(ceil_sqrt(5), 3);
        assert_eq!(ceil_sqrt(9), 3);
        assert_eq!(ceil_sqrt(10), 4);
    }

    // ── Grid: 9 windows = 3x3 ──────────────────────────────────────

    #[test]
    fn grid_nine_windows_3x3() {
        let mgr = no_gap_manager(TilingLayout::Grid);
        let ids: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let tiles = mgr.compute_layout(&ids, 900, 600);
        assert_eq!(tiles.len(), 9);
        // 3 cols x 3 rows. Cell: 300 x 200.
        assert_eq!(
            tiles[0].geometry,
            Geometry {
                x: 0,
                y: 0,
                w: 300,
                h: 200,
            }
        );
        assert_eq!(
            tiles[4].geometry,
            Geometry {
                x: 300,
                y: 200,
                w: 300,
                h: 200,
            }
        );
        assert_eq!(
            tiles[8].geometry,
            Geometry {
                x: 600,
                y: 400,
                w: 300,
                h: 200,
            }
        );
    }
}

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

// ── Tiling manager ──────────────────────────────────────────────────

/// Manages tiling layout for a set of windows.
///
/// The manager holds the active layout algorithm, the configuration
/// (gaps, margins, master ratio), and a set of window IDs that are
/// forced floating (excluded from tiling).
pub struct TilingManager {
    layout: TilingLayout,
    config: TilingConfig,
    /// Window IDs that are forced floating (excluded from tiling).
    floating_overrides: HashSet<String>,
}

impl TilingManager {
    /// Create a new tiling manager with default `MasterStack` layout.
    pub fn new() -> Self {
        Self {
            layout: TilingLayout::MasterStack,
            config: TilingConfig::default(),
            floating_overrides: HashSet::new(),
        }
    }

    /// Create a new tiling manager with the given layout.
    pub fn with_layout(layout: TilingLayout) -> Self {
        Self {
            layout,
            config: TilingConfig::default(),
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

    /// Usable area after subtracting outer margins from both sides. A
    /// margin wider than half the area is clamped (it used to overflow
    /// `m * 2` / `m as i32` for huge configured margins).
    fn usable_area(&self, area_w: u32, area_h: u32) -> (i32, i32, u32, u32) {
        let mx = self.config.margin.min(area_w / 2);
        let my = self.config.margin.min(area_h / 2);
        (
            to_i32(i64::from(mx)),
            to_i32(i64::from(my)),
            area_w - 2 * mx,
            area_h - 2 * my,
        )
    }

    /// The configured gap, shrunk so the `count - 1` gaps between
    /// `count` tiles fit in `span` (so `gap * (count - 1)` cannot
    /// overflow and tiles never start past the span).
    fn gap_for(&self, span: u32, count: u32) -> u32 {
        self.config.gap.min(span / count.saturating_sub(1).max(1))
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
        let gap = self.gap_for(uw, 2);

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
        let ratio = self.config.master_ratio.clamp(0.0, 1.0);
        let master_w_raw = ((uw as f32 * ratio) as u32).min(uw);
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
        let stack_count = u32::try_from(n - 1).unwrap_or(u32::MAX);
        let vgap = self.gap_for(uh, stack_count);
        let total_gaps = vgap * stack_count.saturating_sub(1);
        let per_h = uh.saturating_sub(total_gaps) / stack_count;
        let stack_x = offset(ux, master_w_raw.saturating_add(gap - gap / 2).min(uw), 1);

        for (i, id) in ids.iter().enumerate().skip(1) {
            let idx = u32::try_from(i - 1).unwrap_or(u32::MAX);
            let sy = offset(uy, per_h.saturating_add(vgap), idx);
            // Last window absorbs rounding remainder.
            let sh = if idx == stack_count - 1 {
                remainder(uy, uh, sy)
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

        // Compute grid dimensions.
        let cols = ceil_sqrt(n);
        let rows = n.div_ceil(cols);

        let gap_y = self.gap_for(uh, rows);
        let total_gap_y = gap_y * rows.saturating_sub(1);
        let cell_h = uh.saturating_sub(total_gap_y) / rows;

        let mut result = Vec::with_capacity(ids.len());

        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let col = idx % cols;
            let row = idx / cols;
            // A partial last row spreads its windows across the full width
            // instead of leaving the empty cells' area unused.
            let row_cols = if row == rows - 1 {
                n - row * cols
            } else {
                cols
            };
            let gap_x = self.gap_for(uw, row_cols);
            let total_gap_x = gap_x * row_cols.saturating_sub(1);
            let cell_w = uw.saturating_sub(total_gap_x) / row_cols;

            let cx = offset(ux, cell_w.saturating_add(gap_x), col);
            let cy = offset(uy, cell_h.saturating_add(gap_y), row);

            // Last column (of this row) absorbs the horizontal remainder.
            let w = if col == row_cols - 1 {
                remainder(ux, uw, cx)
            } else {
                cell_w
            };
            // Last row absorbs vertical remainder.
            let h = if row == rows - 1 {
                remainder(uy, uh, cy)
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
        let gap = self.gap_for(uw, n);

        let total_gaps = gap * n.saturating_sub(1);
        let col_w = uw.saturating_sub(total_gaps) / n;

        let mut result = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let cx = offset(ux, col_w.saturating_add(gap), idx);
            // Last column absorbs rounding remainder.
            let w = if idx == n - 1 {
                remainder(ux, uw, cx)
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
        let gap = self.gap_for(uh, n);

        let total_gaps = gap * n.saturating_sub(1);
        let row_h = uh.saturating_sub(total_gaps) / n;

        let mut result = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let idx = i as u32;
            let ry = offset(uy, row_h.saturating_add(gap), idx);
            // Last row absorbs rounding remainder.
            let h = if idx == n - 1 {
                remainder(uy, uh, ry)
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

/// Pixels left between `pos` and the far edge of the span `start..start+len`
/// (the last tile absorbs the rounding remainder). Zero, not a wrapped
/// `u32`, when an overcrowded layout puts `pos` past the edge.
fn remainder(start: i32, len: u32, pos: i32) -> u32 {
    (i64::from(start) + i64::from(len) - i64::from(pos)).max(0) as u32
}

/// `start + step * idx`, computed in `i64` and saturated to `i32`.
fn offset(start: i32, step: u32, idx: u32) -> i32 {
    to_i32(i64::from(start) + i64::from(step) * i64::from(idx))
}

/// Saturating `i64` -> `i32`.
fn to_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

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
mod tests;

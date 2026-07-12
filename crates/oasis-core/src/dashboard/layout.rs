//! Icon layout: grid cells vs. free desktop-style positions.
//!
//! Grid mode keeps the classic uniform-cell layout computed by
//! [`GridLayout::cell_rect`]. Free mode gives every icon a fixed-size cell
//! placed either at a stored per-app position or auto-flowed
//! top-to-bottom, left-to-right in columns (classic desktop fill order).
//! Both modes expose the same [`DashboardState::icon_rect`] /
//! [`DashboardState::icon_at`] interface, so hit-testing and rendering
//! never care which layout is active.
//!
//! [`GridLayout::cell_rect`]: crate::ui::flex::GridLayout::cell_rect

use super::DashboardState;

impl DashboardState {
    /// Cell size for the active layout mode.
    pub fn cell_size(&self) -> (u32, u32) {
        if self.config.free_layout {
            (self.config.free_cell_w, self.config.free_cell_h)
        } else {
            (self.config.cell_w, self.config.cell_h)
        }
    }

    /// Cell origins (top-left, screen coords) for every icon on the
    /// current page, indexed by page position.
    ///
    /// Grid mode delegates to `GridLayout::cell_rect` — identical output
    /// to the historical per-icon computation. Free mode uses stored
    /// positions where available and column-major auto-flow for the rest,
    /// skipping cells already claimed by a placed icon.
    pub fn page_cell_origins(&self) -> Vec<(i32, i32)> {
        let count = self.current_page_apps().len();
        if !self.config.free_layout {
            let per_page = self.config.icons_per_page as usize;
            return (0..count)
                .map(|i| {
                    self.config
                        .grid_layout
                        .cell_rect(
                            i,
                            self.config.grid_x,
                            self.config.grid_y,
                            self.config.grid_w,
                            self.config.grid_h,
                            per_page,
                        )
                        .map(|r| (r.x, r.y))
                        .unwrap_or((self.config.grid_x, self.config.grid_y))
                })
                .collect();
        }

        let cw = self.config.free_cell_w.max(1) as i32;
        let ch = self.config.free_cell_h.max(1) as i32;
        let cols = ((self.config.grid_w as i32 / cw).max(1)) as usize;
        let rows = ((self.config.grid_h as i32 / ch).max(1)) as usize;

        // Pass 1: place icons with stored positions and mark the flow
        // cell nearest each one as occupied so auto-flow steers around it.
        let mut origins: Vec<Option<(i32, i32)>> = vec![None; count];
        let mut occupied = vec![false; cols * rows];
        for (i, app) in self.current_page_apps().iter().enumerate() {
            if let Some(&(x, y)) = self.positions.get(&app.path) {
                let (x, y) = self.clamp_free_position(x, y);
                origins[i] = Some((x, y));
                let col = ((x - self.config.grid_x + cw / 2) / cw).clamp(0, cols as i32 - 1);
                let row = ((y - self.config.grid_y + ch / 2) / ch).clamp(0, rows as i32 - 1);
                occupied[col as usize * rows + row as usize] = true;
            }
        }

        // Pass 2: auto-flow the rest column-major (top-to-bottom, then
        // next column). Overflow past the last column keeps stacking in
        // extra columns off the right edge rather than vanishing.
        let mut flow = 0usize;
        for origin in origins.iter_mut() {
            if origin.is_some() {
                continue;
            }
            while flow < cols * rows && occupied[flow] {
                flow += 1;
            }
            let (col, row) = if flow < cols * rows {
                occupied[flow] = true;
                (flow / rows, flow % rows)
            } else {
                let over = flow - cols * rows;
                (cols + over / rows, over % rows)
            };
            flow += 1;
            *origin = Some((
                self.config.grid_x + col as i32 * cw,
                self.config.grid_y + row as i32 * ch,
            ));
        }

        origins.into_iter().flatten().collect()
    }

    /// Cell rect `(x, y, w, h)` for icon `i` on the current page.
    pub fn icon_rect(&self, i: usize) -> Option<(i32, i32, u32, u32)> {
        let origins = self.page_cell_origins();
        let &(x, y) = origins.get(i)?;
        let (w, h) = self.cell_size();
        Some((x, y, w, h))
    }

    /// Hit-test a point against the current page's icon cells.
    ///
    /// Returns the page index of the topmost icon whose cell contains the
    /// point. Later icons win ties (they draw on top), and an actively
    /// dragged icon always wins.
    pub fn icon_at(&self, x: i32, y: i32) -> Option<usize> {
        let origins = self.page_cell_origins();
        let (w, h) = self.cell_size();
        let hit =
            |&(ox, oy): &(i32, i32)| x >= ox && x < ox + w as i32 && y >= oy && y < oy + h as i32;
        if let Some(drag) = self.drag_index
            && origins.get(drag).is_some_and(hit)
        {
            return Some(drag);
        }
        origins
            .iter()
            .enumerate()
            .rev()
            .find(|(_, o)| hit(o))
            .map(|(i, _)| i)
    }

    /// Clamp a free-mode cell origin so the cell stays inside the grid
    /// area (icons can't be dropped under the bars or off-screen).
    pub fn clamp_free_position(&self, x: i32, y: i32) -> (i32, i32) {
        let max_x = self.config.grid_x
            + (self.config.grid_w.saturating_sub(self.config.free_cell_w)) as i32;
        let max_y = self.config.grid_y
            + (self.config.grid_h.saturating_sub(self.config.free_cell_h)) as i32;
        (
            x.clamp(self.config.grid_x, max_x.max(self.config.grid_x)),
            y.clamp(self.config.grid_y, max_y.max(self.config.grid_y)),
        )
    }

    /// Commit a free-mode position for icon `i` on the current page
    /// (drag drop). Applies grid snapping when `snap_to_grid` is set and
    /// clamps into the grid area. Returns the stored origin, or `None`
    /// if the index has no app or the layout is not free.
    pub fn set_icon_position(&mut self, i: usize, x: i32, y: i32) -> Option<(i32, i32)> {
        if !self.config.free_layout {
            return None;
        }
        let path = self.current_page_apps().get(i)?.path.clone();
        let (mut x, mut y) = (x, y);
        if self.config.snap_to_grid {
            let cw = self.config.free_cell_w.max(1) as i32;
            let ch = self.config.free_cell_h.max(1) as i32;
            x = self.config.grid_x + ((x - self.config.grid_x + cw / 2) / cw) * cw;
            y = self.config.grid_y + ((y - self.config.grid_y + ch / 2) / ch) * ch;
        }
        let (x, y) = self.clamp_free_position(x, y);
        self.positions.insert(path, (x, y));
        Some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{test_apps, test_config};
    use super::super::{DashboardConfig, DashboardState};

    fn free_config() -> DashboardConfig {
        DashboardConfig {
            free_layout: true,
            free_cell_w: 80,
            free_cell_h: 80,
            snap_to_grid: true,
            grid_x: 16,
            grid_y: 48,
            grid_w: 400,
            grid_h: 200,
            ..test_config()
        }
    }

    #[test]
    fn grid_origins_match_cell_rect() {
        let dash = DashboardState::new(test_config(), test_apps(4));
        let origins = dash.page_cell_origins();
        assert_eq!(origins.len(), 4);
        for (i, &(x, y)) in origins.iter().enumerate() {
            let r = dash
                .config
                .grid_layout
                .cell_rect(i, 16, 48, 220, 190, 4)
                .unwrap();
            assert_eq!((x, y), (r.x, r.y));
        }
    }

    #[test]
    fn free_autoflow_is_column_major() {
        let dash = DashboardState::new(free_config(), test_apps(4));
        let origins = dash.page_cell_origins();
        // grid_h=200 / cell 80 => 2 rows per column.
        assert_eq!(origins[0], (16, 48));
        assert_eq!(origins[1], (16, 128));
        assert_eq!(origins[2], (96, 48));
        assert_eq!(origins[3], (96, 128));
    }

    #[test]
    fn free_stored_position_used_and_flow_skips_cell() {
        let mut dash = DashboardState::new(free_config(), test_apps(3));
        // Place app 0 where auto-flow would put the second icon.
        dash.positions.insert("/apps/app0".to_string(), (16, 128));
        let origins = dash.page_cell_origins();
        assert_eq!(origins[0], (16, 128));
        // Others flow around the occupied cell.
        assert_eq!(origins[1], (16, 48));
        assert_eq!(origins[2], (96, 48));
    }

    #[test]
    fn free_overflow_stacks_extra_columns() {
        // 400x200 area with 80px cells = 5 cols x 2 rows = 10 slots.
        let dash = DashboardState::new(
            DashboardConfig {
                icons_per_page: 12,
                ..free_config()
            },
            test_apps(12),
        );
        let origins = dash.page_cell_origins();
        assert_eq!(origins.len(), 12);
        // Slot 10 overflows into a 6th column.
        assert_eq!(origins[10], (16 + 5 * 80, 48));
        assert_eq!(origins[11], (16 + 5 * 80, 128));
    }

    #[test]
    fn icon_at_hits_cells() {
        let dash = DashboardState::new(free_config(), test_apps(2));
        assert_eq!(dash.icon_at(20, 50), Some(0));
        assert_eq!(dash.icon_at(20, 130), Some(1));
        assert_eq!(dash.icon_at(300, 60), None);
        // Grid mode hit-testing works too.
        let grid = DashboardState::new(test_config(), test_apps(4));
        assert_eq!(grid.icon_at(17, 49), Some(0));
        assert_eq!(grid.icon_at(16 + 110, 48), Some(1));
        assert_eq!(grid.icon_at(0, 0), None);
    }

    #[test]
    fn set_position_snaps_and_clamps() {
        let mut dash = DashboardState::new(free_config(), test_apps(2));
        // Near (100, 130) snaps to the (96, 128) cell.
        let stored = dash.set_icon_position(0, 100, 135).unwrap();
        assert_eq!(stored, (96, 128));
        assert_eq!(dash.positions["/apps/app0"], (96, 128));
        // Way off-screen clamps into the grid area.
        let stored = dash.set_icon_position(1, -500, 5000).unwrap();
        assert_eq!(stored.0, 16);
        assert_eq!(stored.1, 48 + 200 - 80);
    }

    #[test]
    fn set_position_noop_in_grid_mode() {
        let mut dash = DashboardState::new(test_config(), test_apps(2));
        assert_eq!(dash.set_icon_position(0, 100, 100), None);
        assert!(dash.positions.is_empty());
    }

    #[test]
    fn drag_index_wins_hit_test() {
        let mut dash = DashboardState::new(free_config(), test_apps(2));
        // Move icon 0 on top of icon 1's cell while dragging it.
        dash.positions.insert("/apps/app0".to_string(), (16, 128));
        dash.drag_index = Some(0);
        assert_eq!(dash.icon_at(20, 130), Some(0));
        dash.drag_index = None;
        // Without a drag, the later icon wins the overlap.
        // (app1 auto-flows to (16, 48), so (20, 130) only hits app0.)
        assert_eq!(dash.icon_at(20, 130), Some(0));
    }
}

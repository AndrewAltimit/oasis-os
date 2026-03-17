//! Tile-based rendering infrastructure for large pages.
//!
//! Divides the page content area into fixed-size tiles and tracks which
//! tiles are visible and dirty. This is infrastructure for future GPU
//! tile caching (Phase 6) — rendered tiles can be cached as GPU textures
//! and only newly visible tiles need re-rendering on scroll.

/// Tile size in pixels (256x256 is a common GPU-friendly size).
pub const TILE_SIZE: u32 = 256;

/// A single rendered tile.
#[derive(Debug, Clone)]
pub struct Tile {
    /// Screen-space Y position of this tile's top edge (relative to
    /// the content origin, not the viewport).
    pub y: i32,
    /// Whether this tile's content needs to be re-rendered.
    pub dirty: bool,
}

/// Manages tile state for the page content area.
///
/// Each tile covers a horizontal strip of `TILE_SIZE` pixels in height
/// spanning the full content width. The grid tracks which tiles are
/// dirty (need re-rendering) and which are visible in the viewport.
#[derive(Debug, Clone)]
pub struct TileGrid {
    /// Content width in pixels.
    width: u32,
    /// Total content height in pixels.
    content_height: u32,
    /// Tile rows (each row covers `TILE_SIZE` pixels of height).
    tiles: Vec<Tile>,
}

impl TileGrid {
    /// Create a new tile grid for the given content dimensions.
    ///
    /// All tiles start as dirty (needing initial render).
    pub fn new(width: u32, content_height: u32) -> Self {
        let num_rows = tile_count(content_height);
        let mut tiles = Vec::with_capacity(num_rows);
        for i in 0..num_rows {
            tiles.push(Tile {
                y: (i as u32 * TILE_SIZE) as i32,
                dirty: true,
            });
        }
        Self {
            width,
            content_height,
            tiles,
        }
    }

    /// Return the range of tile indices that are visible given the
    /// current scroll position and viewport height.
    ///
    /// `scroll_y` is the scroll offset in pixels (0 = top of page).
    /// `viewport_height` is the visible area height in pixels.
    ///
    /// Returns `(start_idx, end_idx)` where `end_idx` is exclusive.
    pub fn visible_range(&self, scroll_y: i32, viewport_height: u32) -> (usize, usize) {
        if self.tiles.is_empty() {
            return (0, 0);
        }
        let first = (scroll_y.max(0) as u32 / TILE_SIZE) as usize;
        let last_pixel = (scroll_y.max(0) as u32).saturating_add(viewport_height);
        // Integer ceiling division for the end index.
        let last = last_pixel.div_ceil(TILE_SIZE) as usize;
        let clamped_first = first.min(self.tiles.len());
        let clamped_last = last.min(self.tiles.len());
        (clamped_first, clamped_last)
    }

    /// Mark all tiles as dirty (e.g. after a layout change).
    pub fn mark_all_dirty(&mut self) {
        for tile in &mut self.tiles {
            tile.dirty = true;
        }
    }

    /// Mark a specific tile as clean (rendered).
    pub fn mark_clean(&mut self, idx: usize) {
        if let Some(tile) = self.tiles.get_mut(idx) {
            tile.dirty = false;
        }
    }

    /// Returns true if the tile at `idx` is dirty.
    pub fn is_dirty(&self, idx: usize) -> bool {
        self.tiles.get(idx).is_some_and(|t| t.dirty)
    }

    /// Number of tile rows.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Content width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Total content height.
    pub fn content_height(&self) -> u32 {
        self.content_height
    }

    /// Access the tiles slice.
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// Resize the grid for new content dimensions.
    ///
    /// Reuses existing tile storage when possible. All tiles are
    /// marked dirty after resize.
    pub fn resize(&mut self, width: u32, content_height: u32) {
        self.width = width;
        self.content_height = content_height;
        let num_rows = tile_count(content_height);
        self.tiles.clear();
        self.tiles
            .reserve(num_rows.saturating_sub(self.tiles.capacity()));
        for i in 0..num_rows {
            self.tiles.push(Tile {
                y: (i as u32 * TILE_SIZE) as i32,
                dirty: true,
            });
        }
    }
}

/// Compute the number of tile rows needed for a given content height.
fn tile_count(content_height: u32) -> usize {
    content_height.div_ceil(TILE_SIZE) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_grid_creates_correct_tiles() {
        let grid = TileGrid::new(480, 600);
        // 600 / 256 = 2.34 → 3 tiles.
        assert_eq!(grid.tile_count(), 3);
        assert_eq!(grid.tiles()[0].y, 0);
        assert_eq!(grid.tiles()[1].y, 256);
        assert_eq!(grid.tiles()[2].y, 512);
        // All start dirty.
        assert!(grid.is_dirty(0));
        assert!(grid.is_dirty(1));
        assert!(grid.is_dirty(2));
    }

    #[test]
    fn visible_range_at_top() {
        let grid = TileGrid::new(480, 1024);
        // 4 tiles: 0-255, 256-511, 512-767, 768-1023.
        assert_eq!(grid.tile_count(), 4);
        let (start, end) = grid.visible_range(0, 272);
        // scroll=0, viewport=272 → visible pixels 0..272.
        // Tile 0 (0..255) and tile 1 (256..511) are visible.
        assert_eq!(start, 0);
        assert_eq!(end, 2);
    }

    #[test]
    fn visible_range_scrolled() {
        let grid = TileGrid::new(480, 2048);
        let (start, end) = grid.visible_range(300, 272);
        // scroll=300, viewport=272 → visible pixels 300..572.
        // Tile 1 (256..511) and tile 2 (512..767) are visible.
        assert_eq!(start, 1);
        assert_eq!(end, 3);
    }

    #[test]
    fn visible_range_clamps_to_bounds() {
        let grid = TileGrid::new(480, 512);
        // 2 tiles: 0-255, 256-511.
        let (start, end) = grid.visible_range(0, 9999);
        assert_eq!(start, 0);
        assert_eq!(end, 2);
    }

    #[test]
    fn mark_clean_and_dirty() {
        let mut grid = TileGrid::new(480, 512);
        assert!(grid.is_dirty(0));
        grid.mark_clean(0);
        assert!(!grid.is_dirty(0));
        assert!(grid.is_dirty(1));
        grid.mark_all_dirty();
        assert!(grid.is_dirty(0));
        assert!(grid.is_dirty(1));
    }

    #[test]
    fn resize_rebuilds_tiles() {
        let mut grid = TileGrid::new(480, 256);
        assert_eq!(grid.tile_count(), 1);
        grid.mark_clean(0);
        assert!(!grid.is_dirty(0));

        grid.resize(480, 600);
        assert_eq!(grid.tile_count(), 3);
        // All dirty after resize.
        assert!(grid.is_dirty(0));
        assert!(grid.is_dirty(1));
        assert!(grid.is_dirty(2));
    }

    #[test]
    fn empty_content_height() {
        let grid = TileGrid::new(480, 0);
        assert_eq!(grid.tile_count(), 0);
        let (start, end) = grid.visible_range(0, 272);
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn exact_tile_boundary() {
        let grid = TileGrid::new(480, 512);
        // Exactly 2 tiles.
        assert_eq!(grid.tile_count(), 2);
        let (start, end) = grid.visible_range(0, 256);
        // Viewport is exactly one tile tall.
        assert_eq!(start, 0);
        assert_eq!(end, 1);
    }
}

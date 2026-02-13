//! CSS 2.1 float layout.
//!
//! Implements `float: left`, `float: right`, and `clear: left/right/both`.
//! The float context tracks active floats and provides available-width
//! queries for inline layout. Inline content wraps around floated boxes
//! by querying the float context for the left offset and available width
//! at each vertical position.

use super::box_model::Rect;

// -------------------------------------------------------------------
// FloatSide
// -------------------------------------------------------------------

/// Whether a float is placed on the left or right side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatSide {
    Left,
    Right,
}

// -------------------------------------------------------------------
// ClearSide
// -------------------------------------------------------------------

/// Which side(s) to clear past when resolving `clear`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearSide {
    Left,
    Right,
    Both,
}

// -------------------------------------------------------------------
// FloatBox
// -------------------------------------------------------------------

/// A floated box with its side and positioned rectangle.
///
/// The `rect` describes the margin box of the float in the coordinate
/// space of its containing block.
#[derive(Debug, Clone)]
pub struct FloatBox {
    /// Whether this is a left or right float.
    pub side: FloatSide,
    /// The positioned rectangle of the float (margin box).
    pub rect: Rect,
}

// -------------------------------------------------------------------
// FloatContext
// -------------------------------------------------------------------

/// Tracks active floats and provides available-width queries for
/// inline layout.
///
/// Left floats are anchored to the left edge of the containing block
/// and stack rightward. Right floats are anchored to the right edge
/// and stack leftward. Inline content queries this context to find
/// the usable horizontal band at any given vertical position.
#[derive(Debug, Default)]
pub struct FloatContext {
    /// All active left floats, ordered by placement.
    left_floats: Vec<FloatBox>,
    /// All active right floats, ordered by placement.
    right_floats: Vec<FloatBox>,
}

impl FloatContext {
    /// Create an empty float context with no active floats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Place a float at the given vertical position within a
    /// containing block of `containing_width`.
    ///
    /// The float is positioned as far to its side as possible without
    /// overlapping existing floats at the same vertical band. Returns
    /// the positioned [`FloatBox`].
    ///
    /// # Arguments
    ///
    /// * `side` -- left or right float
    /// * `width` -- the margin-box width of the float
    /// * `height` -- the margin-box height of the float
    /// * `y` -- the top edge of the line where the float appears
    /// * `containing_width` -- width of the containing block
    pub fn place_float(
        &mut self,
        side: FloatSide,
        width: f32,
        height: f32,
        y: f32,
        containing_width: f32,
    ) -> FloatBox {
        // Find the y position where the float can actually fit.
        // It must be at or below `y`, and there must be enough
        // horizontal room between existing floats.
        let placed_y = self.find_y_for_float(side, width, height, y, containing_width);

        let rect = match side {
            FloatSide::Left => {
                let left_edge = self.left_edge_at(placed_y, height);
                Rect::new(left_edge, placed_y, width, height)
            },
            FloatSide::Right => {
                let right_edge = self.right_edge_at(placed_y, height, containing_width);
                let x = right_edge - width;
                Rect::new(x, placed_y, width, height)
            },
        };

        let float_box = FloatBox { side, rect };

        match side {
            FloatSide::Left => self.left_floats.push(float_box.clone()),
            FloatSide::Right => {
                self.right_floats.push(float_box.clone());
            },
        }

        float_box
    }

    /// Get available width for inline content at a given vertical
    /// band `[y, y + height)`.
    ///
    /// Returns `(left_offset, available_width)` where `left_offset`
    /// is the x coordinate where inline content may start and
    /// `available_width` is the horizontal space remaining.
    pub fn available_width(&self, y: f32, height: f32, containing_width: f32) -> (f32, f32) {
        let left = self.left_offset(y, height);
        let right = self.right_edge_at(y, height, containing_width);
        let width = (right - left).max(0.0);
        (left, width)
    }

    /// Get the left offset for inline content at a given vertical
    /// band `[y, y + height)`.
    ///
    /// This is the rightmost right-edge among all left floats that
    /// overlap the band, or 0.0 if no left floats are active there.
    pub fn left_offset(&self, y: f32, height: f32) -> f32 {
        self.left_edge_at(y, height)
    }

    /// Get the y position below all floats on the specified side(s).
    ///
    /// Used to implement `clear: left|right|both`. The returned y is
    /// at or below the bottom edges of all relevant floats.
    pub fn clear_y(&self, clear: ClearSide) -> f32 {
        let left_bottom = match clear {
            ClearSide::Left | ClearSide::Both => self
                .left_floats
                .iter()
                .map(|f| f.rect.y + f.rect.height)
                .fold(0.0_f32, f32::max),
            ClearSide::Right => 0.0,
        };

        let right_bottom = match clear {
            ClearSide::Right | ClearSide::Both => self
                .right_floats
                .iter()
                .map(|f| f.rect.y + f.rect.height)
                .fold(0.0_f32, f32::max),
            ClearSide::Left => 0.0,
        };

        left_bottom.max(right_bottom)
    }

    /// Remove floats whose bottom edge is at or above `y`.
    ///
    /// This can be used to prune floats that are no longer relevant
    /// as layout progresses downward.
    pub fn remove_expired(&mut self, y: f32) {
        self.left_floats.retain(|f| f.rect.y + f.rect.height > y);
        self.right_floats.retain(|f| f.rect.y + f.rect.height > y);
    }

    /// Returns `true` if there are no active floats.
    pub fn is_empty(&self) -> bool {
        self.left_floats.is_empty() && self.right_floats.is_empty()
    }

    /// Total number of active floats (left + right).
    pub fn len(&self) -> usize {
        self.left_floats.len() + self.right_floats.len()
    }

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    /// The leftmost x where inline content can start at the vertical
    /// band `[y, y + height)`, considering left floats.
    fn left_edge_at(&self, y: f32, height: f32) -> f32 {
        let band_top = y;
        let band_bottom = y + height;
        self.left_floats
            .iter()
            .filter(|f| overlaps_band(f, band_top, band_bottom))
            .map(|f| f.rect.x + f.rect.width)
            .fold(0.0_f32, f32::max)
    }

    /// The rightmost x where inline content can end at the vertical
    /// band `[y, y + height)`, considering right floats.
    fn right_edge_at(&self, y: f32, height: f32, containing_width: f32) -> f32 {
        let band_top = y;
        let band_bottom = y + height;
        self.right_floats
            .iter()
            .filter(|f| overlaps_band(f, band_top, band_bottom))
            .map(|f| f.rect.x)
            .fold(containing_width, f32::min)
    }

    /// Find a y position at or below `start_y` where a float of the
    /// given dimensions can fit without exceeding the containing
    /// width.
    fn find_y_for_float(
        &self,
        side: FloatSide,
        width: f32,
        height: f32,
        start_y: f32,
        containing_width: f32,
    ) -> f32 {
        let mut y = start_y;

        // Iterate up to a reasonable limit to avoid infinite loops.
        for _ in 0..1000 {
            let left = self.left_edge_at(y, height);
            let right = self.right_edge_at(y, height, containing_width);
            let available = right - left;

            match side {
                FloatSide::Left => {
                    // The float needs `width` starting from `left`.
                    if left + width <= right || available >= width {
                        return y;
                    }
                },
                FloatSide::Right => {
                    // The float needs `width` ending at `right`.
                    if available >= width {
                        return y;
                    }
                },
            }

            // Move below the lowest-bottomed overlapping float.
            let next_y = self.next_clear_y_after(y, height);
            if next_y <= y {
                // No more floats to clear; place here regardless.
                return y;
            }
            y = next_y;
        }

        y
    }

    /// Find the smallest float bottom-edge that is within the band
    /// `[y, y + height)`, giving us the next candidate y to try when
    /// the current position does not have enough room.
    fn next_clear_y_after(&self, y: f32, height: f32) -> f32 {
        let band_top = y;
        let band_bottom = y + height;

        self.left_floats
            .iter()
            .chain(self.right_floats.iter())
            .filter(|f| overlaps_band(f, band_top, band_bottom))
            .map(|f| f.rect.y + f.rect.height)
            .fold(y, f32::max)
    }
}

/// Check whether a float's vertical extent overlaps the band
/// `[band_top, band_bottom)`.
fn overlaps_band(float_box: &FloatBox, band_top: f32, band_bottom: f32) -> bool {
    let float_top = float_box.rect.y;
    let float_bottom = float_box.rect.y + float_box.rect.height;
    float_top < band_bottom && float_bottom > band_top
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINING_WIDTH: f32 = 480.0;
    const LINE_HEIGHT: f32 = 20.0;

    #[test]
    fn empty_context_full_width_available() {
        let ctx = FloatContext::new();
        let (offset, width) = ctx.available_width(0.0, LINE_HEIGHT, CONTAINING_WIDTH);
        assert_eq!(offset, 0.0);
        assert_eq!(width, CONTAINING_WIDTH);
    }

    #[test]
    fn place_left_float() {
        let mut ctx = FloatContext::new();
        let fb = ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        assert_eq!(fb.side, FloatSide::Left);
        assert_eq!(fb.rect.x, 0.0);
        assert_eq!(fb.rect.y, 0.0);
        assert_eq!(fb.rect.width, 100.0);
        assert_eq!(fb.rect.height, 50.0);
        assert_eq!(ctx.len(), 1);
    }

    #[test]
    fn place_right_float() {
        let mut ctx = FloatContext::new();
        let fb = ctx.place_float(FloatSide::Right, 120.0, 40.0, 0.0, CONTAINING_WIDTH);
        assert_eq!(fb.side, FloatSide::Right);
        // Right float: x = containing_width - width = 480 - 120 = 360
        assert_eq!(fb.rect.x, 360.0);
        assert_eq!(fb.rect.y, 0.0);
        assert_eq!(fb.rect.width, 120.0);
        assert_eq!(fb.rect.height, 40.0);
    }

    #[test]
    fn available_width_with_left_float() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);

        // Within the float's vertical extent (y=0..50), the left
        // offset should be 100 and available width 380.
        let (offset, width) = ctx.available_width(10.0, LINE_HEIGHT, CONTAINING_WIDTH);
        assert_eq!(offset, 100.0);
        assert_eq!(width, 380.0);

        // Below the float (y=60), full width is available again.
        let (offset, width) = ctx.available_width(60.0, LINE_HEIGHT, CONTAINING_WIDTH);
        assert_eq!(offset, 0.0);
        assert_eq!(width, CONTAINING_WIDTH);
    }

    #[test]
    fn available_width_with_both_floats() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        ctx.place_float(FloatSide::Right, 80.0, 50.0, 0.0, CONTAINING_WIDTH);

        // Left offset = 100 (right edge of left float).
        // Right edge = 480 - 80 = 400 (left edge of right float).
        // Available = 400 - 100 = 300.
        let (offset, width) = ctx.available_width(10.0, LINE_HEIGHT, CONTAINING_WIDTH);
        assert_eq!(offset, 100.0);
        assert_eq!(width, 300.0);
    }

    #[test]
    fn clear_left_moves_below_left_floats() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        ctx.place_float(FloatSide::Left, 80.0, 30.0, 10.0, CONTAINING_WIDTH);

        // The bottom of the tallest left float: max(0+50, 10+30) = 50.
        let y = ctx.clear_y(ClearSide::Left);
        assert_eq!(y, 50.0);
    }

    #[test]
    fn clear_both_moves_below_all_floats() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        ctx.place_float(FloatSide::Right, 80.0, 80.0, 0.0, CONTAINING_WIDTH);

        // Left bottom = 50, right bottom = 80, clear both = 80.
        let y = ctx.clear_y(ClearSide::Both);
        assert_eq!(y, 80.0);
    }

    #[test]
    fn multiple_left_floats_stack_horizontally() {
        let mut ctx = FloatContext::new();
        let f1 = ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        let f2 = ctx.place_float(FloatSide::Left, 80.0, 50.0, 0.0, CONTAINING_WIDTH);

        // First float at x=0, second float should start at x=100
        // (right edge of first float).
        assert_eq!(f1.rect.x, 0.0);
        assert_eq!(f2.rect.x, 100.0);
        assert_eq!(f2.rect.width, 80.0);

        // Available width should account for both floats:
        // 480 - 100 - 80 = 300.
        let (offset, width) = ctx.available_width(10.0, LINE_HEIGHT, CONTAINING_WIDTH);
        assert_eq!(offset, 180.0); // 100 + 80
        assert_eq!(width, 300.0);
    }

    #[test]
    fn left_offset_with_no_floats() {
        let ctx = FloatContext::new();
        assert_eq!(ctx.left_offset(0.0, LINE_HEIGHT), 0.0);
    }

    #[test]
    fn remove_expired_prunes_old_floats() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 30.0, 0.0, CONTAINING_WIDTH);
        ctx.place_float(FloatSide::Right, 80.0, 60.0, 0.0, CONTAINING_WIDTH);

        assert_eq!(ctx.len(), 2);

        // Remove floats that ended at or before y=30.
        ctx.remove_expired(30.0);
        // Left float (bottom=30) is removed; right float
        // (bottom=60) remains.
        assert_eq!(ctx.len(), 1);
        assert!(ctx.left_floats.is_empty());
        assert_eq!(ctx.right_floats.len(), 1);
    }

    #[test]
    fn is_empty_and_len() {
        let mut ctx = FloatContext::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);

        ctx.place_float(FloatSide::Left, 50.0, 20.0, 0.0, CONTAINING_WIDTH);
        assert!(!ctx.is_empty());
        assert_eq!(ctx.len(), 1);
    }

    #[test]
    fn clear_right_only_considers_right_floats() {
        let mut ctx = FloatContext::new();
        ctx.place_float(FloatSide::Left, 100.0, 50.0, 0.0, CONTAINING_WIDTH);
        ctx.place_float(FloatSide::Right, 80.0, 30.0, 0.0, CONTAINING_WIDTH);

        let y = ctx.clear_y(ClearSide::Right);
        // Only right floats: bottom = 30.
        assert_eq!(y, 30.0);
    }

    #[test]
    fn float_drops_below_when_no_room() {
        let mut ctx = FloatContext::new();
        // Place a wide left float that takes most of the width.
        ctx.place_float(FloatSide::Left, 400.0, 30.0, 0.0, CONTAINING_WIDTH);

        // Try to place another left float of width 200. There is
        // only 80px remaining (480 - 400), so it should drop below
        // the first float to y=30.
        let fb = ctx.place_float(FloatSide::Left, 200.0, 25.0, 0.0, CONTAINING_WIDTH);
        assert_eq!(fb.rect.y, 30.0);
        assert_eq!(fb.rect.x, 0.0);
    }

    #[test]
    fn overlaps_band_function() {
        let fb = FloatBox {
            side: FloatSide::Left,
            rect: Rect::new(0.0, 10.0, 50.0, 20.0),
        };
        // Band fully overlaps.
        assert!(overlaps_band(&fb, 10.0, 30.0));
        // Band partially overlaps (bottom edge).
        assert!(overlaps_band(&fb, 25.0, 40.0));
        // Band partially overlaps (top edge).
        assert!(overlaps_band(&fb, 0.0, 15.0));
        // Band does not overlap (entirely below).
        assert!(!overlaps_band(&fb, 30.0, 50.0));
        // Band does not overlap (entirely above).
        assert!(!overlaps_band(&fb, 0.0, 10.0));
    }
}

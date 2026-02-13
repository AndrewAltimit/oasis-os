//! Layout helpers: centering, alignment, padding, distribution.

/// Padding specification for all four sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Padding {
    /// Top padding in pixels.
    pub top: u16,
    /// Right padding in pixels.
    pub right: u16,
    /// Bottom padding in pixels.
    pub bottom: u16,
    /// Left padding in pixels.
    pub left: u16,
}

impl Padding {
    /// Zero padding on all sides.
    pub const ZERO: Self = Self::uniform(0);

    /// Create uniform padding on all sides.
    pub const fn uniform(p: u16) -> Self {
        Self {
            top: p,
            right: p,
            bottom: p,
            left: p,
        }
    }

    /// Create symmetric padding (horizontal and vertical).
    pub const fn symmetric(h: u16, v: u16) -> Self {
        Self {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }

    /// Create padding with individual side values.
    pub const fn new(top: u16, right: u16, bottom: u16, left: u16) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Compute the inner rectangle after applying padding.
    pub fn inner_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        (
            x + self.left as i32,
            y + self.top as i32,
            w.saturating_sub(self.left as u32 + self.right as u32),
            h.saturating_sub(self.top as u32 + self.bottom as u32),
        )
    }

    /// Total horizontal padding (left + right).
    pub fn horizontal(&self) -> u32 {
        self.left as u32 + self.right as u32
    }

    /// Total vertical padding (top + bottom).
    pub fn vertical(&self) -> u32 {
        self.top as u32 + self.bottom as u32
    }
}

/// Compute centered position of a child within a parent.
pub fn center(parent_size: u32, child_size: u32) -> i32 {
    ((parent_size as i32 - child_size as i32) / 2).max(0)
}

/// Compute vertical center for text within a given height.
pub fn center_text_y(height: u32, font_size: u16, ascent: u32) -> i32 {
    let text_h = font_size as i32;
    (height as i32 - text_h) / 2 + ascent as i32
}

/// Distribute `n` items evenly across `total` pixels with `gap` pixels between.
///
/// Returns `(item_size, positions)`.
pub fn distribute(total: u32, n: u32, gap: u32) -> (u32, Vec<i32>) {
    if n == 0 {
        return (0, Vec::new());
    }
    let total_gap = gap * n.saturating_sub(1);
    let item_size = total.saturating_sub(total_gap) / n;
    let positions = (0..n).map(|i| (i * (item_size + gap)) as i32).collect();
    (item_size, positions)
}

/// Horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    /// Align to the left edge.
    Left,
    /// Align to the center.
    Center,
    /// Align to the right edge.
    Right,
}

/// Vertical alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    /// Align to the top edge.
    Top,
    /// Align to the center.
    Center,
    /// Align to the bottom edge.
    Bottom,
}

/// Compute x position for horizontal alignment.
pub fn align_x(container_w: u32, child_w: u32, align: HAlign) -> i32 {
    match align {
        HAlign::Left => 0,
        HAlign::Center => center(container_w, child_w),
        HAlign::Right => (container_w as i32 - child_w as i32).max(0),
    }
}

/// Compute y position for vertical alignment.
pub fn align_y(container_h: u32, child_h: u32, align: VAlign) -> i32 {
    match align {
        VAlign::Top => 0,
        VAlign::Center => center(container_h, child_h),
        VAlign::Bottom => (container_h as i32 - child_h as i32).max(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_inner_rect() {
        let p = Padding::uniform(4);
        let (x, y, w, h) = p.inner_rect(10, 10, 100, 50);
        assert_eq!((x, y, w, h), (14, 14, 92, 42));
    }

    #[test]
    fn center_calculation() {
        assert_eq!(center(100, 20), 40);
        assert_eq!(center(10, 20), 0); // Child larger than parent.
    }

    #[test]
    fn distribute_items() {
        let (size, pos) = distribute(100, 4, 4);
        assert_eq!(size, 22);
        assert_eq!(pos, vec![0, 26, 52, 78]);
    }
}

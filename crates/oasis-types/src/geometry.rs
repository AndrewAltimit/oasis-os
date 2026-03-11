//! Shared geometry primitives used across backends.

/// Axis-aligned rectangle for clipping and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl ClipRect {
    /// Compute the intersection of two clip rectangles.
    ///
    /// Returns `None` if the rectangles do not overlap.
    pub fn intersect(&self, other: &ClipRect) -> Option<ClipRect> {
        let ax2 = self.x.saturating_add(self.w as i32);
        let ay2 = self.y.saturating_add(self.h as i32);
        let bx2 = other.x.saturating_add(other.w as i32);
        let by2 = other.y.saturating_add(other.h as i32);
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let x2 = ax2.min(bx2);
        let y2 = ay2.min(by2);
        if x2 > x && y2 > y {
            Some(ClipRect {
                x,
                y,
                w: (x2 - x) as u32,
                h: (y2 - y) as u32,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_overlapping() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        let b = ClipRect {
            x: 50,
            y: 50,
            w: 100,
            h: 100,
        };
        let r = a.intersect(&b).unwrap();
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 50);
        assert_eq!(r.w, 50);
        assert_eq!(r.h, 50);
    }

    #[test]
    fn intersect_no_overlap() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let b = ClipRect {
            x: 20,
            y: 20,
            w: 10,
            h: 10,
        };
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn intersect_contained() {
        let outer = ClipRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        let inner = ClipRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
        };
        let r = outer.intersect(&inner).unwrap();
        assert_eq!(r, inner);
    }

    #[test]
    fn intersect_touching_edge() {
        let a = ClipRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let b = ClipRect {
            x: 10,
            y: 0,
            w: 10,
            h: 10,
        };
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn intersect_is_symmetric() {
        let a = ClipRect {
            x: 5,
            y: 5,
            w: 30,
            h: 30,
        };
        let b = ClipRect {
            x: 20,
            y: 10,
            w: 40,
            h: 40,
        };
        assert_eq!(a.intersect(&b), b.intersect(&a));
    }

    #[test]
    fn intersect_negative_coords() {
        let a = ClipRect {
            x: -10,
            y: -10,
            w: 30,
            h: 30,
        };
        let b = ClipRect {
            x: 0,
            y: 0,
            w: 20,
            h: 20,
        };
        let r = a.intersect(&b).unwrap();
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.w, 20);
        assert_eq!(r.h, 20);
    }
}

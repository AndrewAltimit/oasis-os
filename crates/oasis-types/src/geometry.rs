//! Shared geometry primitives used across backends.

/// Axis-aligned rectangle for clipping and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipRect {
    /// Left edge X coordinate.
    pub x: i32,
    /// Top edge Y coordinate.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl ClipRect {
    /// A rectangle at `(x, y)` of `w` x `h` pixels.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Exclusive right edge, `x + w`, computed without overflow.
    #[must_use]
    pub const fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }

    /// Exclusive bottom edge, `y + h`, computed without overflow.
    #[must_use]
    pub const fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    /// Compute the intersection of two clip rectangles.
    ///
    /// Returns `None` if the rectangles do not overlap. The edges are
    /// computed in `i64`, so sizes above `i32::MAX` (an "unbounded" clip
    /// such as `u32::MAX` x `u32::MAX`) intersect correctly.
    #[must_use]
    pub fn intersect(&self, other: &ClipRect) -> Option<ClipRect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        let (w, h) = (x2 - x as i64, y2 - y as i64);
        if w > 0 && h > 0 {
            Some(ClipRect {
                x,
                y,
                w: w.min(u32::MAX as i64) as u32,
                h: h.min(u32::MAX as i64) as u32,
            })
        } else {
            None
        }
    }
}

/// Integer square root (floor) for `i32` values.
///
/// Returns 0 for negative inputs. Uses Newton's method with integer
/// arithmetic -- no floating-point required, making this safe for
/// `no_std` / PSP environments.
///
/// Both the SDL and PSP backends need an `i32` variant for computing
/// rounded-rectangle corner insets where the intermediate `r*r - ry*ry`
/// expression is naturally `i32`.
pub fn isqrt_i32(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let n = n as u32;
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x as i32
}

/// Integer square root (floor) for `i64` values; 0 for negative inputs.
///
/// For squared distances that do not fit `i32` (a radius above 46340).
pub fn isqrt_i64(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let n = n as u64;
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x as i64
}

/// Compute the horizontal inset for a rounded-rectangle scanline.
///
/// Given a scanline offset `dy` (0-based from the top of the rect),
/// the total height `h`, and the corner radius `r`, returns the number
/// of pixels to inset from each side. Returns 0 for scanlines in the
/// straight middle section.
///
/// This is the shared formula used by both the SDL gradient rounded-rect
/// fill and the PSP GU rounded-rect fill.
pub fn rounded_rect_inset(dy: i32, h: i32, r: i32) -> i32 {
    // i64: `r * r` overflows i32 for radii above 46340.
    let inset = |ry: i64| {
        let r = r as i64;
        (r - isqrt_i64(r * r - ry * ry)) as i32
    };
    if dy < r {
        inset(r as i64 - dy as i64)
    } else if dy >= h - r {
        inset(dy as i64 - (h as i64 - 1 - r as i64))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_i64_matches_i32_and_handles_large_inputs() {
        for n in [0, 1, 2, 3, 4, 15, 16, 17, 99, 100, 1 << 20, i32::MAX] {
            assert_eq!(isqrt_i64(n as i64), isqrt_i32(n) as i64, "{n}");
        }
        assert_eq!(isqrt_i64(65535 * 65535), 65535);
        assert_eq!(isqrt_i64(-5), 0);
    }

    #[test]
    fn rounded_rect_inset_huge_radius_does_not_overflow() {
        // `r * r` overflowed i32 (a debug-build panic) above r = 46340.
        assert_eq!(rounded_rect_inset(0, 131_070, 65535), 65535);
        assert_eq!(rounded_rect_inset(65535, 131_070, 65535), 1);
    }

    #[test]
    fn intersect_huge_sizes_do_not_wrap() {
        // `w as i32` wrapped u32::MAX to -1, so an "unbounded" clip
        // intersected with anything came back empty.
        let unbounded = ClipRect::new(0, 0, u32::MAX, u32::MAX);
        let small = ClipRect::new(10, 10, 5, 5);
        assert_eq!(unbounded.intersect(&small), Some(small));
        assert_eq!(small.intersect(&unbounded), Some(small));
        assert_eq!(unbounded.intersect(&unbounded), Some(unbounded));
        // Far-left origin with a wide rect: no overflow in `x2 - x`.
        let far = ClipRect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX);
        assert_eq!(far.intersect(&small), Some(small));
        assert_eq!(far.right(), i32::MAX as i64);
    }

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

    #[test]
    fn isqrt_i32_known_values() {
        assert_eq!(isqrt_i32(0), 0);
        assert_eq!(isqrt_i32(1), 1);
        assert_eq!(isqrt_i32(4), 2);
        assert_eq!(isqrt_i32(9), 3);
        assert_eq!(isqrt_i32(16), 4);
        assert_eq!(isqrt_i32(25), 5);
        assert_eq!(isqrt_i32(100), 10);
    }

    #[test]
    fn isqrt_i32_non_perfect_squares() {
        assert_eq!(isqrt_i32(2), 1);
        assert_eq!(isqrt_i32(3), 1);
        assert_eq!(isqrt_i32(5), 2);
        assert_eq!(isqrt_i32(8), 2);
        assert_eq!(isqrt_i32(10), 3);
        assert_eq!(isqrt_i32(99), 9);
    }

    #[test]
    fn isqrt_i32_negative() {
        assert_eq!(isqrt_i32(-1), 0);
        assert_eq!(isqrt_i32(-100), 0);
    }

    #[test]
    fn rounded_rect_inset_middle_is_zero() {
        // Middle scanlines of a 100px-tall rect with r=10 have no inset.
        assert_eq!(rounded_rect_inset(50, 100, 10), 0);
        assert_eq!(rounded_rect_inset(10, 100, 10), 0);
        assert_eq!(rounded_rect_inset(89, 100, 10), 0);
    }

    #[test]
    fn rounded_rect_inset_corners_positive() {
        // Top and bottom corner rows should have a positive inset.
        assert!(rounded_rect_inset(0, 100, 10) > 0);
        assert!(rounded_rect_inset(99, 100, 10) > 0);
    }

    #[test]
    fn rounded_rect_inset_symmetric() {
        // Top and bottom should be symmetric.
        let h = 50;
        let r = 8;
        for dy in 0..r {
            let top = rounded_rect_inset(dy, h, r);
            let bot = rounded_rect_inset(h - 1 - dy, h, r);
            assert_eq!(top, bot, "asymmetric at dy={dy}");
        }
    }
}

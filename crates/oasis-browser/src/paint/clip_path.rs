//! `clip-path` shape resolution + rectangle intersection helpers.
//!
//! Reduces a parsed [`ClipPath`] (inset / rect / circle / ellipse) to
//! an axis-aligned bounding rect in layout coordinates, since the
//! current backend trait only exposes rectangular clipping. Also
//! provides the rectangle-intersection helper used by the recursive
//! paint walker to compose nested `overflow: hidden` clip rects.

use crate::css::values::{ClipLength, ClipPath};
use crate::layout::box_model::Rect;

/// Resolve a [`ClipPath`] shape to a bounding rect in the layout coordinate
/// space, anchored to the element's border box.
///
/// Circle/ellipse shapes are reduced to their axis-aligned bounding box —
/// the only clipping primitive the backend trait exposes today. Returns
/// `None` if the shape collapses to an empty rect.
pub(crate) fn resolve_clip_path_rect(shape: &ClipPath, border_box: &Rect) -> Option<Rect> {
    let bw = border_box.width;
    let bh = border_box.height;
    let bx = border_box.x;
    let by = border_box.y;

    let rect = match *shape {
        ClipPath::Inset {
            top,
            right,
            bottom,
            left,
        } => {
            let t = top.resolve(bh);
            let r = right.resolve(bw);
            let b = bottom.resolve(bh);
            let l = left.resolve(bw);
            Rect {
                x: bx + l,
                y: by + t,
                width: (bw - l - r).max(0.0),
                height: (bh - t - b).max(0.0),
            }
        },
        ClipPath::Rect {
            top,
            right,
            bottom,
            left,
        } => {
            let t = top.unwrap_or(0.0);
            let l = left.unwrap_or(0.0);
            let r = right.unwrap_or(bw);
            let b = bottom.unwrap_or(bh);
            Rect {
                x: bx + l,
                y: by + t,
                width: (r - l).max(0.0),
                height: (b - t).max(0.0),
            }
        },
        ClipPath::Circle { cx, cy, r } => {
            let ref_diag = ((bw * bw + bh * bh) / 2.0).sqrt();
            let radius = match r {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * ref_diag,
            };
            let cx = cx.resolve(bw);
            let cy = cy.resolve(bh);
            Rect {
                x: bx + cx - radius,
                y: by + cy - radius,
                width: (radius * 2.0).max(0.0),
                height: (radius * 2.0).max(0.0),
            }
        },
        ClipPath::Ellipse { cx, cy, rx, ry } => {
            let rx_px = match rx {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * bw,
            };
            let ry_px = match ry {
                ClipLength::Px(v) => v,
                ClipLength::Frac(f) => f * bh,
            };
            let cx = cx.resolve(bw);
            let cy = cy.resolve(bh);
            Rect {
                x: bx + cx - rx_px,
                y: by + cy - ry_px,
                width: (rx_px * 2.0).max(0.0),
                height: (ry_px * 2.0).max(0.0),
            }
        },
    };

    if rect.width <= 0.0 || rect.height <= 0.0 {
        None
    } else {
        Some(rect)
    }
}

/// Intersect two axis-aligned rectangles, returning a (possibly empty)
/// rectangle clamped to the overlap region.
pub(crate) fn intersect_rects(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect {
        x,
        y,
        width: (right - x).max(0.0),
        height: (bottom - y).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{ClipLength, ClipPath};

    #[test]
    fn clip_path_inset_resolves_to_shrunken_rect() {
        let bb = Rect {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
        };
        let shape = ClipPath::Inset {
            top: ClipLength::Px(10.0),
            right: ClipLength::Px(20.0),
            bottom: ClipLength::Px(30.0),
            left: ClipLength::Px(40.0),
        };
        let r = resolve_clip_path_rect(&shape, &bb).expect("non-empty");
        assert_eq!(r.x, 140.0);
        assert_eq!(r.y, 110.0);
        assert_eq!(r.width, 140.0);
        assert_eq!(r.height, 60.0);
    }

    #[test]
    fn clip_path_circle_half_width_bounding_box() {
        let bb = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let shape = ClipPath::Circle {
            cx: ClipLength::Frac(0.5),
            cy: ClipLength::Frac(0.5),
            r: ClipLength::Px(40.0),
        };
        let r = resolve_clip_path_rect(&shape, &bb).expect("non-empty");
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 10.0);
        assert_eq!(r.width, 80.0);
        assert_eq!(r.height, 80.0);
    }

    #[test]
    fn clip_path_inset_fully_collapsed_returns_none() {
        let bb = Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        };
        let shape = ClipPath::Inset {
            top: ClipLength::Frac(0.5),
            right: ClipLength::Frac(0.5),
            bottom: ClipLength::Frac(0.5),
            left: ClipLength::Frac(0.5),
        };
        assert!(resolve_clip_path_rect(&shape, &bb).is_none());
    }

    #[test]
    fn intersect_rects_overlap_returns_overlap_region() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let b = Rect {
            x: 50.0,
            y: 50.0,
            width: 100.0,
            height: 100.0,
        };
        let r = intersect_rects(a, b);
        assert_eq!(r.x, 50.0);
        assert_eq!(r.y, 50.0);
        assert_eq!(r.width, 50.0);
        assert_eq!(r.height, 50.0);
    }

    #[test]
    fn intersect_rects_disjoint_returns_zero_size() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let b = Rect {
            x: 100.0,
            y: 100.0,
            width: 10.0,
            height: 10.0,
        };
        let r = intersect_rects(a, b);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }
}

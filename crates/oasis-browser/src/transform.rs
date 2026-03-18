//! Shared 2D affine transform matrix.
//!
//! Provides [`AffineTransform2D`] — a reusable struct for composing
//! CSS transforms, SVG transform attributes, and hit-test inversions.

use crate::css::values::TransformFunction;

/// A 2D affine transform matrix `[a c e; b d f; 0 0 1]`.
///
/// ```text
/// | a  c  e |   | x |   | a*x + c*y + e |
/// | b  d  f | × | y | = | b*x + d*y + f |
/// | 0  0  1 |   | 1 |   |       1       |
/// ```
#[derive(Debug, Clone, Copy)]
pub(crate) struct AffineTransform2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl AffineTransform2D {
    /// Identity matrix (no transformation).
    pub fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Pure translation.
    pub fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// Pure scale.
    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Pure rotation (angle in degrees).
    pub fn rotate(angle_deg: f32) -> Self {
        let r = angle_deg * std::f32::consts::PI / 180.0;
        let (sin, cos) = (r.sin(), r.cos());
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Pure skew (angles in degrees).
    #[allow(dead_code)]
    pub fn skew(ax_deg: f32, ay_deg: f32) -> Self {
        let tan_x = (ax_deg * std::f32::consts::PI / 180.0).tan();
        let tan_y = (ay_deg * std::f32::consts::PI / 180.0).tan();
        Self {
            a: 1.0,
            b: tan_y,
            c: tan_x,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Compose two transforms: `self * other`.
    pub fn multiply(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    /// Transform a point `(x, y)` through the matrix.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Compute the inverse matrix. Returns `None` if singular.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-10 {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Self {
            a: self.d * inv_det,
            b: -self.b * inv_det,
            c: -self.c * inv_det,
            d: self.a * inv_det,
            e: (self.c * self.f - self.d * self.e) * inv_det,
            f: (self.b * self.e - self.a * self.f) * inv_det,
        })
    }

    /// Returns `true` if this transform is a pure translation (no
    /// rotation, scale, or skew). Used as a fast path — when true
    /// the existing offset-only paint code can run unchanged.
    pub fn is_translation_only(&self) -> bool {
        (self.a - 1.0).abs() < 1e-6
            && self.b.abs() < 1e-6
            && self.c.abs() < 1e-6
            && (self.d - 1.0).abs() < 1e-6
    }

    /// Build an affine matrix from a list of CSS `TransformFunction`s,
    /// applying them around the given transform-origin point.
    pub fn from_css_transforms(
        transforms: &[TransformFunction],
        origin_x: f32,
        origin_y: f32,
    ) -> Self {
        if transforms.is_empty() {
            return Self::identity();
        }

        let mut m = Self::identity();

        // Pre-translate: shift by -origin.
        m.e -= m.a * origin_x + m.b * origin_y;
        m.f -= m.c * origin_x + m.d * origin_y;

        for tf in transforms {
            match tf {
                TransformFunction::Translate(tx, ty) => {
                    m.e += m.a * tx + m.b * ty;
                    m.f += m.c * tx + m.d * ty;
                },
                TransformFunction::Scale(sx, sy) => {
                    m.a *= sx;
                    m.b *= sy;
                    m.c *= sx;
                    m.d *= sy;
                },
                TransformFunction::Rotate(deg) => {
                    let rad = deg.to_radians();
                    let cos = rad.cos();
                    let sin = rad.sin();
                    let na = m.a * cos + m.b * sin;
                    let nb = -m.a * sin + m.b * cos;
                    let nc = m.c * cos + m.d * sin;
                    let nd = -m.c * sin + m.d * cos;
                    m.a = na;
                    m.b = nb;
                    m.c = nc;
                    m.d = nd;
                },
                TransformFunction::Skew(ax, ay) => {
                    let tan_x = ax.to_radians().tan();
                    let tan_y = ay.to_radians().tan();
                    let na = m.a + m.b * tan_y;
                    let nb = m.a * tan_x + m.b;
                    let nc = m.c + m.d * tan_y;
                    let nd = m.c * tan_x + m.d;
                    m.a = na;
                    m.b = nb;
                    m.c = nc;
                    m.d = nd;
                },
                TransformFunction::Matrix(ma, mb, mc, md, me, mf) => {
                    let na = m.a * ma + m.b * mc;
                    let nb = m.a * mb + m.b * md;
                    let ne = m.a * me + m.b * mf + m.e;
                    let nc = m.c * ma + m.d * mc;
                    let nd = m.c * mb + m.d * md;
                    let nf = m.c * me + m.d * mf + m.f;
                    m.a = na;
                    m.b = nb;
                    m.c = nc;
                    m.d = nd;
                    m.e = ne;
                    m.f = nf;
                },
            }
        }

        // Post-translate: shift by +origin.
        m.e += m.a * origin_x + m.b * origin_y;
        m.f += m.c * origin_x + m.d * origin_y;

        m
    }

    /// Transform 4 corners of a rectangle into a quadrilateral.
    pub fn transform_rect_to_quad(&self, x: f32, y: f32, w: f32, h: f32) -> [(i32, i32); 4] {
        let (x0, y0) = self.apply(x, y);
        let (x1, y1) = self.apply(x + w, y);
        let (x2, y2) = self.apply(x + w, y + h);
        let (x3, y3) = self.apply(x, y + h);
        [
            (x0 as i32, y0 as i32),
            (x1 as i32, y1 as i32),
            (x2 as i32, y2 as i32),
            (x3 as i32, y3 as i32),
        ]
    }
}

/// Ear-clipping triangulation for concave polygons.
///
/// Returns a list of triangle index triples suitable for rendering
/// via `fill_triangle()`. Falls back gracefully for degenerate inputs.
pub(crate) fn ear_clip_triangulate(points: &[(f32, f32)]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);

    // Determine winding order (signed area).
    let area2: f32 = indices
        .windows(2)
        .map(|w| {
            let (ax, ay) = points[w[0]];
            let (bx, by) = points[w[1]];
            ax * by - bx * ay
        })
        .sum::<f32>()
        + {
            let (ax, ay) = points[*indices.last().unwrap_or(&0)];
            let (bx, by) = points[indices[0]];
            ax * by - bx * ay
        };
    let ccw = area2 > 0.0;

    let mut max_iters = n * n; // safety limit
    let mut i = 0;
    while indices.len() > 2 && max_iters > 0 {
        max_iters -= 1;
        let len = indices.len();
        let prev = indices[(i + len - 1) % len];
        let curr = indices[i % len];
        let next = indices[(i + 1) % len];

        if is_ear(points, &indices, prev, curr, next, ccw) {
            triangles.push([prev, curr, next]);
            indices.remove(i % len);
            if indices.len() <= 2 {
                break;
            }
            // Step back so we recheck the new triangle at this position.
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
        if i >= indices.len() {
            i = 0;
        }
    }

    triangles
}

/// Check if vertex `curr` forms an ear (convex vertex with no other
/// points inside the triangle prev-curr-next).
fn is_ear(
    points: &[(f32, f32)],
    indices: &[usize],
    prev: usize,
    curr: usize,
    next: usize,
    ccw: bool,
) -> bool {
    let (ax, ay) = points[prev];
    let (bx, by) = points[curr];
    let (cx, cy) = points[next];

    // Cross product determines convexity.
    let cross = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    if (ccw && cross <= 0.0) || (!ccw && cross >= 0.0) {
        return false; // reflex vertex
    }

    // Check no other vertex lies inside the triangle.
    for &idx in indices {
        if idx == prev || idx == curr || idx == next {
            continue;
        }
        if point_in_triangle(points[idx], (ax, ay), (bx, by), (cx, cy)) {
            return false;
        }
    }
    true
}

/// Barycentric point-in-triangle test.
fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let (px, py) = p;
    let d1 = (px - b.0) * (a.1 - b.1) - (a.0 - b.0) * (py - b.1);
    let d2 = (px - c.0) * (b.1 - c.1) - (b.0 - c.0) * (py - c.1);
    let d3 = (px - a.0) * (c.1 - a.1) - (c.0 - a.0) * (py - a.1);

    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(has_neg && has_pos)
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::TransformFunction;

    #[test]
    fn identity_is_no_op() {
        let m = AffineTransform2D::identity();
        let (x, y) = m.apply(10.0, 20.0);
        assert!((x - 10.0).abs() < 1e-6);
        assert!((y - 20.0).abs() < 1e-6);
        assert!(m.is_translation_only());
    }

    #[test]
    fn translate_moves_point() {
        let m = AffineTransform2D::translate(5.0, -3.0);
        let (x, y) = m.apply(10.0, 20.0);
        assert!((x - 15.0).abs() < 1e-6);
        assert!((y - 17.0).abs() < 1e-6);
        assert!(m.is_translation_only());
    }

    #[test]
    fn scale_stretches_point() {
        let m = AffineTransform2D::scale(2.0, 3.0);
        let (x, y) = m.apply(10.0, 20.0);
        assert!((x - 20.0).abs() < 1e-6);
        assert!((y - 60.0).abs() < 1e-6);
        assert!(!m.is_translation_only());
    }

    #[test]
    fn rotate_90_degrees() {
        let m = AffineTransform2D::rotate(90.0);
        let (x, y) = m.apply(10.0, 0.0);
        assert!(x.abs() < 1e-4);
        assert!((y - 10.0).abs() < 1e-4);
        assert!(!m.is_translation_only());
    }

    #[test]
    fn multiply_composes() {
        let t = AffineTransform2D::translate(10.0, 0.0);
        let s = AffineTransform2D::scale(2.0, 2.0);
        let m = t.multiply(&s);
        let (x, y) = m.apply(5.0, 0.0);
        // translate first, then scale applied to: (2*5 + 10, 0)
        assert!((x - 20.0).abs() < 1e-4);
        assert!(y.abs() < 1e-4);
    }

    #[test]
    fn inverse_roundtrip() {
        let m = AffineTransform2D::translate(10.0, 5.0)
            .multiply(&AffineTransform2D::rotate(30.0))
            .multiply(&AffineTransform2D::scale(2.0, 1.5));
        let inv = m.inverse().expect("non-singular");
        let composed = m.multiply(&inv);
        assert!(
            composed.is_translation_only() || {
                // Check it's close to identity
                (composed.a - 1.0).abs() < 1e-4
                    && composed.b.abs() < 1e-4
                    && composed.c.abs() < 1e-4
                    && (composed.d - 1.0).abs() < 1e-4
                    && composed.e.abs() < 1e-3
                    && composed.f.abs() < 1e-3
            }
        );
    }

    #[test]
    fn from_css_translate_only_is_translation() {
        let transforms = vec![TransformFunction::Translate(10.0, 20.0)];
        let m = AffineTransform2D::from_css_transforms(&transforms, 50.0, 25.0);
        assert!(m.is_translation_only());
        // The translation should be exactly (10, 20) relative to base.
        assert!((m.e - 10.0).abs() < 1e-4);
        assert!((m.f - 20.0).abs() < 1e-4);
    }

    #[test]
    fn from_css_rotate_is_not_translation() {
        let transforms = vec![TransformFunction::Rotate(45.0)];
        let m = AffineTransform2D::from_css_transforms(&transforms, 50.0, 25.0);
        assert!(!m.is_translation_only());
    }

    #[test]
    fn from_css_empty_is_identity() {
        let m = AffineTransform2D::from_css_transforms(&[], 50.0, 25.0);
        assert!(m.is_translation_only());
        assert!(m.e.abs() < 1e-6);
        assert!(m.f.abs() < 1e-6);
    }

    #[test]
    fn transform_rect_to_quad_identity() {
        let m = AffineTransform2D::identity();
        let quad = m.transform_rect_to_quad(10.0, 20.0, 100.0, 50.0);
        assert_eq!(quad[0], (10, 20));
        assert_eq!(quad[1], (110, 20));
        assert_eq!(quad[2], (110, 70));
        assert_eq!(quad[3], (10, 70));
    }

    #[test]
    fn ear_clip_triangle() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)];
        let tris = ear_clip_triangulate(&pts);
        assert_eq!(tris.len(), 1);
        assert_eq!(tris[0], [0, 1, 2]);
    }

    #[test]
    fn ear_clip_quad() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let tris = ear_clip_triangulate(&pts);
        assert_eq!(tris.len(), 2);
    }

    #[test]
    fn ear_clip_concave() {
        // L-shaped polygon (concave)
        let pts = vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (5.0, 5.0),
            (5.0, 10.0),
            (0.0, 10.0),
        ];
        let tris = ear_clip_triangulate(&pts);
        assert_eq!(tris.len(), 4); // n-2 = 4
    }

    #[test]
    fn ear_clip_degenerate() {
        let pts: Vec<(f32, f32)> = vec![];
        assert!(ear_clip_triangulate(&pts).is_empty());
        let pts = vec![(0.0, 0.0), (1.0, 1.0)];
        assert!(ear_clip_triangulate(&pts).is_empty());
    }
}

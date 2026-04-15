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
    ///
    /// 3D transform functions (`rotateX`, `translate3d`, `perspective`,
    /// etc.) are evaluated in 4×4 space via [`Matrix3d`] and then
    /// flattened orthographically — the Z column/row is dropped so
    /// `rotateX(60deg)` becomes a vertical squash, `rotateY(60deg)` a
    /// horizontal squash, etc. True perspective projection is a
    /// follow-up; until it lands, `perspective()` and `perspective`
    /// container properties only affect backface-visibility culling.
    pub fn from_css_transforms(
        transforms: &[TransformFunction],
        origin_x: f32,
        origin_y: f32,
    ) -> Self {
        if transforms.is_empty() {
            return Self::identity();
        }
        Matrix3d::from_css_transforms_3d(transforms, origin_x, origin_y).flatten_to_affine()
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

// -------------------------------------------------------------------
// 4x4 matrix for 3D transforms
// -------------------------------------------------------------------

/// A 4×4 transform matrix in column-major order.
///
/// Storage matches the CSS `matrix3d(...)` function arg order:
/// `m[0..4]` is column 0, `m[4..8]` is column 1, etc. Reading as a
/// mathematical matrix `M[row][col]` corresponds to `m[row + 4*col]`.
///
/// Used to compose CSS 3D transforms (`rotateX`, `translate3d`,
/// `perspective`, `matrix3d`, …) before flattening to a 2D affine for
/// paint. A full perspective-correct paint path is a follow-up — for
/// now we evaluate the 3D pipeline so the math is correct, then drop
/// the Z column/row at the end.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Matrix3d {
    pub m: [f32; 16],
}

impl Matrix3d {
    pub fn identity() -> Self {
        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        Self { m }
    }

    pub fn translate(tx: f32, ty: f32, tz: f32) -> Self {
        let mut m = Self::identity();
        m.m[12] = tx;
        m.m[13] = ty;
        m.m[14] = tz;
        m
    }

    pub fn scale(sx: f32, sy: f32, sz: f32) -> Self {
        let mut m = Self::identity();
        m.m[0] = sx;
        m.m[5] = sy;
        m.m[10] = sz;
        m
    }

    pub fn rotate_x(deg: f32) -> Self {
        let r = deg.to_radians();
        let (s, c) = (r.sin(), r.cos());
        let mut m = Self::identity();
        m.m[5] = c;
        m.m[6] = s;
        m.m[9] = -s;
        m.m[10] = c;
        m
    }

    pub fn rotate_y(deg: f32) -> Self {
        let r = deg.to_radians();
        let (s, c) = (r.sin(), r.cos());
        let mut m = Self::identity();
        m.m[0] = c;
        m.m[2] = -s;
        m.m[8] = s;
        m.m[10] = c;
        m
    }

    pub fn rotate_z(deg: f32) -> Self {
        let r = deg.to_radians();
        let (s, c) = (r.sin(), r.cos());
        let mut m = Self::identity();
        m.m[0] = c;
        m.m[1] = s;
        m.m[4] = -s;
        m.m[5] = c;
        m
    }

    /// Rotate `deg` around an arbitrary axis `(x, y, z)`. The axis is
    /// normalised internally; a zero-length axis returns the identity.
    pub fn rotate_axis(x: f32, y: f32, z: f32, deg: f32) -> Self {
        let len = (x * x + y * y + z * z).sqrt();
        if len < 1e-6 {
            return Self::identity();
        }
        let (x, y, z) = (x / len, y / len, z / len);
        let r = deg.to_radians();
        let (s, c) = (r.sin(), r.cos());
        let omc = 1.0 - c;
        // Standard Rodrigues / axis-angle to matrix, then transposed
        // for column-major storage.
        let mut m = Self::identity();
        m.m[0] = c + x * x * omc;
        m.m[1] = y * x * omc + z * s;
        m.m[2] = z * x * omc - y * s;
        m.m[4] = x * y * omc - z * s;
        m.m[5] = c + y * y * omc;
        m.m[6] = z * y * omc + x * s;
        m.m[8] = x * z * omc + y * s;
        m.m[9] = y * z * omc - x * s;
        m.m[10] = c + z * z * omc;
        m
    }

    /// CSS `perspective(d)` — viewer at distance `d` along +Z. Points
    /// further from the viewer (more negative z) shrink toward the
    /// origin under the eventual perspective divide.
    pub fn perspective(d: f32) -> Self {
        let mut m = Self::identity();
        if d > 0.0 {
            m.m[11] = -1.0 / d;
        }
        m
    }

    pub fn from_2d_affine(a: AffineTransform2D) -> Self {
        let mut m = Self::identity();
        m.m[0] = a.a;
        m.m[1] = a.b;
        m.m[4] = a.c;
        m.m[5] = a.d;
        m.m[12] = a.e;
        m.m[13] = a.f;
        m
    }

    /// Compose two matrices: `self * other`.
    pub fn multiply(&self, other: &Self) -> Self {
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.m[row + 4 * k] * other.m[k + 4 * col];
                }
                out[row + 4 * col] = sum;
            }
        }
        Self { m: out }
    }

    /// Transform a 3D point. Returns the homogeneous coordinates
    /// `(x', y', z', w')` so callers can do their own perspective
    /// divide if they want a screen position.
    pub fn apply_homogeneous(&self, x: f32, y: f32, z: f32) -> (f32, f32, f32, f32) {
        let m = &self.m;
        let xo = m[0] * x + m[4] * y + m[8] * z + m[12];
        let yo = m[1] * x + m[5] * y + m[9] * z + m[13];
        let zo = m[2] * x + m[6] * y + m[10] * z + m[14];
        let wo = m[3] * x + m[7] * y + m[11] * z + m[15];
        (xo, yo, zo, wo)
    }

    /// Transform a point and return its 3D position after perspective
    /// divide. Only used for normal-vector / backface checks where we
    /// care about the actual post-transform geometry.
    pub fn apply_point_3d(&self, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        let (xo, yo, zo, wo) = self.apply_homogeneous(x, y, z);
        if wo.abs() < 1e-6 {
            (xo, yo, zo)
        } else {
            (xo / wo, yo / wo, zo / wo)
        }
    }

    /// Drop the Z column/row to produce a 2D affine. This is an
    /// orthographic projection — `rotateX(deg)` becomes
    /// `scale(1, cos(deg))`, etc. True perspective projection is a
    /// follow-up.
    pub fn flatten_to_affine(&self) -> AffineTransform2D {
        let m = &self.m;
        // Apply the perspective divide using the homogeneous w from
        // (0,0,0) so that `perspective(d)` followed by a translation
        // doesn't silently shift the origin off-screen during the
        // flatten. For a pure-affine matrix w=1 and this is a no-op.
        let w0 = m[15];
        let inv_w = if w0.abs() < 1e-6 { 1.0 } else { 1.0 / w0 };
        AffineTransform2D {
            a: m[0] * inv_w,
            b: m[1] * inv_w,
            c: m[4] * inv_w,
            d: m[5] * inv_w,
            e: m[12] * inv_w,
            f: m[13] * inv_w,
        }
    }

    /// Build a `Matrix3d` from a list of CSS `TransformFunction`s,
    /// pre/post-translated by the given 2D transform-origin point
    /// (transform-origin Z is treated as 0 — `transform-origin: Z`
    /// is parsed but not yet plumbed through here).
    pub fn from_css_transforms_3d(
        transforms: &[TransformFunction],
        origin_x: f32,
        origin_y: f32,
    ) -> Self {
        let pre = Self::translate(-origin_x, -origin_y, 0.0);
        let post = Self::translate(origin_x, origin_y, 0.0);
        let mut m = Self::identity();
        for tf in transforms {
            let step = match *tf {
                TransformFunction::Translate(tx, ty) => Self::translate(tx, ty, 0.0),
                TransformFunction::Scale(sx, sy) => Self::scale(sx, sy, 1.0),
                TransformFunction::Rotate(deg) => Self::rotate_z(deg),
                TransformFunction::Skew(ax, ay) => {
                    let mut sk = Self::identity();
                    sk.m[4] = ax.to_radians().tan();
                    sk.m[1] = ay.to_radians().tan();
                    sk
                },
                TransformFunction::Matrix(a, b, c, d, e, f) => {
                    Self::from_2d_affine(AffineTransform2D { a, b, c, d, e, f })
                },
                TransformFunction::Translate3d(tx, ty, tz) => Self::translate(tx, ty, tz),
                TransformFunction::TranslateZ(tz) => Self::translate(0.0, 0.0, tz),
                TransformFunction::Scale3d(sx, sy, sz) => Self::scale(sx, sy, sz),
                TransformFunction::ScaleZ(sz) => Self::scale(1.0, 1.0, sz),
                TransformFunction::RotateX(deg) => Self::rotate_x(deg),
                TransformFunction::RotateY(deg) => Self::rotate_y(deg),
                TransformFunction::RotateZ(deg) => Self::rotate_z(deg),
                TransformFunction::Rotate3d(x, y, z, deg) => Self::rotate_axis(x, y, z, deg),
                TransformFunction::Matrix3d(values) => Self { m: values },
                TransformFunction::Perspective(d) => Self::perspective(d),
            };
            m = m.multiply(&step);
        }
        post.multiply(&m).multiply(&pre)
    }

    /// Returns the surface-normal Z component of the transformed
    /// front-face triangle `(0,0,0) → (w,0,0) → (0,h,0)`. Negative
    /// values mean the face has rotated away from the viewer.
    pub fn front_face_normal_z(&self, w: f32, h: f32) -> f32 {
        let p0 = self.apply_point_3d(0.0, 0.0, 0.0);
        let p1 = self.apply_point_3d(w, 0.0, 0.0);
        let p2 = self.apply_point_3d(0.0, h, 0.0);
        let v1 = (p1.0 - p0.0, p1.1 - p0.1, p1.2 - p0.2);
        let v2 = (p2.0 - p0.0, p2.1 - p0.1, p2.2 - p0.2);
        v1.0 * v2.1 - v1.1 * v2.0
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

    // -------------------------------------------------------------
    // 3D transform tests
    // -------------------------------------------------------------

    #[test]
    fn matrix3d_identity_is_no_op() {
        let m = Matrix3d::identity();
        let (x, y, z) = m.apply_point_3d(3.0, 4.0, 5.0);
        assert!((x - 3.0).abs() < 1e-5);
        assert!((y - 4.0).abs() < 1e-5);
        assert!((z - 5.0).abs() < 1e-5);
    }

    #[test]
    fn matrix3d_rotate_x_90_swaps_y_and_z() {
        let m = Matrix3d::rotate_x(90.0);
        let (x, y, z) = m.apply_point_3d(0.0, 1.0, 0.0);
        assert!(x.abs() < 1e-5);
        assert!(y.abs() < 1e-5);
        assert!((z - 1.0).abs() < 1e-5);
    }

    #[test]
    fn matrix3d_rotate_y_90_swaps_x_and_z() {
        let m = Matrix3d::rotate_y(90.0);
        let (x, y, z) = m.apply_point_3d(1.0, 0.0, 0.0);
        assert!(x.abs() < 1e-5);
        assert!(y.abs() < 1e-5);
        assert!((z + 1.0).abs() < 1e-5);
    }

    #[test]
    fn matrix3d_rotate_z_matches_2d_rotate() {
        let m3 = Matrix3d::rotate_z(45.0);
        let a2 = AffineTransform2D::rotate(45.0);
        let (x3, y3, _) = m3.apply_point_3d(10.0, 0.0, 0.0);
        let (x2, y2) = a2.apply(10.0, 0.0);
        assert!((x3 - x2).abs() < 1e-4);
        assert!((y3 - y2).abs() < 1e-4);
    }

    #[test]
    fn matrix3d_scale3d_independent_axes() {
        let m = Matrix3d::scale(2.0, 3.0, 4.0);
        let (x, y, z) = m.apply_point_3d(1.0, 1.0, 1.0);
        assert!((x - 2.0).abs() < 1e-5);
        assert!((y - 3.0).abs() < 1e-5);
        assert!((z - 4.0).abs() < 1e-5);
    }

    #[test]
    fn matrix3d_translate3d_offsets_each_axis() {
        let m = Matrix3d::translate(5.0, 6.0, 7.0);
        let (x, y, z) = m.apply_point_3d(1.0, 1.0, 1.0);
        assert!((x - 6.0).abs() < 1e-5);
        assert!((y - 7.0).abs() < 1e-5);
        assert!((z - 8.0).abs() < 1e-5);
    }

    #[test]
    fn matrix3d_rotate_axis_z_matches_rotate_z() {
        let a = Matrix3d::rotate_axis(0.0, 0.0, 1.0, 30.0);
        let b = Matrix3d::rotate_z(30.0);
        for i in 0..16 {
            assert!((a.m[i] - b.m[i]).abs() < 1e-5, "slot {i}");
        }
    }

    #[test]
    fn flatten_to_affine_drops_z_component() {
        // rotateX(60deg) flattened orthographically should compress
        // the Y axis by cos(60deg) = 0.5 and leave X alone.
        let m = Matrix3d::rotate_x(60.0);
        let a = m.flatten_to_affine();
        assert!((a.a - 1.0).abs() < 1e-5); // X scale
        assert!((a.d - 0.5).abs() < 1e-4); // Y scale = cos(60)
        assert!(a.b.abs() < 1e-5);
        assert!(a.c.abs() < 1e-5);
    }

    #[test]
    fn from_css_transforms_3d_translate3d() {
        let transforms = vec![TransformFunction::Translate3d(10.0, 20.0, 30.0)];
        let m = Matrix3d::from_css_transforms_3d(&transforms, 0.0, 0.0);
        let (x, y, z) = m.apply_point_3d(0.0, 0.0, 0.0);
        assert!((x - 10.0).abs() < 1e-5);
        assert!((y - 20.0).abs() < 1e-5);
        assert!((z - 30.0).abs() < 1e-5);
    }

    #[test]
    fn from_css_transforms_3d_rotate_x_around_origin() {
        // rotateX around the box center (origin = (50, 25)) should
        // leave the center point fixed and flip the top/bottom Y.
        let transforms = vec![TransformFunction::RotateX(180.0)];
        let m = Matrix3d::from_css_transforms_3d(&transforms, 50.0, 25.0);
        let center = m.apply_point_3d(50.0, 25.0, 0.0);
        assert!((center.0 - 50.0).abs() < 1e-4);
        assert!((center.1 - 25.0).abs() < 1e-4);
        let top = m.apply_point_3d(50.0, 0.0, 0.0);
        // top of the box (y=0) rotates to the bottom (y=50).
        assert!((top.1 - 50.0).abs() < 1e-4);
    }

    #[test]
    fn front_face_normal_z_positive_for_identity() {
        let m = Matrix3d::identity();
        assert!(m.front_face_normal_z(100.0, 50.0) > 0.0);
    }

    #[test]
    fn front_face_normal_z_negative_when_flipped_180() {
        let m = Matrix3d::rotate_y(180.0);
        assert!(m.front_face_normal_z(100.0, 50.0) < 0.0);
    }

    #[test]
    fn front_face_normal_z_positive_at_60_degrees() {
        // 60° still shows the front face; 120° has flipped past edge-on.
        let m60 = Matrix3d::rotate_y(60.0);
        let m120 = Matrix3d::rotate_y(120.0);
        assert!(m60.front_face_normal_z(100.0, 50.0) > 0.0);
        assert!(m120.front_face_normal_z(100.0, 50.0) < 0.0);
    }

    #[test]
    fn from_css_transforms_2d_path_unchanged_by_3d_pipeline() {
        // The 2D affine from from_css_transforms must still be exactly
        // a translation when only translate() is in the list — the
        // is_translation_only fast path in paint depends on this.
        let transforms = vec![TransformFunction::Translate(10.0, 20.0)];
        let m = AffineTransform2D::from_css_transforms(&transforms, 50.0, 25.0);
        assert!(m.is_translation_only());
        assert!((m.e - 10.0).abs() < 1e-4);
        assert!((m.f - 20.0).abs() < 1e-4);
    }

    #[test]
    fn ear_clip_degenerate() {
        let pts: Vec<(f32, f32)> = vec![];
        assert!(ear_clip_triangulate(&pts).is_empty());
        let pts = vec![(0.0, 0.0), (1.0, 1.0)];
        assert!(ear_clip_triangulate(&pts).is_empty());
    }
}

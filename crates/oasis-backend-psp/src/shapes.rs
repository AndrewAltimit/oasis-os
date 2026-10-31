//! Extended shape primitives for the PSP GU backend.
//!
//! Implements `fill_rounded_rect`, `fill_circle`, `draw_line`, and
//! gradient fills using GU `Lines`, `LineStrip`, `TriangleFan`, and
//! `Sprites` primitives with per-vertex colors.

use std::ffi::c_void;
use std::ptr;

use psp::sys::{self, GuPrimitive, VertexType};

use oasis_core::backend::{Color, GradientStyle};

use oasis_core::geometry::rounded_rect_inset;

use crate::gu_mem::{self, coord};
use crate::{PspBackend, SCREEN_HEIGHT};

// ---------------------------------------------------------------------------
// Color-only vertex (no texture, position + color)
// ---------------------------------------------------------------------------

/// Untextured vertex with a per-vertex ABGR color.
///
/// Used for shape primitives (lines, circles, triangles) that do not
/// sample a texture. Texture2D must be disabled before drawing.
#[repr(C, align(4))]
struct ColorVertex {
    color: u32,
    x: i16,
    y: i16,
    z: i16,
    _pad: i16,
}

/// Vertex type flags for `ColorVertex`.
const COLOR_VTYPE: VertexType = VertexType::from_bits_truncate(
    VertexType::COLOR_8888.bits()
        | VertexType::VERTEX_16BIT.bits()
        | VertexType::TRANSFORM_2D.bits(),
);

// ---------------------------------------------------------------------------
// Helper: integer sin/cos table for circle rendering
// ---------------------------------------------------------------------------

/// Number of segments for a full circle. 32 is a good compromise
/// between visual smoothness and vertex count on a 480x272 screen.
const CIRCLE_SEGMENTS: usize = 32;

/// Precomputed (cos, sin) pairs for `CIRCLE_SEGMENTS` points around
/// a unit circle, scaled by 1024 for fixed-point integer math.
///
/// Using fixed-point avoids pulling in libm for `f32::sin`/`cos`
/// on every frame.
const CIRCLE_TABLE: [(i32, i32); CIRCLE_SEGMENTS] = {
    // Build at compile time using a Horner-style Taylor approximation.
    // For a 32-segment circle, each step is 360/32 = 11.25 degrees.
    // We precompute with f64 and convert to fixed-point (scale 1024).
    const SCALE: f64 = 1024.0;
    const PI2: f64 = 2.0 * std::f64::consts::PI;
    let mut table = [(0i32, 0i32); CIRCLE_SEGMENTS];
    let mut i = 0;
    while i < CIRCLE_SEGMENTS {
        let angle = (i as f64) * PI2 / (CIRCLE_SEGMENTS as f64);
        // cos/sin via Taylor series (enough terms for f64 precision).
        let c = cos_approx(angle);
        let s = sin_approx(angle);
        table[i] = ((c * SCALE) as i32, (s * SCALE) as i32);
        i += 1;
    }
    table
};

/// Compile-time cosine approximation (Taylor series, 10 terms).
const fn cos_approx(x: f64) -> f64 {
    // Reduce to [0, 2*pi).
    let pi2 = 2.0 * std::f64::consts::PI;
    let mut x = x % pi2;
    if x < 0.0 {
        x += pi2;
    }
    let x2 = x * x;
    let mut result = 1.0;
    let mut term = 1.0;
    let mut i = 1;
    while i <= 10 {
        term *= -x2 / ((2 * i - 1) as f64 * (2 * i) as f64);
        result += term;
        i += 1;
    }
    result
}

/// Compile-time sine approximation (Taylor series, 10 terms).
const fn sin_approx(x: f64) -> f64 {
    let pi2 = 2.0 * std::f64::consts::PI;
    let mut x = x % pi2;
    if x < 0.0 {
        x += pi2;
    }
    let x2 = x * x;
    let mut result = x;
    let mut term = x;
    let mut i = 1;
    while i <= 10 {
        term *= -x2 / ((2 * i) as f64 * (2 * i + 1) as f64);
        result += term;
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// PspBackend extended shape methods
// ---------------------------------------------------------------------------

impl PspBackend {
    /// Draw a filled rectangle with rounded corners using GU line strips.
    ///
    /// Draws the shape as a series of horizontal scanlines. The corner
    /// insets are computed using an integer square root approximation
    /// to avoid the midpoint circle overhead for each frame. Only the
    /// scanlines on screen are emitted (the PSP has no render targets), so
    /// a tall browser box costs at most `SCREEN_HEIGHT` rows.
    pub fn fill_rounded_rect_inner(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) {
        if radius == 0 || w == 0 || h == 0 {
            self.fill_rect_inner(x, y, w, h, color);
            return;
        }
        let h = h.min(i32::MAX as u32) as i32;
        let w = w.min(i32::MAX as u32) as i32;
        let r = (radius as i32).min(w / 2).min(h / 2);
        let abgr = color.to_abgr();

        // Visible rows `dy` with `0 <= y + dy < SCREEN_HEIGHT`.
        let first = y.saturating_neg().clamp(0, h);
        let last = (SCREEN_HEIGHT as i32).saturating_sub(y).clamp(first, h);
        if first == last {
            return;
        }
        // Vertices for the visible scanlines (2 per line: left and right
        // endpoints as a Sprites primitive).
        let vert_count = ((last - first) as usize) * 2;

        // Draw scanline by scanline: each scanline is a 1px-tall
        // filled rect. The rounded corners are achieved by insetting
        // the left/right edges in the top and bottom `r` rows.
        // SAFETY: `verts` holds `vert_count` vertices, one pair written per
        // row in `first..last`.
        unsafe {
            let Some(verts) = gu_mem::alloc::<ColorVertex>(vert_count, "rounded_rect") else {
                return;
            };
            let mut vi = 0usize;
            for dy in first..last {
                let inset = rounded_rect_inset(dy, h, r);
                let lx = x.saturating_add(inset);
                let rx = x.saturating_add(w - inset);
                write_color_vert(verts, vi, abgr, lx, y + dy);
                write_color_vert(verts, vi + 1, abgr, rx, y + dy + 1);
                vi += 2;
            }
            draw_untextured(GuPrimitive::Sprites, verts, vert_count);
        }
    }

    /// Draw a filled circle using a GU triangle fan.
    ///
    /// The fan has `CIRCLE_SEGMENTS` outer vertices plus a center
    /// vertex. Fixed-point cos/sin avoids per-frame floating-point.
    pub fn fill_circle_inner(&mut self, cx: i32, cy: i32, radius: u16, color: Color) {
        if radius == 0 {
            return;
        }
        let abgr = color.to_abgr();
        let r = radius as i32;
        // center + CIRCLE_SEGMENTS + 1 (closing vertex).
        let vert_count = CIRCLE_SEGMENTS + 2;

        // SAFETY: `verts` holds `vert_count` vertices: the center, the
        // CIRCLE_SEGMENTS perimeter points and the closing point.
        unsafe {
            let Some(verts) = gu_mem::alloc::<ColorVertex>(vert_count, "circle") else {
                return;
            };
            write_color_vert(verts, 0, abgr, cx, cy);
            for (i, &(cos_val, sin_val)) in CIRCLE_TABLE.iter().enumerate() {
                let px = cx + (r * cos_val) / 1024;
                let py = cy + (r * sin_val) / 1024;
                write_color_vert(verts, 1 + i, abgr, px, py);
            }
            // Close the fan by repeating the first perimeter vertex.
            let (cos0, sin0) = CIRCLE_TABLE[0];
            write_color_vert(
                verts,
                1 + CIRCLE_SEGMENTS,
                abgr,
                cx + (r * cos0) / 1024,
                cy + (r * sin0) / 1024,
            );
            draw_untextured(GuPrimitive::TriangleFan, verts, vert_count);
        }
    }

    /// Draw a line between two points using GU line primitives.
    ///
    /// For `width > 1`, draws parallel lines offset perpendicular to
    /// the line direction.
    pub fn draw_line_inner(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) {
        let abgr = color.to_abgr();
        let w = (width as i32).max(1);

        if w <= 1 {
            // SAFETY: `verts` holds the 2 vertices written below.
            unsafe {
                let Some(verts) = gu_mem::alloc::<ColorVertex>(2, "line") else {
                    return;
                };
                write_color_vert(verts, 0, abgr, x1, y1);
                write_color_vert(verts, 1, abgr, x2, y2);
                draw_untextured(GuPrimitive::Lines, verts, 2);
            }
            return;
        }

        // Multiple parallel lines for thickness.
        let dx = (x2 - x1) as f32;
        let dy = (y2 - y1) as f32;
        let len = libm::sqrtf(dx * dx + dy * dy).max(1.0);
        let nx = -dy / len;
        let ny = dx / len;
        let half = w / 2;
        let line_count = w as usize;

        // SAFETY: `verts` holds `line_count * 2` vertices, one pair written
        // per line.
        unsafe {
            let Some(verts) = gu_mem::alloc::<ColorVertex>(line_count * 2, "thick_line") else {
                return;
            };
            for i in 0..line_count {
                let offset = i as i32 - half;
                let ox = (nx * offset as f32) as i32;
                let oy = (ny * offset as f32) as i32;
                write_color_vert(verts, i * 2, abgr, x1 + ox, y1 + oy);
                write_color_vert(verts, i * 2 + 1, abgr, x2 + ox, y2 + oy);
            }
            draw_untextured(GuPrimitive::Lines, verts, line_count * 2);
        }
    }

    /// Draw a filled rectangle with a gradient using per-vertex colors.
    ///
    /// Every style is two triangles forming the quad; the GE interpolates
    /// the vertex colors across them (vertical: top / bottom colors,
    /// horizontal: left / right, four-corner: one color per corner).
    pub fn fill_rect_gradient_inner(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        // Corner colors: top-left, top-right, bottom-left, bottom-right.
        let [tl, tr, bl, br] = match *gradient {
            GradientStyle::Vertical { top, bottom } => {
                let (t, b) = (top.to_abgr(), bottom.to_abgr());
                [t, t, b, b]
            },
            GradientStyle::Horizontal { left, right } => {
                let (l, r) = (left.to_abgr(), right.to_abgr());
                [l, r, l, r]
            },
            GradientStyle::FourCorner {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => [
                top_left.to_abgr(),
                top_right.to_abgr(),
                bottom_left.to_abgr(),
                bottom_right.to_abgr(),
            ],
        };
        let x2 = x.saturating_add_unsigned(w);
        let y2 = y.saturating_add_unsigned(h);

        // SAFETY: `verts` holds the 6 vertices written below.
        unsafe {
            let Some(verts) = gu_mem::alloc::<ColorVertex>(6, "gradient") else {
                return;
            };
            // Triangle 1: TL, TR, BL.
            write_color_vert(verts, 0, tl, x, y);
            write_color_vert(verts, 1, tr, x2, y);
            write_color_vert(verts, 2, bl, x, y2);
            // Triangle 2: TR, BR, BL.
            write_color_vert(verts, 3, tr, x2, y);
            write_color_vert(verts, 4, br, x2, y2);
            write_color_vert(verts, 5, bl, x, y2);
            draw_untextured(GuPrimitive::Triangles, verts, 6);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a `ColorVertex` at `verts[index]` (coordinates saturated to the
/// GE's 16-bit range).
///
/// # Safety
///
/// `verts.add(index)` must be valid for a write.
unsafe fn write_color_vert(verts: *mut ColorVertex, index: usize, color: u32, x: i32, y: i32) {
    // SAFETY: caller guarantees `verts.add(index)` is within the allocation.
    unsafe {
        ptr::write(
            verts.add(index),
            ColorVertex {
                color,
                x: coord(x),
                y: coord(y),
                z: 0,
                _pad: 0,
            },
        );
    }
}

/// Draw `count` untextured `verts` as `prim`: Texture2D is disabled for the
/// draw and re-enabled after it (the rest of the backend draws textured).
///
/// # Safety
///
/// Inside an open GU frame; `verts` is display-list memory holding `count`
/// initialized vertices.
unsafe fn draw_untextured(prim: GuPrimitive, verts: *const ColorVertex, count: usize) {
    // SAFETY: caller contract; the GE reads `verts` while running the list.
    unsafe {
        sys::sceGuDisable(sys::GuState::Texture2D);
        sys::sceGuDrawArray(
            prim,
            COLOR_VTYPE,
            count as i32,
            ptr::null(),
            verts as *const c_void,
        );
        sys::sceGuEnable(sys::GuState::Texture2D);
    }
}

// `isqrt_i32` and `rounded_rect_inset` are now shared via
// `oasis_core::geometry` (re-exported from `oasis_types::geometry`).

//! Extended shape primitives for the PSP GU backend.
//!
//! Implements `fill_rounded_rect`, `fill_circle`, `draw_line`, and
//! gradient fills using GU `Lines`, `LineStrip`, `TriangleFan`, and
//! `Sprites` primitives with per-vertex colors.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use psp::sys::{self, GuPrimitive, VertexType};

use oasis_core::backend::{Color, GradientStyle};

use crate::{ColorExt, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH};

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
    /// to avoid the midpoint circle overhead for each frame.
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
        let r = (radius as i32).min(w as i32 / 2).min(h as i32 / 2);
        let abgr = color.to_abgr();

        // Draw scanline by scanline: each scanline is a 1px-tall
        // filled rect. The rounded corners are achieved by insetting
        // the left/right edges in the top and bottom `r` rows.
        // SAFETY: Disabling Texture2D and using sceGuGetMemory for
        // vertices within the active display list frame.
        unsafe {
            sys::sceGuDisable(sys::GuState::Texture2D);

            // Allocate vertices for all scanlines (2 per line: left
            // and right endpoints as a Sprites primitive).
            let vert_count = (h as usize) * 2;
            let verts = sys::sceGuGetMemory(
                (vert_count * size_of::<ColorVertex>()) as i32,
            ) as *mut ColorVertex;
            if verts.is_null() {
                sys::sceGuEnable(sys::GuState::Texture2D);
                return;
            }

            let mut vi = 0usize;
            for dy in 0..h as i32 {
                let inset = if dy < r {
                    let ry = r - dy;
                    r - isqrt_i32(r * r - ry * ry)
                } else if dy >= h as i32 - r {
                    let ry = dy - (h as i32 - 1 - r);
                    r - isqrt_i32(r * r - ry * ry)
                } else {
                    0
                };

                let lx = x + inset;
                let rx = x + w as i32 - inset;
                ptr::write(
                    verts.add(vi),
                    ColorVertex {
                        color: abgr,
                        x: lx as i16,
                        y: (y + dy) as i16,
                        z: 0,
                        _pad: 0,
                    },
                );
                ptr::write(
                    verts.add(vi + 1),
                    ColorVertex {
                        color: abgr,
                        x: rx as i16,
                        y: (y + dy + 1) as i16,
                        z: 0,
                        _pad: 0,
                    },
                );
                vi += 2;
            }

            sys::sceGuDrawArray(
                GuPrimitive::Sprites,
                COLOR_VTYPE,
                vert_count as i32,
                ptr::null(),
                verts as *const c_void,
            );

            sys::sceGuEnable(sys::GuState::Texture2D);
        }
    }

    /// Draw a filled circle using a GU triangle fan.
    ///
    /// The fan has `CIRCLE_SEGMENTS` outer vertices plus a center
    /// vertex. Fixed-point cos/sin avoids per-frame floating-point.
    pub fn fill_circle_inner(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    ) {
        if radius == 0 {
            return;
        }
        let abgr = color.to_abgr();
        let r = radius as i32;

        // SAFETY: Disabling Texture2D and allocating GU memory for
        // the triangle fan vertices within the active display list.
        unsafe {
            sys::sceGuDisable(sys::GuState::Texture2D);

            // center + CIRCLE_SEGMENTS + 1 (closing vertex).
            let vert_count = CIRCLE_SEGMENTS + 2;
            let verts = sys::sceGuGetMemory(
                (vert_count * size_of::<ColorVertex>()) as i32,
            ) as *mut ColorVertex;
            if verts.is_null() {
                sys::sceGuEnable(sys::GuState::Texture2D);
                return;
            }

            // Center vertex.
            ptr::write(
                verts,
                ColorVertex {
                    color: abgr,
                    x: cx as i16,
                    y: cy as i16,
                    z: 0,
                    _pad: 0,
                },
            );

            // Perimeter vertices.
            for i in 0..CIRCLE_SEGMENTS {
                let (cos_val, sin_val) = CIRCLE_TABLE[i];
                let px = cx + (r * cos_val) / 1024;
                let py = cy + (r * sin_val) / 1024;
                ptr::write(
                    verts.add(1 + i),
                    ColorVertex {
                        color: abgr,
                        x: px as i16,
                        y: py as i16,
                        z: 0,
                        _pad: 0,
                    },
                );
            }

            // Close the fan by repeating the first perimeter vertex.
            let (cos0, sin0) = CIRCLE_TABLE[0];
            ptr::write(
                verts.add(1 + CIRCLE_SEGMENTS),
                ColorVertex {
                    color: abgr,
                    x: (cx + (r * cos0) / 1024) as i16,
                    y: (cy + (r * sin0) / 1024) as i16,
                    z: 0,
                    _pad: 0,
                },
            );

            sys::sceGuDrawArray(
                GuPrimitive::TriangleFan,
                COLOR_VTYPE,
                vert_count as i32,
                ptr::null(),
                verts as *const c_void,
            );

            sys::sceGuEnable(sys::GuState::Texture2D);
        }
    }

    /// Draw a line between two points using GU line primitives.
    ///
    /// For `width > 1`, draws parallel lines offset perpendicular to
    /// the line direction. Uses integer arithmetic only.
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

        // SAFETY: Disabling Texture2D and using GU memory for line
        // vertices within the active display list.
        unsafe {
            sys::sceGuDisable(sys::GuState::Texture2D);

            if w <= 1 {
                // Single line: 2 vertices.
                let verts = sys::sceGuGetMemory(
                    (2 * size_of::<ColorVertex>()) as i32,
                ) as *mut ColorVertex;
                if verts.is_null() {
                    sys::sceGuEnable(sys::GuState::Texture2D);
                    return;
                }

                ptr::write(
                    verts,
                    ColorVertex {
                        color: abgr,
                        x: x1 as i16,
                        y: y1 as i16,
                        z: 0,
                        _pad: 0,
                    },
                );
                ptr::write(
                    verts.add(1),
                    ColorVertex {
                        color: abgr,
                        x: x2 as i16,
                        y: y2 as i16,
                        z: 0,
                        _pad: 0,
                    },
                );

                sys::sceGuDrawArray(
                    GuPrimitive::Lines,
                    COLOR_VTYPE,
                    2,
                    ptr::null(),
                    verts as *const c_void,
                );
            } else {
                // Multiple parallel lines for thickness.
                let dx = (x2 - x1) as f32;
                let dy = (y2 - y1) as f32;
                let len = libm::sqrtf(dx * dx + dy * dy).max(1.0);
                let nx = (-dy / len) as i32;
                let ny = (dx / len) as i32;
                let half = w / 2;
                let line_count = w as usize;

                let verts = sys::sceGuGetMemory(
                    (line_count * 2 * size_of::<ColorVertex>()) as i32,
                ) as *mut ColorVertex;
                if verts.is_null() {
                    sys::sceGuEnable(sys::GuState::Texture2D);
                    return;
                }

                for i in 0..line_count {
                    let offset = i as i32 - half;
                    let ox = nx * offset;
                    let oy = ny * offset;
                    ptr::write(
                        verts.add(i * 2),
                        ColorVertex {
                            color: abgr,
                            x: (x1 + ox) as i16,
                            y: (y1 + oy) as i16,
                            z: 0,
                            _pad: 0,
                        },
                    );
                    ptr::write(
                        verts.add(i * 2 + 1),
                        ColorVertex {
                            color: abgr,
                            x: (x2 + ox) as i16,
                            y: (y2 + oy) as i16,
                            z: 0,
                            _pad: 0,
                        },
                    );
                }

                sys::sceGuDrawArray(
                    GuPrimitive::Lines,
                    COLOR_VTYPE,
                    (line_count * 2) as i32,
                    ptr::null(),
                    verts as *const c_void,
                );
            }

            sys::sceGuEnable(sys::GuState::Texture2D);
        }
    }

    /// Draw a filled rectangle with a gradient using per-vertex colors.
    ///
    /// Vertical gradients use a single Sprites primitive with the top
    /// and bottom vertex colors set to the gradient endpoints. The GE
    /// hardware interpolates the color across the primitive.
    ///
    /// Horizontal and four-corner gradients use two or four triangles
    /// to achieve bilinear interpolation.
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

        // SAFETY: Disabling Texture2D and allocating GU vertices
        // for gradient rendering within the active display list.
        unsafe {
            sys::sceGuDisable(sys::GuState::Texture2D);

            match *gradient {
                GradientStyle::Vertical { top, bottom } => {
                    // Two triangles forming a quad. The GE interpolates
                    // vertex colors linearly across the primitive.
                    let top_abgr = top.to_abgr();
                    let bot_abgr = bottom.to_abgr();

                    let verts = sys::sceGuGetMemory(
                        (6 * size_of::<ColorVertex>()) as i32,
                    ) as *mut ColorVertex;
                    if !verts.is_null() {
                        let x2 = x + w as i32;
                        let y2 = y + h as i32;

                        // Triangle 1: top-left, top-right, bottom-left.
                        write_color_vert(
                            verts, 0, top_abgr, x, y,
                        );
                        write_color_vert(
                            verts, 1, top_abgr, x2, y,
                        );
                        write_color_vert(
                            verts, 2, bot_abgr, x, y2,
                        );
                        // Triangle 2: top-right, bottom-right, bottom-left.
                        write_color_vert(
                            verts, 3, top_abgr, x2, y,
                        );
                        write_color_vert(
                            verts, 4, bot_abgr, x2, y2,
                        );
                        write_color_vert(
                            verts, 5, bot_abgr, x, y2,
                        );

                        sys::sceGuDrawArray(
                            GuPrimitive::Triangles,
                            COLOR_VTYPE,
                            6,
                            ptr::null(),
                            verts as *const c_void,
                        );
                    }
                },
                GradientStyle::Horizontal { left, right } => {
                    let left_abgr = left.to_abgr();
                    let right_abgr = right.to_abgr();

                    let verts = sys::sceGuGetMemory(
                        (6 * size_of::<ColorVertex>()) as i32,
                    ) as *mut ColorVertex;
                    if !verts.is_null() {
                        let x2 = x + w as i32;
                        let y2 = y + h as i32;

                        // Triangle 1: top-left, top-right, bottom-left.
                        write_color_vert(
                            verts, 0, left_abgr, x, y,
                        );
                        write_color_vert(
                            verts, 1, right_abgr, x2, y,
                        );
                        write_color_vert(
                            verts, 2, left_abgr, x, y2,
                        );
                        // Triangle 2: top-right, bottom-right, bottom-left.
                        write_color_vert(
                            verts, 3, right_abgr, x2, y,
                        );
                        write_color_vert(
                            verts, 4, right_abgr, x2, y2,
                        );
                        write_color_vert(
                            verts, 5, left_abgr, x, y2,
                        );

                        sys::sceGuDrawArray(
                            GuPrimitive::Triangles,
                            COLOR_VTYPE,
                            6,
                            ptr::null(),
                            verts as *const c_void,
                        );
                    }
                },
                GradientStyle::FourCorner {
                    top_left,
                    top_right,
                    bottom_left,
                    bottom_right,
                } => {
                    let tl = top_left.to_abgr();
                    let tr = top_right.to_abgr();
                    let bl = bottom_left.to_abgr();
                    let br = bottom_right.to_abgr();

                    let verts = sys::sceGuGetMemory(
                        (6 * size_of::<ColorVertex>()) as i32,
                    ) as *mut ColorVertex;
                    if !verts.is_null() {
                        let x2 = x + w as i32;
                        let y2 = y + h as i32;

                        // Triangle 1: TL, TR, BL.
                        write_color_vert(verts, 0, tl, x, y);
                        write_color_vert(verts, 1, tr, x2, y);
                        write_color_vert(verts, 2, bl, x, y2);
                        // Triangle 2: TR, BR, BL.
                        write_color_vert(verts, 3, tr, x2, y);
                        write_color_vert(verts, 4, br, x2, y2);
                        write_color_vert(verts, 5, bl, x, y2);

                        sys::sceGuDrawArray(
                            GuPrimitive::Triangles,
                            COLOR_VTYPE,
                            6,
                            ptr::null(),
                            verts as *const c_void,
                        );
                    }
                },
            }

            sys::sceGuEnable(sys::GuState::Texture2D);
        }
    }

    /// Dim the entire screen using a full-viewport semi-transparent rect.
    pub fn dim_screen_inner(&mut self, alpha: u8) {
        self.fill_rect_inner(
            0,
            0,
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
            Color::rgba(0, 0, 0, alpha),
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a `ColorVertex` at `verts[index]`.
///
/// SAFETY: Caller must ensure `verts.add(index)` is valid for a write.
unsafe fn write_color_vert(
    verts: *mut ColorVertex,
    index: usize,
    color: u32,
    x: i32,
    y: i32,
) {
    // SAFETY: Caller guarantees `verts.add(index)` is within the allocated sceGuGetMemory buffer.
    unsafe {
        ptr::write(
            verts.add(index),
            ColorVertex {
                color,
                x: x as i16,
                y: y as i16,
                z: 0,
                _pad: 0,
            },
        );
    }
}

/// Integer square root (floor) for positive i32 values.
fn isqrt_i32(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    // Newton's method with integer arithmetic.
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

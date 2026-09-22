//! Backend conformance scenarios.
//!
//! A shared list of `SdiBackend` draw scenarios that every pixel-producing
//! backend (SDL3, the UE5 software framebuffer, anything built on
//! `oasis-rasterize`) must render the same way. Each [`Scenario`] draws
//! onto a [`CANVAS_W`] x [`CANVAS_H`] surface through `&mut dyn
//! SdiBackend`, so one list drives every backend:
//!
//! - backend crates run [`SCENARIOS`] and compare the captured
//!   [`Frame`]s pairwise with [`Frame::diff`], within the scenario's
//!   [`Tolerance`];
//! - [`Scenario::check`] adds *absolute* expectations (a filled rect has
//!   exact bounds, a clip rejects everything outside it, an odd-width
//!   texture comes back unsheared, ...), so two backends cannot agree on
//!   the same wrong answer.
//!
//! Tolerances are per scenario and documented where they are not exact:
//! a non-zero tolerance marks an *intentional* backend difference (e.g.
//! SDL's linear texture filtering), never a hidden bug.

use oasis_types::backend::{
    BatchRect, BatchText, BlendMode, Color, GradientStyle, SdiBackend, TextureId,
};
use oasis_types::error::{OasisError, Result};

/// Width of the scenario canvas.
pub const CANVAS_W: u32 = 128;
/// Height of the scenario canvas.
pub const CANVAS_H: u32 = 96;
/// Background every scenario starts from (opaque).
pub const BG: Color = Color::rgb(24, 32, 48);

/// How far two backends may disagree on a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tolerance {
    /// Largest per-channel difference that still counts as a match.
    pub channel: u8,
    /// Pixels allowed to exceed `channel` (edge disagreements of
    /// antialiased / filtered paths).
    pub pixels: usize,
}

impl Tolerance {
    /// Bit-exact.
    pub const EXACT: Self = Self {
        channel: 0,
        pixels: 0,
    };
    /// Alpha-blend rounding: SDL's blitters compute `(s*a + d*(255-a)) / 255`
    /// with truncating or shift-based division, the software rasterizer
    /// rounds to nearest. Up to 2/255 per channel, no structural difference.
    pub const BLEND: Self = Self {
        channel: 2,
        pixels: 0,
    };
}

/// An absolute expectation on a rendered frame (`Err` explains the failure).
pub type CheckFn = fn(&Frame) -> std::result::Result<(), String>;

/// One draw scenario (see module docs).
pub struct Scenario {
    /// Unique, stable name (used in failure messages and dump files).
    pub name: &'static str,
    /// Draws the scenario. The canvas is already cleared to [`BG`], with
    /// no clip, translate or render target active; the scenario must
    /// leave it that way.
    pub draw: fn(&mut dyn SdiBackend) -> Result<()>,
    /// Absolute expectations on the rendered frame.
    pub check: Option<CheckFn>,
    /// Allowed cross-backend difference.
    pub tolerance: Tolerance,
    /// Why `tolerance` is not exact (empty when it is).
    pub note: &'static str,
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// A captured RGBA8 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Per-pixel comparison of two frames (RGB only: backends disagree on the
/// alpha of an opaque window surface, which is never displayed).
#[derive(Debug, Clone, Default)]
pub struct Diff {
    /// Pixels whose largest channel difference exceeds the tolerance.
    pub mismatched: usize,
    /// Largest channel difference seen anywhere.
    pub max_delta: u8,
    /// The first few mismatches: `(x, y, a_rgb, b_rgb)`.
    pub samples: Vec<(u32, u32, [u8; 3], [u8; 3])>,
}

impl Diff {
    /// Whether the difference is within `tol`.
    pub fn within(&self, tol: Tolerance) -> bool {
        self.mismatched <= tol.pixels
    }
}

impl Frame {
    /// Read the whole `w` x `h` surface back from `backend`.
    pub fn capture(backend: &dyn SdiBackend, w: u32, h: u32) -> Result<Self> {
        let rgba = backend.read_pixels(0, 0, w, h)?;
        if rgba.len() != (w * h * 4) as usize {
            return Err(OasisError::Backend(
                format!(
                    "read_pixels({w}x{h}) returned {} bytes, expected {}",
                    rgba.len(),
                    w * h * 4
                )
                .into(),
            ));
        }
        Ok(Self { w, h, rgba })
    }

    /// RGB of pixel `(x, y)`; panics when out of range.
    pub fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    /// Compare against `other` with a per-channel tolerance of
    /// `tol.channel`.
    pub fn diff(&self, other: &Frame, tol: Tolerance) -> Diff {
        let mut d = Diff::default();
        for y in 0..self.h.min(other.h) {
            for x in 0..self.w.min(other.w) {
                let (a, b) = (self.rgb(x, y), other.rgb(x, y));
                let delta = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
                d.max_delta = d.max_delta.max(delta);
                if delta > tol.channel {
                    d.mismatched += 1;
                    if d.samples.len() < 12 {
                        d.samples.push((x, y, a, b));
                    }
                }
            }
        }
        d
    }

    /// Bounding box `(x, y, w, h)` of pixels that differ from [`BG`].
    pub fn ink_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let bg = rgb_of(BG);
        let mut b: Option<(u32, u32, u32, u32)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.rgb(x, y) != bg {
                    b = Some(match b {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        b.map(|(x0, y0, x1, y1)| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Number of pixels exactly `rgb`.
    pub fn count(&self, rgb: [u8; 3]) -> usize {
        (0..self.h)
            .flat_map(|y| (0..self.w).map(move |x| (x, y)))
            .filter(|&(x, y)| self.rgb(x, y) == rgb)
            .count()
    }

    /// A text rendering of the frame for failure messages: one char per
    /// pixel of the region, `.` for background.
    pub fn ascii(&self, x0: u32, y0: u32, w: u32, h: u32) -> String {
        let bg = rgb_of(BG);
        let mut s = String::new();
        for y in y0..(y0 + h).min(self.h) {
            for x in x0..(x0 + w).min(self.w) {
                s.push(if self.rgb(x, y) == bg { '.' } else { '#' });
            }
            s.push('\n');
        }
        s
    }
}

/// RGB bytes of `c`.
pub const fn rgb_of(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b]
}

/// Source-over blend of `src` on an opaque `dst`, rounded to nearest (the
/// mathematically expected value backends must approximate).
pub fn blend(src: Color, dst: [u8; 3]) -> [u8; 3] {
    let a = src.a as u32;
    let mix = |s: u8, d: u8| ((s as u32 * a + d as u32 * (255 - a) + 127) / 255) as u8;
    [mix(src.r, dst[0]), mix(src.g, dst[1]), mix(src.b, dst[2])]
}

// ---------------------------------------------------------------------------
// Check helpers
// ---------------------------------------------------------------------------

type Check = std::result::Result<(), String>;

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|c| a[c].abs_diff(b[c]) <= tol)
}

/// Every pixel of the rect (clipped to the frame) is `rgb` (within `tol`).
pub fn expect_fill(f: &Frame, rect: (i32, i32, u32, u32), rgb: [u8; 3], tol: u8) -> Check {
    let (x, y, w, h) = rect;
    for py in y.max(0)..(y + h as i32).min(f.h as i32) {
        for px in x.max(0)..(x + w as i32).min(f.w as i32) {
            let got = f.rgb(px as u32, py as u32);
            if !near(got, rgb, tol) {
                return Err(format!(
                    "pixel ({px},{py}) inside {rect:?} is {got:?}, expected {rgb:?}"
                ));
            }
        }
    }
    Ok(())
}

/// The 1-pixel ring just outside `rect` (clipped to the frame) is `rgb`.
pub fn expect_ring(f: &Frame, rect: (i32, i32, u32, u32), rgb: [u8; 3]) -> Check {
    let (x, y, w, h) = rect;
    let (x0, y0, x1, y1) = (x - 1, y - 1, x + w as i32, y + h as i32);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let edge = py == y0 || py == y1 || px == x0 || px == x1;
            if !edge || px < 0 || py < 0 || px >= f.w as i32 || py >= f.h as i32 {
                continue;
            }
            let got = f.rgb(px as u32, py as u32);
            if got != rgb {
                return Err(format!(
                    "pixel ({px},{py}) just outside {rect:?} is {got:?}, expected {rgb:?}"
                ));
            }
        }
    }
    Ok(())
}

/// A filled rect with exact bounds over the background.
pub fn expect_rect_exact(f: &Frame, rect: (i32, i32, u32, u32), rgb: [u8; 3]) -> Check {
    expect_fill(f, rect, rgb, 0)?;
    expect_ring(f, rect, rgb_of(BG))
}

/// Nothing was drawn.
pub fn expect_blank(f: &Frame) -> Check {
    match f.ink_bounds() {
        None => Ok(()),
        Some(b) => Err(format!("expected an untouched canvas, ink at {b:?}")),
    }
}

/// Ink stays inside `rect`.
pub fn expect_ink_within(f: &Frame, rect: (i32, i32, u32, u32)) -> Check {
    let Some((x, y, w, h)) = f.ink_bounds() else {
        return Err("expected some ink, canvas untouched".into());
    };
    let (rx, ry, rw, rh) = rect;
    let inside = x as i32 >= rx
        && y as i32 >= ry
        && (x + w) as i32 <= rx + rw as i32
        && (y + h) as i32 <= ry + rh as i32;
    if inside {
        Ok(())
    } else {
        Err(format!("ink bounds {:?} escape {rect:?}", (x, y, w, h)))
    }
}

// ---------------------------------------------------------------------------
// Texture patterns
// ---------------------------------------------------------------------------

/// An RGBA image where every pixel of a `w` x `h` image differs from its
/// neighbours, so a row-stride / pitch mismatch shears it visibly instead
/// of reproducing the same colours. Fully opaque.
pub fn stride_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[
                (x * 7 + y * 3) as u8,
                (y * 37 + x) as u8,
                (200 + x + y * 5) as u8,
                255,
            ]);
        }
    }
    px
}

/// Opaque checkerboard of `cell`-pixel squares in two colors.
pub fn checker(w: u32, h: u32, cell: u32, a: Color, b: Color) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = if ((x / cell) + (y / cell)).is_multiple_of(2) {
                a
            } else {
                b
            };
            px.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
    px
}

/// Expected RGB of `pattern` pixel `(x, y)` (an image `w` pixels wide).
fn pattern_rgb(pattern: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [pattern[i], pattern[i + 1], pattern[i + 2]]
}

/// `f` shows `pattern` (`pw` x `ph`) unscaled at `(dx, dy)`, clipped to
/// the frame.
pub fn expect_image(f: &Frame, pattern: &[u8], pw: u32, ph: u32, dx: i32, dy: i32) -> Check {
    for y in 0..ph as i32 {
        for x in 0..pw as i32 {
            let (fx, fy) = (dx + x, dy + y);
            if fx < 0 || fy < 0 || fx >= f.w as i32 || fy >= f.h as i32 {
                continue;
            }
            let want = pattern_rgb(pattern, pw, x as u32, y as u32);
            let got = f.rgb(fx as u32, fy as u32);
            if got != want {
                return Err(format!(
                    "image pixel ({x},{y}) at screen ({fx},{fy}) is {got:?}, expected {want:?} \
                     (row pitch / stride mismatch?)"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenario list
// ---------------------------------------------------------------------------

const RED: Color = Color::rgb(230, 40, 30);
const GREEN: Color = Color::rgb(40, 210, 70);
const BLUE: Color = Color::rgb(40, 90, 240);
const YELLOW: Color = Color::rgb(250, 220, 40);
const WHITE: Color = Color::rgb(255, 255, 255);

macro_rules! scenario {
    ($name:expr, $draw:expr) => {
        Scenario {
            name: $name,
            draw: $draw,
            check: None,
            tolerance: Tolerance::EXACT,
            note: "",
        }
    };
    ($name:expr, $draw:expr, check = $check:expr) => {
        Scenario {
            name: $name,
            draw: $draw,
            check: Some($check),
            tolerance: Tolerance::EXACT,
            note: "",
        }
    };
    ($name:expr, $draw:expr, $tol:expr, $note:expr) => {
        Scenario {
            name: $name,
            draw: $draw,
            check: None,
            tolerance: $tol,
            note: $note,
        }
    };
    ($name:expr, $draw:expr, check = $check:expr, $tol:expr, $note:expr) => {
        Scenario {
            name: $name,
            draw: $draw,
            check: Some($check),
            tolerance: $tol,
            note: $note,
        }
    };
}

/// Every conformance scenario, in a stable order.
pub static SCENARIOS: &[Scenario] = &[
    // -- clear / fill_rect ---------------------------------------------------
    scenario!(
        "clear",
        |b| b.clear(Color::rgb(200, 100, 50)),
        check = |f| expect_fill(f, (0, 0, CANVAS_W, CANVAS_H), [200, 100, 50], 0)
    ),
    scenario!(
        "fill_rect_exact_bounds",
        |b| b.fill_rect(10, 12, 30, 20, RED),
        check = |f| expect_rect_exact(f, (10, 12, 30, 20), rgb_of(RED))
    ),
    scenario!(
        "fill_rect_one_pixel",
        |b| {
            b.fill_rect(5, 5, 1, 1, WHITE)?;
            b.fill_rect(20, 5, 1, 9, WHITE)?;
            b.fill_rect(30, 5, 9, 1, WHITE)
        },
        check = |f| {
            expect_rect_exact(f, (5, 5, 1, 1), rgb_of(WHITE))?;
            expect_rect_exact(f, (20, 5, 1, 9), rgb_of(WHITE))?;
            expect_rect_exact(f, (30, 5, 9, 1), rgb_of(WHITE))
        }
    ),
    scenario!(
        "fill_rect_negative_and_offscreen",
        |b| {
            b.fill_rect(-10, -5, 30, 20, RED)?;
            b.fill_rect(110, 80, 40, 40, GREEN)?;
            b.fill_rect(200, 200, 10, 10, BLUE)?;
            b.fill_rect(-50, -50, 10, 10, BLUE)?;
            b.fill_rect(-5, 50, 200, 3, YELLOW)
        },
        check = |f| {
            expect_fill(f, (0, 0, 20, 15), rgb_of(RED), 0)?;
            expect_ring(f, (-10, -5, 30, 20), rgb_of(BG))?;
            expect_fill(f, (110, 80, 18, 16), rgb_of(GREEN), 0)?;
            expect_fill(f, (0, 50, CANVAS_W, 3), rgb_of(YELLOW), 0)?;
            if f.count(rgb_of(BLUE)) != 0 {
                return Err("fully offscreen rect left pixels".into());
            }
            Ok(())
        }
    ),
    scenario!(
        "fill_rect_zero_size",
        |b| {
            b.fill_rect(10, 10, 0, 20, RED)?;
            b.fill_rect(10, 10, 20, 0, RED)?;
            b.fill_rect(10, 10, 0, 0, RED)
        },
        check = expect_blank
    ),
    scenario!(
        "fill_rect_alpha_zero_is_noop",
        |b| b.fill_rect(10, 10, 40, 40, Color::rgba(255, 255, 255, 0)),
        check = expect_blank
    ),
    scenario!(
        "fill_rect_translucent",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_rect(10, 10, 100, 30, Color::rgba(230, 40, 30, 128))?;
            b.fill_rect(30, 20, 60, 50, Color::rgba(40, 90, 240, 77))?;
            b.fill_rect(5, 60, 110, 20, Color::rgba(0, 0, 0, 254))?;
            b.fill_rect(5, 82, 110, 10, Color::rgba(255, 255, 255, 1))
        },
        check = |f| {
            let over_bg = blend(Color::rgba(230, 40, 30, 128), rgb_of(BG));
            expect_fill(f, (70, 10, 30, 10), over_bg, 2)?;
            let over_white = blend(Color::rgba(230, 40, 30, 128), [255, 255, 255]);
            expect_fill(f, (10, 10, 20, 10), over_white, 2)
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!(
        "dim_screen_and_fill_rect_alpha",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_rect_alpha(70, 10, 40, 40, GREEN, 100)?;
            b.dim_screen(128)
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    // -- shapes --------------------------------------------------------------
    scenario!("rounded_rects", |b| {
        b.fill_rounded_rect(4, 4, 40, 30, 8, RED)?;
        b.fill_rounded_rect(50, 4, 30, 30, 15, GREEN)?;
        b.fill_rounded_rect(86, 4, 31, 17, 40, BLUE)?;
        b.fill_rounded_rect(4, 40, 9, 40, 4, YELLOW)?;
        b.fill_rounded_rect(20, 40, 60, 4, 30, WHITE)?;
        b.fill_rounded_rect(90, 60, 50, 50, 10, RED)?;
        b.fill_rounded_rect(-10, 70, 30, 20, 6, GREEN)
    }),
    scenario!(
        "rounded_rect_translucent",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_rounded_rect(8, 8, 100, 70, 16, Color::rgba(40, 90, 240, 140))
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!(
        "stroke_rects",
        |b| {
            b.stroke_rect(4, 4, 40, 30, 1, RED)?;
            b.stroke_rect(50, 4, 40, 30, 3, GREEN)?;
            b.stroke_rect(96, 4, 6, 6, 4, BLUE)?;
            b.stroke_rect(-5, 60, 30, 30, 2, YELLOW)
        },
        check = |f| {
            expect_fill(f, (4, 4, 40, 1), rgb_of(RED), 0)?;
            expect_fill(f, (4, 33, 40, 1), rgb_of(RED), 0)?;
            expect_fill(f, (5, 5, 38, 28), rgb_of(BG), 0)?;
            expect_ring(f, (4, 4, 40, 30), rgb_of(BG))?;
            expect_fill(f, (50, 4, 40, 3), rgb_of(GREEN), 0)?;
            expect_fill(f, (53, 7, 34, 24), rgb_of(BG), 0)
        }
    ),
    scenario!("stroke_rounded_rects", |b| {
        b.stroke_rounded_rect(4, 4, 50, 36, 10, 1, RED)?;
        b.stroke_rounded_rect(60, 4, 50, 36, 12, 3, GREEN)?;
        b.stroke_rounded_rect(4, 50, 30, 30, 15, 2, BLUE)
    }),
    scenario!(
        "lines_thin",
        |b| {
            b.draw_line(2, 2, 120, 2, 1, RED)?;
            b.draw_line(2, 6, 2, 90, 1, GREEN)?;
            b.draw_line(10, 10, 60, 60, 1, BLUE)?;
            b.draw_line(120, 10, 70, 40, 1, YELLOW)?;
            b.draw_line(70, 90, 110, 50, 1, WHITE)?;
            b.draw_line(-10, 80, 40, 95, 1, WHITE)
        },
        check = |f| {
            expect_fill(f, (2, 2, 119, 1), rgb_of(RED), 0)?;
            expect_fill(f, (2, 6, 1, 70), rgb_of(GREEN), 0)?;
            for i in 0..=50 {
                if f.rgb(10 + i, 10 + i) != rgb_of(BLUE) {
                    return Err(format!("diagonal pixel {i} missing"));
                }
            }
            Ok(())
        }
    ),
    scenario!(
        "lines_thick",
        |b| {
            b.draw_line(10, 10, 110, 10, 3, RED)?;
            b.draw_line(10, 20, 10, 80, 4, GREEN)?;
            b.draw_line(20, 20, 70, 70, 3, BLUE)?;
            b.draw_line(120, 20, 80, 60, 5, YELLOW)?;
            b.draw_line(30, 85, 115, 70, 2, WHITE)
        },
        check = |f| {
            // A 3px diagonal must be thicker than a 1px one: at least 2
            // pixels per row across its body.
            for y in 30..60u32 {
                let n = (0..CANVAS_W)
                    .filter(|&x| f.rgb(x, y) == rgb_of(BLUE))
                    .count();
                if n < 2 {
                    return Err(format!("3px diagonal line is {n}px wide on row {y}"));
                }
            }
            Ok(())
        }
    ),
    scenario!(
        "circles",
        |b| {
            b.fill_circle(20, 20, 12, RED)?;
            b.fill_circle(50, 20, 5, GREEN)?;
            b.fill_circle(70, 20, 1, WHITE)?;
            b.fill_circle(80, 20, 0, WHITE)?;
            b.fill_circle(125, 90, 20, BLUE)?;
            b.stroke_circle(30, 65, 20, 1, YELLOW)?;
            b.stroke_circle(85, 60, 18, 4, GREEN)
        },
        check = |f| {
            expect_fill(f, (20 - 8, 20 - 8, 17, 17), rgb_of(RED), 0)?;
            if f.rgb(30, 65) != rgb_of(BG) {
                return Err("stroked circle has a filled center".into());
            }
            if f.rgb(85, 60) != rgb_of(BG) {
                return Err("thick stroked circle has a filled center".into());
            }
            Ok(())
        }
    ),
    scenario!(
        "circle_translucent",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_circle(64, 48, 30, Color::rgba(230, 40, 30, 120))
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!("triangles", |b| {
        b.fill_triangle(10, 5, 2, 40, 40, 30, RED)?;
        b.fill_triangle(50, 5, 90, 5, 70, 45, GREEN)?;
        b.fill_triangle(100, 10, 125, 50, 95, 45, BLUE)?;
        b.fill_triangle(10, 60, 60, 60, 35, 60, YELLOW)?;
        b.fill_triangle(-20, 70, 50, 95, 30, 110, WHITE)
    }),
    scenario!("polygons", |b| {
        // Convex hexagon.
        b.fill_polygon(
            &[(20, 5), (40, 10), (40, 30), (20, 38), (5, 30), (5, 10)],
            RED,
        )?;
        // Concave arrow.
        b.fill_polygon(
            &[
                (60, 20),
                (80, 5),
                (80, 14),
                (110, 14),
                (110, 26),
                (80, 26),
                (80, 35),
            ],
            GREEN,
        )?;
        // Five-point star outline order (self-intersecting).
        b.fill_polygon(&[(30, 50), (38, 90), (10, 64), (50, 64), (22, 90)], BLUE)?;
        b.stroke_polygon(&[(70, 50), (120, 55), (100, 90), (65, 80)], 1, YELLOW)?;
        b.stroke_polyline(&[(70, 92), (85, 70), (100, 92), (115, 70)], 1, WHITE)
    }),
    scenario!("arcs", |b| {
        b.fill_arc(30, 30, 25, 0.0, 1.5, RED)?;
        b.fill_arc(30, 30, 25, 3.2, 4.4, GREEN)?;
        b.stroke_arc(90, 40, 30, 0.5, 3.0, 1, YELLOW)?;
        b.stroke_arc(90, 40, 20, 3.5, 6.0, 3, BLUE)
    }),
    scenario!("dashed_lines", |b| {
        b.stroke_line_dashed(5, 10, 120, 10, 1, RED, 6, 4)?;
        b.stroke_line_dashed(5, 20, 120, 60, 1, GREEN, 5, 5)?;
        b.stroke_line_dashed(5, 80, 120, 80, 3, BLUE, 8, 3)
    }),
    scenario!(
        "fill_shadow",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_shadow(
                20,
                20,
                60,
                40,
                6.0,
                2.0,
                3.0,
                4.0,
                Color::rgba(0, 0, 0, 200),
                0.0,
            )?;
            b.fill_shadow(
                70,
                50,
                40,
                30,
                4.0,
                0.0,
                0.0,
                0.0,
                Color::rgba(0, 0, 0, 255),
                6.0,
            )
        },
        Tolerance {
            channel: 3,
            pixels: 0,
        },
        "alpha-blend rounding, accumulated over the shadow's stacked translucent layers"
    ),
    // -- gradients -----------------------------------------------------------
    scenario!("gradient_vertical", |b| b.fill_rect_gradient(
        4,
        4,
        60,
        88,
        &GradientStyle::Vertical {
            top: RED,
            bottom: BLUE,
        }
    )),
    scenario!("gradient_horizontal", |b| b.fill_rect_gradient(
        4,
        4,
        120,
        40,
        &GradientStyle::Horizontal {
            left: GREEN,
            right: YELLOW,
        }
    )),
    scenario!("gradient_four_corner", |b| b.fill_rect_gradient(
        4,
        4,
        120,
        88,
        &GradientStyle::FourCorner {
            top_left: RED,
            top_right: GREEN,
            bottom_left: BLUE,
            bottom_right: YELLOW,
        }
    )),
    scenario!("gradient_offscreen_and_tiny", |b| {
        let g = GradientStyle::Vertical {
            top: WHITE,
            bottom: RED,
        };
        b.fill_rect_gradient(-20, -10, 60, 40, &g)?;
        b.fill_rect_gradient(100, 70, 60, 60, &g)?;
        b.fill_rect_gradient(60, 40, 1, 1, &g)?;
        b.fill_rect_gradient(
            70,
            40,
            1,
            20,
            &GradientStyle::Horizontal {
                left: GREEN,
                right: BLUE,
            },
        )
    }),
    scenario!(
        "gradient_translucent",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.fill_rect_gradient(
                4,
                4,
                120,
                88,
                &GradientStyle::Vertical {
                    top: Color::rgba(230, 40, 30, 200),
                    bottom: Color::rgba(40, 90, 240, 40),
                },
            )
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!("rounded_rect_gradient", |b| {
        b.fill_rounded_rect_gradient(
            4,
            4,
            56,
            80,
            12,
            &GradientStyle::Vertical {
                top: RED,
                bottom: YELLOW,
            },
        )?;
        b.fill_rounded_rect_gradient(
            68,
            4,
            56,
            40,
            8,
            &GradientStyle::Horizontal {
                left: GREEN,
                right: BLUE,
            },
        )
    }),
    // -- text ----------------------------------------------------------------
    scenario!(
        "text_integer_scales",
        |b| {
            b.draw_text("Hello, OASIS!", 2, 2, 8, WHITE)?;
            b.draw_text("AgWq|_", 2, 14, 16, YELLOW)?;
            b.draw_text("Mj", 2, 36, 24, GREEN)?;
            b.draw_text("\u{25B2}\u{25BC}", 60, 36, 16, RED)
        },
        check = |f| {
            let bounds = f.ink_bounds().ok_or("no text drawn")?;
            if bounds.0 < 2 || bounds.1 < 2 {
                return Err(format!("text ink starts before its origin: {bounds:?}"));
            }
            Ok(())
        }
    ),
    scenario!("text_fractional_scales", |b| {
        b.draw_text("Settings 12px", 2, 2, 12, WHITE)?;
        b.draw_text("Tiny 10px", 2, 18, 10, YELLOW)?;
        b.draw_text("Big 20", 2, 32, 20, GREEN)?;
        b.draw_text("small 6", 2, 60, 6, RED)?;
        b.draw_text("14px text", 2, 72, 14, WHITE)
    }),
    scenario!("text_styles", |b| {
        b.draw_text_styled("Bold 8", 2, 2, 8, WHITE, true, false)?;
        b.draw_text_styled("Bold 16", 2, 12, 16, YELLOW, true, false)?;
        b.draw_text_styled("Ital 16", 2, 32, 16, GREEN, false, true)?;
        b.draw_text_styled("BI 12", 2, 52, 12, RED, true, true)?;
        b.draw_text_styled("Bold 12", 60, 52, 12, WHITE, true, false)
    }),
    scenario!(
        "text_translucent",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            b.draw_text("Fade AB", 2, 4, 16, Color::rgba(230, 40, 30, 128))?;
            b.draw_text_styled(
                "Bold",
                2,
                30,
                16,
                Color::rgba(40, 90, 240, 100),
                true,
                false,
            )
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!("text_clipped_and_offscreen", |b| {
        b.draw_text("Offscreen", -20, 2, 16, WHITE)?;
        b.draw_text("Right edge", 100, 40, 16, YELLOW)?;
        b.draw_text("Bottom", 10, 88, 16, GREEN)?;
        b.draw_text("", 10, 10, 16, RED)?;
        b.draw_text("zero", 10, 60, 0, RED)
    }),
    scenario!("text_ellipsis_and_wrapped", |b| {
        b.draw_text_ellipsis("A very long label", 2, 2, 8, WHITE, 60)?;
        b.draw_text_wrapped("wrap this text into lines", 2, 20, 8, YELLOW, 70, 0)?;
        Ok(())
    }),
    // -- textures ------------------------------------------------------------
    scenario!(
        "texture_odd_width_37",
        |b| blit_pattern(b, 37, 6, 3, 3),
        check = |f| expect_image(f, &stride_pattern(37, 6), 37, 6, 3, 3)
    ),
    scenario!(
        "texture_width_1",
        |b| {
            let px = stride_pattern(1, 9);
            let tex = b.load_texture(1, 9, &px)?;
            b.blit(tex, 10, 10, 1, 9)?;
            b.destroy_texture(tex)
        },
        check = |f| expect_image(f, &stride_pattern(1, 9), 1, 9, 10, 10)
    ),
    scenario!(
        "texture_1x1",
        |b| {
            let tex = b.load_texture(1, 1, &[250, 20, 90, 255])?;
            b.blit(tex, 64, 48, 1, 1)?;
            b.destroy_texture(tex)
        },
        check = |f| expect_rect_exact(f, (64, 48, 1, 1), [250, 20, 90])
    ),
    scenario!(
        "texture_width_250_offscreen",
        |b| blit_pattern(b, 250, 7, -60, 20),
        check = |f| expect_image(f, &stride_pattern(250, 7), 250, 7, -60, 20)
    ),
    scenario!(
        "texture_width_333",
        |b| blit_pattern(b, 333, 5, -100, 50),
        check = |f| expect_image(f, &stride_pattern(333, 5), 333, 5, -100, 50)
    ),
    scenario!(
        "texture_odd_widths_sweep",
        |b| {
            // Widths whose row size is not a multiple of 16/64/256 bytes.
            let mut y = 0;
            for w in [3u32, 5, 13, 17, 31, 33, 63, 65, 127] {
                blit_pattern(b, w, 3, 0, y)?;
                y += 4;
            }
            Ok(())
        },
        check = |f| {
            let mut y = 0;
            for w in [3u32, 5, 13, 17, 31, 33, 63, 65, 127] {
                expect_image(f, &stride_pattern(w, 3), w, 3, 0, y)?;
                y += 4;
            }
            Ok(())
        }
    ),
    scenario!(
        "texture_translucent_pixels",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            let mut px = Vec::new();
            for i in 0..(40 * 40u32) {
                let a = ((i % 40) * 6) as u8;
                px.extend_from_slice(&[230, 40, 30, a]);
            }
            let tex = b.load_texture(40, 40, &px)?;
            b.blit(tex, 40, 20, 40, 40)?;
            b.destroy_texture(tex)
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!(
        "texture_blit_upscaled",
        |b| {
            let px = checker(8, 8, 1, RED, BLUE);
            let tex = b.load_texture(8, 8, &px)?;
            b.blit(tex, 10, 10, 64, 64)?;
            b.destroy_texture(tex)
        },
        check = |f| {
            // Whatever the filter, the texel centers are dominated by their
            // own texel and nothing lands outside the destination rect.
            for ty in 0..8u32 {
                for tx in 0..8u32 {
                    let (want, other) = if (tx + ty).is_multiple_of(2) {
                        (RED, BLUE)
                    } else {
                        (BLUE, RED)
                    };
                    let (x, y) = (10 + tx * 8 + 4, 10 + ty * 8 + 4);
                    let got = f.rgb(x, y);
                    let dist = |c: Color| {
                        (0..3)
                            .map(|i| got[i].abs_diff(rgb_of(c)[i]) as u32)
                            .sum::<u32>()
                    };
                    if dist(want) >= dist(other) {
                        return Err(format!("texel ({tx},{ty}) center ({x},{y}) is {got:?}"));
                    }
                }
            }
            expect_ring(f, (10, 10, 64, 64), rgb_of(BG))
        },
        Tolerance {
            channel: 120,
            pixels: 0,
        },
        "intentional: SDL textures use the renderer's default linear filter when \
         scaled (smooth photos and wallpapers); the software rasterizer samples \
         nearest. Texel interiors blend towards their neighbours on SDL."
    ),
    scenario!(
        "texture_blit_downscaled",
        |b| blit_pattern_scaled(b, 100, 60, (4, 4, 50, 30)),
        check = |f| expect_ink_within(f, (4, 4, 50, 30)),
        Tolerance {
            channel: 255,
            pixels: 0,
        },
        "SDL linear filtering vs rasterizer nearest sampling: only the \
         destination bounds are compared"
    ),
    scenario!(
        "texture_blit_sub",
        |b| {
            let px = stride_pattern(41, 23);
            let tex = b.load_texture(41, 23, &px)?;
            b.blit_sub(tex, 5, 3, 20, 10, 10, 10, 20, 10)?;
            b.blit_sub(tex, 0, 0, 41, 23, 60, 10, 41, 23)?;
            b.destroy_texture(tex)
        },
        check = |f| {
            let px = stride_pattern(41, 23);
            expect_image(f, &px, 41, 23, 60, 10)?;
            for y in 0..10u32 {
                for x in 0..20u32 {
                    let want = pattern_rgb(&px, 41, 5 + x, 3 + y);
                    if f.rgb(10 + x, 10 + y) != want {
                        return Err(format!("blit_sub pixel ({x},{y})"));
                    }
                }
            }
            expect_ring(f, (10, 10, 20, 10), rgb_of(BG))
        }
    ),
    scenario!(
        "texture_blit_tinted",
        |b| {
            let px = checker(16, 16, 4, WHITE, Color::rgb(128, 128, 128));
            let tex = b.load_texture(16, 16, &px)?;
            b.blit_tinted(tex, 4, 4, 16, 16, Color::rgb(255, 128, 0))?;
            b.blit_tinted(tex, 24, 4, 16, 16, Color::rgba(40, 200, 255, 128))?;
            b.blit_sub_tinted(tex, 4, 4, 8, 8, 44, 4, 8, 8, Color::rgb(0, 255, 0))?;
            // An untinted blit afterwards must not inherit the tint.
            b.blit(tex, 60, 4, 16, 16)?;
            b.destroy_texture(tex)
        },
        check = |f| expect_image(
            f,
            &checker(16, 16, 4, WHITE, Color::rgb(128, 128, 128)),
            16,
            16,
            60,
            4
        ),
        Tolerance::BLEND,
        "color-mod rounding: SDL truncates `s*m/255`, the rasterizer rounds"
    ),
    scenario!(
        "texture_blit_flipped",
        |b| {
            let px = stride_pattern(20, 12);
            let tex = b.load_texture(20, 12, &px)?;
            b.blit_flipped(tex, 4, 4, 20, 12, false, false)?;
            b.blit_flipped(tex, 30, 4, 20, 12, true, false)?;
            b.blit_flipped(tex, 56, 4, 20, 12, false, true)?;
            b.blit_flipped(tex, 82, 4, 20, 12, true, true)?;
            b.destroy_texture(tex)
        },
        check = |f| {
            let px = stride_pattern(20, 12);
            for y in 0..12u32 {
                for x in 0..20u32 {
                    let src = pattern_rgb(&px, 20, x, y);
                    let checks = [
                        (4 + x, 4 + y),
                        (30 + 19 - x, 4 + y),
                        (56 + x, 4 + 11 - y),
                        (82 + 19 - x, 4 + 11 - y),
                    ];
                    for (i, (sx, sy)) in checks.into_iter().enumerate() {
                        if f.rgb(sx, sy) != src {
                            return Err(format!("flip variant {i}: texel ({x},{y})"));
                        }
                    }
                }
            }
            Ok(())
        }
    ),
    scenario!(
        "texture_destroy_and_reuse",
        |b| {
            let a = b.load_texture(4, 4, &checker(4, 4, 2, RED, RED))?;
            let c = b.load_texture(4, 4, &checker(4, 4, 2, GREEN, GREEN))?;
            b.destroy_texture(a)?;
            // A new texture may reuse `a`'s id; it must show its own pixels.
            let d = b.load_texture(6, 6, &checker(6, 6, 3, BLUE, BLUE))?;
            b.blit(c, 4, 4, 4, 4)?;
            b.blit(d, 12, 4, 6, 6)?;
            // Blitting a destroyed id is an error, never stale pixels.
            if d != a && b.blit(a, 30, 4, 4, 4).is_ok() {
                return Err(OasisError::Backend("blit of destroyed texture".into()));
            }
            b.destroy_texture(c)?;
            b.destroy_texture(d)?;
            // Destroying twice is harmless.
            b.destroy_texture(d)
        },
        check = |f| {
            expect_rect_exact(f, (4, 4, 4, 4), rgb_of(GREEN))?;
            expect_rect_exact(f, (12, 4, 6, 6), rgb_of(BLUE))?;
            if f.count(rgb_of(RED)) != 0 {
                return Err("destroyed texture pixels visible".into());
            }
            Ok(())
        }
    ),
    scenario!(
        "texture_bad_data_rejected",
        |b| {
            if b.load_texture(4, 4, &[0; 10]).is_ok() {
                return Err(OasisError::Backend("short RGBA data accepted".into()));
            }
            if b.blit(TextureId(987_654), 0, 0, 4, 4).is_ok() {
                return Err(OasisError::Backend("unknown texture id blitted".into()));
            }
            Ok(())
        },
        check = expect_blank
    ),
    // -- clip / translate ----------------------------------------------------
    scenario!(
        "clip_set_and_reset",
        |b| {
            b.set_clip_rect(20, 10, 40, 30)?;
            b.fill_rect(0, 0, 128, 96, RED)?;
            b.draw_text("CLIPPED TEXT", 0, 20, 16, WHITE)?;
            b.reset_clip_rect()?;
            b.fill_rect(100, 80, 5, 5, GREEN)
        },
        check = |f| {
            expect_ink_within(f, (20, 10, 85, 75))?;
            expect_fill(f, (0, 0, 20, 96), rgb_of(BG), 0)?;
            expect_fill(f, (60, 0, 40, 96), rgb_of(BG), 0)?;
            expect_rect_exact(f, (100, 80, 5, 5), rgb_of(GREEN))
        }
    ),
    scenario!(
        "clip_zero_size_rejects_all",
        |b| {
            b.set_clip_rect(10, 40, 100, 0)?;
            b.fill_rect(0, 0, 128, 96, RED)?;
            b.set_clip_rect(10, 40, 0, 10)?;
            b.fill_rect(0, 0, 128, 96, RED)?;
            b.draw_text("X", 10, 40, 16, RED)?;
            b.reset_clip_rect()
        },
        check = expect_blank
    ),
    scenario!(
        "clip_stack_nesting",
        |b| {
            b.push_clip_rect(10, 10, 100, 70)?;
            b.fill_rect(0, 0, 128, 96, Color::rgb(60, 60, 60))?;
            b.push_clip_rect(30, 30, 100, 100)?;
            b.fill_rect(0, 0, 128, 96, RED)?;
            b.push_clip_rect(200, 200, 10, 10)?; // disjoint: empty
            b.fill_rect(0, 0, 128, 96, BLUE)?;
            b.pop_clip_rect()?;
            b.fill_rect(40, 40, 10, 10, GREEN)?;
            b.pop_clip_rect()?;
            b.fill_rect(0, 75, 128, 20, YELLOW)?;
            b.pop_clip_rect()?;
            b.fill_rect(120, 90, 8, 6, WHITE)
        },
        check = |f| {
            expect_fill(f, (10, 10, 20, 65), [60, 60, 60], 0)?;
            expect_fill(f, (30, 10, 80, 20), [60, 60, 60], 0)?;
            expect_fill(f, (60, 30, 50, 45), rgb_of(RED), 0)?;
            expect_fill(f, (40, 40, 10, 10), rgb_of(GREEN), 0)?;
            expect_fill(f, (10, 75, 100, 5), rgb_of(YELLOW), 0)?;
            expect_fill(f, (0, 80, 110, 10), rgb_of(BG), 0)?;
            expect_rect_exact(f, (120, 90, 8, 6), rgb_of(WHITE))?;
            if f.count(rgb_of(BLUE)) != 0 {
                return Err("draw under an empty clip leaked".into());
            }
            Ok(())
        }
    ),
    scenario!(
        "translate_stack",
        |b| {
            b.push_translate(20, 10)?;
            b.fill_rect(0, 0, 10, 10, RED)?;
            b.push_translate(30, 5)?;
            b.fill_rect(0, 0, 10, 10, GREEN)?;
            b.fill_rounded_rect(0, 20, 20, 16, 5, BLUE)?;
            b.fill_circle(40, 10, 6, YELLOW)?;
            b.draw_line(0, 40, 30, 40, 1, WHITE)?;
            b.draw_text("T", 50, 30, 8, WHITE)?;
            b.push_clip_rect(0, 0, 5, 5)?; // clip is translated too
            b.fill_rect(-10, -10, 50, 50, Color::rgb(90, 0, 90))?;
            b.pop_clip_rect()?;
            b.pop_translate()?;
            b.fill_rect_gradient(
                0,
                60,
                20,
                10,
                &GradientStyle::Horizontal {
                    left: RED,
                    right: BLUE,
                },
            )?;
            b.pop_translate()?;
            b.fill_rect(0, 0, 3, 3, WHITE)
        },
        check = |f| {
            expect_rect_exact(f, (20, 10, 10, 10), rgb_of(RED))?;
            expect_fill(f, (55, 20, 5, 5), rgb_of(GREEN), 0)?;
            expect_fill(f, (50, 15, 5, 5), [90, 0, 90], 0)?;
            expect_rect_exact(f, (0, 0, 3, 3), rgb_of(WHITE))
        }
    ),
    scenario!(
        "translated_textures",
        |b| {
            let px = stride_pattern(13, 7);
            let tex = b.load_texture(13, 7, &px)?;
            b.push_translate(17, 9)?;
            b.blit(tex, 0, 0, 13, 7)?;
            b.blit_sub(tex, 0, 0, 13, 7, 20, 0, 13, 7)?;
            b.blit_tinted(tex, 40, 0, 13, 7, WHITE)?;
            b.blit_flipped(tex, 60, 0, 13, 7, false, false)?;
            b.pop_translate()?;
            b.destroy_texture(tex)
        },
        check = |f| {
            let px = stride_pattern(13, 7);
            for x in [17, 37, 57, 77] {
                expect_image(f, &px, 13, 7, x, 9)?;
            }
            Ok(())
        }
    ),
    // -- batches -------------------------------------------------------------
    scenario!(
        "batch_rects",
        |b| {
            let mut rects = Vec::new();
            for i in 0..12i32 {
                rects.push(BatchRect {
                    x: (i % 4) * 30 + 2,
                    y: (i / 4) * 30 + 2,
                    w: 24,
                    h: 20,
                    color: if i % 3 == 0 { RED } else { GREEN },
                });
            }
            // Overlapping translucent, zero-size and invisible entries.
            rects.push(BatchRect {
                x: 10,
                y: 10,
                w: 100,
                h: 10,
                color: Color::rgba(40, 90, 240, 128),
            });
            rects.push(BatchRect {
                x: 5,
                y: 5,
                w: 0,
                h: 10,
                color: WHITE,
            });
            rects.push(BatchRect {
                x: 5,
                y: 5,
                w: 10,
                h: 10,
                color: Color::rgba(255, 255, 255, 0),
            });
            b.push_translate(1, 1)?;
            b.begin_batch()?;
            b.submit_rect_batch(&rects)?;
            b.flush_batch()?;
            b.pop_translate()
        },
        Tolerance::BLEND,
        "alpha-blend rounding"
    ),
    scenario!("batch_text", |b| {
        let items = [
            BatchText {
                text: "batched",
                x: 2,
                y: 2,
                color: WHITE,
            },
            BatchText {
                text: "labels",
                x: 2,
                y: 30,
                color: YELLOW,
            },
        ];
        b.submit_text_batch(&items, 16, false, false)?;
        b.submit_text_batch(&items[..1], 16, true, false)?;
        Ok(())
    }),
    // -- render targets ------------------------------------------------------
    scenario!(
        "render_target_opaque",
        |b| {
            if !b.supports_render_targets() {
                return Ok(());
            }
            let rt = b.create_render_target(40, 30)?;
            b.bind_render_target(rt)?;
            b.fill_rect(0, 0, 40, 30, RED)?;
            b.fill_rect(5, 5, 10, 10, GREEN)?;
            b.unbind_render_target()?;
            b.composite_render_target(rt, 10, 10, 40, 30, BlendMode::Normal, 1.0)?;
            b.composite_render_target(rt, 70, 50, 40, 30, BlendMode::Normal, 1.0)?;
            b.destroy_render_target(rt)
        },
        check = |f| {
            expect_fill(f, (15, 15, 10, 10), rgb_of(GREEN), 0)?;
            expect_fill(f, (30, 10, 20, 30), rgb_of(RED), 0)?;
            expect_fill(f, (10, 25, 40, 15), rgb_of(RED), 0)?;
            expect_ring(f, (10, 10, 40, 30), rgb_of(BG))?;
            expect_fill(f, (75, 55, 10, 10), rgb_of(GREEN), 0)
        }
    ),
    scenario!(
        "render_target_starts_transparent",
        |b| {
            let rt = b.create_render_target(30, 20)?;
            b.bind_render_target(rt)?;
            b.fill_rect(5, 5, 5, 5, GREEN)?;
            b.unbind_render_target()?;
            b.composite_render_target(rt, 20, 20, 30, 20, BlendMode::Normal, 1.0)?;
            b.destroy_render_target(rt)
        },
        check = |f| expect_rect_exact(f, (25, 25, 5, 5), rgb_of(GREEN))
    ),
    scenario!(
        "render_target_opacity",
        |b| {
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            let rt = b.create_render_target(80, 40)?;
            b.bind_render_target(rt)?;
            b.fill_rect(0, 0, 80, 40, BLUE)?;
            b.unbind_render_target()?;
            b.composite_render_target(rt, 20, 20, 80, 40, BlendMode::Normal, 0.5)?;
            b.destroy_render_target(rt)
        },
        check = |f| {
            let want = blend(BLUE.with_alpha(128), [255, 255, 255]);
            expect_fill(f, (22, 22, 30, 36), want, 3)?;
            let want = blend(BLUE.with_alpha(128), rgb_of(BG));
            expect_fill(f, (70, 22, 28, 36), want, 3)
        },
        Tolerance {
            channel: 3,
            pixels: 0,
        },
        "opacity quantization: SDL alpha-mod is round(op*255), the \
         rasterizer scales by round(op*256)/256"
    ),
    scenario!(
        "render_target_translucent_content",
        |b| {
            // A layer holding translucent paint composited at full opacity
            // must look exactly like painting it directly.
            b.fill_rect(0, 0, 64, 96, WHITE)?;
            let rt = b.create_render_target(100, 60)?;
            b.bind_render_target(rt)?;
            b.fill_rect(0, 0, 100, 60, Color::rgba(230, 40, 30, 128))?;
            b.fill_rect(20, 20, 40, 20, Color::rgba(40, 90, 240, 160))?;
            b.unbind_render_target()?;
            b.composite_render_target(rt, 10, 10, 100, 60, BlendMode::Normal, 1.0)?;
            b.destroy_render_target(rt)
        },
        check = |f| {
            let red = Color::rgba(230, 40, 30, 128);
            expect_fill(f, (12, 12, 10, 10), blend(red, [255, 255, 255]), 3)?;
            expect_fill(f, (80, 12, 20, 10), blend(red, rgb_of(BG)), 3)?;
            let both = blend(Color::rgba(40, 90, 240, 160), blend(red, rgb_of(BG)));
            expect_fill(f, (66, 32, 3, 6), both, 3)
        },
        Tolerance {
            channel: 3,
            pixels: 0,
        },
        "alpha-blend rounding (two blend steps)"
    ),
    scenario!(
        "render_target_nested",
        |b| {
            let outer = b.create_render_target(60, 60)?;
            let inner = b.create_render_target(20, 20)?;
            b.bind_render_target(outer)?;
            b.fill_rect(0, 0, 60, 60, Color::rgb(80, 80, 80))?;
            b.bind_render_target(inner)?;
            b.fill_rect(0, 0, 20, 20, YELLOW)?;
            b.unbind_render_target()?;
            b.composite_render_target(inner, 20, 20, 20, 20, BlendMode::Normal, 1.0)?;
            b.unbind_render_target()?;
            b.composite_render_target(outer, 30, 20, 60, 60, BlendMode::Normal, 1.0)?;
            b.destroy_render_target(inner)?;
            b.destroy_render_target(outer)
        },
        check = |f| {
            expect_fill(f, (50, 40, 20, 20), rgb_of(YELLOW), 0)?;
            expect_fill(f, (30, 20, 20, 20), [80, 80, 80], 0)?;
            expect_ring(f, (30, 20, 60, 60), rgb_of(BG))
        }
    ),
    scenario!(
        "render_target_multiply",
        |b| {
            b.fill_rect(0, 0, 128, 96, Color::rgb(200, 200, 200))?;
            let rt = b.create_render_target(60, 60)?;
            b.bind_render_target(rt)?;
            b.fill_rect(10, 10, 20, 20, Color::rgb(255, 128, 0))?;
            b.unbind_render_target()?;
            b.composite_render_target(rt, 20, 20, 60, 60, BlendMode::Multiply, 1.0)?;
            b.destroy_render_target(rt)
        },
        check = |f| {
            // Transparent layer pixels leave the backdrop untouched.
            expect_fill(f, (20, 20, 60, 10), [200, 200, 200], 0)?;
            // Painted pixels multiply: 200 * (255, 128, 0) / 255.
            expect_fill(f, (30, 30, 20, 20), [200, 100, 0], 1)
        },
        Tolerance {
            channel: 1,
            pixels: 0,
        },
        "multiply rounding"
    ),
];

fn blit_pattern(b: &mut dyn SdiBackend, w: u32, h: u32, x: i32, y: i32) -> Result<()> {
    let px = stride_pattern(w, h);
    let tex = b.load_texture(w, h, &px)?;
    b.blit(tex, x, y, w, h)?;
    b.destroy_texture(tex)
}

fn blit_pattern_scaled(
    b: &mut dyn SdiBackend,
    w: u32,
    h: u32,
    dst: (i32, i32, u32, u32),
) -> Result<()> {
    let px = stride_pattern(w, h);
    let tex = b.load_texture(w, h, &px)?;
    b.blit(tex, dst.0, dst.1, dst.2, dst.3)?;
    b.destroy_texture(tex)
}

/// Run one scenario on `backend` from a clean [`BG`] canvas and capture
/// the result. Resets clip state afterwards so a failed scenario cannot
/// leak into the next one.
pub fn render(backend: &mut dyn SdiBackend, scenario: &Scenario) -> Result<Frame> {
    backend.reset_clip_rect()?;
    backend.clear(BG)?;
    let drawn = (scenario.draw)(backend);
    backend.reset_clip_rect()?;
    drawn?;
    if backend.current_translate() != (0, 0) {
        return Err(OasisError::Backend(
            format!("scenario {} leaked a translate", scenario.name).into(),
        ));
    }
    Frame::capture(backend, CANVAS_W, CANVAS_H)
}

/// Text metrics every backend must agree on, as `(label, value)` pairs.
pub fn text_metrics(backend: &dyn SdiBackend) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for fs in [6u16, 8, 10, 12, 14, 16, 20, 24, 32] {
        for text in ["", "Hello, OASIS!", "Wj|_", "\u{25B2}\u{25BC}", "caf\u{e9}"] {
            out.push((
                format!("measure_text({text:?}, {fs})"),
                backend.measure_text(text, fs),
            ));
        }
        out.push((
            format!("measure_text_height({fs})"),
            backend.measure_text_height(fs),
        ));
        out.push((format!("font_ascent({fs})"), backend.font_ascent(fs)));
    }
    out
}

/// Scenario names must be unique (dump files and failure messages key on
/// them).
pub fn assert_unique_names() {
    let mut names: Vec<&str> = SCENARIOS.iter().map(|s| s.name).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len(), "duplicate scenario names");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_unique() {
        assert_unique_names();
    }

    #[test]
    fn blend_matches_reference_formula() {
        assert_eq!(blend(Color::rgba(255, 0, 0, 255), [0, 0, 0]), [255, 0, 0]);
        assert_eq!(blend(Color::rgba(255, 0, 0, 0), [1, 2, 3]), [1, 2, 3]);
        assert_eq!(
            blend(Color::rgba(255, 255, 255, 128), [0, 0, 0]),
            [128, 128, 128]
        );
    }

    #[test]
    fn frame_diff_counts_mismatches() {
        let a = Frame {
            w: 2,
            h: 1,
            rgba: vec![10, 10, 10, 255, 20, 20, 20, 255],
        };
        let mut b = a.clone();
        b.rgba[4] = 23;
        let d = a.diff(&b, Tolerance::BLEND);
        assert_eq!(d.mismatched, 1);
        assert_eq!(d.max_delta, 3);
        assert!(
            a.diff(
                &b,
                Tolerance {
                    channel: 3,
                    pixels: 0
                }
            )
            .within(Tolerance::EXACT)
        );
    }

    #[test]
    fn stride_pattern_rows_differ() {
        // A one-row shift must change pixels (what makes shear visible).
        let p = stride_pattern(37, 3);
        assert_ne!(&p[..37 * 4], &p[37 * 4..74 * 4]);
    }
}

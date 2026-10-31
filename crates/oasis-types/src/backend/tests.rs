//! Tests for the backend trait hierarchy.
//!
//! Extracted into a sibling file so `backend/mod.rs` can stay a thin
//! re-export shell. Loaded via `#[cfg(test)] mod tests;` from mod.rs;
//! `super::*` therefore resolves to the parent `backend` module.

use super::*;
use std::cell::RefCell;

/// A test backend that records all draw calls for assertion.
struct RecordingBackend {
    calls: RefCell<Vec<String>>,
}

impl RecordingBackend {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    #[allow(dead_code)]
    fn clear_calls(&self) {
        self.calls.borrow_mut().clear();
    }
}

impl SdiCore for RecordingBackend {
    fn init(&mut self, _w: u32, _h: u32) -> crate::error::Result<()> {
        Ok(())
    }
    fn clear(&mut self, color: Color) -> crate::error::Result<()> {
        self.calls.borrow_mut().push(format!(
            "clear({},{},{},{})",
            color.r, color.g, color.b, color.a
        ));
        Ok(())
    }
    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> crate::error::Result<()> {
        self.calls
            .borrow_mut()
            .push(format!("blit({},{x},{y},{w},{h})", tex.0));
        Ok(())
    }
    fn fill_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    ) -> crate::error::Result<()> {
        self.calls.borrow_mut().push(format!(
            "fill_rect({x},{y},{w},{h},{},{},{},{})",
            color.r, color.g, color.b, color.a
        ));
        Ok(())
    }
    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> crate::error::Result<()> {
        self.calls.borrow_mut().push(format!(
            "draw_text({text},{x},{y},{font_size},{},{},{},{})",
            color.r, color.g, color.b, color.a
        ));
        Ok(())
    }
    fn swap_buffers(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
    fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> crate::error::Result<TextureId> {
        Ok(TextureId(1))
    }
    fn destroy_texture(&mut self, _tex: TextureId) -> crate::error::Result<()> {
        Ok(())
    }
    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> crate::error::Result<()> {
        self.calls
            .borrow_mut()
            .push(format!("set_clip({x},{y},{w},{h})"));
        Ok(())
    }
    fn reset_clip_rect(&mut self) -> crate::error::Result<()> {
        self.calls.borrow_mut().push("reset_clip".into());
        Ok(())
    }
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        bitmap_measure_text(text, font_size)
    }
    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<Vec<u8>> {
        Ok(Vec::new())
    }
    fn shutdown(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
}

impl SdiShapes for RecordingBackend {}
impl SdiGradients for RecordingBackend {}
impl SdiText for RecordingBackend {}
impl SdiTextures for RecordingBackend {}
impl SdiClipTransform for RecordingBackend {}
impl SdiVector for RecordingBackend {}
impl SdiBatch for RecordingBackend {}

/// Parse the recorded `fill_rect(x,y,w,h,..)` calls into a per-pixel
/// coverage count, so shape defaults can be checked by the pixels they
/// cover rather than by how they split into rectangles.
fn coverage(calls: &[String]) -> std::collections::BTreeMap<(i32, i32), u32> {
    let mut cov = std::collections::BTreeMap::new();
    for call in calls {
        let Some(args) = call.strip_prefix("fill_rect(") else {
            continue;
        };
        let v: Vec<i64> = args
            .trim_end_matches(')')
            .split(',')
            .map(|n| n.parse().unwrap())
            .collect();
        let (x, y, w, h) = (v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32);
        for py in y..y + h {
            for px in x..x + w {
                *cov.entry((px, py)).or_insert(0) += 1;
            }
        }
    }
    cov
}

/// The pixels covered by the recorded `fill_rect` calls, asserting that
/// none is covered twice (translucent fills would double-blend).
fn pixels_once(b: &RecordingBackend) -> std::collections::BTreeSet<(i32, i32)> {
    let cov = coverage(&b.calls());
    if let Some((p, n)) = cov.iter().find(|&(_, &n)| n > 1) {
        panic!("pixel {p:?} covered {n} times");
    }
    cov.into_keys().collect()
}

/// Bounding box `(min_x, min_y, max_x, max_y)` of a pixel set.
fn bbox(px: &std::collections::BTreeSet<(i32, i32)>) -> (i32, i32, i32, i32) {
    let xs = px.iter().map(|p| p.0);
    let ys = px.iter().map(|p| p.1);
    (
        xs.clone().min().unwrap(),
        ys.clone().min().unwrap(),
        xs.max().unwrap(),
        ys.max().unwrap(),
    )
}

// -- Color tests --

#[test]
fn color_rgb_alpha_255() {
    let c = Color::rgb(10, 20, 30);
    assert_eq!(c.a, 255);
}

#[test]
fn color_rgba_explicit() {
    let c = Color::rgba(1, 2, 3, 128);
    assert_eq!((c.r, c.g, c.b, c.a), (1, 2, 3, 128));
}

#[test]
fn color_with_alpha() {
    let c = Color::rgb(100, 200, 50).with_alpha(64);
    assert_eq!(c.r, 100);
    assert_eq!(c.a, 64);
}

#[test]
fn color_constants() {
    assert_eq!(Color::BLACK, Color::rgb(0, 0, 0));
    assert_eq!(Color::WHITE, Color::rgb(255, 255, 255));
    assert_eq!(Color::TRANSPARENT, Color::rgba(0, 0, 0, 0));
}

// -- TextureId tests --

#[test]
fn texture_id_equality() {
    assert_eq!(TextureId(42), TextureId(42));
    assert_ne!(TextureId(1), TextureId(2));
}

#[test]
fn texture_id_hash() {
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(TextureId(1));
    set.insert(TextureId(2));
    set.insert(TextureId(1));
    assert_eq!(set.len(), 2);
}

// -- Default: fill_rounded_rect rounds its corners --

#[test]
fn fill_rounded_rect_default_rounds_corners() {
    let mut b = RecordingBackend::new();
    b.fill_rounded_rect(10, 20, 100, 50, 8, Color::rgb(255, 0, 0))
        .unwrap();
    let px = pixels_once(&b);
    assert_eq!(bbox(&px), (10, 20, 109, 69));
    // Corners are cut, the middle of each edge is filled.
    for corner in [(10, 20), (109, 20), (10, 69), (109, 69)] {
        assert!(!px.contains(&corner), "corner {corner:?} must be cut");
    }
    for edge in [(60, 20), (60, 69), (10, 45), (109, 45)] {
        assert!(px.contains(&edge), "edge pixel {edge:?} must be filled");
    }
    // The straight body is merged into one rectangle, not one per row.
    assert!(b.calls().len() < 50, "{} calls", b.calls().len());
}

// -- Default: stroke_rect emits 4 fill_rect calls --

#[test]
fn stroke_rect_emits_four_rects() {
    let mut b = RecordingBackend::new();
    b.stroke_rect(0, 0, 100, 80, 2, Color::WHITE).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 4, "stroke_rect should emit 4 fill_rect calls");
    for call in &calls {
        assert!(call.starts_with("fill_rect("));
    }
}

#[test]
fn stroke_rect_top_edge() {
    let mut b = RecordingBackend::new();
    b.stroke_rect(5, 10, 100, 80, 3, Color::WHITE).unwrap();
    let calls = b.calls();
    // First call is the top edge: fill_rect(5,10,100,3,...)
    assert!(calls[0].starts_with("fill_rect(5,10,100,3,"));
}

// -- Default: stroke_rounded_rect honours the radius --

#[test]
fn stroke_rounded_rect_default_is_rounded_outline() {
    let mut b = RecordingBackend::new();
    b.stroke_rounded_rect(0, 0, 50, 50, 5, 1, Color::WHITE)
        .unwrap();
    let px = pixels_once(&b);
    assert!(!px.contains(&(0, 0)), "corner is rounded off");
    assert!(px.contains(&(25, 0)) && px.contains(&(0, 25)));
    assert!(!px.contains(&(25, 25)), "interior stays empty");

    // A zero radius strokes the plain rectangle.
    let mut b = RecordingBackend::new();
    b.stroke_rounded_rect(0, 0, 50, 50, 0, 1, Color::WHITE)
        .unwrap();
    assert_eq!(b.calls().len(), 4);
}

// -- Default: draw_line horizontal --

#[test]
fn draw_line_horizontal() {
    let mut b = RecordingBackend::new();
    b.draw_line(10, 50, 100, 50, 2, Color::WHITE).unwrap();
    let calls = b.calls();
    // Both endpoints included; the 2 px brush spans rows -1..=0 around
    // the line, exactly as the SDL / UE5 rasterizers draw it.
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("fill_rect(9,49,92,2,"), "{calls:?}");
}

#[test]
fn draw_line_vertical() {
    let mut b = RecordingBackend::new();
    b.draw_line(50, 10, 50, 80, 3, Color::WHITE).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("fill_rect(49,9,3,73,"), "{calls:?}");
}

#[test]
fn draw_line_diagonal_uses_bresenham() {
    let mut b = RecordingBackend::new();
    b.draw_line(0, 0, 5, 5, 1, Color::WHITE).unwrap();
    let calls = b.calls();
    // Bresenham plots one fill_rect per pixel along the diagonal.
    assert_eq!(calls.len(), 6); // (0,0) through (5,5) inclusive
    assert!(calls[0].starts_with("fill_rect(0,0,1,1,"));
    assert!(calls[5].starts_with("fill_rect(5,5,1,1,"));
}

// -- Default: fill_circle uses midpoint algorithm (scanline spans) --

#[test]
fn fill_circle_default() {
    let mut b = RecordingBackend::new();
    b.fill_circle(50, 50, 10, Color::rgb(0, 255, 0)).unwrap();
    let calls = b.calls();
    // Midpoint algorithm emits multiple fill_rect scanlines, not a single box.
    assert!(
        calls.len() > 1,
        "fill_circle should emit multiple scanlines"
    );
    // All calls should be fill_rect.
    for call in &calls {
        assert!(call.starts_with("fill_rect("));
    }
}

// -- Default: stroke_circle falls back to fill_circle --

#[test]
fn stroke_circle_default() {
    let mut b = RecordingBackend::new();
    b.stroke_circle(50, 50, 10, 2, Color::WHITE).unwrap();
    let px = pixels_once(&b);
    // A ring: the centre is untouched (it used to be a filled disc).
    assert!(!px.contains(&(50, 50)), "stroke_circle must not fill");
    assert!(px.contains(&(60, 50)) && px.contains(&(40, 50)));
    assert!(px.contains(&(50, 60)) && px.contains(&(50, 40)));
    assert!(!px.contains(&(57, 50)), "ring is only 2 px thick");
}

#[test]
fn fill_circle_default_covers_each_pixel_once() {
    // The classic midpoint fill emitted some rows twice, which banded
    // translucent circles.
    for r in [0u16, 1, 2, 5, 10, 33] {
        let mut b = RecordingBackend::new();
        b.fill_circle(40, 40, r, Color::rgba(255, 0, 0, 128))
            .unwrap();
        let px = pixels_once(&b);
        let r = r as i32;
        assert_eq!(bbox(&px), (40 - r, 40 - r, 40 + r, 40 + r), "r={r}");
    }
}

#[test]
fn fill_polygon_default_fills_concave_shapes() {
    // An L shape: the triangle fan from the first vertex used to cover
    // the notch at the top right.
    let mut b = RecordingBackend::new();
    let l = [(0, 0), (10, 0), (10, 20), (30, 20), (30, 30), (0, 30)];
    b.fill_polygon(&l, Color::WHITE).unwrap();
    let px = pixels_once(&b);
    assert!(px.contains(&(5, 5)), "inside the vertical bar");
    assert!(px.contains(&(25, 25)), "inside the foot");
    assert!(!px.contains(&(20, 10)), "the notch stays empty");
}

#[test]
fn fill_arc_default_stays_within_radius() {
    use core::f32::consts::PI;
    // A half disc through angle pi: with the old Taylor cosine the left
    // tip landed at cx - 60 for r = 50.
    let mut b = RecordingBackend::new();
    b.fill_arc(100, 100, 50, 0.0, PI, Color::WHITE).unwrap();
    let px = coverage(&b.calls());
    let min_x = px.keys().map(|p| p.0).min().unwrap();
    assert!((49..=51).contains(&(100 - min_x)), "left extent {min_x}");
}

// -- Default: fill_triangle uses scanline fill --

#[test]
fn fill_triangle_default() {
    let mut b = RecordingBackend::new();
    b.fill_triangle(0, 0, 10, 0, 5, 10, Color::WHITE).unwrap();
    let px = pixels_once(&b);
    // One span per row, y = 0..=10, apex at x = 5.
    assert_eq!(px.iter().map(|p| p.1).min(), Some(0));
    assert_eq!(px.iter().map(|p| p.1).max(), Some(10));
    assert!(px.contains(&(0, 0)) && px.contains(&(10, 0)) && px.contains(&(5, 10)));
}

/// Regression: a triangle whose vertices share one row filled
/// `max - min` pixels (dropping the rightmost vertex) where every other
/// row is inclusive and `SoftwareBuffer` / SDL cover `min..=max`.
#[test]
fn fill_triangle_flat_includes_both_ends() {
    let mut b = RecordingBackend::new();
    b.fill_triangle(5, 3, 0, 3, 10, 3, Color::WHITE).unwrap();
    assert_eq!(b.calls(), vec!["fill_rect(0,3,11,1,255,255,255,255)"]);
}

/// The default `fill_arc` fills one polygon, so a translucent wedge
/// covers every pixel once (a triangle fan covered the spokes twice).
#[test]
fn fill_arc_covers_each_pixel_once() {
    let mut b = RecordingBackend::new();
    b.fill_arc(50, 50, 30, 0.2, 4.0, Color::rgba(255, 0, 0, 128))
        .unwrap();
    let px = pixels_once(&b);
    assert!(px.contains(&(52, 51)), "near the centre");
    assert!(px.len() > 1000);
}

/// Regression: `dash == 0 && gap == 0` looped forever (the cycle length
/// was zero); a zero gap is a solid line and a zero dash draws nothing.
#[test]
fn stroke_line_dashed_zero_dash_or_gap_terminates() {
    let mut b = RecordingBackend::new();
    b.stroke_line_dashed(0, 0, 20, 0, 1, Color::WHITE, 0, 0)
        .unwrap();
    assert_eq!(b.calls(), vec!["fill_rect(0,0,21,1,255,255,255,255)"]);
    b.clear_calls();
    b.stroke_line_dashed(0, 0, 20, 0, 1, Color::WHITE, 0, 5)
        .unwrap();
    assert!(b.calls().is_empty());
}

// -- Gradient defaults --

#[test]
fn gradient_v_defaults_to_fill_rect() {
    let mut b = RecordingBackend::new();
    let grad = GradientStyle::Vertical {
        top: Color::rgb(255, 0, 0),
        bottom: Color::rgb(0, 0, 255),
    };
    b.fill_rect_gradient(0, 0, 100, 50, &grad).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("255,0,0")); // Uses top color
}

#[test]
fn gradient_h_defaults_to_fill_rect() {
    let mut b = RecordingBackend::new();
    let grad = GradientStyle::Horizontal {
        left: Color::rgb(0, 255, 0),
        right: Color::rgb(0, 0, 255),
    };
    b.fill_rect_gradient(0, 0, 100, 50, &grad).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("0,255,0")); // Uses left color
}

#[test]
fn gradient_4_defaults_to_fill_rect() {
    let mut b = RecordingBackend::new();
    let grad = GradientStyle::FourCorner {
        top_left: Color::rgb(10, 20, 30),
        top_right: Color::WHITE,
        bottom_left: Color::WHITE,
        bottom_right: Color::WHITE,
    };
    b.fill_rect_gradient(0, 0, 100, 50, &grad).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("10,20,30")); // Uses top_left
}

#[test]
fn rounded_rect_gradient_default() {
    let mut b = RecordingBackend::new();
    let grad = GradientStyle::Vertical {
        top: Color::rgb(255, 0, 0),
        bottom: Color::BLACK,
    };
    b.fill_rounded_rect_gradient(0, 0, 100, 50, 5, &grad)
        .unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("fill_rect(")); // Falls to fill_rounded_rect -> fill_rect
}

// -- Viewport --

#[test]
fn viewport_size_default_is_psp_native() {
    let b = RecordingBackend::new();
    assert_eq!(b.viewport_size(), (480, 272));
}

// -- Text system defaults --

#[test]
fn measure_text_height_default() {
    let b = RecordingBackend::new();
    // font_size 10 -> 10 * 1.2 = 12
    assert_eq!(b.measure_text_height(10), 12);
    // Below 8 px the bitmap font keeps its 8 px line box, matching every
    // desktop backend.
    assert_eq!(b.measure_text_height(6), 10);
    assert_eq!(b.font_ascent(6), 7);
}

#[test]
fn measure_text_extents_default() {
    let b = RecordingBackend::new();
    let (w, h) = b.measure_text_extents("ABCD", 10);
    assert_eq!(w, 32); // sub-pixel: A(7*10/8=8)+B(8)+C(8)+D(8) = 32
    assert_eq!(h, 12); // 10 * 1.2
}

#[test]
fn font_ascent_default() {
    let b = RecordingBackend::new();
    assert_eq!(b.font_ascent(10), 9); // ceil(10 * 0.85)
}

#[test]
fn draw_text_ellipsis_short_text() {
    let mut b = RecordingBackend::new();
    let drawn = b
        .draw_text_ellipsis("Hi", 0, 0, 8, Color::WHITE, 200)
        .unwrap();
    assert_eq!(drawn, 12); // proportional: H(7)+i(5) = 12
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("Hi"));
}

#[test]
fn draw_text_ellipsis_truncates() {
    let mut b = RecordingBackend::new();
    let long_text = "Hello World This Is Long";
    let drawn = b
        .draw_text_ellipsis(long_text, 0, 0, 8, Color::WHITE, 80)
        .unwrap();
    assert!(drawn <= 80);
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("..."));
}

#[test]
fn draw_text_wrapped_single_line() {
    let mut b = RecordingBackend::new();
    let h = b
        .draw_text_wrapped("Short", 0, 0, 8, Color::WHITE, 200, 10)
        .unwrap();
    assert_eq!(h, 10); // One line of height 10
}

#[test]
fn draw_text_wrapped_wraps_long_line() {
    let mut b = RecordingBackend::new();
    // max_width=40 -> 5 chars fit per line. "Hello World" should wrap.
    let h = b
        .draw_text_wrapped("Hello World", 0, 0, 8, Color::WHITE, 40, 10)
        .unwrap();
    assert_eq!(h, 20); // Two lines
}

#[test]
fn draw_text_wrapped_newlines() {
    let mut b = RecordingBackend::new();
    let h = b
        .draw_text_wrapped("A\nB\nC", 0, 0, 8, Color::WHITE, 200, 10)
        .unwrap();
    assert_eq!(h, 30); // Three lines
}

// -- Texture operation defaults --

#[test]
fn blit_sub_defaults_to_blit() {
    let mut b = RecordingBackend::new();
    let tex = TextureId(5);
    b.blit_sub(tex, 0, 0, 32, 32, 10, 20, 64, 64).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("blit(5,10,20,64,64)"));
}

#[test]
fn blit_tinted_defaults_to_blit() {
    let mut b = RecordingBackend::new();
    let tex = TextureId(3);
    b.blit_tinted(tex, 5, 10, 32, 32, Color::rgb(255, 0, 0))
        .unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("blit(3,5,10,32,32)"));
}

#[test]
fn blit_flipped_defaults_to_blit() {
    let mut b = RecordingBackend::new();
    let tex = TextureId(7);
    b.blit_flipped(tex, 0, 0, 16, 16, true, false).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("blit(7,"));
}

// -- Clip/transform stack defaults --

#[test]
fn push_pop_clip_rect() {
    let mut b = RecordingBackend::new();
    b.push_clip_rect(10, 20, 100, 50).unwrap();
    b.pop_clip_rect().unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].starts_with("set_clip("));
    assert_eq!(calls[1], "reset_clip");
}

#[test]
fn current_clip_rect_default_none() {
    let b = RecordingBackend::new();
    assert!(b.current_clip_rect().is_none());
}

#[test]
fn push_pop_translate_noop() {
    let mut b = RecordingBackend::new();
    b.push_translate(10, 20).unwrap();
    b.pop_translate().unwrap();
    assert!(b.calls().is_empty()); // Default is no-op
}

#[test]
fn current_translate_default_zero() {
    let b = RecordingBackend::new();
    assert_eq!(b.current_translate(), (0, 0));
}

#[test]
fn push_pop_region() {
    let mut b = RecordingBackend::new();
    b.push_region(10, 20, 100, 50).unwrap();
    b.pop_region().unwrap();
    let calls = b.calls();
    // push_region: push_translate (no-op) + push_clip_rect (set_clip)
    // pop_region: pop_clip_rect (reset_clip) + pop_translate (no-op)
    assert!(calls.contains(&"set_clip(0,0,100,50)".to_string()));
    assert!(calls.contains(&"reset_clip".to_string()));
}

// -- stroke_rect edge tests --

#[test]
fn stroke_rect_bottom_edge() {
    let mut b = RecordingBackend::new();
    b.stroke_rect(5, 10, 100, 80, 3, Color::WHITE).unwrap();
    let calls = b.calls();
    // Second call is bottom edge: fill_rect(5, 10+80-3=87, 100, 3)
    assert!(calls[1].starts_with("fill_rect(5,87,100,3,"));
}

#[test]
fn stroke_rect_left_right_edges() {
    let mut b = RecordingBackend::new();
    b.stroke_rect(5, 10, 100, 80, 3, Color::WHITE).unwrap();
    let calls = b.calls();
    // Third call (left edge): fill_rect(5, 13, 3, 74)
    assert!(calls[2].starts_with("fill_rect(5,13,3,74,"));
    // Fourth call (right edge): fill_rect(102, 13, 3, 74)
    assert!(calls[3].starts_with("fill_rect(102,13,3,74,"));
}

#[test]
fn stroke_rect_width_saturates() {
    let mut b = RecordingBackend::new();
    // stroke_width=50 on a 20px tall rect: the edges would overlap, so
    // the whole rect is filled once.
    b.stroke_rect(0, 0, 100, 20, 50, Color::WHITE).unwrap();
    assert_eq!(b.calls(), vec!["fill_rect(0,0,100,20,255,255,255,255)"]);
}

#[test]
fn stroke_rect_1x1() {
    let mut b = RecordingBackend::new();
    b.stroke_rect(0, 0, 1, 1, 1, Color::WHITE).unwrap();
    assert_eq!(b.calls(), vec!["fill_rect(0,0,1,1,255,255,255,255)"]);
}

#[test]
fn stroke_rect_large_stroke_small_rect() {
    let mut b = RecordingBackend::new();
    // stroke_width=10 on a 4x4 rect
    b.stroke_rect(0, 0, 4, 4, 10, Color::WHITE).unwrap();
    assert_eq!(pixels_once(&b).len(), 16);
}

#[test]
fn stroke_rect_never_double_covers() {
    for (w, h, sw) in [
        (10u32, 10u32, 1u16),
        (10, 3, 1),
        (3, 10, 1),
        (9, 9, 4),
        (8, 20, 4),
    ] {
        let mut b = RecordingBackend::new();
        b.stroke_rect(0, 0, w, h, sw, Color::WHITE).unwrap();
        let px = pixels_once(&b);
        assert_eq!(
            bbox(&px),
            (0, 0, w as i32 - 1, h as i32 - 1),
            "{w}x{h} sw{sw}"
        );
    }
    // Zero-sized rects and zero-width strokes draw nothing.
    let mut b = RecordingBackend::new();
    b.stroke_rect(0, 0, 0, 10, 1, Color::WHITE).unwrap();
    b.stroke_rect(0, 0, 10, 10, 0, Color::WHITE).unwrap();
    assert!(b.calls().is_empty());
}

// -- draw_line reversed coords --

#[test]
fn draw_line_reversed_horizontal() {
    let mut b = RecordingBackend::new();
    // x2 < x1 (reversed direction) covers the same pixels.
    b.draw_line(100, 50, 10, 50, 2, Color::WHITE).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("fill_rect(9,49,92,2,"), "{calls:?}");
}

#[test]
fn draw_line_reversed_vertical() {
    let mut b = RecordingBackend::new();
    b.draw_line(50, 80, 50, 10, 3, Color::WHITE).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("fill_rect(49,9,3,73,"), "{calls:?}");
}

// -- draw_text_ellipsis edge cases --

#[test]
fn draw_text_ellipsis_empty_string() {
    let mut b = RecordingBackend::new();
    let drawn = b
        .draw_text_ellipsis("", 0, 0, 8, Color::WHITE, 200)
        .unwrap();
    assert_eq!(drawn, 0);
}

#[test]
fn draw_text_ellipsis_exact_fit() {
    let mut b = RecordingBackend::new();
    // "ABCDE" proportional: A(7)+B(7)+C(7)+D(7)+E(7) = 35px, max_width=35 => no ellipsis
    let drawn = b
        .draw_text_ellipsis("ABCDE", 0, 0, 8, Color::WHITE, 35)
        .unwrap();
    assert_eq!(drawn, 35);
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("ABCDE"));
    assert!(!calls[0].contains("..."));
}

#[test]
fn draw_text_ellipsis_utf8_boundary() {
    let mut b = RecordingBackend::new();
    // Multi-byte UTF-8: 'e' is 2 bytes, so "aeb" = 4 bytes = 32px in mock.
    // With max_width=40, it fits entirely without panicking on char boundaries.
    let drawn = b
        .draw_text_ellipsis("a\u{00e9}b", 0, 0, 8, Color::WHITE, 40)
        .unwrap();
    assert!(drawn <= 40);
    let calls = b.calls();
    assert!(!calls[0].contains("..."));
}

// -- draw_text_wrapped edge cases --

#[test]
fn draw_text_wrapped_empty_lines() {
    let mut b = RecordingBackend::new();
    // "\n\n".split('\n') yields 3 segments: ["", "", ""]
    let h = b
        .draw_text_wrapped("\n\n", 0, 0, 8, Color::WHITE, 200, 10)
        .unwrap();
    // Three empty lines = 30px, no draw_text calls (empty words)
    assert_eq!(h, 30);
    assert_eq!(b.calls().len(), 0);
}

#[test]
fn draw_text_wrapped_long_word_gets_own_line() {
    let mut b = RecordingBackend::new();
    // max_width=24 (3 chars). "ABCDEFGH" is 64px, won't fit on any line
    // but should still be drawn on its own line.
    let h = b
        .draw_text_wrapped("ABCDEFGH", 0, 0, 8, Color::WHITE, 24, 10)
        .unwrap();
    assert_eq!(h, 10); // Still one line (the word gets its own line)
    assert!(!b.calls().is_empty());
}

#[test]
fn draw_text_wrapped_y_positions() {
    let mut b = RecordingBackend::new();
    let _ = b
        .draw_text_wrapped("A\nB\nC", 5, 10, 8, Color::WHITE, 200, 15)
        .unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 3);
    // First line at y=10, second at y=25, third at y=40
    assert!(calls[0].contains(",5,10,"));
    assert!(calls[1].contains(",5,25,"));
    assert!(calls[2].contains(",5,40,"));
}

#[test]
fn draw_text_wrapped_zero_line_height_uses_default() {
    let mut b = RecordingBackend::new();
    // line_height=0 should fall back to measure_text_height
    let h = b
        .draw_text_wrapped("A\nB", 0, 0, 10, Color::WHITE, 200, 0)
        .unwrap();
    // Default line height for font_size 10 = 12
    assert_eq!(h, 24);
}

// -- blit_sub_tinted --

#[test]
fn blit_sub_tinted_defaults_to_blit_sub() {
    let mut b = RecordingBackend::new();
    let tex = TextureId(9);
    b.blit_sub_tinted(tex, 0, 0, 16, 16, 10, 20, 32, 32, Color::rgb(255, 0, 0))
        .unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    // Should ultimately delegate to blit
    assert!(calls[0].starts_with("blit(9,10,20,32,32)"));
}

// -- push_region / pop_region --

#[test]
fn push_region_translates_and_clips() {
    let mut b = RecordingBackend::new();
    b.push_region(50, 60, 200, 150).unwrap();
    let calls = b.calls();
    // push_translate is no-op, push_clip_rect calls set_clip
    assert!(calls.contains(&"set_clip(0,0,200,150)".to_string()));
}

// -- AudioTrackId --

#[test]
fn audio_track_id_equality() {
    assert_eq!(AudioTrackId(1), AudioTrackId(1));
    assert_ne!(AudioTrackId(1), AudioTrackId(2));
}

// -- BackendErrExt tests --

#[test]
fn backend_err_ext_converts_string_error() {
    let res: std::result::Result<(), String> = Err("oops".to_string());
    let err = res.backend_err().unwrap_err();
    assert!(format!("{err}").contains("oops"));
}

#[test]
fn backend_err_ext_ok_passes_through() {
    let res: std::result::Result<i32, String> = Ok(42);
    assert_eq!(res.backend_err().unwrap(), 42);
}

#[test]
fn backend_err_ext_io_error() {
    let res: std::result::Result<(), std::io::Error> =
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
    let err = res.backend_err().unwrap_err();
    assert!(format!("{err}").contains("gone"));
}

#[test]
fn texture_not_found_message() {
    let err = texture_not_found(42);
    assert_eq!(format!("{err}"), "backend error: texture not found: 42");
    assert!(matches!(
        err,
        crate::error::OasisError::Backend(crate::error::BackendError::TextureNotFound { id: 42 })
    ));
}

// -- validate_rgba_data tests --

#[test]
fn validate_rgba_data_correct_size() {
    let data = vec![0u8; 4 * 4 * 4]; // 4x4 RGBA
    assert!(validate_rgba_data(4, 4, &data).is_ok());
}

#[test]
fn validate_rgba_data_too_small() {
    let data = vec![0u8; 10];
    let err = validate_rgba_data(4, 4, &data).unwrap_err();
    assert!(format!("{err}").contains("size mismatch"));
}

#[test]
fn validate_rgba_data_too_large() {
    let data = vec![0u8; 100];
    let err = validate_rgba_data(4, 4, &data).unwrap_err();
    assert!(format!("{err}").contains("size mismatch"));
}

#[test]
fn validate_rgba_data_zero_dimensions() {
    assert!(validate_rgba_data(0, 0, &[]).is_ok());
}

#[test]
fn validate_rgba_data_overflow_dimensions() {
    let err = validate_rgba_data(u32::MAX, u32::MAX, &[]).unwrap_err();
    assert!(format!("{err}").contains("overflow"));
}

#[test]
fn validate_rgba_data_1x1() {
    let data = vec![255, 0, 0, 255]; // 1x1 red pixel
    assert!(validate_rgba_data(1, 1, &data).is_ok());
}

// -----------------------------------------------------------------------
// PSP backend integration tests: trait object safety & delegation
// -----------------------------------------------------------------------

/// Verify `SdiCore` is object-safe: a mock can be used as `&dyn SdiCore`.
#[test]
fn sdi_core_is_object_safe() {
    let mut backend = RecordingBackend::new();
    let core: &mut dyn SdiCore = &mut backend;
    core.clear(Color::BLACK).unwrap();
    core.fill_rect(0, 0, 10, 10, Color::WHITE).unwrap();
    core.draw_text("test", 0, 0, 8, Color::WHITE).unwrap();
    assert_eq!(core.measure_text("AB", 8), backend.measure_text("AB", 8));
}

/// Verify `SdiBackend` is object-safe: a mock can be used as `&dyn SdiBackend`.
#[test]
fn sdi_backend_is_object_safe() {
    let mut backend = RecordingBackend::new();
    let b: &mut dyn SdiBackend = &mut backend;
    // Call a mix of SdiCore and SdiBackend default methods through the
    // trait object to prove they dispatch correctly.
    b.clear(Color::BLACK).unwrap();
    b.fill_rounded_rect(0, 0, 50, 50, 5, Color::WHITE).unwrap();
    b.stroke_rect(0, 0, 20, 20, 1, Color::WHITE).unwrap();
    b.draw_line(0, 0, 10, 10, 1, Color::WHITE).unwrap();
    b.fill_circle(25, 25, 5, Color::WHITE).unwrap();
    b.fill_rect_gradient(
        0,
        0,
        10,
        10,
        &GradientStyle::Vertical {
            top: Color::BLACK,
            bottom: Color::WHITE,
        },
    )
    .unwrap();
    b.push_clip_rect(0, 0, 100, 100).unwrap();
    b.pop_clip_rect().unwrap();
    assert_eq!(b.viewport_size(), (480, 272));
    assert_eq!(b.current_translate(), (0, 0));
    assert!(b.current_clip_rect().is_none());
}

/// A minimal `SdiCore`-only impl (no `SdiBackend`) can be used as
/// `&dyn SdiCore`, proving that PSP-like backends only need 13 methods.
#[test]
fn minimal_sdi_core_impl_is_sufficient() {
    struct MinimalCore;
    impl SdiCore for MinimalCore {
        fn init(&mut self, _w: u32, _h: u32) -> crate::error::Result<()> {
            Ok(())
        }
        fn clear(&mut self, _c: Color) -> crate::error::Result<()> {
            Ok(())
        }
        fn blit(
            &mut self,
            _t: TextureId,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn fill_rect(
            &mut self,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
            _c: Color,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn draw_text(
            &mut self,
            _t: &str,
            _x: i32,
            _y: i32,
            _fs: u16,
            _c: Color,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn swap_buffers(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _d: &[u8]) -> crate::error::Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> crate::error::Result<()> {
            Ok(())
        }
        fn set_clip_rect(
            &mut self,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
        fn measure_text(&self, _t: &str, _fs: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<Vec<u8>> {
            Ok(vec![])
        }
        fn shutdown(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
    }

    // Can be used as &dyn SdiCore (13 methods only).
    let mut m = MinimalCore;
    let core: &mut dyn SdiCore = &mut m;
    core.init(480, 272).unwrap();
    core.clear(Color::BLACK).unwrap();
    core.fill_rect(0, 0, 10, 10, Color::WHITE).unwrap();
    core.draw_text("hi", 0, 0, 8, Color::WHITE).unwrap();
    core.swap_buffers().unwrap();
    let tex = core.load_texture(1, 1, &[0, 0, 0, 255]).unwrap();
    core.blit(tex, 0, 0, 1, 1).unwrap();
    core.destroy_texture(tex).unwrap();
    core.set_clip_rect(0, 0, 100, 100).unwrap();
    core.reset_clip_rect().unwrap();
    assert_eq!(core.measure_text("x", 8), 0);
    core.read_pixels(0, 0, 1, 1).unwrap();
    core.shutdown().unwrap();
}

/// Adding an empty `impl SdiBackend` gives all 30+ default methods.
#[test]
fn empty_sdi_backend_impl_provides_defaults() {
    struct DefaultBackend {
        fill_rect_count: std::cell::Cell<u32>,
    }
    impl SdiCore for DefaultBackend {
        fn init(&mut self, _w: u32, _h: u32) -> crate::error::Result<()> {
            Ok(())
        }
        fn clear(&mut self, _c: Color) -> crate::error::Result<()> {
            Ok(())
        }
        fn blit(
            &mut self,
            _t: TextureId,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn fill_rect(
            &mut self,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
            _c: Color,
        ) -> crate::error::Result<()> {
            self.fill_rect_count.set(self.fill_rect_count.get() + 1);
            Ok(())
        }
        fn draw_text(
            &mut self,
            _t: &str,
            _x: i32,
            _y: i32,
            _fs: u16,
            _c: Color,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn swap_buffers(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _d: &[u8]) -> crate::error::Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> crate::error::Result<()> {
            Ok(())
        }
        fn set_clip_rect(
            &mut self,
            _x: i32,
            _y: i32,
            _w: u32,
            _h: u32,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
        fn measure_text(&self, t: &str, fs: u16) -> u32 {
            bitmap_measure_text(t, fs)
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<Vec<u8>> {
            Ok(vec![])
        }
        fn shutdown(&mut self) -> crate::error::Result<()> {
            Ok(())
        }
    }
    impl SdiShapes for DefaultBackend {}
    impl SdiGradients for DefaultBackend {}
    impl SdiText for DefaultBackend {}
    impl SdiTextures for DefaultBackend {}
    impl SdiClipTransform for DefaultBackend {}
    impl SdiVector for DefaultBackend {}
    impl SdiBatch for DefaultBackend {}
    // Blanket impl gives SdiBackend for free!

    let mut b = DefaultBackend {
        fill_rect_count: std::cell::Cell::new(0),
    };

    // fill_rounded_rect emits the corner rows plus the merged body.
    b.fill_rounded_rect(0, 0, 50, 50, 10, Color::WHITE).unwrap();
    assert!(b.fill_rect_count.get() > 1);

    // stroke_rect should call fill_rect 4 times.
    b.fill_rect_count.set(0);
    b.stroke_rect(0, 0, 100, 100, 2, Color::WHITE).unwrap();
    assert_eq!(b.fill_rect_count.get(), 4);

    // fill_circle should call fill_rect multiple times (scanlines).
    b.fill_rect_count.set(0);
    b.fill_circle(50, 50, 5, Color::WHITE).unwrap();
    assert!(b.fill_rect_count.get() > 1);

    // Viewport defaults to PSP resolution.
    assert_eq!(b.viewport_size(), (480, 272));
}

// -----------------------------------------------------------------------
// Bresenham line algorithm correctness
// -----------------------------------------------------------------------

#[test]
fn draw_line_bresenham_45_degree_plots_all_points() {
    let mut b = RecordingBackend::new();
    b.draw_line(0, 0, 4, 4, 1, Color::WHITE).unwrap();
    let calls = b.calls();
    // 45-degree line: one pixel at each (0,0), (1,1), (2,2), (3,3), (4,4)
    assert_eq!(calls.len(), 5);
    for (i, call) in calls.iter().enumerate() {
        assert!(
            call.starts_with(&format!("fill_rect({i},{i},1,1,")),
            "expected pixel at ({i},{i}), got: {call}"
        );
    }
}

#[test]
fn draw_line_bresenham_negative_slope() {
    let mut b = RecordingBackend::new();
    b.draw_line(4, 4, 0, 0, 1, Color::WHITE).unwrap();
    // Same pixels as the forward diagonal.
    let expected: std::collections::BTreeSet<_> = (0..5).map(|i| (i, i)).collect();
    assert_eq!(pixels_once(&b), expected);
}

#[test]
fn draw_line_bresenham_steep_line() {
    // Steep line: more vertical than horizontal (dx=1, dy=4)
    let mut b = RecordingBackend::new();
    b.draw_line(0, 0, 1, 4, 1, Color::WHITE).unwrap();
    let px = pixels_once(&b);
    // One pixel per row (y from 0 to 4 inclusive), endpoints included.
    assert_eq!(px.len(), 5);
    assert!(px.contains(&(0, 0)) && px.contains(&(1, 4)));
}

#[test]
fn draw_line_bresenham_shallow_line() {
    // Shallow line: more horizontal than vertical (dx=4, dy=1)
    let mut b = RecordingBackend::new();
    b.draw_line(0, 0, 4, 1, 1, Color::WHITE).unwrap();
    let px = pixels_once(&b);
    // One pixel per column (x from 0 to 4 inclusive).
    assert_eq!(px.len(), 5);
    assert!(px.contains(&(0, 0)) && px.contains(&(4, 1)));
}

#[test]
fn draw_line_bresenham_single_point() {
    // Diagonal from (5,5) to (5,5) -- single point.
    // This is actually horizontal (y1==y2), handled by the fast path.
    let mut b = RecordingBackend::new();
    b.draw_line(5, 5, 5, 5, 1, Color::WHITE).unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
}

/// Spans outside the bound surface are never generated: a line reaching
/// a billion pixels off-screen draws only its visible rows, quickly.
#[test]
fn draw_line_default_generates_only_visible_spans() {
    let mut b = RecordingBackend::new();
    let start = std::time::Instant::now();
    b.draw_line(-1_000_000_000, 10, 1_000_000_000, 10, 3, Color::WHITE)
        .unwrap();
    b.draw_line(20, -1_000_000_000, 20, 1_000_000_000, 1, Color::WHITE)
        .unwrap();
    b.fill_triangle(
        0,
        -2_000_000_000,
        2_000_000_000,
        0,
        0,
        2_000_000_000,
        Color::WHITE,
    )
    .unwrap();
    b.fill_polygon(
        &[
            (-2_000_000_000, -2_000_000_000),
            (2_000_000_000, 0),
            (0, 2_000_000_000),
        ],
        Color::WHITE,
    )
    .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    let (vw, vh) = b.viewport_size();
    let cov = coverage(&b.calls());
    assert!(!cov.is_empty());
    for &(x, y) in cov.keys() {
        assert!((0..vw as i32).contains(&x) && (0..vh as i32).contains(&y));
    }
}

#[test]
fn draw_line_bresenham_width_scales_pixels() {
    let mut b = RecordingBackend::new();
    b.draw_line(5, 5, 7, 7, 3, Color::WHITE).unwrap();
    // The union of a centred 3x3 brush at (5,5), (6,6), (7,7), each pixel
    // covered once.
    let mut expected = std::collections::BTreeSet::new();
    for i in 5..8 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                expected.insert((i + dx, i + dy));
            }
        }
    }
    assert_eq!(pixels_once(&b), expected);
}

// -----------------------------------------------------------------------
// fill_rounded_rect fallback verification
// -----------------------------------------------------------------------

#[test]
fn fill_rounded_rect_zero_radius_is_fill_rect() {
    let mut b = RecordingBackend::new();
    b.fill_rounded_rect(10, 20, 100, 50, 0, Color::rgb(1, 2, 3))
        .unwrap();
    let calls = b.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "fill_rect(10,20,100,50,1,2,3,255)");
}

#[test]
fn fill_rounded_rect_large_radius_is_clamped() {
    let mut b = RecordingBackend::new();
    // radius (999) >> half of min dimension (25): clamped to 25.
    b.fill_rounded_rect(0, 0, 100, 50, 999, Color::WHITE)
        .unwrap();
    let px = pixels_once(&b);
    assert_eq!(bbox(&px), (0, 0, 99, 49));
    assert!(!px.contains(&(5, 5)), "a 25 px radius cuts deep corners");
    assert!(px.contains(&(50, 25)));
}

#[test]
fn fill_rounded_rect_preserves_exact_color() {
    let mut b = RecordingBackend::new();
    let color = Color::rgba(12, 34, 56, 78);
    b.fill_rounded_rect(0, 0, 10, 10, 3, color).unwrap();
    for call in b.calls() {
        assert!(call.ends_with(",12,34,56,78)"), "{call}");
    }
}

// -----------------------------------------------------------------------
// Text metrics coherence
// -----------------------------------------------------------------------

#[test]
fn text_ascent_less_than_height() {
    let b = RecordingBackend::new();
    for fs in [1, 5, 8, 10, 12, 16, 20, 32, 64] {
        let (ascent, height) = (b.font_ascent(fs), b.measure_text_height(fs));
        assert!(
            ascent < height,
            "ascent ({ascent}) must be < height ({height}) for font_size {fs}"
        );
    }
}

#[test]
fn text_extents_match_width_and_height() {
    let b = RecordingBackend::new();
    let (w, h) = b.measure_text_extents("ABCD", 12);
    assert_eq!(w, b.measure_text("ABCD", 12));
    assert_eq!(h, b.measure_text_height(12));
}

// -----------------------------------------------------------------------
// SdiRenderTarget defaults
// -----------------------------------------------------------------------

impl SdiRenderTarget for RecordingBackend {}

#[test]
fn render_target_default_unsupported() {
    let mut b = RecordingBackend::new();
    assert!(!b.supports_render_targets());
    assert!(!b.supports_render_target_readback());
    assert!(b.create_render_target(64, 64).is_err());
    assert!(b.bind_render_target(RenderTargetId(0)).is_err());
    assert!(b.unbind_render_target().is_err());
    assert!(
        b.composite_render_target(RenderTargetId(0), 0, 0, 64, 64, BlendMode::Normal, 1.0,)
            .is_err()
    );
    let mut buf = vec![0u8; 64 * 64 * 4];
    assert!(b.read_render_target(RenderTargetId(0), &mut buf).is_err());
    // destroy_render_target is the *only* method that defaults to Ok —
    // backends that opted out never created any to destroy.
    assert!(b.destroy_render_target(RenderTargetId(0)).is_ok());
}

/// Capability-supporting test backend.  Records the bind/composite
/// stack so we can assert ordering on the compositor's behalf in
/// later PRs.  This is the foundational fixture for PR2 (the
/// test-backend implementation) and is duplicated here so the
/// types crate alone can verify trait wiring.
struct RtRecordingBackend {
    next_id: u64,
    targets: Vec<(RenderTargetId, u32, u32)>,
    bind_stack: Vec<RenderTargetId>,
    log: Vec<String>,
}

impl RtRecordingBackend {
    fn new() -> Self {
        Self {
            next_id: 1,
            targets: Vec::new(),
            bind_stack: Vec::new(),
            log: Vec::new(),
        }
    }
    fn log(&self) -> &[String] {
        &self.log
    }
}

impl SdiCore for RtRecordingBackend {
    fn init(&mut self, _w: u32, _h: u32) -> crate::error::Result<()> {
        Ok(())
    }
    fn clear(&mut self, _color: Color) -> crate::error::Result<()> {
        Ok(())
    }
    fn blit(
        &mut self,
        _tex: TextureId,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn fill_rect(
        &mut self,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
        _color: Color,
    ) -> crate::error::Result<()> {
        self.log.push("fill_rect".into());
        Ok(())
    }
    fn draw_text(
        &mut self,
        _text: &str,
        _x: i32,
        _y: i32,
        _font_size: u16,
        _color: Color,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn swap_buffers(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
    fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> crate::error::Result<TextureId> {
        Ok(TextureId(0))
    }
    fn destroy_texture(&mut self, _tex: TextureId) -> crate::error::Result<()> {
        Ok(())
    }
    fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<()> {
        Ok(())
    }
    fn reset_clip_rect(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        bitmap_measure_text(text, font_size)
    }
    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<Vec<u8>> {
        Ok(Vec::new())
    }
    fn shutdown(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
}

impl SdiShapes for RtRecordingBackend {}
impl SdiGradients for RtRecordingBackend {}
impl SdiText for RtRecordingBackend {}
impl SdiTextures for RtRecordingBackend {}
impl SdiClipTransform for RtRecordingBackend {}
impl SdiVector for RtRecordingBackend {}
impl SdiBatch for RtRecordingBackend {}

impl SdiRenderTarget for RtRecordingBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> crate::error::Result<RenderTargetId> {
        let id = RenderTargetId(self.next_id);
        self.next_id += 1;
        self.targets.push((id, w, h));
        self.log.push(format!("create({},{w}x{h})", id.0));
        Ok(id)
    }
    fn bind_render_target(&mut self, id: RenderTargetId) -> crate::error::Result<()> {
        if !self.targets.iter().any(|(t, ..)| *t == id) {
            return Err(crate::error::OasisError::Backend(
                format!("unknown rt {}", id.0).into(),
            ));
        }
        self.bind_stack.push(id);
        self.log.push(format!("bind({})", id.0));
        Ok(())
    }
    fn unbind_render_target(&mut self) -> crate::error::Result<()> {
        let popped = self
            .bind_stack
            .pop()
            .ok_or_else(|| crate::error::OasisError::Backend("unbind underflow".into()))?;
        self.log.push(format!("unbind({})", popped.0));
        Ok(())
    }
    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        _dst_x: i32,
        _dst_y: i32,
        _dst_w: u32,
        _dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    ) -> crate::error::Result<()> {
        if !self.targets.iter().any(|(t, ..)| *t == id) {
            return Err(crate::error::OasisError::Backend(
                format!("unknown rt {}", id.0).into(),
            ));
        }
        debug_assert!(
            (0.0..=1.0).contains(&opacity),
            "opacity must be in [0.0, 1.0], got {opacity}"
        );
        self.log
            .push(format!("composite({},{blend:?},{opacity})", id.0));
        Ok(())
    }
    fn destroy_render_target(&mut self, id: RenderTargetId) -> crate::error::Result<()> {
        if !self.targets.iter().any(|(t, ..)| *t == id) {
            return Err(crate::error::OasisError::Backend(
                format!("double-destroy rt {}", id.0).into(),
            ));
        }
        self.targets.retain(|(t, ..)| *t != id);
        self.log.push(format!("destroy({})", id.0));
        Ok(())
    }
    fn supports_render_targets(&self) -> bool {
        true
    }
}

#[test]
fn render_target_basic_lifecycle() {
    let mut b = RtRecordingBackend::new();
    let id = b.create_render_target(64, 32).unwrap();
    b.bind_render_target(id).unwrap();
    b.fill_rect(0, 0, 64, 32, Color::WHITE).unwrap();
    b.unbind_render_target().unwrap();
    b.composite_render_target(id, 10, 20, 64, 32, BlendMode::Multiply, 0.5)
        .unwrap();
    b.destroy_render_target(id).unwrap();

    assert_eq!(
        b.log(),
        vec![
            "create(1,64x32)",
            "bind(1)",
            "fill_rect",
            "unbind(1)",
            "composite(1,Multiply,0.5)",
            "destroy(1)",
        ],
    );
}

#[test]
fn render_target_nested_bind_stack() {
    let mut b = RtRecordingBackend::new();
    let outer = b.create_render_target(128, 128).unwrap();
    let inner = b.create_render_target(32, 32).unwrap();

    // Outer layer drawn into outer; inner layer nested inside it.
    b.bind_render_target(outer).unwrap();
    b.fill_rect(0, 0, 128, 128, Color::BLACK).unwrap();
    b.bind_render_target(inner).unwrap();
    b.fill_rect(0, 0, 32, 32, Color::WHITE).unwrap();
    b.unbind_render_target().unwrap();
    // Composite inner back into outer (still bound).
    b.composite_render_target(inner, 4, 4, 32, 32, BlendMode::Screen, 1.0)
        .unwrap();
    b.unbind_render_target().unwrap();
    // Composite outer back into the framebuffer.
    b.composite_render_target(outer, 0, 0, 128, 128, BlendMode::Normal, 1.0)
        .unwrap();

    // Stack must be empty at the end.
    assert!(b.bind_stack.is_empty());
    // Inner must be unbound before outer.
    let log = b.log();
    let inner_unbind = log.iter().position(|s| s == "unbind(2)").unwrap();
    let outer_unbind = log.iter().position(|s| s == "unbind(1)").unwrap();
    assert!(
        inner_unbind < outer_unbind,
        "inner must unbind before outer: {log:?}"
    );
}

#[test]
fn render_target_unbind_underflow_errors() {
    let mut b = RtRecordingBackend::new();
    assert!(b.unbind_render_target().is_err());
}

#[test]
fn blend_mode_default_is_normal() {
    assert_eq!(BlendMode::default(), BlendMode::Normal);
    assert!(BlendMode::Normal.is_normal());
    assert!(!BlendMode::Multiply.is_normal());
}

// -----------------------------------------------------------------------
// Comprehensive PR1 validation
// -----------------------------------------------------------------------
//
// The tests below validate the trait surface from the angles a real
// browser compositor will hit it: object safety, generic bounds,
// exhaustive blend-mode coverage, default-Err message stability,
// multi-level nesting, RenderTargetId handle semantics, and the
// canonical record/replay sequence the recorder will emit in PR3.

/// Single source of truth for the 16 CSS-aligned blend modes.
/// Used by several tests; if anyone adds a 17th variant they only
/// need to update this constant and the exhaustive `match` below.
const ALL_BLEND_MODES: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

/// Compile-time exhaustiveness check: if a variant is ever added
/// to or removed from `BlendMode`, this function fails to build.
/// Catches the case where ALL_BLEND_MODES drifts from the enum.
#[allow(dead_code)]
fn blend_mode_exhaustive_match(m: BlendMode) -> u8 {
    match m {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

// -- BlendMode value semantics ----------------------------------------

#[test]
fn blend_mode_all_16_distinct() {
    use std::collections::HashSet;
    let set: HashSet<_> = ALL_BLEND_MODES.iter().copied().collect();
    assert_eq!(set.len(), 16, "all 16 blend modes must be distinct");
}

#[test]
fn blend_mode_only_normal_is_normal() {
    for mode in ALL_BLEND_MODES {
        assert_eq!(
            mode.is_normal(),
            mode == BlendMode::Normal,
            "is_normal() should be true only for Normal, got {mode:?}"
        );
    }
}

#[test]
fn blend_mode_copy_eq_hash() {
    let a = BlendMode::Multiply;
    let b = a; // Copy
    assert_eq!(a, b);
    let mut set = std::collections::HashSet::new();
    for mode in ALL_BLEND_MODES {
        set.insert(mode);
    }
    assert_eq!(set.len(), 16);
    // Inserting an existing one is a no-op.
    set.insert(BlendMode::Multiply);
    assert_eq!(set.len(), 16);
}

#[test]
fn blend_mode_debug_format_distinct() {
    // Each variant must produce a distinct Debug string so logs and
    // operation traces don't collapse two modes together.
    let mut seen = std::collections::HashSet::new();
    for mode in ALL_BLEND_MODES {
        let s = format!("{mode:?}");
        assert!(
            seen.insert(s.clone()),
            "Debug for {mode:?} collides with another variant"
        );
        assert!(!s.is_empty());
    }
}

// -- RenderTargetId handle semantics ----------------------------------

#[test]
fn render_target_id_copy_eq_hash() {
    let a = RenderTargetId(42);
    let b = a;
    assert_eq!(a, b);
    assert_eq!(RenderTargetId(1), RenderTargetId(1));
    assert_ne!(RenderTargetId(1), RenderTargetId(2));

    let mut set = std::collections::HashSet::new();
    set.insert(RenderTargetId(7));
    set.insert(RenderTargetId(8));
    set.insert(RenderTargetId(7));
    assert_eq!(set.len(), 2);
}

#[test]
fn render_target_id_debug_includes_value() {
    let s = format!("{:?}", RenderTargetId(99));
    assert!(s.contains("99"), "Debug should expose inner u64: {s}");
}

// -- Default-Err contract --------------------------------------------

/// A backend that implements only `SdiCore` and picks up every extension
/// default through [`impl_sdi_defaults!`](crate::impl_sdi_defaults).
/// Validates that every `SdiRenderTarget` method has a working default
/// impl and that the macro produces an `SdiBackend`.
struct UntouchedBackend;

impl SdiCore for UntouchedBackend {
    fn init(&mut self, _w: u32, _h: u32) -> crate::error::Result<()> {
        Ok(())
    }
    fn clear(&mut self, _color: Color) -> crate::error::Result<()> {
        Ok(())
    }
    fn blit(
        &mut self,
        _tex: TextureId,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn fill_rect(
        &mut self,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
        _color: Color,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn draw_text(
        &mut self,
        _text: &str,
        _x: i32,
        _y: i32,
        _font_size: u16,
        _color: Color,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn swap_buffers(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
    fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> crate::error::Result<TextureId> {
        Ok(TextureId(0))
    }
    fn destroy_texture(&mut self, _tex: TextureId) -> crate::error::Result<()> {
        Ok(())
    }
    fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<()> {
        Ok(())
    }
    fn reset_clip_rect(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
    fn measure_text(&self, t: &str, fs: u16) -> u32 {
        bitmap_measure_text(t, fs)
    }
    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> crate::error::Result<Vec<u8>> {
        Ok(Vec::new())
    }
    fn shutdown(&mut self) -> crate::error::Result<()> {
        Ok(())
    }
}

crate::impl_sdi_defaults!(UntouchedBackend);

#[test]
fn untouched_backend_satisfies_sdi_backend() {
    // Compile-time assertion: `impl_sdi_defaults!` plus `SdiCore` is
    // enough for the `SdiBackend` blanket impl.
    fn assert_sdi_backend<T: SdiBackend>() {}
    assert_sdi_backend::<UntouchedBackend>();
}

#[test]
fn untouched_backend_default_render_target_errors() {
    let mut b = UntouchedBackend;
    // Capability flags default to false.
    assert!(!b.supports_render_targets());
    assert!(!b.supports_render_target_readback());

    // All write methods default to Err.
    let create_err = b.create_render_target(64, 64).unwrap_err();
    let msg = format!("{create_err}");
    assert!(
        msg.contains("not supported"),
        "default error message should mention 'not supported': {msg}"
    );
    assert!(
        matches!(
            create_err,
            crate::error::OasisError::Backend(crate::error::BackendError::Unsupported { .. })
        ),
        "defaults return the typed Unsupported variant: {create_err:?}"
    );

    assert!(b.bind_render_target(RenderTargetId(0)).is_err());
    assert!(b.unbind_render_target().is_err());
    assert!(
        b.composite_render_target(RenderTargetId(0), 0, 0, 64, 64, BlendMode::Normal, 1.0,)
            .is_err()
    );
    let mut buf = vec![0u8; 64 * 64 * 4];
    let read_err = b
        .read_render_target(RenderTargetId(0), &mut buf)
        .unwrap_err();
    assert!(
        format!("{read_err}").contains("not supported"),
        "read error message should mention 'not supported'"
    );

    // destroy_render_target is the lone exception — defaults to Ok.
    assert!(b.destroy_render_target(RenderTargetId(0)).is_ok());
}

// -- Object safety ----------------------------------------------------

#[test]
fn sdi_render_target_is_object_safe() {
    // If SdiRenderTarget were not object-safe (e.g. due to a
    // generic method or a Self return type), this would not
    // compile.  The compositor's per-frame target pool stores
    // backend handles behind &mut dyn SdiRenderTarget, so this is
    // a load-bearing requirement.
    let mut b = RtRecordingBackend::new();
    let dyn_b: &mut dyn SdiRenderTarget = &mut b;
    assert!(dyn_b.supports_render_targets());
    let id = dyn_b.create_render_target(8, 8).unwrap();
    dyn_b.bind_render_target(id).unwrap();
    dyn_b.unbind_render_target().unwrap();
    dyn_b.destroy_render_target(id).unwrap();
}

// -- Compositor-style generic bound ----------------------------------

/// The compositor in PR3 will be generic over this combined bound.
/// Asserting it compiles here freezes the public API contract.
fn compositor_round_trip<B>(backend: &mut B) -> crate::error::Result<()>
where
    B: SdiBackend + SdiRenderTarget,
{
    let id = backend.create_render_target(16, 16)?;
    backend.bind_render_target(id)?;
    backend.fill_rect(0, 0, 16, 16, Color::WHITE)?;
    backend.unbind_render_target()?;
    backend.composite_render_target(id, 0, 0, 16, 16, BlendMode::Multiply, 0.75)?;
    backend.destroy_render_target(id)?;
    Ok(())
}

#[test]
fn compositor_generic_bound_compiles_and_runs() {
    let mut b = RtRecordingBackend::new();
    compositor_round_trip(&mut b).unwrap();
}

// -- Multi-level nesting ---------------------------------------------

#[test]
fn render_target_three_level_nesting() {
    let mut b = RtRecordingBackend::new();
    let l0 = b.create_render_target(256, 256).unwrap();
    let l1 = b.create_render_target(128, 128).unwrap();
    let l2 = b.create_render_target(64, 64).unwrap();

    b.bind_render_target(l0).unwrap();
    b.fill_rect(0, 0, 256, 256, Color::BLACK).unwrap();

    b.bind_render_target(l1).unwrap();
    b.fill_rect(0, 0, 128, 128, Color::rgb(255, 0, 0)).unwrap();

    b.bind_render_target(l2).unwrap();
    b.fill_rect(0, 0, 64, 64, Color::rgb(0, 255, 0)).unwrap();
    b.unbind_render_target().unwrap(); // pop l2

    b.composite_render_target(l2, 0, 0, 64, 64, BlendMode::Screen, 1.0)
        .unwrap();
    b.unbind_render_target().unwrap(); // pop l1

    b.composite_render_target(l1, 0, 0, 128, 128, BlendMode::Multiply, 0.5)
        .unwrap();
    b.unbind_render_target().unwrap(); // pop l0

    b.composite_render_target(l0, 0, 0, 256, 256, BlendMode::Normal, 1.0)
        .unwrap();

    // Stack must be drained.
    assert!(b.bind_stack.is_empty(), "bind stack should be empty");

    // Verify pop ordering: each unbind comes after the matching bind
    // and before any earlier bind's unbind.
    let log = b.log();
    let pos = |needle: &str| log.iter().position(|s| s == needle).unwrap();
    assert!(pos("bind(1)") < pos("bind(2)"));
    assert!(pos("bind(2)") < pos("bind(3)"));
    assert!(pos("bind(3)") < pos("unbind(3)"));
    assert!(pos("unbind(3)") < pos("unbind(2)"));
    assert!(pos("unbind(2)") < pos("unbind(1)"));

    // Cleanup
    for id in [l2, l1, l0] {
        b.destroy_render_target(id).unwrap();
    }
    assert!(b.targets.is_empty(), "all targets destroyed");
}

#[test]
fn render_target_destroy_drops_from_pool() {
    let mut b = RtRecordingBackend::new();
    let a = b.create_render_target(8, 8).unwrap();
    let c = b.create_render_target(16, 16).unwrap();
    assert_eq!(b.targets.len(), 2);
    b.destroy_render_target(a).unwrap();
    assert_eq!(b.targets.len(), 1);
    // The remaining one is still bindable.
    b.bind_render_target(c).unwrap();
    b.unbind_render_target().unwrap();
    b.destroy_render_target(c).unwrap();
    assert!(b.targets.is_empty());
}

#[test]
fn render_target_bind_unknown_id_errors() {
    let mut b = RtRecordingBackend::new();
    // The backend is fully capable but the id was never created.
    let bogus = RenderTargetId(99_999);
    assert!(b.bind_render_target(bogus).is_err());
}

#[test]
fn render_target_supports_flags_are_idempotent() {
    // Flag queries must not depend on internal state — calling
    // them in any order, before or after operations, should yield
    // the same answer.
    let mut b = RtRecordingBackend::new();
    let initial = b.supports_render_targets();
    let initial_rb = b.supports_render_target_readback();
    let id = b.create_render_target(4, 4).unwrap();
    assert_eq!(b.supports_render_targets(), initial);
    assert_eq!(b.supports_render_target_readback(), initial_rb);
    b.destroy_render_target(id).unwrap();
    assert_eq!(b.supports_render_targets(), initial);
    assert_eq!(b.supports_render_target_readback(), initial_rb);
}

#[test]
fn render_target_canonical_recorder_sequence() {
    // Replicates the exact event sequence the PR3 display-list
    // recorder will emit for a single PushCompositingLayer /
    // PopCompositingLayer pair around two FillRects.  Locks the
    // operation order so PR3 can write its assertion against the
    // same trace shape.
    let mut b = RtRecordingBackend::new();
    let id = b.create_render_target(100, 80).unwrap();

    b.bind_render_target(id).unwrap();
    b.fill_rect(0, 0, 100, 80, Color::rgb(20, 20, 20)).unwrap();
    b.fill_rect(10, 10, 80, 60, Color::rgb(200, 50, 50))
        .unwrap();
    b.unbind_render_target().unwrap();

    b.composite_render_target(id, 50, 30, 100, 80, BlendMode::Overlay, 0.85)
        .unwrap();
    b.destroy_render_target(id).unwrap();

    assert_eq!(
        b.log(),
        vec![
            "create(1,100x80)",
            "bind(1)",
            "fill_rect",
            "fill_rect",
            "unbind(1)",
            "composite(1,Overlay,0.85)",
            "destroy(1)",
        ]
    );
}

#[test]
fn render_target_unbind_underflow_message() {
    // The error path the recorder will hit if it ever emits a
    // PopCompositingLayer without a matching push — must produce a
    // diagnosable message rather than panic.
    let mut b = RtRecordingBackend::new();
    let err = b.unbind_render_target().unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("underflow") || msg.contains("unbind"),
        "underflow error should be diagnosable: {msg}"
    );
}

#[test]
fn render_target_id_zero_is_valid_handle() {
    // The trait makes no claim that 0 is reserved.  A backend that
    // chooses to start id allocation at 0 must work — verify by
    // round-tripping the constructor.
    let id = RenderTargetId(0);
    assert_eq!(id.0, 0);
    let copied = id;
    assert_eq!(copied, id);
}

#[test]
fn render_target_pool_isolation_between_backends() {
    // Two independent backend instances must not share state.  The
    // earlier version of this test only checked that id sequences
    // start at 1 and that unknown ids error — but identical id
    // values are exactly what *shared* state would produce too, so
    // that wasn't a real isolation proof.
    //
    // This version drives the two backends through divergent
    // histories and asserts observable state on each afterward:
    //
    // - backend A creates 3 targets, destroys the middle one, then
    //   binds + unbinds the first.  Ends up with 2 live targets,
    //   empty bind stack, specific log trace.
    // - backend C creates 1 target, binds it and leaves it bound.
    //   Ends up with 1 live target, bind stack depth 1, different
    //   log trace.
    //
    // If the pools shared any state, at least one of these
    // assertions would collapse.
    let mut a = RtRecordingBackend::new();
    let mut c = RtRecordingBackend::new();

    let a1 = a.create_render_target(4, 4).unwrap();
    let a2 = a.create_render_target(8, 8).unwrap();
    let a3 = a.create_render_target(16, 16).unwrap();
    a.destroy_render_target(a2).unwrap();
    a.bind_render_target(a1).unwrap();
    a.unbind_render_target().unwrap();

    let c1 = c.create_render_target(32, 32).unwrap();
    c.bind_render_target(c1).unwrap();

    // Id sequences are independent.
    assert_eq!(a1.0, 1);
    assert_eq!(a2.0, 2);
    assert_eq!(a3.0, 3);
    assert_eq!(c1.0, 1);

    // Live-target pools diverged.
    assert_eq!(a.targets.len(), 2, "A should have 2 live targets (a1, a3)");
    assert!(a.targets.iter().any(|(t, ..)| *t == a1));
    assert!(a.targets.iter().any(|(t, ..)| *t == a3));
    assert!(!a.targets.iter().any(|(t, ..)| *t == a2));
    assert_eq!(c.targets.len(), 1, "C should have 1 live target (c1)");

    // Bind stacks diverged.
    assert!(a.bind_stack.is_empty(), "A's bind stack should be empty");
    assert_eq!(c.bind_stack.len(), 1, "C should have c1 still bound");
    assert_eq!(c.bind_stack[0], c1);

    // Logs diverged — A saw create+destroy+bind+unbind, C only saw
    // create+bind.
    assert_eq!(
        a.log(),
        vec![
            "create(1,4x4)",
            "create(2,8x8)",
            "create(3,16x16)",
            "destroy(2)",
            "bind(1)",
            "unbind(1)",
        ],
    );
    assert_eq!(c.log(), vec!["create(1,32x32)", "bind(1)"]);

    // Cross-pool id lookups still fail: c1 (id 1) is unknown to A
    // *relative to the other ids A still owns* — but because both
    // pools happen to use the u64 value 1, A would actually accept
    // it if it did a membership check.  That's the limitation the
    // earlier version of this test glossed over.  The real proof
    // of isolation is the divergent state above, not the handle
    // values.

    // Clean up C so the test leaves no dangling bind.
    c.unbind_render_target().unwrap();
    c.destroy_render_target(c1).unwrap();
}

// -- SdiBackend bounds for trait objects -----------------------------

#[test]
fn dyn_sdi_render_target_with_lifetime_bounds() {
    // Doc-test analog: ensure boxing works.  The compositor's
    // per-frame target pool will hold owned backend handles in a
    // Box<dyn ...> when running multiple backends side-by-side
    // (e.g. desktop SDL + offscreen capture for tests).
    let b: Box<dyn SdiRenderTarget> = Box::new(RtRecordingBackend::new());
    assert!(b.supports_render_targets());
}

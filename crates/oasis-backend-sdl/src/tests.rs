//! Unit tests for the SDL3 backend (input mapping, shapes, text, render
//! targets) that need no SDL window, or a headless one.

use super::*;

use oasis_types::backend::{BlendMode, SdiRenderTarget};

use oasis_types::backend::SdiCore;
use sdl3::keyboard::Keycode;

use oasis_types::backend::SdiText;
use oasis_types::input::{Button, InputEvent, Trigger};

// ---------------------------------------------------------------
// Input mapping tests
// ---------------------------------------------------------------

#[test]
fn key_down_arrow_keys() {
    assert_eq!(
        map_key_down(Keycode::Up),
        Some(InputEvent::ButtonPress(Button::Up))
    );
    assert_eq!(
        map_key_down(Keycode::Down),
        Some(InputEvent::ButtonPress(Button::Down))
    );
    assert_eq!(
        map_key_down(Keycode::Left),
        Some(InputEvent::ButtonPress(Button::Left))
    );
    assert_eq!(
        map_key_down(Keycode::Right),
        Some(InputEvent::ButtonPress(Button::Right))
    );
}

#[test]
fn key_down_action_keys() {
    assert_eq!(
        map_key_down(Keycode::Return),
        Some(InputEvent::ButtonPress(Button::Confirm))
    );
    assert_eq!(
        map_key_down(Keycode::Escape),
        Some(InputEvent::ButtonPress(Button::Cancel))
    );
    assert_eq!(
        map_key_down(Keycode::Space),
        Some(InputEvent::ButtonPress(Button::Triangle))
    );
    // Tab is handled in map_sdl_event (with Shift detection).
    assert_eq!(map_key_down(Keycode::Tab), None);
}

#[test]
fn key_down_function_keys() {
    assert_eq!(
        map_key_down(Keycode::F1),
        Some(InputEvent::ButtonPress(Button::Start))
    );
    assert_eq!(
        map_key_down(Keycode::F2),
        Some(InputEvent::ButtonPress(Button::Select))
    );
}

#[test]
fn key_down_triggers() {
    assert_eq!(
        map_key_down(Keycode::Q),
        Some(InputEvent::TriggerPress(Trigger::Left))
    );
    assert_eq!(
        map_key_down(Keycode::E),
        Some(InputEvent::TriggerPress(Trigger::Right))
    );
}

#[test]
fn key_down_backspace() {
    assert_eq!(
        map_key_down(Keycode::Backspace),
        Some(InputEvent::Backspace)
    );
}

#[test]
fn key_down_f11_toggle_fullscreen() {
    assert_eq!(
        map_key_down(Keycode::F11),
        Some(InputEvent::ToggleFullscreen)
    );
}

#[test]
fn key_down_unmapped_returns_none() {
    assert_eq!(map_key_down(Keycode::A), None);
    assert_eq!(map_key_down(Keycode::Z), None);
    assert_eq!(map_key_down(Keycode::_0), None);
    assert_eq!(map_key_down(Keycode::F5), None);
}

#[test]
fn key_up_arrow_keys() {
    assert_eq!(
        map_key_up(Keycode::Up),
        Some(InputEvent::ButtonRelease(Button::Up))
    );
    assert_eq!(
        map_key_up(Keycode::Down),
        Some(InputEvent::ButtonRelease(Button::Down))
    );
    assert_eq!(
        map_key_up(Keycode::Left),
        Some(InputEvent::ButtonRelease(Button::Left))
    );
    assert_eq!(
        map_key_up(Keycode::Right),
        Some(InputEvent::ButtonRelease(Button::Right))
    );
}

#[test]
fn key_up_action_keys() {
    assert_eq!(
        map_key_up(Keycode::Return),
        Some(InputEvent::ButtonRelease(Button::Confirm))
    );
    assert_eq!(
        map_key_up(Keycode::Escape),
        Some(InputEvent::ButtonRelease(Button::Cancel))
    );
    assert_eq!(
        map_key_up(Keycode::Space),
        Some(InputEvent::ButtonRelease(Button::Triangle))
    );
    // Tab is handled in map_sdl_event (with Shift detection).
    assert_eq!(map_key_up(Keycode::Tab), None);
}

#[test]
fn key_up_triggers() {
    assert_eq!(
        map_key_up(Keycode::Q),
        Some(InputEvent::TriggerRelease(Trigger::Left))
    );
    assert_eq!(
        map_key_up(Keycode::E),
        Some(InputEvent::TriggerRelease(Trigger::Right))
    );
}

#[test]
fn key_up_unmapped_returns_none() {
    assert_eq!(map_key_up(Keycode::A), None);
    assert_eq!(map_key_up(Keycode::Backspace), None);
}

#[test]
fn key_down_up_symmetry() {
    // Every mapped key_down should have a corresponding key_up
    // (except Backspace and F11).
    // Tab is handled in map_sdl_event (with Shift detection).
    let keys = [
        Keycode::Up,
        Keycode::Down,
        Keycode::Left,
        Keycode::Right,
        Keycode::Return,
        Keycode::Escape,
        Keycode::Space,
        Keycode::F1,
        Keycode::F2,
        Keycode::Q,
        Keycode::E,
    ];
    for key in keys {
        let down = map_key_down(key);
        let up = map_key_up(key);
        assert!(down.is_some(), "key_down({key:?}) should be mapped");
        assert!(up.is_some(), "key_up({key:?}) should be mapped");
        // Verify press/release correspondence.
        match (down.unwrap(), up.unwrap()) {
            (InputEvent::ButtonPress(b1), InputEvent::ButtonRelease(b2)) => {
                assert_eq!(b1, b2, "press/release mismatch {key:?}");
            },
            (InputEvent::TriggerPress(t1), InputEvent::TriggerRelease(t2)) => {
                assert_eq!(t1, t2, "press/release mismatch {key:?}");
            },
            (d, u) => panic!("unexpected pair {key:?}: {d:?}/{u:?}"),
        }
    }
}

#[test]
fn key_up_backspace_not_mapped() {
    // Backspace has no key_up mapping (fire-and-forget).
    assert_eq!(map_key_up(Keycode::Backspace), None);
}

#[test]
fn key_up_f11_not_mapped() {
    // F11 (ToggleFullscreen) has no key_up mapping.
    assert_eq!(map_key_up(Keycode::F11), None);
}

#[test]
fn key_down_f1_f2_mapped_but_other_fn_keys_not() {
    assert!(map_key_down(Keycode::F1).is_some());
    assert!(map_key_down(Keycode::F2).is_some());
    assert!(map_key_down(Keycode::F3).is_none());
    assert!(map_key_down(Keycode::F4).is_none());
    assert!(map_key_down(Keycode::F5).is_none());
    assert!(map_key_down(Keycode::F6).is_none());
    assert!(map_key_down(Keycode::F7).is_none());
    assert!(map_key_down(Keycode::F8).is_none());
    assert!(map_key_down(Keycode::F9).is_none());
    assert!(map_key_down(Keycode::F10).is_none());
    assert!(map_key_down(Keycode::F12).is_none());
}

#[test]
fn key_down_letter_keys_only_q_e_mapped() {
    // Q and E are trigger keys; no other letters are mapped.
    assert!(map_key_down(Keycode::Q).is_some());
    assert!(map_key_down(Keycode::E).is_some());
    assert!(map_key_down(Keycode::A).is_none());
    assert!(map_key_down(Keycode::W).is_none());
    assert!(map_key_down(Keycode::S).is_none());
    assert!(map_key_down(Keycode::D).is_none());
}

#[test]
fn mouse_coordinate_truncation() {
    // SDL3 mouse coords are f32; truncation to i32 floors.
    let fx: f32 = 42.7;
    let fy: f32 = 99.9;
    assert_eq!(fx as i32, 42);
    assert_eq!(fy as i32, 99);

    // Negative coords truncate toward zero.
    let neg_x: f32 = -1.5;
    assert_eq!(neg_x as i32, -1);
}

#[test]
fn frect_conversion() {
    let r = frect(10, 20, 100, 50);
    assert_eq!(r.x, 10.0);
    assert_eq!(r.y, 20.0);
    assert_eq!(r.w, 100.0);
    assert_eq!(r.h, 50.0);
}

#[test]
fn frect_zero_size() {
    let r = frect(0, 0, 0, 0);
    assert_eq!(r.x, 0.0);
    assert_eq!(r.w, 0.0);
}

// ---------------------------------------------------------------
// Helper function tests
// ---------------------------------------------------------------

#[test]
fn isqrt_known_values() {
    assert_eq!(isqrt_i32(0), 0);
    assert_eq!(isqrt_i32(1), 1);
    assert_eq!(isqrt_i32(4), 2);
    assert_eq!(isqrt_i32(9), 3);
    assert_eq!(isqrt_i32(16), 4);
    assert_eq!(isqrt_i32(25), 5);
    assert_eq!(isqrt_i32(100), 10);
}

#[test]
fn isqrt_known_non_perfect_squares() {
    assert_eq!(isqrt_i32(2), 1);
    assert_eq!(isqrt_i32(3), 1);
    assert_eq!(isqrt_i32(5), 2);
    assert_eq!(isqrt_i32(8), 2);
    assert_eq!(isqrt_i32(10), 3);
    assert_eq!(isqrt_i32(99), 9);
}

#[test]
fn isqrt_negative() {
    assert_eq!(isqrt_i32(-1), 0);
    assert_eq!(isqrt_i32(-100), 0);
}

#[test]
fn edge_x_horizontal_line() {
    // Horizontal line: y0 == y1, should return x0.
    assert_eq!(edge_x(10, 5, 20, 5, 5), 10);
}

#[test]
fn edge_x_vertical_line() {
    // Vertical line from (10,0) to (10,100).
    assert_eq!(edge_x(10, 0, 10, 100, 50), 10);
}

#[test]
fn edge_x_diagonal() {
    // Line from (0,0) to (100,100): at y=50, x should be 50.
    assert_eq!(edge_x(0, 0, 100, 100, 50), 50);
}

#[test]
fn edge_x_endpoints() {
    // At y=y0, should return x0.
    assert_eq!(edge_x(10, 20, 50, 80, 20), 10);
    // At y=y1, should return x1.
    assert_eq!(edge_x(10, 20, 50, 80, 80), 50);
}

// ---------------------------------------------------------------
// SDL rendering correctness tests (require display)
//
// These tests require a working SDL3 display. In CI, set
// SDL_VIDEO_DRIVER=dummy (or x11/wayland) and run:
//   cargo test -p oasis-backend-sdl -- --ignored
//
// Locally with a display, they can be run directly.
// ---------------------------------------------------------------

fn try_create_backend() -> Option<SdlBackend> {
    SdlBackend::new("test", 64, 64).ok()
}

#[test]
#[ignore]
fn render_clear_red() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    let red = Color::rgb(255, 0, 0);
    backend.clear(red).unwrap();
    backend.swap_buffers().unwrap();

    let pixels = backend.read_pixels(0, 0, 64, 64).unwrap();
    // read_pixels returns RGBA bytes, 4 per pixel, regardless of the
    // renderer's native format.
    assert_eq!(pixels.len(), 64 * 64 * 4);
    assert_eq!(
        &pixels[..4],
        &[255, 0, 0, 255],
        "pixel (0,0) must be RGBA red"
    );
}

/// RGBA pattern where every pixel of a `w` x `h` image is distinct,
/// so a row-stride mismatch shows up as a mismatch rather than
/// happening to reproduce the same colours.
fn stride_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x * 6) as u8, (y * 40) as u8, 200, 255]);
        }
    }
    px
}

/// Texture uploads must honour the locked texture's row pitch.
/// Hardware renderers (Direct3D 11/12, SDL GPU, Metal) commonly pad
/// rows to 64- or 256-byte boundaries, so a width whose row is not a
/// multiple of that (here 37 px = 148 bytes) turned every uploaded
/// image into diagonal stripes when the pitch was ignored. The
/// software renderer packs rows tightly, so run this under a
/// hardware `SDL_RENDER_DRIVER` to exercise the padded path.
#[test]
#[ignore]
fn render_texture_odd_width_honours_pitch() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    let (w, h) = (37u32, 6u32);
    let pattern = stride_pattern(w, h);

    let tex = backend.load_texture(w, h, &pattern).unwrap();
    backend.clear(Color::BLACK).unwrap();
    backend.blit(tex, 0, 0, w, h).unwrap();
    assert_eq!(backend.read_pixels(0, 0, w, h).unwrap(), pattern);

    // update_texture takes the same lock path.
    let mut flipped = pattern.clone();
    for px in flipped.as_chunks_mut::<4>().0 {
        px[2] = 40;
    }
    backend.update_texture(tex, w, h, &flipped).unwrap();
    backend.clear(Color::BLACK).unwrap();
    backend.blit(tex, 0, 0, w, h).unwrap();
    assert_eq!(backend.read_pixels(0, 0, w, h).unwrap(), flipped);
}

#[test]
#[ignore]
fn render_target_composites_under_active_clip() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    // A browser window's content clip, away from the origin.
    backend.set_clip_rect(20, 20, 40, 40).unwrap();
    let rt = backend.create_render_target(13, 7).unwrap();
    backend.bind_render_target(rt).unwrap();
    backend
        .fill_rect(0, 0, 13, 7, Color::rgb(0, 200, 0))
        .unwrap();
    backend.unbind_render_target().unwrap();
    backend
        .composite_render_target(rt, 30, 30, 13, 7, BlendMode::Normal, 1.0)
        .unwrap();
    backend.reset_clip_rect().unwrap();
    let px = backend.read_pixels(30, 30, 13, 7).unwrap();
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        assert_eq!(p, &[0, 200, 0, 255], "pixel {i} of the composited layer");
    }
}

/// `mix-blend-mode: multiply` layers are bigger than their painted
/// content; the transparent remainder must not darken the backdrop.
#[test]
#[ignore]
fn render_target_multiply_keeps_transparent_pixels() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::rgb(200, 200, 200)).unwrap();
    let rt = backend.create_render_target(12, 12).unwrap();
    backend.bind_render_target(rt).unwrap();
    backend
        .fill_rect(4, 4, 4, 4, Color::rgb(255, 128, 0))
        .unwrap();
    backend.unbind_render_target().unwrap();
    backend
        .composite_render_target(rt, 10, 10, 12, 12, BlendMode::Multiply, 1.0)
        .unwrap();
    // Transparent corner: backdrop unchanged.
    assert_eq!(
        &backend.read_pixels(10, 10, 1, 1).unwrap()[..],
        &[200, 200, 200, 255]
    );
    // Painted center: backdrop * source.
    let c = backend.read_pixels(15, 15, 1, 1).unwrap();
    assert!(
        c[0] >= 198 && (99..=101).contains(&c[1]) && c[2] == 0,
        "{c:?}"
    );
}

#[test]
#[ignore]
fn render_target_starts_transparent() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    // Churn some textures so a recycled allocation would carry junk.
    for _ in 0..4 {
        let rt = backend.create_render_target(13, 7).unwrap();
        backend.bind_render_target(rt).unwrap();
        backend
            .fill_rect(0, 0, 13, 7, Color::rgb(255, 128, 0))
            .unwrap();
        backend.unbind_render_target().unwrap();
        backend
            .composite_render_target(rt, 0, 0, 13, 7, BlendMode::Normal, 1.0)
            .unwrap();
        backend.destroy_render_target(rt).unwrap();
        backend.swap_buffers().unwrap();
    }
    backend.clear(Color::rgb(0, 0, 90)).unwrap();
    let rt = backend.create_render_target(13, 7).unwrap();
    // Draw nothing into it: compositing must leave the frame as-is.
    backend.bind_render_target(rt).unwrap();
    backend.unbind_render_target().unwrap();
    backend
        .composite_render_target(rt, 10, 10, 13, 7, BlendMode::Normal, 1.0)
        .unwrap();
    let px = backend.read_pixels(10, 10, 13, 7).unwrap();
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        assert_eq!(p, &[0, 0, 90, 255], "pixel {i} under an empty layer");
    }
}

#[test]
#[ignore]
fn render_fill_rect_at_position() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    // Clear to black, then fill a small rect with white.
    backend.clear(Color::BLACK).unwrap();
    backend.fill_rect(10, 10, 4, 4, Color::WHITE).unwrap();
    backend.swap_buffers().unwrap();

    // Read the filled area.
    let pixels = backend.read_pixels(10, 10, 4, 4).unwrap();
    assert_eq!(pixels.len(), 4 * 4 * 4);
    // All pixels in the rect should be non-zero (white).
    let all_nonzero = pixels.chunks(4).all(|px| px[0] > 200 || px[2] > 200);
    assert!(all_nonzero, "filled rect should have white pixels");

    // Read an area outside the rect (should be black).
    let outside = backend.read_pixels(0, 0, 4, 4).unwrap();
    let all_dark = outside
        .chunks(4)
        .all(|px| px[0] < 50 && px[1] < 50 && px[2] < 50);
    assert!(all_dark, "area outside rect should be black");
}

#[test]
#[ignore]
fn render_texture_load_and_blit() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    // Create a 2x2 red texture.
    let rgba = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let tex = backend.load_texture(2, 2, &rgba).unwrap();
    backend.clear(Color::BLACK).unwrap();
    backend.blit(tex, 0, 0, 2, 2).unwrap();
    backend.swap_buffers().unwrap();

    let pixels = backend.read_pixels(0, 0, 2, 2).unwrap();
    // Should have non-black pixels where the texture was blitted.
    let has_color = pixels.iter().any(|&b| b > 50);
    assert!(has_color, "blitted texture should produce colored pixels");

    backend.destroy_texture(tex).unwrap();
}

#[test]
#[ignore]
fn render_draw_text_produces_pixels() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    backend.draw_text("A", 0, 0, 8, Color::WHITE).unwrap();
    backend.swap_buffers().unwrap();

    let pixels = backend.read_pixels(0, 0, 8, 8).unwrap();
    // The letter 'A' should have some white pixels.
    let white_count = pixels
        .chunks(4)
        .filter(|px| px[0] > 200 || px[2] > 200)
        .count();
    assert!(
        white_count > 5,
        "letter 'A' should have visible pixels, got {white_count}"
    );
}

#[test]
#[ignore]
fn render_measure_text_width() {
    let backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    // At font_size=8, each char is 8px wide.
    assert_eq!(backend.measure_text("A", 8), 8);
    assert_eq!(backend.measure_text("AB", 8), 16);
    assert_eq!(backend.measure_text("Hello", 8), 40);
    assert_eq!(backend.measure_text("", 8), 0);

    // At font_size=16, each char is 16px wide (scale=2).
    assert_eq!(backend.measure_text("A", 16), 16);
}

#[test]
#[ignore]
fn render_clip_rect_restricts_drawing() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    // Set clip to top-left 16x16.
    backend.set_clip_rect(0, 0, 16, 16).unwrap();
    // Fill entire 64x64 with white -- only 16x16 should be affected.
    backend.fill_rect(0, 0, 64, 64, Color::WHITE).unwrap();
    backend.reset_clip_rect().unwrap();
    backend.swap_buffers().unwrap();

    // Inside clip: should be white.
    let inside = backend.read_pixels(0, 0, 16, 16).unwrap();
    let inside_white = inside
        .chunks(4)
        .filter(|px| px[0] > 200 || px[2] > 200)
        .count();
    assert!(inside_white > 200, "inside clip should be mostly white");

    // Outside clip: should be black.
    let outside = backend.read_pixels(32, 32, 16, 16).unwrap();
    let outside_dark = outside
        .chunks(4)
        .all(|px| px[0] < 50 && px[1] < 50 && px[2] < 50);
    assert!(outside_dark, "outside clip should remain black");
}

// ---------------------------------------------------------------
// Glyph cache tests
// ---------------------------------------------------------------

#[test]
fn glyph_cache_key_roundtrip() {
    // Verify that GlyphCacheKey packing produces distinct keys
    // for different parameters and equal keys for the same
    // parameters.
    let c = Color::rgba(255, 128, 64, 200);
    let k1 = GlyphCacheKey::new('A', 16, c, false, false);
    let k2 = GlyphCacheKey::new('A', 16, c, false, false);
    assert_eq!(k1, k2);
    assert_eq!(k1.raw(), k2.raw());

    // Different character produces a different key.
    let k3 = GlyphCacheKey::new('B', 16, c, false, false);
    assert_ne!(k1, k3);

    // Different font size produces a different key.
    let k4 = GlyphCacheKey::new('A', 24, c, false, false);
    assert_ne!(k1, k4);

    // Bold flag produces a different key.
    let k5 = GlyphCacheKey::new('A', 16, c, true, false);
    assert_ne!(k1, k5);

    // Italic flag produces a different key.
    let k6 = GlyphCacheKey::new('A', 16, c, false, true);
    assert_ne!(k1, k6);
}

#[test]
fn glyph_cache_fields_initialized() {
    // Verify that glyph cache fields exist and are properly
    // initialized (empty maps and zero counter). This test
    // can only run when an SDL display is available.
    let backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    assert!(backend.glyph_cache.is_empty());
    assert_eq!(backend.glyph_access_counter, 0);
}

#[test]
#[ignore]
fn glyph_cache_populates_on_draw() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    assert!(backend.glyph_cache.is_empty());
    backend
        .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
        .unwrap();
    // Two distinct characters should create two cache entries.
    assert_eq!(backend.glyph_cache.len(), 2);

    // Drawing the same text again should not increase cache
    // size (cache hits).
    backend
        .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
        .unwrap();
    assert_eq!(backend.glyph_cache.len(), 2);
}

#[test]
#[ignore]
fn glyph_cache_is_color_independent() {
    // Glyphs are rasterized white and tinted at blit time, so drawing the
    // same character in a dozen colors (themes, hovers, fades) must reuse
    // one cache entry — the whole point of D2.
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    for i in 0..12u8 {
        let color = Color::rgb(i * 20, 255 - i * 20, 128);
        backend
            .draw_text_styled("A", 0, 0, 16, color, false, false)
            .unwrap();
    }
    assert_eq!(backend.glyph_cache.len(), 1);

    // Style variants still get their own entries.
    backend
        .draw_text_styled("A", 0, 0, 16, Color::WHITE, true, false)
        .unwrap();
    backend
        .draw_text_styled("A", 0, 0, 24, Color::WHITE, false, false)
        .unwrap();
    assert_eq!(backend.glyph_cache.len(), 3);
}

#[test]
#[ignore]
fn glyph_cache_entry_carries_dimensions() {
    // The draw path reads dimensions from the entry instead of calling
    // Texture::query() per glyph per frame (D6).
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend
        .draw_text_styled("A", 0, 0, 16, Color::WHITE, false, false)
        .unwrap();
    let entry = *backend
        .glyph_cache
        .values()
        .next()
        .expect("one glyph cached");
    assert_eq!(entry.h, 16);
    assert!(entry.w > 0);
    let texture = backend.textures.get(&entry.texture).expect("glyph texture");
    let query = texture.query();
    assert_eq!((entry.w, entry.h), (query.width, query.height));
}

/// Regression: a degenerate (zero-width or zero-height) clip rect
/// must reject every subsequent draw. Before the fix, the SDL3 Rust
/// wrapper's `Rect::new` silently clamped a 0-dimension to 1, so a
/// `set_clip_rect(.., 0)` produced a 1-pixel slot that let the top
/// row of glyphs/fills through — the source of the dotted-underline
/// leak below the OASIS browser window on old.reddit.com. The
/// browser intersects nested clip rects during display-list replay;
/// when a child rect's bottom is at or above the parent's top the
/// intersection collapses to height 0, and the OASIS clip needs to
/// reject every subsequent draw rather than admit a 1-pixel sliver.
#[test]
#[ignore]
fn degenerate_clip_rejects_all_draws() {
    use oasis_types::backend::SdiCore;
    let backend_opt = SdlBackend::new("clip-degen", 256, 256).ok();
    let mut backend = match backend_opt {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::rgb(255, 0, 0)).unwrap();
    // Zero-height clip — must clip everything.
    backend.set_clip_rect(0, 100, 256, 0).unwrap();
    backend
        .fill_rect(0, 100, 256, 5, Color::rgb(0, 255, 0))
        .unwrap();
    let pixels = backend.read_pixels(50, 100, 1, 1).unwrap();
    assert_eq!(
        pixels[0], 255,
        "zero-height clip should reject all fills, got {pixels:?}",
    );
}

// ---------------------------------------------------------------
// Skin TTF font tests
// ---------------------------------------------------------------

/// The oasis-browser web-font test fixture: a tiny but valid TTF.
const TEST_TTF: &[u8] = include_bytes!("../../oasis-browser/test_data/minimal.ttf");

#[test]
fn set_font_installs_and_clears_ttf_state() {
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    assert!(backend.ttf_font.is_none());

    // Garbage bytes are rejected; the bitmap font stays active.
    backend.set_font(Some(b"not a font"));
    assert!(backend.ttf_font.is_none());

    backend.set_font(Some(TEST_TTF));
    assert!(backend.ttf_font.is_some());

    backend.set_font(None);
    assert!(backend.ttf_font.is_none());
}

#[test]
#[ignore]
fn set_font_evicts_glyph_cache_textures() {
    use oasis_types::backend::SdiCore;
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.clear(Color::BLACK).unwrap();
    backend
        .draw_text_styled("AB", 0, 0, 16, Color::WHITE, false, false)
        .unwrap();
    assert_eq!(backend.glyph_cache.len(), 2);
    let glyph_textures: Vec<u64> = backend.glyph_cache.values().map(|e| e.texture).collect();

    backend.set_font(Some(TEST_TTF));
    assert!(backend.glyph_cache.is_empty());
    for id in glyph_textures {
        assert!(
            !backend.textures.contains_key(&id),
            "stale glyph texture {id} survived the font swap"
        );
    }
}

#[test]
fn ttf_measure_matches_per_char_advances() {
    use oasis_types::backend::SdiCore;
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    // Bitmap path is untouched without a font.
    assert_eq!(
        backend.measure_text("Hello", 16),
        oasis_types::backend::bitmap_measure_text("Hello", 16)
    );

    backend.set_font(Some(TEST_TTF));
    let ttf = backend.ttf_font.as_ref().expect("font installed");
    // Whatever glyphs the fixture has, measurement must equal the same
    // per-character integer advances the draw loop accumulates — with
    // bitmap fallback for characters the font lacks.
    let text = "Hello ▲▼ world";
    let expected: u32 = text
        .chars()
        .map(|ch| {
            if ttf.has_glyph(ch) {
                ttf.advance(ch, 16.0).max(0) as u32
            } else {
                oasis_types::bitmap_font::glyph_advance_scaled(ch, 16)
            }
        })
        .sum();
    assert_eq!(backend.measure_text(text, 16), expected);
}

#[test]
#[ignore]
fn ttf_draw_advances_agree_with_measure() {
    use oasis_types::backend::SdiCore;
    let mut backend = match try_create_backend() {
        Some(b) => b,
        None => return,
    };
    backend.set_font(Some(TEST_TTF));
    backend.clear(Color::BLACK).unwrap();
    let text = "AV.";
    backend
        .draw_text_styled(text, 0, 0, 16, Color::WHITE, false, false)
        .unwrap();
    // Sum the advances of the entries the draw created, in draw order.
    let drawn: i32 = text
        .chars()
        .map(|ch| backend.glyph_cache[&GlyphCacheKey::colorless(ch, 16, false, false)].advance)
        .sum();
    assert_eq!(drawn.max(0) as u32, backend.measure_text(text, 16));
}

//! Paint unit tests (tools, undo / redo, file I/O, clicks, menus).

use super::*;
use canvas::alpha_blend;
use oasis_app_core::AsAny;
use oasis_app_core::testing::click_at;
use oasis_vfs::MemoryVfs;

fn make_vfs() -> MemoryVfs {
    MemoryVfs::new()
}

// -- Canvas creation --

#[test]
fn canvas_new_dimensions() {
    let c = Canvas::new(64, 48);
    assert_eq!(c.width(), 64);
    assert_eq!(c.height(), 48);
}

#[test]
fn canvas_new_all_white() {
    let c = Canvas::new(4, 4);
    let white = Color::rgb(255, 255, 255);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(c.get_pixel(x, y), white);
        }
    }
}

#[test]
fn canvas_new_one_layer() {
    let c = Canvas::new(8, 8);
    assert_eq!(c.layer_count(), 1);
    assert_eq!(c.active_layer(), 0);
}

// -- Pixel get/set --

#[test]
fn canvas_set_get_pixel() {
    let mut c = Canvas::new(8, 8);
    let red = Color::rgb(255, 0, 0);
    c.set_pixel(3, 4, red);
    assert_eq!(c.get_pixel(3, 4), red);
}

#[test]
fn canvas_get_pixel_out_of_bounds() {
    let c = Canvas::new(4, 4);
    assert_eq!(c.get_pixel(10, 10), Color::rgba(0, 0, 0, 0));
}

#[test]
fn canvas_set_pixel_out_of_bounds_noop() {
    let mut c = Canvas::new(4, 4);
    c.set_pixel(100, 100, Color::rgb(255, 0, 0));
    // Should not panic or corrupt data.
    assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 255, 255));
}

// -- Canvas fill/clear --

#[test]
fn canvas_fill() {
    let mut c = Canvas::new(4, 4);
    let blue = Color::rgb(0, 0, 255);
    c.fill(blue);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(c.get_pixel(x, y), blue);
        }
    }
}

#[test]
fn canvas_clear_makes_transparent() {
    let mut c = Canvas::new(4, 4);
    c.clear();
    // Transparent over nothing = transparent.
    let px = c.get_pixel(0, 0);
    assert_eq!(px.a, 0);
}

// -- Layer operations --

#[test]
fn canvas_add_layer() {
    let mut c = Canvas::new(4, 4);
    let idx = c.add_layer("Layer 1");
    assert_eq!(idx, 1);
    assert_eq!(c.layer_count(), 2);
}

#[test]
fn canvas_set_active_layer() {
    let mut c = Canvas::new(4, 4);
    c.add_layer("Top");
    c.set_active_layer(1);
    assert_eq!(c.active_layer(), 1);
}

#[test]
fn canvas_set_active_layer_invalid() {
    let mut c = Canvas::new(4, 4);
    c.set_active_layer(99);
    assert_eq!(c.active_layer(), 0);
}

#[test]
fn canvas_layer_name() {
    let c = Canvas::new(4, 4);
    assert_eq!(c.layer_name(0), "Background");
    assert_eq!(c.layer_name(99), "");
}

#[test]
fn canvas_toggle_layer_visibility() {
    let mut c = Canvas::new(4, 4);
    // Fill background red.
    c.fill(Color::rgb(255, 0, 0));
    // Toggle off.
    c.toggle_layer_visibility(0);
    // Pixel should be transparent when only layer is hidden.
    assert_eq!(c.get_pixel(0, 0).a, 0);
    // Toggle back on.
    c.toggle_layer_visibility(0);
    assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 0, 0));
}

#[test]
fn canvas_flatten_two_layers() {
    let mut c = Canvas::new(2, 2);
    // Background is white.
    c.add_layer("Top");
    c.set_active_layer(1);
    // Draw an opaque red pixel on top layer.
    c.set_pixel(0, 0, Color::rgb(255, 0, 0));
    // That pixel should be red (opaque overrides).
    assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 0, 0));
    // Others should be white (from background).
    assert_eq!(c.get_pixel(1, 0), Color::rgb(255, 255, 255));
}

#[test]
fn canvas_flatten_hidden_layer_ignored() {
    let mut c = Canvas::new(2, 2);
    c.add_layer("Top");
    c.set_active_layer(1);
    c.set_pixel(0, 0, Color::rgb(255, 0, 0));
    c.toggle_layer_visibility(1);
    // Red pixel from hidden layer should not show.
    assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 255, 255));
}

// -- Drawing primitives --

#[test]
fn draw_pixel_basic() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16];
    let red = Color::rgb(255, 0, 0);
    draw_pixel(&mut buf, 4, 2, 3, red);
    assert_eq!(buf[3 * 4 + 2], red);
}

#[test]
fn draw_pixel_negative_coords_noop() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16];
    draw_pixel(&mut buf, 4, -1, -1, Color::rgb(255, 0, 0));
    // No change expected.
    assert_eq!(buf[0], Color::rgb(0, 0, 0));
}

#[test]
fn draw_pixel_out_of_bounds_noop() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16];
    draw_pixel(&mut buf, 4, 10, 10, Color::rgb(255, 0, 0));
    assert_eq!(buf[0], Color::rgb(0, 0, 0));
}

#[test]
fn draw_line_horizontal() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let red = Color::rgb(255, 0, 0);
    draw_line(&mut buf, 8, 8, 1, 3, 5, 3, red, 1);
    for x in 1..=5 {
        assert_eq!(buf[3 * 8 + x], red, "pixel at ({x}, 3) should be red");
    }
}

#[test]
fn draw_line_vertical() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let green = Color::rgb(0, 255, 0);
    draw_line(&mut buf, 8, 8, 2, 1, 2, 6, green, 1);
    for y in 1..=6 {
        assert_eq!(buf[y * 8 + 2], green, "pixel at (2, {y}) should be green");
    }
}

#[test]
fn draw_line_diagonal() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let blue = Color::rgb(0, 0, 255);
    draw_line(&mut buf, 8, 8, 0, 0, 7, 7, blue, 1);
    // At minimum, endpoints should be set.
    assert_eq!(buf[0], blue);
    assert_eq!(buf[7 * 8 + 7], blue);
}

#[test]
fn draw_line_single_point() {
    let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
    let c = Color::rgb(128, 128, 128);
    draw_line(&mut buf, 4, 4, 2, 2, 2, 2, c, 1);
    assert_eq!(buf[2 * 4 + 2], c);
}

#[test]
fn draw_rect_outline() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(255, 0, 0);
    draw_rect(&mut buf, 8, 8, 1, 1, 4, 3, c);
    // Top edge: y=1, x=1..4
    for x in 1..=4 {
        assert_eq!(buf[8 + x], c);
    }
    // Bottom edge: y=3, x=1..4
    for x in 1..=4 {
        assert_eq!(buf[3 * 8 + x], c);
    }
    // Left edge: x=1, y=1..3
    for y in 1..=3 {
        assert_eq!(buf[y * 8 + 1], c);
    }
    // Interior should be untouched.
    assert_eq!(buf[2 * 8 + 2], Color::rgb(0, 0, 0));
}

#[test]
fn draw_rect_zero_size_noop() {
    let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
    draw_rect(&mut buf, 4, 4, 0, 0, 0, 0, Color::rgb(255, 0, 0));
    assert_eq!(buf[0], Color::rgb(0, 0, 0));
}

#[test]
fn draw_filled_rect_basic() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(0, 255, 0);
    draw_filled_rect(&mut buf, 8, 8, 2, 2, 3, 2, c);
    for y in 2..4 {
        for x in 2..5 {
            assert_eq!(buf[y * 8 + x], c, "pixel at ({x}, {y})");
        }
    }
}

#[test]
fn draw_circle_zero_radius() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(255, 0, 0);
    draw_circle(&mut buf, 8, 8, 4, 4, 0, c);
    assert_eq!(buf[4 * 8 + 4], c);
}

#[test]
fn draw_circle_outline() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
    let c = Color::rgb(0, 0, 255);
    draw_circle(&mut buf, 16, 16, 8, 8, 3, c);
    // Center should not be filled (outline only).
    assert_eq!(buf[8 * 16 + 8], Color::rgb(0, 0, 0));
    // Some point on the circle should be drawn.
    assert_eq!(buf[(8 - 3) * 16 + 8], c);
}

#[test]
fn draw_filled_circle_basic() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
    let c = Color::rgb(255, 128, 0);
    draw_filled_circle(&mut buf, 16, 16, 8, 8, 3, c);
    // Center should be filled.
    assert_eq!(buf[8 * 16 + 8], c);
    // Point exactly at radius should be filled.
    assert_eq!(buf[8 * 16 + 11], c); // (11, 8), dx=3
}

#[test]
fn draw_filled_circle_zero_radius() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(255, 0, 0);
    draw_filled_circle(&mut buf, 8, 8, 4, 4, 0, c);
    assert_eq!(buf[4 * 8 + 4], c);
}

// -- Flood fill --

#[test]
fn flood_fill_basic() {
    let mut buf = vec![Color::rgb(255, 255, 255); 4 * 4];
    let red = Color::rgb(255, 0, 0);
    flood_fill(&mut buf, 4, 4, 0, 0, red);
    for px in &buf {
        assert_eq!(*px, red);
    }
}

#[test]
fn flood_fill_bounded() {
    // Create a 4x4 canvas with a barrier.
    let w = Color::rgb(255, 255, 255);
    let b = Color::rgb(0, 0, 0);
    #[rustfmt::skip]
    let mut buf = vec![
        w, w, b, w,
        w, w, b, w,
        b, b, b, w,
        w, w, w, w,
    ];
    let red = Color::rgb(255, 0, 0);
    flood_fill(&mut buf, 4, 4, 0, 0, red);
    // Top-left region should be red.
    assert_eq!(buf[0], red);
    assert_eq!(buf[1], red);
    assert_eq!(buf[4], red);
    assert_eq!(buf[5], red);
    // Beyond barrier should be unchanged.
    assert_eq!(buf[3], w);
    assert_eq!(buf[7], w);
}

#[test]
fn flood_fill_same_color_noop() {
    let red = Color::rgb(255, 0, 0);
    let mut buf = vec![red; 4 * 4];
    flood_fill(&mut buf, 4, 4, 0, 0, red);
    // No infinite loop, same color fill is a no-op.
    assert_eq!(buf[0], red);
}

#[test]
fn flood_fill_out_of_bounds() {
    let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
    // Should not panic.
    flood_fill(&mut buf, 4, 4, -1, -1, Color::rgb(255, 0, 0));
    flood_fill(&mut buf, 4, 4, 10, 10, Color::rgb(255, 0, 0));
}

// -- Brush --

#[test]
fn draw_brush_size_1() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(255, 0, 0);
    draw_brush(&mut buf, 8, 8, 4, 4, 1, c);
    assert_eq!(buf[4 * 8 + 4], c);
}

#[test]
fn draw_brush_size_3() {
    let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
    let c = Color::rgb(0, 255, 0);
    draw_brush(&mut buf, 8, 8, 4, 4, 3, c);
    // Should fill a 3x3 area centered on (4,4).
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            let x = (4 + dx) as usize;
            let y = (4 + dy) as usize;
            assert_eq!(buf[y * 8 + x], c, "pixel at ({x}, {y})");
        }
    }
}

#[test]
fn draw_brush_at_edge() {
    let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
    let c = Color::rgb(255, 0, 0);
    // Brush at corner, some pixels out of bounds.
    draw_brush(&mut buf, 4, 4, 0, 0, 3, c);
    assert_eq!(buf[0], c);
    // Should not panic.
}

// -- Undo/redo --

#[test]
fn undo_restores_state() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Move cursor to (0, 0).
    for _ in 0..32 {
        app.handle_input(&Button::Left, &vfs);
    }
    for _ in 0..24 {
        app.handle_input(&Button::Up, &vfs);
    }
    let before = app.canvas.get_pixel(0, 0);
    // Draw at (0, 0).
    app.handle_input(&Button::Confirm, &vfs);
    let after = app.canvas.get_pixel(0, 0);
    assert_ne!(before, after);
    // Undo.
    app.handle_input(&Button::Start, &vfs);
    let restored = app.canvas.get_pixel(0, 0);
    assert_eq!(restored, before);
}

#[test]
fn redo_after_undo() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    for _ in 0..32 {
        app.handle_input(&Button::Left, &vfs);
    }
    for _ in 0..24 {
        app.handle_input(&Button::Up, &vfs);
    }
    app.handle_input(&Button::Confirm, &vfs);
    let drawn = app.canvas.get_pixel(0, 0);
    app.handle_input(&Button::Start, &vfs);
    app.handle_input(&Button::Select, &vfs);
    assert_eq!(app.canvas.get_pixel(0, 0), drawn);
}

#[test]
fn undo_stack_limited() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Perform more than MAX_UNDO operations.
    for _ in 0..MAX_UNDO + 5 {
        app.handle_input(&Button::Confirm, &vfs);
        app.stop_drawing();
    }
    assert!(app.undo_stack.len() <= MAX_UNDO);
}

#[test]
fn undo_empty_noop() {
    let mut app = PaintApp::new("/apps/paint");
    // Should not panic.
    app.undo();
    assert!(app.redo_stack.is_empty());
}

#[test]
fn redo_empty_noop() {
    let mut app = PaintApp::new("/apps/paint");
    app.redo();
    assert!(app.undo_stack.is_empty());
}

// -- Color palette --

#[test]
fn palette_has_16_colors() {
    assert_eq!(palette().len(), 16);
}

#[test]
fn palette_first_is_black() {
    assert_eq!(palette()[0], Color::rgb(0, 0, 0));
}

#[test]
fn palette_second_is_white() {
    assert_eq!(palette()[1], Color::rgb(255, 255, 255));
}

#[test]
fn palette_all_opaque() {
    for c in &palette() {
        assert_eq!(c.a, 255);
    }
}

// -- Tool cycling --

#[test]
fn tool_cycle_all() {
    let mut t = Tool::Pencil;
    let mut seen = Vec::new();
    for _ in 0..8 {
        seen.push(t);
        t = t.next();
    }
    assert_eq!(seen.len(), 8);
    // Should wrap back to pencil.
    assert_eq!(t, Tool::Pencil);
}

#[test]
fn tool_names_nonempty() {
    for t in &Tool::ALL {
        assert!(!t.name().is_empty());
    }
}

// -- PaintApp state --

#[test]
fn paint_app_title_and_path() {
    let app = PaintApp::new("/apps/paint");
    assert_eq!(app.title(), "Paint");
    assert_eq!(app.path(), "/apps/paint");
}

#[test]
fn paint_app_initial_cursor() {
    let app = PaintApp::new("/apps/paint");
    assert_eq!(app.cursor_x, (CANVAS_W / 2) as i32);
    assert_eq!(app.cursor_y, (CANVAS_H / 2) as i32);
}

#[test]
fn paint_app_initial_tool() {
    let app = PaintApp::new("/apps/paint");
    assert_eq!(app.tool, Tool::Pencil);
}

#[test]
fn paint_app_cursor_movement() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    let start_x = app.cursor_x;
    let start_y = app.cursor_y;
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.cursor_x, start_x + 1);
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.cursor_y, start_y + 1);
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.cursor_x, start_x);
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.cursor_y, start_y);
}

#[test]
fn paint_app_cursor_clamp_bounds() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Move far left/up to hit (0, 0).
    for _ in 0..100 {
        app.handle_input(&Button::Left, &vfs);
        app.handle_input(&Button::Up, &vfs);
    }
    assert_eq!(app.cursor_x, 0);
    assert_eq!(app.cursor_y, 0);
    // Move far right/down.
    for _ in 0..200 {
        app.handle_input(&Button::Right, &vfs);
        app.handle_input(&Button::Down, &vfs);
    }
    assert_eq!(app.cursor_x, CANVAS_W as i32 - 1);
    assert_eq!(app.cursor_y, CANVAS_H as i32 - 1);
}

#[test]
fn paint_app_tool_cycle_via_input() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    assert_eq!(app.tool, Tool::Pencil);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.tool, Tool::Line);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.tool, Tool::Rectangle);
}

#[test]
fn paint_app_color_cycle() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    let pal = palette();
    assert_eq!(app.color, pal[0]);
    app.handle_input(&Button::Square, &vfs);
    assert_eq!(app.color, pal[1]);
}

#[test]
fn paint_app_brush_size_cycle() {
    let mut app = PaintApp::new("/apps/paint");
    assert_eq!(app.brush_size, 1);
    app.cycle_brush_size();
    assert_eq!(app.brush_size, 2);
    app.cycle_brush_size();
    assert_eq!(app.brush_size, 3);
    // Cycle through to 5, then back to 1.
    app.cycle_brush_size();
    app.cycle_brush_size();
    assert_eq!(app.brush_size, 5);
    app.cycle_brush_size();
    assert_eq!(app.brush_size, 1);
}

#[test]
fn paint_app_grid_toggle() {
    let mut app = PaintApp::new("/apps/paint");
    assert!(!app.show_grid);
    app.show_grid = true;
    assert!(app.show_grid);
    app.show_grid = false;
    assert!(!app.show_grid);
}

#[test]
fn paint_app_start_undoes() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Start button should trigger undo (no-op when stack
    // is empty, but should not panic).
    let action = app.handle_input(&Button::Start, &vfs);
    assert_eq!(action, AppAction::None);
}

#[test]
fn paint_app_select_redoes() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Select button should trigger redo (no-op when stack
    // is empty, but should not panic).
    let action = app.handle_input(&Button::Select, &vfs);
    assert_eq!(action, AppAction::None);
}

#[test]
fn paint_app_cancel_exits() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    let action = app.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::Exit);
}

#[test]
fn paint_app_display_lines_nonempty() {
    let app = PaintApp::new("/apps/paint");
    assert!(!app.lines().is_empty());
    // The heading shows canvas dimensions, not the app title — the
    // title already lives in the WM / app-chrome title bar.
    assert!(app.lines().iter().any(|l| l.contains("Canvas:")));
}

#[test]
fn paint_app_display_lines_show_tool() {
    let app = PaintApp::new("/apps/paint");
    assert!(app.lines().iter().any(|l| l.contains("Pencil")));
}

#[test]
fn paint_app_downcast() {
    let app = PaintApp::new("/apps/paint");
    let any = app.as_any();
    assert!(any.downcast_ref::<PaintApp>().is_some());
}

#[test]
fn wheel_scrolls_only_the_picker() {
    let mut vfs = make_vfs();
    let mut app = PaintApp::new("/apps/paint");
    assert!(!app.handle_scroll(3, &vfs), "the canvas ignores the wheel");
    for i in 0..5 {
        app.new_canvas(4 + i, 4);
        app.file_path = None;
        app.save_to_vfs(&mut vfs).expect("save");
    }
    app.begin_open();
    app.apply_vfs_ops(&mut vfs);
    assert!(app.handle_scroll(2, &vfs));
    assert_eq!(app.picker.as_ref().map(|p| p.selected), Some(2));
    assert!(app.handle_scroll(-5, &vfs));
    assert_eq!(app.picker.as_ref().map(|p| p.selected), Some(0));
}

#[test]
fn paint_app_downcast_mut() {
    let mut app = PaintApp::new("/apps/paint");
    let any = app.as_any_mut();
    assert!(any.downcast_mut::<PaintApp>().is_some());
}

// -- Edge cases --

#[test]
fn draw_at_canvas_boundary() {
    let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
    let c = Color::rgb(255, 0, 0);
    // All four corners.
    draw_pixel(&mut buf, 4, 0, 0, c);
    draw_pixel(&mut buf, 4, 3, 0, c);
    draw_pixel(&mut buf, 4, 0, 3, c);
    draw_pixel(&mut buf, 4, 3, 3, c);
    assert_eq!(buf[0], c);
    assert_eq!(buf[3], c);
    assert_eq!(buf[12], c);
    assert_eq!(buf[15], c);
}

#[test]
fn line_with_brush_size() {
    let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
    let c = Color::rgb(255, 0, 0);
    draw_line(&mut buf, 16, 16, 2, 8, 14, 8, c, 3);
    // Center row should be filled.
    for x in 2..=14 {
        assert_eq!(buf[8 * 16 + x], c, "center at ({x}, 8)");
    }
    // Row above should also be partially filled
    // (brush extends 1 above).
    assert_eq!(buf[7 * 16 + 2], c);
}

#[test]
fn alpha_blend_opaque_over_transparent() {
    let src = Color::rgb(255, 0, 0);
    let dst = Color::rgba(0, 0, 0, 0);
    let result = alpha_blend(src, dst, 255);
    assert_eq!(result.r, 255);
    assert_eq!(result.g, 0);
    assert_eq!(result.b, 0);
    assert_eq!(result.a, 255);
}

#[test]
fn alpha_blend_transparent_over_opaque() {
    let src = Color::rgba(0, 0, 0, 0);
    let dst = Color::rgb(0, 255, 0);
    let result = alpha_blend(src, dst, 255);
    assert_eq!(result, dst);
}

#[test]
fn alpha_blend_semi_transparent() {
    let src = Color::rgba(255, 0, 0, 128);
    let dst = Color::rgb(0, 0, 255);
    let result = alpha_blend(src, dst, 255);
    // Should be a blend of red and blue.
    assert!(result.r > 0);
    assert!(result.b > 0);
    assert!(result.a > 128);
}

#[test]
fn canvas_1x1() {
    let mut c = Canvas::new(1, 1);
    let red = Color::rgb(255, 0, 0);
    c.set_pixel(0, 0, red);
    assert_eq!(c.get_pixel(0, 0), red);
}

#[test]
fn encode_bmp_valid_header() {
    let pixels = vec![Color::rgb(255, 0, 0); 4];
    let bmp = encode_bmp(&pixels, 2, 2);
    assert_eq!(&bmp[0..2], b"BM");
    // File size: 54 header + 2*2*4 = 70.
    let file_size = u32::from_le_bytes([bmp[2], bmp[3], bmp[4], bmp[5]]);
    assert_eq!(file_size, 70);
}

#[test]
fn save_to_vfs_creates_file() {
    let mut app = PaintApp::new("/apps/paint");
    let mut vfs = make_vfs();
    // The pictures folder (and its parents) are created on demand.
    let path = app.save_to_vfs(&mut vfs).expect("save");
    assert_eq!(path, "/home/user/pictures/paint_001.bmp");
    assert!(vfs.exists(&path));
}

#[test]
fn save_reuses_file_but_save_as_never_overwrites() {
    let mut app = PaintApp::new("/apps/paint");
    let mut vfs = make_vfs();
    let first = app.save_to_vfs(&mut vfs).expect("save");
    // Save again: same file.
    assert_eq!(app.save_to_vfs(&mut vfs).expect("save"), first);
    // Save As: fresh unique file.
    let second = app.save_as_to_vfs(&mut vfs).expect("save as");
    assert_ne!(second, first);
    assert_eq!(second, "/home/user/pictures/paint_002.bmp");
    // New canvas is untitled again and gets yet another name.
    app.new_canvas(16, 16);
    let third = app.save_to_vfs(&mut vfs).expect("save");
    assert_eq!(third, "/home/user/pictures/paint_003.bmp");
}

/// Content rect used by the click tests (a typical window).
const CW: u32 = 460;
const CH: u32 = 240;

fn click_canvas_pixel(app: &mut PaintApp, px: i32, py: i32) {
    let l = PaintLayout::compute(0, 0, CW, CH, app.canvas.width(), app.canvas.height());
    let r = l.pixel_rect(px, py);
    // Aim at the middle of the on-screen cell.
    click_at(
        app,
        r.x + r.w as i32 / 2,
        r.y + r.h as i32 / 2,
        CW,
        CH,
        &make_vfs(),
    );
}

#[test]
fn click_on_screen_pixel_sets_matching_cursor() {
    let mut app = PaintApp::new("/apps/paint");
    let red = palette()[2];
    app.select_color(2);
    for (px, py) in [(0, 0), (10, 7), (63, 47), (31, 20)] {
        click_canvas_pixel(&mut app, px, py);
        assert_eq!((app.cursor_x, app.cursor_y), (px, py));
        assert_eq!(app.canvas.get_pixel(px as u32, py as u32), red);
    }
}

#[test]
fn click_matches_windowed_draw_offset() {
    // Draw into a recording backend and check the cursor outline sits
    // exactly around the clicked canvas cell.
    let mut app = PaintApp::new("/apps/paint");
    click_canvas_pixel(&mut app, 5, 6);
    let l = PaintLayout::compute(0, 0, CW, CH, app.canvas.width(), app.canvas.height());
    let cell = l.pixel_rect(5, 6);
    let mut backend = oasis_test_backend::RecordingBackend::new(CW, CH);
    let at = ActiveTheme::default();
    app.draw_windowed(0, 0, CW, CH, &mut backend, &at)
        .expect("draw");
    let accent = render::PaintColors::from_theme(&at).cursor;
    // Top edge of the cursor outline hugs the clicked cell.
    let expected = oasis_test_backend::DrawCommand::FillRect {
        x: cell.x - 1,
        y: cell.y - 1,
        w: cell.w + 2,
        h: 1,
        color: accent,
    };
    assert!(backend.commands().contains(&expected));
}

#[test]
fn clicks_outside_canvas_do_not_paint() {
    let mut app = PaintApp::new("/apps/paint");
    let before = app.canvas.pixels.clone();
    click_at(&mut app, 1, CH as i32 - 30, CW, CH, &make_vfs());
    assert_eq!(app.canvas.pixels, before);
    assert!(app.undo_stack.is_empty());
}

#[test]
fn palette_click_selects_color() {
    let mut app = PaintApp::new("/apps/paint");
    let l = PaintLayout::compute(0, 0, CW, CH, 64, 48);
    let r = l.swatch_rect(4);
    click_at(&mut app, r.x + 1, r.y + 1, CW, CH, &make_vfs());
    assert_eq!(app.palette_index, 4);
    assert_eq!(app.color, palette()[4]);
}

#[test]
fn cancelled_shape_leaves_history_alone() {
    let mut app = PaintApp::new("/apps/paint");
    // A redo entry that a cancelled shape must not discard.
    app.tool = Tool::Fill;
    click_canvas_pixel(&mut app, 1, 1);
    app.undo();
    assert_eq!(app.redo_stack.len(), 1);
    app.modified = false;

    app.tool = Tool::Rectangle;
    click_canvas_pixel(&mut app, 2, 2);
    assert!(app.drag_start.is_some());
    app.handle_input(&Button::Triangle, &make_vfs()); // switch tool: cancels
    assert!(app.drag_start.is_none());
    assert!(app.undo_stack.is_empty());
    assert_eq!(app.redo_stack.len(), 1);
    assert!(!app.modified);

    // Committing a shape is one undo step.
    app.tool = Tool::Line;
    click_canvas_pixel(&mut app, 2, 2);
    click_canvas_pixel(&mut app, 8, 2);
    assert_eq!(app.undo_stack.len(), 1);
    assert!(app.modified);
    app.undo();
    assert_ne!(app.canvas.get_pixel(5, 2), app.color);
}

#[test]
fn shape_tool_takes_two_clicks() {
    let mut app = PaintApp::new("/apps/paint");
    app.tool = Tool::Line;
    click_canvas_pixel(&mut app, 2, 2);
    assert_eq!(app.drag_start, Some((2, 2)));
    click_canvas_pixel(&mut app, 8, 2);
    assert_eq!(app.drag_start, None);
    for x in 2..=8 {
        assert_eq!(app.canvas.get_pixel(x, 2), app.color);
    }
}

#[test]
fn save_then_open_round_trip() {
    let mut vfs = make_vfs();
    let mut app = PaintApp::new("/apps/paint");
    app.select_color(4);
    click_canvas_pixel(&mut app, 3, 4);
    let drawn = app.canvas.pixels.clone();

    // Ctrl+S queues the save; the host applies it with a mutable VFS.
    let consumed = app.handle_key(&Key::Char('s'), Modifiers::CTRL, &vfs);
    assert_eq!(consumed, Some(AppAction::None));
    assert!(app.apply_vfs_ops(&mut vfs));
    let path = app.file_path.clone().expect("saved path");
    assert!(!app.modified);

    // A fresh app opens it through File > Open.
    let mut other = PaintApp::new("/apps/paint");
    other.run_menu_action("file.open");
    other.apply_vfs_ops(&mut vfs);
    let files = other
        .picker
        .as_ref()
        .and_then(|p| p.files.clone())
        .expect("listing");
    assert_eq!(files, vec![path.clone()]);
    other.handle_input(&Button::Confirm, &vfs);
    assert!(other.picker.is_none());
    assert_eq!(other.file_path.as_deref(), Some(path.as_str()));
    assert_eq!(other.canvas.pixels, drawn);
    assert_eq!(other.canvas.get_pixel(3, 4), palette()[4]);
}

#[test]
fn picker_row_click_loads_file() {
    let mut vfs = make_vfs();
    let mut app = PaintApp::new("/apps/paint");
    app.new_canvas(16, 8);
    app.save_to_vfs(&mut vfs).expect("save");
    let mut other = PaintApp::new("/apps/paint");
    other.begin_open();
    other.apply_vfs_ops(&mut vfs);
    let l = PaintLayout::compute(0, 0, CW, CH, 64, 48);
    let p = l.picker();
    click_at(&mut other, p.x + 5, p.y + PICKER_ROW + 2, CW, CH, &vfs);
    assert_eq!((other.canvas.width(), other.canvas.height()), (16, 8));
}

const PICKER_ROW: i32 = layout::PICKER_ROW_H as i32;

#[test]
fn shortcuts_new_undo_grid() {
    let vfs = make_vfs();
    let mut app = PaintApp::new("/apps/paint");
    click_canvas_pixel(&mut app, 1, 1);
    assert_eq!(app.undo_stack.len(), 1);
    app.handle_key(&Key::Char('z'), Modifiers::CTRL, &vfs);
    assert_eq!(app.canvas.get_pixel(1, 1), Color::rgb(255, 255, 255));
    app.handle_key(&Key::Char('y'), Modifiers::CTRL, &vfs);
    assert_eq!(app.canvas.get_pixel(1, 1), app.color);
    app.handle_key(&Key::Char('g'), Modifiers::NONE, &vfs);
    assert!(app.show_grid);
    app.handle_key(&Key::Char(']'), Modifiers::NONE, &vfs);
    assert_eq!(app.brush_size, 2);
    app.handle_key(&Key::Char('n'), Modifiers::CTRL, &vfs);
    assert!(app.undo_stack.is_empty());
    assert_eq!(app.canvas.get_pixel(1, 1), Color::rgb(255, 255, 255));
    // Plain letters other than shortcuts fall through.
    assert!(
        app.handle_key(&Key::Char('q'), Modifiers::NONE, &vfs)
            .is_none()
    );
}

#[test]
fn menu_actions_layers_and_brush() {
    let mut app = PaintApp::new("/apps/paint");
    app.run_menu_action("layer.add");
    assert_eq!(app.canvas.layer_count(), 2);
    assert_eq!(app.canvas.active_layer(), 1);
    app.run_menu_action("layer.next");
    assert_eq!(app.canvas.active_layer(), 0);
    app.run_menu_action("brush.size.4");
    assert_eq!(app.brush_size, 4);
    app.run_menu_action("tool.7");
    assert_eq!(app.tool, Tool::Fill);
    app.run_menu_action("brush.color.prev");
    assert_eq!(app.palette_index, 15);
    assert_eq!(app.run_menu_action("file.exit"), AppAction::Exit);
}

#[test]
fn menu_click_opens_dropdown() {
    let mut app = PaintApp::new("/apps/paint");
    // "File" label sits at the left of the bar.
    click_at(&mut app, 12, 8, CW, CH, &make_vfs());
    assert_eq!(app.menu.open, Some(0));
    // Clicking the canvas while open just closes the menu.
    click_canvas_pixel(&mut app, 0, 0);
    assert!(!app.menu.is_open());
    assert!(app.undo_stack.is_empty());
}

#[test]
fn open_rejects_bad_file() {
    let mut vfs = make_vfs();
    oasis_app_core::vfs_util::ensure_dir(&mut vfs, PICTURES_DIR).expect("mkdir");
    vfs.write("/home/user/pictures/bad.bmp", b"garbage")
        .expect("write");
    let mut app = PaintApp::new("/apps/paint");
    assert!(
        app.open_from_vfs(&vfs, "/home/user/pictures/bad.bmp")
            .is_err()
    );
    assert_eq!(app.canvas.width(), CANVAS_W);
}

#[test]
fn new_canvas_resets_state() {
    let mut app = PaintApp::new("/apps/paint");
    let vfs = make_vfs();
    // Draw something and add undo entries.
    app.handle_input(&Button::Confirm, &vfs);
    app.stop_drawing();
    assert!(!app.undo_stack.is_empty());
    // Reset canvas.
    app.new_canvas(32, 32);
    assert!(app.undo_stack.is_empty());
    assert_eq!(app.canvas.width(), 32);
    assert_eq!(app.canvas.height(), 32);
}

#[test]
fn new_canvas_clamps_size() {
    let mut app = PaintApp::new("/apps/paint");
    app.new_canvas(4, 4); // below minimum
    assert_eq!(app.canvas.width(), 8);
    assert_eq!(app.canvas.height(), 8);
    app.new_canvas(999, 999); // above maximum
    assert_eq!(app.canvas.width(), 256);
    assert_eq!(app.canvas.height(), 256);
}

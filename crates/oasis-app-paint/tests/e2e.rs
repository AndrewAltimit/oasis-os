//! End-to-end Paint sessions driven through the `App` trait: clicks on the
//! drawn canvas / palette / menu bar, keyboard shortcuts, VFS saves and
//! reloads, checked against the rendered output and the saved BMP.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_paint::{PICTURES_DIR, PaintApp, decode_bmp, palette};
use oasis_test_backend::{ClipAuditBackend, Color};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

const WHITE: Color = Color::rgb(255, 255, 255);

fn paint() -> AppHarness {
    AppHarness::new(Box::new(PaintApp::new("/apps/Paint")))
}

/// On-screen geometry of the canvas, discovered from what was drawn:
/// a fresh canvas is all white, one square rect per canvas pixel.
#[derive(Debug, Clone, Copy)]
struct CanvasGeom {
    /// Absolute origin of canvas pixel (0, 0).
    x: i32,
    y: i32,
    scale: u32,
    /// Window origin (to convert to content-local click coordinates).
    wx: i32,
    wy: i32,
}

impl CanvasGeom {
    fn discover(h: &AppHarness) -> Self {
        let b = h.draw();
        let mut sizes: std::collections::HashMap<u32, usize> = Default::default();
        for r in b.rects() {
            if r.color == Some(WHITE) && r.w == r.h {
                *sizes.entry(r.w).or_default() += 1;
            }
        }
        let (&scale, _) = sizes.iter().max_by_key(|(_, n)| **n).unwrap();
        let px: Vec<_> = b
            .rects()
            .iter()
            .filter(|r| r.color == Some(WHITE) && r.w == scale && r.h == scale)
            .collect();
        let x = px.iter().map(|r| r.x).min().unwrap();
        let y = px.iter().map(|r| r.y).min().unwrap();
        // The harness places the window content at (8, 20).
        Self {
            x,
            y,
            scale,
            wx: 8,
            wy: 20,
        }
    }

    /// Content-local click point at the centre of canvas pixel (px, py).
    fn local(&self, px: i32, py: i32) -> (i32, i32) {
        let s = self.scale as i32;
        (
            self.x + px * s + s / 2 - self.wx,
            self.y + py * s + s / 2 - self.wy,
        )
    }
}

/// Color the renderer drew for canvas pixel (px, py) (last pixel rect).
fn drawn_pixel(b: &ClipAuditBackend, g: CanvasGeom, px: i32, py: i32) -> Option<Color> {
    let s = g.scale as i32;
    let (x, y) = (g.x + px * s, g.y + py * s);
    b.rects()
        .iter()
        .filter(|r| r.x == x && r.y == y && r.w == g.scale && r.h == g.scale)
        .filter_map(|r| r.color)
        .next_back()
}

/// Click the palette swatch drawn in `color` (swatches are 12px tall).
fn click_swatch(h: &mut AppHarness, color: Color) {
    let b = h.draw();
    let r = b
        .rects()
        .iter()
        .find(|r| r.color == Some(color) && r.h == 12 && r.w > 1)
        .unwrap_or_else(|| panic!("no swatch drawn for {color:?}"));
    let (x, y) = (r.x + r.w as i32 / 2 - 8, r.y + r.h as i32 / 2 - 20);
    h.click(x, y);
}

fn click_px(h: &mut AppHarness, g: CanvasGeom, px: i32, py: i32) {
    let (x, y) = g.local(px, py);
    assert_eq!(h.click(x, y), AppAction::None);
}

fn menu(h: &mut AppHarness, menu: &str, item: &str) {
    h.click_text(menu);
    h.click_text_last(item);
}

fn saved_files(h: &AppHarness) -> Vec<String> {
    h.vfs()
        .readdir(PICTURES_DIR)
        .map(|v| v.into_iter().map(|e| e.name).collect())
        .unwrap_or_default()
}

fn saved_pixels(h: &AppHarness, name: &str) -> (u32, u32, Vec<Color>) {
    let data = h.vfs().read(&format!("{PICTURES_DIR}/{name}")).unwrap();
    decode_bmp(&data).unwrap()
}

#[test]
fn pencil_strokes_change_pixels_and_survive_save_and_reopen() {
    let mut h = paint();
    let g = CanvasGeom::discover(&h);
    let red = palette()[2];
    click_swatch(&mut h, red);
    assert!(h.app().lines().join("\n").contains("(Red)"));
    click_px(&mut h, g, 10, 10);
    click_px(&mut h, g, 11, 10);

    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 10, 10), Some(red));
    assert_eq!(drawn_pixel(&b, g, 11, 10), Some(red));
    assert_eq!(drawn_pixel(&b, g, 12, 10), Some(WHITE));
    assert!(h.app().lines()[0].contains("untitled*"));

    // Ctrl+S queues the save; the host performs it in apply_vfs_ops.
    h.ctrl('s');
    assert!(
        saved_files(&h).is_empty(),
        "save must wait for apply_vfs_ops"
    );
    assert!(h.frame(16));
    assert_eq!(saved_files(&h), vec!["paint_001.bmp".to_string()]);
    let (w, hh, px) = saved_pixels(&h, "paint_001.bmp");
    assert_eq!((w, hh), (64, 48));
    assert_eq!(px[(10 * w + 10) as usize], red);
    assert_eq!(px[(10 * w + 11) as usize], red);
    assert_eq!(px[(10 * w + 12) as usize], WHITE);
    assert!(h.app().lines()[0].contains("paint_001.bmp"));
    assert!(!h.app().lines()[0].contains('*'));

    // Reopen in a fresh Paint through File > Open and the picker.
    h.replace_app(Box::new(PaintApp::new("/apps/Paint")));
    menu(&mut h, "File", "Open...");
    h.click_text("paint_001.bmp");
    h.frame(16);
    assert!(
        h.app().lines().join("\n").contains("Opened paint_001.bmp"),
        "{:?}",
        h.app().lines()
    );
    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 10, 10), Some(red));
    assert_eq!(drawn_pixel(&b, g, 12, 10), Some(WHITE));

    // Saving again overwrites the opened file instead of making a new one
    // (the fresh app starts with the default black brush).
    click_px(&mut h, g, 0, 0);
    h.ctrl('s');
    h.frame(16);
    assert_eq!(saved_files(&h), vec!["paint_001.bmp".to_string()]);
    let (w, _, px) = saved_pixels(&h, "paint_001.bmp");
    assert_eq!(px[0], palette()[0]);
    assert_eq!(px[(10 * w + 10) as usize], red);
}

#[test]
fn undo_and_redo_from_the_edit_menu_and_keyboard() {
    let mut h = paint();
    let g = CanvasGeom::discover(&h);
    let blue = palette()[4];
    click_swatch(&mut h, blue);
    click_px(&mut h, g, 5, 5);
    click_px(&mut h, g, 6, 5);
    menu(&mut h, "Edit", "Undo");
    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 6, 5), Some(WHITE));
    assert_eq!(drawn_pixel(&b, g, 5, 5), Some(blue));
    h.ctrl('z');
    assert_eq!(drawn_pixel(&h.draw(), g, 5, 5), Some(WHITE));
    // Nothing left to undo: stays white, no panic.
    h.ctrl('z');
    h.ctrl('y');
    menu(&mut h, "Edit", "Redo");
    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 5, 5), Some(blue));
    assert_eq!(drawn_pixel(&b, g, 6, 5), Some(blue));
    // A new stroke clears the redo stack.
    h.ctrl('z');
    click_px(&mut h, g, 20, 20);
    h.ctrl('y');
    assert_eq!(drawn_pixel(&h.draw(), g, 6, 5), Some(WHITE));
}

#[test]
fn shape_and_fill_tools_selected_from_the_menu() {
    let mut h = paint();
    let g = CanvasGeom::discover(&h);
    let black = palette()[0];
    menu(&mut h, "Tool", "Line");
    assert!(h.app().lines().join("\n").contains("Tool: Line"));
    click_px(&mut h, g, 2, 2);
    click_px(&mut h, g, 12, 2);
    let b = h.draw();
    for x in 2..=12 {
        assert_eq!(drawn_pixel(&b, g, x, 2), Some(black), "line pixel {x}");
    }
    assert_eq!(drawn_pixel(&b, g, 13, 2), Some(WHITE));

    menu(&mut h, "Tool", "Rect");
    click_px(&mut h, g, 20, 20);
    click_px(&mut h, g, 30, 30);
    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 20, 25), Some(black));
    assert_eq!(drawn_pixel(&b, g, 30, 30), Some(black));
    assert_eq!(drawn_pixel(&b, g, 25, 25), Some(WHITE), "outline only");

    // Flood fill the inside of the rectangle.
    let green = palette()[3];
    menu(&mut h, "Tool", "Fill");
    click_swatch(&mut h, green);
    click_px(&mut h, g, 25, 25);
    let b = h.draw();
    assert_eq!(drawn_pixel(&b, g, 25, 25), Some(green));
    assert_eq!(drawn_pixel(&b, g, 21, 29), Some(green));
    assert_eq!(drawn_pixel(&b, g, 20, 25), Some(black), "border kept");
    assert_eq!(drawn_pixel(&b, g, 40, 40), Some(WHITE), "fill leaked out");

    // Eraser makes pixels transparent: nothing is drawn for them.
    menu(&mut h, "Tool", "Eraser");
    click_px(&mut h, g, 25, 25);
    assert_eq!(drawn_pixel(&h.draw(), g, 25, 25), None);
}

#[test]
fn gamepad_only_drawing_session() {
    let mut h = paint();
    // Pencil at the centre cursor, move right twice while drawing.
    h.press(Button::Confirm);
    h.press(Button::Right);
    h.press(Button::Right);
    // Saving needs a pointer or Ctrl+S (no gamepad binding); check the
    // canvas via the saved BMP.
    h.ctrl('s');
    h.frame(16);
    let (w, _, px) = saved_pixels(&h, "paint_001.bmp");
    let (cx, cy) = (32u32, 24u32);
    for dx in 0..3 {
        assert_eq!(px[(cy * w + cx + dx) as usize], palette()[0], "dx {dx}");
    }
    assert_eq!(px[(cy * w + cx + 3) as usize], WHITE);
    // Start = undo removes the whole stroke.
    h.press(Button::Start);
    h.ctrl('s');
    h.frame(16);
    let (_, _, px) = saved_pixels(&h, "paint_001.bmp");
    assert_eq!(px[(cy * w + cx) as usize], WHITE);
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn corrupt_and_foreign_bmps_report_errors_in_the_picker() {
    let mut vfs = MemoryVfs::new();
    for d in ["/home", "/home/user", PICTURES_DIR] {
        vfs.mkdir(d).unwrap();
    }
    vfs.write(&format!("{PICTURES_DIR}/a_empty.bmp"), b"")
        .unwrap();
    vfs.write(
        &format!("{PICTURES_DIR}/b_garbage.bmp"),
        b"BM\x00\x01garbage",
    )
    .unwrap();
    // Valid header claiming a 100000 x 100000 image with no pixel data.
    let mut huge = vec![0u8; 54];
    huge[0] = b'B';
    huge[1] = b'M';
    huge[10] = 54;
    huge[14] = 40;
    huge[18..22].copy_from_slice(&100_000u32.to_le_bytes());
    huge[22..26].copy_from_slice(&100_000u32.to_le_bytes());
    huge[26] = 1;
    huge[28] = 24;
    vfs.write(&format!("{PICTURES_DIR}/c_huge.bmp"), &huge)
        .unwrap();
    vfs.write(&format!("{PICTURES_DIR}/notes.txt"), b"not listed")
        .unwrap();

    let mut h = AppHarness::with_vfs(Box::new(PaintApp::new("/apps/Paint")), vfs);
    let g = CanvasGeom::discover(&h);
    for name in ["a_empty.bmp", "b_garbage.bmp", "c_huge.bmp"] {
        h.ctrl('o');
        let screen = h.screen_text();
        assert!(!screen.contains("notes.txt"), "non-BMP listed");
        h.click_text(name);
        h.frame(16);
        let lines = h.app().lines().join("\n");
        assert!(lines.contains("Error"), "{name}: {lines}");
        // The canvas is untouched and still usable.
        assert_eq!(drawn_pixel(&h.draw(), g, 1, 1), Some(WHITE));
    }
    click_px(&mut h, g, 1, 1);
    assert_eq!(drawn_pixel(&h.draw(), g, 1, 1), Some(palette()[0]));
}

#[test]
fn picker_keyboard_navigation_and_cancel() {
    let mut h = paint();
    h.ctrl('s');
    h.ctrl('s');
    h.frame(16);
    // Two saves of an unsaved canvas: the second overwrites the first.
    assert_eq!(saved_files(&h).len(), 1);
    menu(&mut h, "File", "Save As");
    h.frame(16);
    assert_eq!(saved_files(&h).len(), 2);
    h.ctrl('o');
    assert!(h.screen_text().contains("paint_002.bmp"));
    h.key(Key::Down);
    h.key(Key::Enter);
    h.frame(16);
    assert!(
        h.app().lines()[0].contains("paint_002.bmp"),
        "{:?}",
        h.app().lines()
    );
    // Escape in the picker closes only the picker.
    h.ctrl('o');
    assert_eq!(h.key(Key::Escape), AppAction::None);
    assert!(!h.closed());
    assert!(!h.screen_text().contains("Open picture"));
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = paint();
    h.draw_all_sizes_and_themes();
    h.key(Key::Char('g'));
    h.ctrl('o');
    h.draw_all_sizes_and_themes();
    h.key(Key::Escape);
    h.click_text("Tool");
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |_: &dyn Vfs| -> Box<dyn App> { Box::new(PaintApp::new("/apps/Paint")) };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}

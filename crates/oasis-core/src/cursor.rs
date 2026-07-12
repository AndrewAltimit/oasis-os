//! Mouse cursor state and procedural cursor image generation.
//!
//! Provides a visible pointer cursor that follows mouse movement.
//! The cursor is rendered as an SDI overlay object at the highest z-order.

use crate::input::InputEvent;
use crate::sdi::SdiRegistry;

/// Cursor arrow dimensions.
const CURSOR_W: u32 = 12;
const CURSOR_H: u32 = 18;

/// SDI object name for the cursor.
const CURSOR_SDI_NAME: &str = "mouse_cursor";

/// Runtime state for the mouse cursor.
#[derive(Debug)]
pub struct CursorState {
    /// Current X position.
    pub x: i32,
    /// Current Y position.
    pub y: i32,
    /// Whether the cursor is visible.
    pub visible: bool,
    /// Scale factor (1 = base 12x18, 2 = 24x36, 3 = 36x54).
    pub scale: u32,
    /// Click hotspot offset subtracted from the draw position (themed
    /// cursors; the procedural arrow points from its top-left corner).
    pub hotspot: (i32, i32),
    /// Custom cursor bitmap size for themed textures.
    /// `None` = procedural 12x18 scaled by `scale`.
    pub size: Option<(u32, u32)>,
}

impl CursorState {
    /// Create a new cursor state centered on screen.
    pub fn new(screen_w: u32, screen_h: u32) -> Self {
        Self {
            x: screen_w as i32 / 2,
            y: screen_h as i32 / 2,
            visible: true,
            scale: 1,
            hotspot: (0, 0),
            size: None,
        }
    }

    /// Set the cursor position directly (useful for tests/screenshots).
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    /// Handle an input event, updating cursor position on mouse move.
    pub fn handle_input(&mut self, event: &InputEvent) {
        if let InputEvent::CursorMove { x, y } = event {
            self.x = *x;
            self.y = *y;
            self.visible = true;
        }
    }

    /// Update the cursor SDI object to reflect current position.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry) {
        let s = self.scale.max(1);
        let (w, h) = self.size.unwrap_or((CURSOR_W * s, CURSOR_H * s));
        if !sdi.contains(CURSOR_SDI_NAME) {
            let obj = sdi.create(CURSOR_SDI_NAME);
            obj.overlay = true;
            obj.z = 10000; // Always on top.
        }
        if let Ok(obj) = sdi.get_mut(CURSOR_SDI_NAME) {
            obj.w = w;
            obj.h = h;
            obj.x = self.x - self.hotspot.0;
            obj.y = self.y - self.hotspot.1;
            obj.visible = self.visible;
            // The texture is assigned externally after load_texture.
        }
    }

    /// Hide the cursor SDI object.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        if let Ok(obj) = sdi.get_mut(CURSOR_SDI_NAME) {
            obj.visible = false;
        }
    }
}

/// Generate a procedural arrow cursor as RGBA pixel data.
///
/// `scale` controls resolution: each bitmap pixel becomes a `scale x scale`
/// block. Returns `(pixels, width, height)` where dimensions are
/// `CURSOR_W * scale` by `CURSOR_H * scale`.
pub fn generate_cursor_pixels(scale: u32) -> (Vec<u8>, u32, u32) {
    let scale = scale.max(1);
    // 12x18 cursor bitmap. Legend: 0=transparent, 1=black outline, 2=white fill.
    #[rustfmt::skip]
    let bitmap: [[u8; 12]; 18] = [
        [1,0,0,0,0,0,0,0,0,0,0,0],
        [1,1,0,0,0,0,0,0,0,0,0,0],
        [1,2,1,0,0,0,0,0,0,0,0,0],
        [1,2,2,1,0,0,0,0,0,0,0,0],
        [1,2,2,2,1,0,0,0,0,0,0,0],
        [1,2,2,2,2,1,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,0,0,0,0,0],
        [1,2,2,2,2,2,2,1,0,0,0,0],
        [1,2,2,2,2,2,2,2,1,0,0,0],
        [1,2,2,2,2,2,2,2,2,1,0,0],
        [1,2,2,2,2,2,2,2,2,2,1,0],
        [1,2,2,2,2,2,1,1,1,1,1,0],
        [1,2,2,2,2,2,1,0,0,0,0,0],
        [1,2,2,1,2,2,1,0,0,0,0,0],
        [1,2,1,0,1,2,2,1,0,0,0,0],
        [1,1,0,0,1,2,2,1,0,0,0,0],
        [1,0,0,0,0,1,2,1,0,0,0,0],
        [0,0,0,0,0,1,1,0,0,0,0,0],
    ];

    let w = CURSOR_W * scale;
    let h = CURSOR_H * scale;
    let mut pixels = vec![0u8; (w * h * 4) as usize];

    for (by, row) in bitmap.iter().enumerate() {
        for (bx, &val) in row.iter().enumerate() {
            let (r, g, b, a) = match val {
                1 => (0, 0, 0, 255),       // Black outline
                2 => (255, 255, 255, 255), // White fill
                _ => (0, 0, 0, 0),         // Transparent
            };
            // Fill scale x scale block.
            for sy in 0..scale {
                for sx in 0..scale {
                    let px = bx as u32 * scale + sx;
                    let py = by as u32 * scale + sy;
                    let offset = (py * w + px) as usize * 4;
                    pixels[offset] = r;
                    pixels[offset + 1] = g;
                    pixels[offset + 2] = b;
                    pixels[offset + 3] = a;
                }
            }
        }
    }

    (pixels, w, h)
}

impl Default for CursorState {
    fn default() -> Self {
        Self::new(480, 272)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_initial_position() {
        let cursor = CursorState::new(480, 272);
        assert_eq!(cursor.x, 240);
        assert_eq!(cursor.y, 136);
        assert!(cursor.visible);
    }

    #[test]
    fn cursor_updates_on_move() {
        let mut cursor = CursorState::new(480, 272);
        cursor.handle_input(&InputEvent::CursorMove { x: 100, y: 50 });
        assert_eq!(cursor.x, 100);
        assert_eq!(cursor.y, 50);
    }

    #[test]
    fn cursor_ignores_other_events() {
        let mut cursor = CursorState::new(480, 272);
        cursor.handle_input(&InputEvent::Quit);
        assert_eq!(cursor.x, 240);
        assert_eq!(cursor.y, 136);
    }

    #[test]
    fn generate_cursor_correct_size() {
        let (pixels, w, h) = generate_cursor_pixels(1);
        assert_eq!(w, CURSOR_W);
        assert_eq!(h, CURSOR_H);
        assert_eq!(pixels.len(), (w * h * 4) as usize);
    }

    #[test]
    fn generate_cursor_top_left_is_outline() {
        let (pixels, _, _) = generate_cursor_pixels(1);
        // Top-left pixel should be black outline (r=0,g=0,b=0,a=255).
        assert_eq!(pixels[0], 0);
        assert_eq!(pixels[1], 0);
        assert_eq!(pixels[2], 0);
        assert_eq!(pixels[3], 255);
    }

    #[test]
    fn cursor_sdi_creates_object() {
        let cursor = CursorState::new(480, 272);
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        assert!(sdi.contains(CURSOR_SDI_NAME));
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert!(obj.overlay);
        assert_eq!(obj.z, 10000);
    }

    #[test]
    fn cursor_color_has_white_fill() {
        let (pixels, w, _) = generate_cursor_pixels(1);
        // Pixel at (1,2) should be white fill.
        let offset = (2 * w + 1) as usize * 4;
        assert_eq!(pixels[offset], 255); // R
        assert_eq!(pixels[offset + 1], 255); // G
        assert_eq!(pixels[offset + 2], 255); // B
        assert_eq!(pixels[offset + 3], 255); // A
    }

    #[test]
    fn cursor_set_position() {
        let mut cursor = CursorState::new(480, 272);
        cursor.set_position(100, 200);
        assert_eq!(cursor.x, 100);
        assert_eq!(cursor.y, 200);
    }

    #[test]
    fn cursor_default_trait() {
        let cursor = CursorState::default();
        assert_eq!(cursor.x, 240);
        assert_eq!(cursor.y, 136);
    }

    #[test]
    fn cursor_visible_after_move() {
        let mut cursor = CursorState::new(480, 272);
        cursor.visible = false;
        cursor.handle_input(&InputEvent::CursorMove { x: 100, y: 50 });
        assert!(cursor.visible);
    }

    #[test]
    fn cursor_sdi_updates_position() {
        let mut cursor = CursorState::new(480, 272);
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        cursor.x = 100;
        cursor.y = 200;
        cursor.update_sdi(&mut sdi);
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert_eq!(obj.x, 100);
        assert_eq!(obj.y, 200);
    }

    #[test]
    fn cursor_sdi_sets_size() {
        let cursor = CursorState::new(480, 272);
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert_eq!(obj.w, CURSOR_W);
        assert_eq!(obj.h, CURSOR_H);
    }

    #[test]
    fn cursor_sdi_is_always_on_top() {
        let cursor = CursorState::new(480, 272);
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert_eq!(obj.z, 10000);
    }

    #[test]
    fn cursor_hide_sdi_hides_cursor() {
        let cursor = CursorState::new(480, 272);
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        assert!(sdi.get(CURSOR_SDI_NAME).unwrap().visible);
        CursorState::hide_sdi(&mut sdi);
        assert!(!sdi.get(CURSOR_SDI_NAME).unwrap().visible);
    }

    #[test]
    fn cursor_respects_visibility_flag() {
        let mut cursor = CursorState::new(480, 272);
        cursor.visible = false;
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert!(!obj.visible);
    }

    #[test]
    fn generate_cursor_has_transparent_pixels() {
        let (pixels, w, _) = generate_cursor_pixels(1);
        // Pixel at (5,0) should be transparent.
        let offset = (0 * w + 5) as usize * 4;
        assert_eq!(pixels[offset + 3], 0); // Alpha = 0
    }

    #[test]
    fn generate_cursor_has_black_outline() {
        let (pixels, w, _) = generate_cursor_pixels(1);
        // Pixel at (1,1) should be black outline.
        let offset = (1 * w + 1) as usize * 4;
        assert_eq!(pixels[offset], 0);
        assert_eq!(pixels[offset + 1], 0);
        assert_eq!(pixels[offset + 2], 0);
        assert_eq!(pixels[offset + 3], 255);
    }

    #[test]
    fn cursor_button_press_ignored() {
        let mut cursor = CursorState::new(480, 272);
        let original_x = cursor.x;
        let original_y = cursor.y;
        cursor.handle_input(&InputEvent::ButtonPress(crate::input::Button::Confirm));
        assert_eq!(cursor.x, original_x);
        assert_eq!(cursor.y, original_y);
    }

    #[test]
    fn cursor_multiple_moves() {
        let mut cursor = CursorState::new(480, 272);
        cursor.handle_input(&InputEvent::CursorMove { x: 10, y: 20 });
        assert_eq!(cursor.x, 10);
        assert_eq!(cursor.y, 20);
        cursor.handle_input(&InputEvent::CursorMove { x: 30, y: 40 });
        assert_eq!(cursor.x, 30);
        assert_eq!(cursor.y, 40);
    }

    #[test]
    fn cursor_negative_coordinates() {
        let mut cursor = CursorState::new(480, 272);
        cursor.set_position(-10, -20);
        assert_eq!(cursor.x, -10);
        assert_eq!(cursor.y, -20);
    }

    #[test]
    fn cursor_large_coordinates() {
        let mut cursor = CursorState::new(480, 272);
        cursor.set_position(10000, 10000);
        assert_eq!(cursor.x, 10000);
        assert_eq!(cursor.y, 10000);
    }

    #[test]
    fn generate_cursor_scale_2() {
        let (pixels, w, h) = generate_cursor_pixels(2);
        assert_eq!(w, CURSOR_W * 2);
        assert_eq!(h, CURSOR_H * 2);
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        // Top-left 2x2 block should all be black outline.
        for dy in 0..2u32 {
            for dx in 0..2u32 {
                let offset = (dy * w + dx) as usize * 4;
                assert_eq!(pixels[offset + 3], 255, "alpha at ({dx},{dy})");
                assert_eq!(pixels[offset], 0, "red at ({dx},{dy})");
            }
        }
    }

    #[test]
    fn generate_cursor_scale_3() {
        let (pixels, w, h) = generate_cursor_pixels(3);
        assert_eq!(w, CURSOR_W * 3);
        assert_eq!(h, CURSOR_H * 3);
        assert_eq!(pixels.len(), (w * h * 4) as usize);
    }

    #[test]
    fn cursor_sdi_scaled_size() {
        let mut cursor = CursorState::new(1024, 768);
        cursor.scale = 3;
        let mut sdi = SdiRegistry::new();
        cursor.update_sdi(&mut sdi);
        let obj = sdi.get(CURSOR_SDI_NAME).unwrap();
        assert_eq!(obj.w, CURSOR_W * 3);
        assert_eq!(obj.h, CURSOR_H * 3);
    }
}

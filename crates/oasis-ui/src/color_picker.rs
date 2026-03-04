//! ColorPicker widget: HSV color selection with RGB preview.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

/// Default size of the hue/saturation area.
const SV_SIZE: u32 = 80;

/// Width of the hue bar.
const HUE_BAR_WIDTH: u32 = 14;

/// Gap between SV area and hue bar.
const GAP: u32 = 4;

/// Height of the preview swatch.
const PREVIEW_HEIGHT: u32 = 16;

/// A color picker using HSV model.
pub struct ColorPicker {
    /// Hue (0.0 to 360.0).
    pub hue: f32,
    /// Saturation (0.0 to 1.0).
    pub saturation: f32,
    /// Value/brightness (0.0 to 1.0).
    pub value: f32,
    /// Whether the picker is disabled.
    pub disabled: bool,
}

impl Default for ColorPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorPicker {
    /// Create a new color picker with default red.
    pub fn new() -> Self {
        Self {
            hue: 0.0,
            saturation: 1.0,
            value: 1.0,
            disabled: false,
        }
    }

    /// Create from an RGB color (approximate conversion).
    pub fn from_rgb(c: Color) -> Self {
        let (h, s, v) = rgb_to_hsv(c.r, c.g, c.b);
        Self {
            hue: h,
            saturation: s,
            value: v,
            disabled: false,
        }
    }

    /// Get the currently selected color as RGB.
    pub fn color(&self) -> Color {
        let (r, g, b) = hsv_to_rgb(self.hue, self.saturation, self.value);
        Color::rgb(r, g, b)
    }

    /// Set hue (clamped to 0..360).
    pub fn set_hue(&mut self, h: f32) {
        if !self.disabled {
            self.hue = h.clamp(0.0, 360.0);
        }
    }

    /// Set saturation (clamped to 0..1).
    pub fn set_saturation(&mut self, s: f32) {
        if !self.disabled {
            self.saturation = s.clamp(0.0, 1.0);
        }
    }

    /// Set value/brightness (clamped to 0..1).
    pub fn set_value(&mut self, v: f32) {
        if !self.disabled {
            self.value = v.clamp(0.0, 1.0);
        }
    }

    /// Hex string representation of the current color.
    pub fn hex_string(&self) -> String {
        let c = self.color();
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    }
}

/// Convert HSV to RGB.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

/// Convert RGB to HSV.
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { delta / max };

    (h, s, max)
}

impl Widget for ColorPicker {
    fn measure(&self, _ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let w = SV_SIZE + GAP + HUE_BAR_WIDTH;
        let h = SV_SIZE + GAP + PREVIEW_HEIGHT;
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, _w: u32, _h: u32) -> Result<()> {
        let border = ctx.theme.interactive_border(self.disabled, false);

        // Draw saturation-value area as a simplified gradient.
        // Top-left = white, top-right = hue at full saturation, bottom = black.
        let sv_steps = 8u32;
        let cell_w = SV_SIZE / sv_steps;
        let cell_h = SV_SIZE / sv_steps;
        for sy in 0..sv_steps {
            for sx in 0..sv_steps {
                let s = sx as f32 / (sv_steps - 1) as f32;
                let v = 1.0 - sy as f32 / (sv_steps - 1) as f32;
                let (r, g, b) = hsv_to_rgb(self.hue, s, v);
                let cx = x + (sx * cell_w) as i32;
                let cy = y + (sy * cell_h) as i32;
                ctx.backend
                    .fill_rect(cx, cy, cell_w, cell_h, Color::rgb(r, g, b))?;
            }
        }
        ctx.backend.stroke_rect(x, y, SV_SIZE, SV_SIZE, 1, border)?;

        // Cursor in SV area.
        let cur_x = x + (self.saturation * (SV_SIZE - 1) as f32) as i32;
        let cur_y = y + ((1.0 - self.value) * (SV_SIZE - 1) as f32) as i32;
        ctx.backend
            .stroke_rect(cur_x - 2, cur_y - 2, 5, 5, 1, Color::WHITE)?;

        // Hue bar.
        let hue_x = x + SV_SIZE as i32 + GAP as i32;
        let hue_steps = 12u32;
        let step_h = SV_SIZE / hue_steps;
        for i in 0..hue_steps {
            let h = i as f32 * 360.0 / hue_steps as f32;
            let (r, g, b) = hsv_to_rgb(h, 1.0, 1.0);
            let hy = y + (i * step_h) as i32;
            ctx.backend
                .fill_rect(hue_x, hy, HUE_BAR_WIDTH, step_h, Color::rgb(r, g, b))?;
        }
        ctx.backend
            .stroke_rect(hue_x, y, HUE_BAR_WIDTH, SV_SIZE, 1, border)?;

        // Hue cursor.
        let hue_cursor_y = y + (self.hue / 360.0 * (SV_SIZE - 1) as f32) as i32;
        ctx.backend
            .fill_rect(hue_x - 1, hue_cursor_y, HUE_BAR_WIDTH + 2, 2, Color::WHITE)?;

        // Preview swatch.
        let preview_y = y + SV_SIZE as i32 + GAP as i32;
        let preview_w = SV_SIZE + GAP + HUE_BAR_WIDTH;
        ctx.backend
            .fill_rect(x, preview_y, preview_w, PREVIEW_HEIGHT, self.color())?;
        ctx.backend
            .stroke_rect(x, preview_y, preview_w, PREVIEW_HEIGHT, 1, border)?;

        // Hex label.
        let fs = ctx.theme.font_size_sm;
        let text_h = ctx.backend.measure_text_height(fs);
        let hex = self.hex_string();
        let tx = x + layout::center(preview_w, ctx.backend.measure_text(&hex, fs));
        let ty = preview_y + layout::center(PREVIEW_HEIGHT, text_h);
        let text_color = ctx.theme.interactive_text(self.disabled);
        ctx.backend.draw_text(&hex, tx, ty, fs, text_color)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_red() {
        let cp = ColorPicker::new();
        assert_eq!(cp.hue, 0.0);
        assert_eq!(cp.saturation, 1.0);
        assert_eq!(cp.value, 1.0);
        let c = cp.color();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn from_rgb_roundtrip() {
        let original = Color::rgb(100, 150, 200);
        let cp = ColorPicker::from_rgb(original);
        let back = cp.color();
        // Allow small rounding errors.
        assert!((back.r as i32 - original.r as i32).abs() <= 2);
        assert!((back.g as i32 - original.g as i32).abs() <= 2);
        assert!((back.b as i32 - original.b as i32).abs() <= 2);
    }

    #[test]
    fn set_hue_clamps() {
        let mut cp = ColorPicker::new();
        cp.set_hue(400.0);
        assert_eq!(cp.hue, 360.0);
        cp.set_hue(-10.0);
        assert_eq!(cp.hue, 0.0);
    }

    #[test]
    fn disabled_ignores_changes() {
        let mut cp = ColorPicker::new();
        cp.disabled = true;
        cp.set_hue(180.0);
        assert_eq!(cp.hue, 0.0);
        cp.set_saturation(0.5);
        assert_eq!(cp.saturation, 1.0);
        cp.set_value(0.5);
        assert_eq!(cp.value, 1.0);
    }

    #[test]
    fn hex_string_format() {
        let cp = ColorPicker::new();
        assert_eq!(cp.hex_string(), "#FF0000");
    }

    #[test]
    fn hsv_black() {
        let (r, g, b) = hsv_to_rgb(0.0, 0.0, 0.0);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn hsv_white() {
        let (r, g, b) = hsv_to_rgb(0.0, 0.0, 1.0);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn rgb_to_hsv_black() {
        let (_h, s, v) = rgb_to_hsv(0, 0, 0);
        assert_eq!(v, 0.0);
        assert_eq!(s, 0.0);
    }

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn draw_shows_hex() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let cp = ColorPicker::new();
            cp.draw(&mut ctx, 0, 0, 100, 100).unwrap();
        }
        assert!(backend.has_text("#FF0000"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let cp = ColorPicker::new();
            cp.draw(ctx, 0, 0, 100, 100).unwrap();
        });
    }
}

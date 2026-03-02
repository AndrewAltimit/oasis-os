//! Checkbox widget.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A checkbox with a label.
pub struct Checkbox {
    /// Whether the checkbox is checked.
    pub checked: bool,
    /// Label text displayed next to the checkbox.
    pub label: String,
    /// Whether the checkbox is disabled.
    pub disabled: bool,
}

/// Size of the checkbox box in pixels.
const BOX_SIZE: u32 = 14;

/// Gap between box and label.
const LABEL_GAP: u32 = 6;

impl Checkbox {
    /// Create a new checkbox.
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            checked,
            label: label.into(),
            disabled: false,
        }
    }

    /// Toggle the checked state. Returns the new state.
    pub fn toggle(&mut self) -> bool {
        if !self.disabled {
            self.checked = !self.checked;
        }
        self.checked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_checked() {
        let c = Checkbox::new("Enable", true);
        assert!(c.checked);
        assert_eq!(c.label, "Enable");
        assert!(!c.disabled);
    }

    #[test]
    fn new_unchecked() {
        let c = Checkbox::new("Enable", false);
        assert!(!c.checked);
    }

    #[test]
    fn toggle_flips() {
        let mut c = Checkbox::new("Test", false);
        assert!(c.toggle());
        assert!(c.checked);
        assert!(!c.toggle());
        assert!(!c.checked);
    }

    #[test]
    fn toggle_disabled_noop() {
        let mut c = Checkbox::new("Test", false);
        c.disabled = true;
        assert!(!c.toggle());
        assert!(!c.checked);
    }

    #[test]
    fn from_string() {
        let c = Checkbox::new(String::from("Dynamic"), true);
        assert_eq!(c.label, "Dynamic");
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_includes_box_and_label() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let c = Checkbox::new("Hello", false);
        let (w, h) = c.measure(&ctx, 200, 100);
        assert!(w > BOX_SIZE, "width should include label");
        assert!(h >= BOX_SIZE, "height should be at least box size");
    }

    #[test]
    fn draw_unchecked_shows_label() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let c = Checkbox::new("Accept Terms", false);
            c.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.has_text("Accept Terms"));
        // Empty box = fill_rect for bg + stroke for border
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_checked_shows_checkmark() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let c = Checkbox::new("Accept", true);
            c.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        // Checkmark character
        assert!(backend.has_text("\u{2713}"));
    }

    #[test]
    fn draw_disabled_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut c = Checkbox::new("Off", false);
            c.disabled = true;
            c.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.has_text("Off"));
    }

    #[test]
    fn draw_empty_label() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let c = Checkbox::new("", true);
            c.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        // Should not panic even with empty label.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let c = Checkbox::new("Test", true);
            c.draw(ctx, 0, 0, 200, 20).unwrap();
        });
    }
}

impl Widget for Checkbox {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;
        let text_w = ctx.backend.measure_text(&self.label, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        let h = BOX_SIZE.max(text_h);
        let w = BOX_SIZE + LABEL_GAP + text_w;
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, _w: u32, h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let radius = ctx.theme.border_radius_sm;
        let box_y = y + layout::center(h, BOX_SIZE);

        // Box background.
        let box_bg = if self.checked {
            ctx.theme.accent
        } else {
            ctx.theme.input_bg
        };
        ctx.backend
            .fill_rounded_rect(x, box_y, BOX_SIZE, BOX_SIZE, radius, box_bg)?;

        // Box border.
        let border_color = ctx.theme.interactive_border(self.disabled, self.checked);
        ctx.backend
            .stroke_rounded_rect(x, box_y, BOX_SIZE, BOX_SIZE, radius, 1, border_color)?;

        // Checkmark.
        if self.checked {
            let check_fs = fs.min(10);
            let ch_w = ctx.backend.measure_text("\u{2713}", check_fs);
            let ch_h = ctx.backend.measure_text_height(check_fs);
            let cx = x + layout::center(BOX_SIZE, ch_w);
            let cy = box_y + layout::center(BOX_SIZE, ch_h);
            ctx.backend
                .draw_text("\u{2713}", cx, cy, check_fs, ctx.theme.text_on_accent)?;
        }

        // Label.
        if !self.label.is_empty() {
            let text_h = ctx.backend.measure_text_height(fs);
            let tx = x + BOX_SIZE as i32 + LABEL_GAP as i32;
            let ty = y + layout::center(h, text_h);
            ctx.backend.draw_text(
                &self.label,
                tx,
                ty,
                fs,
                ctx.theme.interactive_text(self.disabled),
            )?;
        }

        Ok(())
    }
}

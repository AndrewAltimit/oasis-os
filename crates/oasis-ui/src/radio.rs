//! RadioGroup widget: mutually exclusive option selection.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A group of radio buttons where exactly one option is selected.
///
/// # Example
///
/// ```ignore
/// let mut radio = RadioGroup::new(vec![
///     "Small".into(), "Medium".into(), "Large".into(),
/// ]);
/// assert_eq!(radio.selected, 0);
/// radio.select(1);
/// assert_eq!(radio.options[radio.selected], "Medium");
/// radio.select_next(); // wraps around
/// assert_eq!(radio.selected, 2);
/// ```
pub struct RadioGroup {
    /// Option labels.
    pub options: Vec<String>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Whether the group is disabled.
    pub disabled: bool,
}

/// Diameter of the radio circle in pixels.
const CIRCLE_SIZE: u32 = 14;

use crate::layout::LABEL_GAP;

/// Vertical spacing between radio options.
const ITEM_SPACING: u32 = 4;

impl RadioGroup {
    /// Create a new radio group.
    pub fn new(options: Vec<String>) -> Self {
        Self {
            options,
            selected: 0,
            disabled: false,
        }
    }

    /// Select the option at the given index (clamped to bounds).
    pub fn select(&mut self, index: usize) {
        if !self.disabled && index < self.options.len() {
            self.selected = index;
        }
    }

    /// Select the next option (wrapping).
    pub fn select_next(&mut self) {
        if !self.disabled && !self.options.is_empty() {
            self.selected = (self.selected + 1) % self.options.len();
        }
    }

    /// Select the previous option (wrapping).
    pub fn select_prev(&mut self) {
        if !self.disabled && !self.options.is_empty() {
            self.selected = if self.selected == 0 {
                self.options.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Return the label of the currently selected option.
    pub fn selected_label(&self) -> Option<&str> {
        self.options.get(self.selected).map(String::as_str)
    }

    /// Height of a single radio item row.
    fn row_height(ctx: &DrawContext<'_>) -> u32 {
        let text_h = ctx.backend.measure_text_height(ctx.theme.font_size_md);
        CIRCLE_SIZE.max(text_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<String> {
        vec!["Small".into(), "Medium".into(), "Large".into()]
    }

    #[test]
    fn new_defaults() {
        let r = RadioGroup::new(sample());
        assert_eq!(r.selected, 0);
        assert!(!r.disabled);
        assert_eq!(r.options.len(), 3);
    }

    #[test]
    fn select_valid_index() {
        let mut r = RadioGroup::new(sample());
        r.select(2);
        assert_eq!(r.selected, 2);
    }

    #[test]
    fn select_out_of_bounds_noop() {
        let mut r = RadioGroup::new(sample());
        r.select(10);
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn select_disabled_noop() {
        let mut r = RadioGroup::new(sample());
        r.disabled = true;
        r.select(2);
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn select_next_wraps() {
        let mut r = RadioGroup::new(sample());
        r.select_next();
        assert_eq!(r.selected, 1);
        r.select_next();
        assert_eq!(r.selected, 2);
        r.select_next();
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn select_prev_wraps() {
        let mut r = RadioGroup::new(sample());
        r.select_prev();
        assert_eq!(r.selected, 2);
    }

    #[test]
    fn select_next_empty_noop() {
        let mut r = RadioGroup::new(vec![]);
        r.select_next();
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn select_next_disabled_noop() {
        let mut r = RadioGroup::new(sample());
        r.disabled = true;
        r.select_next();
        assert_eq!(r.selected, 0);
    }

    #[test]
    fn selected_label_returns_correct() {
        let mut r = RadioGroup::new(sample());
        assert_eq!(r.selected_label(), Some("Small"));
        r.select(1);
        assert_eq!(r.selected_label(), Some("Medium"));
    }

    #[test]
    fn selected_label_empty_options() {
        let r = RadioGroup::new(vec![]);
        assert_eq!(r.selected_label(), None);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_accounts_for_all_items() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let r = RadioGroup::new(sample());
        let (w, h) = r.measure(&ctx, 200, 300);
        let row_h = RadioGroup::row_height(&ctx);
        assert!(w > CIRCLE_SIZE, "width should include labels");
        // 3 rows + 2 gaps
        let expected_h = row_h * 3 + ITEM_SPACING * 2;
        assert_eq!(h, expected_h);
    }

    #[test]
    fn measure_empty_zero_height() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let r = RadioGroup::new(vec![]);
        let (_, h) = r.measure(&ctx, 200, 300);
        assert_eq!(h, 0);
    }

    #[test]
    fn draw_shows_all_labels() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let r = RadioGroup::new(sample());
            r.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Small"));
        assert!(backend.has_text("Medium"));
        assert!(backend.has_text("Large"));
    }

    #[test]
    fn draw_selected_item_has_filled_dot() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let r = RadioGroup::new(sample());
            r.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Selected item (index 0) gets an extra fill_rect for the inner dot.
        // Unselected items only get the outer circle (stroke).
        // We verify by checking there are more fill_rects than just outer circles.
        assert!(backend.fill_rect_count() > 3);
    }

    #[test]
    fn draw_empty_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let r = RadioGroup::new(vec![]);
            r.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn draw_disabled_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut r = RadioGroup::new(sample());
            r.disabled = true;
            r.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Small"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let r = RadioGroup::new(sample());
            r.draw(ctx, 0, 0, 200, 100).unwrap();
        });
    }
}

impl Widget for RadioGroup {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        if self.options.is_empty() {
            return (0, 0);
        }
        let fs = ctx.theme.font_size_md;
        let row_h = Self::row_height(ctx);

        let max_label_w = self
            .options
            .iter()
            .map(|o| ctx.backend.measure_text(o, fs))
            .max()
            .unwrap_or(0);
        let w = CIRCLE_SIZE + LABEL_GAP + max_label_w;
        let h = row_h * self.options.len() as u32
            + ITEM_SPACING * self.options.len().saturating_sub(1) as u32;
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, _w: u32, _h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let row_h = Self::row_height(ctx);
        let text_h = ctx.backend.measure_text_height(fs);
        let r = CIRCLE_SIZE / 2;

        for (i, option) in self.options.iter().enumerate() {
            let iy = y + (i as u32 * (row_h + ITEM_SPACING)) as i32;
            let is_selected = i == self.selected;

            // Outer circle (as rounded rect with full radius).
            let circle_y = iy + layout::center(row_h, CIRCLE_SIZE);
            let border_color = ctx.theme.interactive_border(self.disabled, is_selected);
            ctx.backend.stroke_rounded_rect(
                x,
                circle_y,
                CIRCLE_SIZE,
                CIRCLE_SIZE,
                r as u16,
                1,
                border_color,
            )?;

            // Inner filled dot for selected.
            if is_selected {
                let dot_size = CIRCLE_SIZE.saturating_sub(6);
                let dot_r = dot_size / 2;
                let dot_x = x + layout::center(CIRCLE_SIZE, dot_size);
                let dot_y = circle_y + layout::center(CIRCLE_SIZE, dot_size);
                ctx.backend.fill_rounded_rect(
                    dot_x,
                    dot_y,
                    dot_size,
                    dot_size,
                    dot_r as u16,
                    ctx.theme.interactive_accent(self.disabled),
                )?;
            }

            // Label text.
            let tx = x + CIRCLE_SIZE as i32 + LABEL_GAP as i32;
            let ty = iy + layout::center(row_h, text_h);
            ctx.backend.draw_text(
                option,
                tx,
                ty,
                fs,
                ctx.theme.interactive_text(self.disabled),
            )?;
        }

        Ok(())
    }
}

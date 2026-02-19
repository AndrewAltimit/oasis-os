//! Dropdown / combobox widget.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A dropdown selector that shows a list of options when opened.
pub struct Dropdown {
    /// Available options.
    pub options: Vec<String>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Whether the dropdown menu is open.
    pub open: bool,
    /// Placeholder text shown when no option is selected.
    pub placeholder: String,
}

impl Dropdown {
    /// Create a new dropdown with the given options.
    pub fn new(options: Vec<String>) -> Self {
        Self {
            options,
            selected: 0,
            open: false,
            placeholder: String::new(),
        }
    }

    /// Return the currently selected option text, or the placeholder.
    pub fn selected_text(&self) -> &str {
        self.options
            .get(self.selected)
            .map(String::as_str)
            .unwrap_or(&self.placeholder)
    }

    /// Select the next option (wrapping).
    pub fn select_next(&mut self) {
        if !self.options.is_empty() {
            self.selected = (self.selected + 1) % self.options.len();
        }
    }

    /// Select the previous option (wrapping).
    pub fn select_prev(&mut self) {
        if !self.options.is_empty() {
            self.selected = if self.selected == 0 {
                self.options.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Toggle the open/closed state.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Height of each item row.
    fn row_height(ctx: &DrawContext<'_>) -> u32 {
        ctx.backend.measure_text_height(ctx.theme.font_size_md) + 6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_options() -> Vec<String> {
        vec!["Alpha".into(), "Beta".into(), "Gamma".into()]
    }

    #[test]
    fn new_defaults() {
        let d = Dropdown::new(sample_options());
        assert_eq!(d.selected, 0);
        assert!(!d.open);
        assert!(d.placeholder.is_empty());
        assert_eq!(d.options.len(), 3);
    }

    #[test]
    fn selected_text_returns_current() {
        let d = Dropdown::new(sample_options());
        assert_eq!(d.selected_text(), "Alpha");
    }

    #[test]
    fn selected_text_fallback_to_placeholder() {
        let mut d = Dropdown::new(vec![]);
        d.placeholder = "Pick one".into();
        assert_eq!(d.selected_text(), "Pick one");
    }

    #[test]
    fn select_next_wraps() {
        let mut d = Dropdown::new(sample_options());
        d.select_next();
        assert_eq!(d.selected, 1);
        d.select_next();
        assert_eq!(d.selected, 2);
        d.select_next();
        assert_eq!(d.selected, 0); // wraps
    }

    #[test]
    fn select_prev_wraps() {
        let mut d = Dropdown::new(sample_options());
        d.select_prev();
        assert_eq!(d.selected, 2); // wraps from 0 to last
        d.select_prev();
        assert_eq!(d.selected, 1);
    }

    #[test]
    fn select_next_empty_noop() {
        let mut d = Dropdown::new(vec![]);
        d.select_next();
        assert_eq!(d.selected, 0);
    }

    #[test]
    fn select_prev_empty_noop() {
        let mut d = Dropdown::new(vec![]);
        d.select_prev();
        assert_eq!(d.selected, 0);
    }

    #[test]
    fn toggle_open_close() {
        let mut d = Dropdown::new(sample_options());
        assert!(!d.open);
        d.toggle();
        assert!(d.open);
        d.toggle();
        assert!(!d.open);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_closed_single_row() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let d = Dropdown::new(sample_options());
        let (w, h) = d.measure(&ctx, 200, 300);
        assert_eq!(w, 200);
        let row_h = Dropdown::row_height(&ctx);
        assert_eq!(h, row_h);
    }

    #[test]
    fn measure_open_includes_menu() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut d = Dropdown::new(sample_options());
        d.open = true;
        let (_, h) = d.measure(&ctx, 200, 300);
        let row_h = Dropdown::row_height(&ctx);
        // header + 3 option rows
        assert_eq!(h, row_h + row_h * 3);
    }

    #[test]
    fn draw_closed_shows_selected_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let d = Dropdown::new(sample_options());
            d.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.has_text("Alpha"));
        // Arrow indicator
        assert!(backend.has_text("\u{25BC}"));
    }

    #[test]
    fn draw_open_shows_all_options() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Dropdown::new(sample_options());
            d.open = true;
            d.draw(&mut ctx, 0, 0, 200, 80).unwrap();
        }
        assert!(backend.has_text("Alpha"));
        assert!(backend.has_text("Beta"));
        assert!(backend.has_text("Gamma"));
    }

    #[test]
    fn draw_empty_options_shows_placeholder() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Dropdown::new(vec![]);
            d.placeholder = "Select...".into();
            d.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        assert!(backend.has_text("Select..."));
    }

    #[test]
    fn draw_selected_option_highlighted() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Dropdown::new(sample_options());
            d.selected = 1;
            d.open = true;
            d.draw(&mut ctx, 0, 0, 200, 80).unwrap();
        }
        // The selected item row gets a highlight fill_rect
        assert!(backend.fill_rect_count() > 2);
        assert!(backend.has_text("Beta"));
    }

    #[test]
    fn draw_closed_no_menu_items() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut d = Dropdown::new(sample_options());
            d.selected = 0;
            d.draw(&mut ctx, 0, 0, 200, 20).unwrap();
        }
        // Only "Alpha" and the arrow should be drawn, not Beta/Gamma
        assert!(!backend.has_text("Beta"));
        assert!(!backend.has_text("Gamma"));
    }
}

impl Widget for Dropdown {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let row_h = Self::row_height(ctx);
        let h = if self.open {
            row_h + row_h * self.options.len() as u32
        } else {
            row_h
        };
        (available_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let radius = ctx.theme.border_radius_md;
        let row_h = Self::row_height(ctx);
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let ty_off = layout::center(row_h, text_h);

        // -- Header row --
        ctx.backend
            .fill_rounded_rect(x, y, w, row_h, radius, ctx.theme.input_bg)?;
        ctx.backend
            .stroke_rounded_rect(x, y, w, row_h, radius, 1, ctx.theme.input_border)?;

        let label = self.selected_text();
        ctx.backend.draw_text_ellipsis(
            label,
            x + 6,
            y + ty_off,
            fs,
            if self.options.is_empty() {
                ctx.theme.text_disabled
            } else {
                ctx.theme.text_primary
            },
            w.saturating_sub(20),
        )?;

        // Down arrow.
        let arrow = if self.open { "\u{25B2}" } else { "\u{25BC}" };
        let arrow_w = ctx.backend.measure_text(arrow, fs);
        ctx.backend.draw_text(
            arrow,
            x + w as i32 - arrow_w as i32 - 4,
            y + ty_off,
            fs,
            ctx.theme.text_secondary,
        )?;

        // -- Menu panel (only when open) --
        if self.open && !self.options.is_empty() {
            let menu_y = y + row_h as i32;
            let menu_h = h.saturating_sub(row_h);

            // Shadow + background.
            ctx.theme
                .shadow_dropdown
                .draw(ctx.backend, x, menu_y, w, menu_h, radius)?;
            ctx.backend
                .fill_rounded_rect(x, menu_y, w, menu_h, radius, ctx.theme.surface)?;
            ctx.backend.stroke_rounded_rect(
                x,
                menu_y,
                w,
                menu_h,
                radius,
                1,
                ctx.theme.border_subtle,
            )?;

            for (i, option) in self.options.iter().enumerate() {
                let iy = menu_y + (i as u32 * row_h) as i32;

                // Highlight selected row.
                if i == self.selected {
                    ctx.backend.fill_rect(
                        x + 1,
                        iy,
                        w.saturating_sub(2),
                        row_h,
                        ctx.theme.accent_subtle,
                    )?;
                }

                ctx.backend.draw_text_ellipsis(
                    option,
                    x + 6,
                    iy + ty_off,
                    fs,
                    if i == self.selected {
                        ctx.theme.accent
                    } else {
                        ctx.theme.text_primary
                    },
                    w.saturating_sub(12),
                )?;
            }
        }

        Ok(())
    }
}

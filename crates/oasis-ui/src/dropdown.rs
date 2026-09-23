//! Dropdown / combobox widget.

use crate::context::DrawContext;
use crate::layout;
use crate::states::{WidgetState, WidgetStateColors};
use crate::widget::Widget;
use oasis_types::error::Result;

/// A dropdown selector that shows a list of options when opened.
///
/// # Example
///
/// ```ignore
/// let mut dd = Dropdown::new(vec![
///     "Option A".into(), "Option B".into(), "Option C".into(),
/// ]);
/// assert_eq!(dd.selected_text(), "Option A");
/// dd.toggle(); // open menu
/// dd.select_next();
/// assert_eq!(dd.selected_text(), "Option B");
/// dd.toggle(); // close menu
/// ```
pub struct Dropdown {
    /// Available options.
    pub options: Vec<String>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Whether the dropdown menu is open.
    pub open: bool,
    /// Placeholder text shown when no option is selected.
    pub placeholder: String,
    /// Whether the dropdown has keyboard focus (rings the header row).
    pub focused: bool,
    /// Whether the pointer is over the header row.
    pub hovered: bool,
    /// Whether the header row is being pressed.
    pub pressed: bool,
    /// Whether the dropdown is disabled ([`toggle`](Self::toggle) is a
    /// no-op and the header is greyed out).
    pub disabled: bool,
    /// Index of the open menu's option under the pointer, if any.
    pub hovered_index: Option<usize>,
}

impl Dropdown {
    /// Create a new dropdown with the given options.
    pub fn new(options: Vec<String>) -> Self {
        Self {
            options,
            selected: 0,
            open: false,
            placeholder: String::new(),
            focused: false,
            hovered: false,
            pressed: false,
            disabled: false,
            hovered_index: None,
        }
    }

    /// Resolved interaction state of the header row.
    pub fn state(&self) -> WidgetState {
        WidgetState::from_flags(self.hovered, self.pressed, self.disabled)
    }

    /// Index of the open menu option containing local offset `dy`
    /// (pixels from the top of the widget, i.e. including the header).
    pub fn option_at(&self, ctx: &DrawContext<'_>, dy: i32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let row_h = Self::row_height(ctx) as i32;
        let rel = dy - row_h;
        if rel < 0 {
            return None;
        }
        let i = (rel / row_h) as usize;
        (i < self.options.len()).then_some(i)
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
        if self.disabled {
            self.open = false;
            return;
        }
        self.open = !self.open;
        if !self.open {
            self.hovered_index = None;
        }
    }

    /// Height of each item row.
    fn row_height(ctx: &DrawContext<'_>) -> u32 {
        ctx.backend.measure_text_height(ctx.theme.font_size_md) + 6
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
        let state = self.state();
        ctx.backend.fill_rounded_rect(
            x,
            y,
            w,
            row_h,
            radius,
            WidgetStateColors::input_bg(ctx.theme, state),
        )?;
        let header_border = if state.is_disabled() {
            WidgetStateColors::border(ctx.theme, state)
        } else {
            ctx.theme.input_border
        };
        ctx.backend
            .stroke_rounded_rect(x, y, w, row_h, radius, 1, header_border)?;

        // Keyboard focus ring around the header row.
        if self.focused && !self.disabled {
            crate::focus::FocusStyle::from_theme(ctx.theme).draw(ctx.backend, x, y, w, row_h)?;
        }

        let label = self.selected_text();
        ctx.backend.draw_text_ellipsis(
            label,
            x + 6,
            y + ty_off,
            fs,
            if self.options.is_empty() || self.disabled {
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

                // Highlight selected / hovered row.
                let hovered = self.hovered_index == Some(i);
                if let Some(fill) =
                    WidgetStateColors::row_bg(ctx.theme, i == self.selected, hovered)
                {
                    ctx.backend
                        .fill_rect(x + 1, iy, w.saturating_sub(2), row_h, fill)?;
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

    #[test]
    fn each_header_state_has_distinct_fill() {
        crate::test_utils::assert_states_distinct(|st, ctx| {
            let mut d = Dropdown::new(sample_options());
            d.hovered = st == WidgetState::Hover;
            d.pressed = st == WidgetState::Pressed;
            d.disabled = st == WidgetState::Disabled;
            d.draw(ctx, 0, 0, 200, 20).unwrap();
        });
    }

    #[test]
    fn hovered_row_is_highlighted() {
        let theme = Theme::dark();
        let draw = |hover: Option<usize>| {
            crate::test_utils::fill_colors_of(|ctx| {
                let mut d = Dropdown::new(sample_options());
                d.open = true;
                d.hovered_index = hover;
                d.draw(ctx, 0, 0, 200, 80).unwrap();
            })
        };
        let rest = draw(None);
        let hovered = draw(Some(2));
        let hover_fill = WidgetStateColors::row_bg(&theme, false, true).unwrap();
        assert!(!rest.contains(&hover_fill));
        assert!(hovered.contains(&hover_fill));
    }

    #[test]
    fn option_at_maps_rows() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut d = Dropdown::new(sample_options());
        let row_h = Dropdown::row_height(&ctx) as i32;
        assert_eq!(d.option_at(&ctx, row_h + 1), None, "closed");
        d.open = true;
        assert_eq!(d.option_at(&ctx, 1), None, "header");
        assert_eq!(d.option_at(&ctx, row_h + 1), Some(0));
        assert_eq!(d.option_at(&ctx, row_h * 3 + 1), Some(2));
        assert_eq!(d.option_at(&ctx, row_h * 9), None);
    }

    #[test]
    fn disabled_toggle_stays_closed() {
        let mut d = Dropdown::new(sample_options());
        d.disabled = true;
        d.toggle();
        assert!(!d.open);
    }
}

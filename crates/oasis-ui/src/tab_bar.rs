//! TabBar widget.

use crate::context::DrawContext;
use crate::layout;
use crate::states::{WidgetState, WidgetStateColors};
use crate::widget::Widget;
use oasis_types::error::Result;

/// Tab visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStyle {
    /// Active tab has bottom underline.
    Underline,
    /// Active tab is filled with background.
    Filled,
    /// Pill-shaped tabs.
    Pill,
}

/// A horizontal tab strip.
pub struct TabBar {
    /// Tab labels.
    pub tabs: Vec<String>,
    /// Index of active tab.
    pub active: usize,
    /// Visual style variant.
    pub style: TabStyle,
    /// Whether the tab bar is disabled (non-interactive).
    pub disabled: bool,
    /// Whether the tab bar has keyboard focus (rings the active tab).
    pub focused: bool,
    /// Index of the tab under the pointer, if any.
    pub hovered: Option<usize>,
    /// Whether the hovered tab is being pressed.
    pub pressed: bool,
}

impl TabBar {
    /// Create a new tab bar.
    pub fn new(tabs: Vec<String>) -> Self {
        Self {
            tabs,
            active: 0,
            style: TabStyle::Underline,
            disabled: false,
            focused: false,
            hovered: None,
            pressed: false,
        }
    }

    /// Resolved interaction state of tab `index`.
    pub fn tab_state(&self, index: usize) -> WidgetState {
        let hover = self.hovered == Some(index);
        WidgetState::from_flags(hover, hover && self.pressed, self.disabled)
    }

    /// Index of the tab containing local offset `dx` (pixels from the
    /// bar's left edge) for a bar drawn `w` pixels wide.
    pub fn tab_at(&self, dx: i32, w: u32) -> Option<usize> {
        if self.tabs.is_empty() || dx < 0 || dx >= w as i32 {
            return None;
        }
        let n = self.tabs.len() as u32;
        let (tab_w, remainder) = (w / n, w % n);
        // Mirror `draw`: the first `remainder` tabs are 1px wider.
        let wide = remainder * (tab_w + 1);
        let dx = dx as u32;
        let i = if dx < wide {
            dx / (tab_w + 1)
        } else {
            remainder + (dx - wide).checked_div(tab_w)?
        };
        ((i as usize) < self.tabs.len()).then_some(i as usize)
    }

    /// Select a tab by index (respects disabled state).
    pub fn select(&mut self, index: usize) {
        if !self.disabled && index < self.tabs.len() {
            self.active = index;
        }
    }
}

impl Widget for TabBar {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let h = ctx.backend.measure_text_height(ctx.theme.font_size_md) + 8;
        (available_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        if self.tabs.is_empty() {
            return Ok(());
        }
        let n = self.tabs.len() as u32;
        let tab_w = w / n;
        let remainder = w % n;
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);

        for (i, tab) in self.tabs.iter().enumerate() {
            // Distribute remainder pixels across the first N tabs.
            let extra_before: u32 = (i as u32).min(remainder);
            let this_tab_w = tab_w + if (i as u32) < remainder { 1 } else { 0 };
            let tx = x + (i as u32 * tab_w + extra_before) as i32;
            let active = i == self.active;
            let state = self.tab_state(i);
            let active_fill = WidgetStateColors::accent_bg(ctx.theme, state);

            // Hover / press feedback on inactive tabs (none at rest).
            if !active && matches!(state, WidgetState::Hover | WidgetState::Pressed) {
                ctx.backend.fill_rounded_rect(
                    tx + 2,
                    y + 2,
                    this_tab_w.saturating_sub(4),
                    h.saturating_sub(4),
                    ctx.theme.border_radius_sm,
                    WidgetStateColors::surface_bg(ctx.theme, state),
                )?;
            }

            match self.style {
                TabStyle::Underline => {
                    if active {
                        ctx.backend
                            .fill_rect(tx, y + h as i32 - 2, this_tab_w, 2, active_fill)?;
                    }
                },
                TabStyle::Filled => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            this_tab_w.saturating_sub(4),
                            h - 4,
                            ctx.theme.border_radius_sm,
                            active_fill,
                        )?;
                    }
                },
                TabStyle::Pill => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            this_tab_w.saturating_sub(4),
                            h - 4,
                            (h - 4) as u16 / 2,
                            active_fill,
                        )?;
                    }
                },
            }

            // Keyboard focus ring around the active tab.
            if active && self.focused && !self.disabled {
                crate::focus::FocusStyle::from_theme(ctx.theme).draw(
                    ctx.backend,
                    tx,
                    y,
                    this_tab_w,
                    h,
                )?;
            }

            let text_w = ctx.backend.measure_text(tab, fs);
            let label_x = tx + layout::center(this_tab_w, text_w);
            let label_y = y + layout::center(h, text_h);
            let color = if active {
                match self.style {
                    TabStyle::Underline => ctx.theme.accent,
                    TabStyle::Filled | TabStyle::Pill => ctx.theme.text_on_accent,
                }
            } else if self.disabled {
                ctx.theme.text_disabled
            } else {
                ctx.theme.text_secondary
            };
            ctx.backend.draw_text(tab, label_x, label_y, fs, color)?;
        }

        // Bottom border for underline style.
        if self.style == TabStyle::Underline {
            ctx.backend.draw_line(
                x,
                y + h as i32 - 1,
                x + w as i32,
                y + h as i32 - 1,
                1,
                ctx.theme.border_subtle,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let tabs = vec!["Home".into(), "Settings".into(), "About".into()];
        let tb = TabBar::new(tabs);
        assert_eq!(tb.tabs.len(), 3);
        assert_eq!(tb.active, 0);
        assert_eq!(tb.style, TabStyle::Underline);
    }

    #[test]
    fn active_index_settable() {
        let mut tb = TabBar::new(vec!["A".into(), "B".into()]);
        tb.active = 1;
        assert_eq!(tb.active, 1);
    }

    #[test]
    fn style_variants() {
        assert_ne!(TabStyle::Underline, TabStyle::Filled);
        assert_ne!(TabStyle::Filled, TabStyle::Pill);
    }

    #[test]
    fn empty_tabs() {
        let tb = TabBar::new(Vec::new());
        assert!(tb.tabs.is_empty());
    }

    #[test]
    fn single_tab() {
        let tb = TabBar::new(vec!["Only".into()]);
        assert_eq!(tb.tabs.len(), 1);
        assert_eq!(tb.active, 0);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_spans_all_tabs() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let tb = TabBar::new(vec!["A".into(), "B".into(), "C".into()]);
        let (w, h) = tb.measure(&ctx, 300, 100);
        assert_eq!(w, 300);
        assert!(h > 0);
    }

    #[test]
    fn draw_active_tab_highlight() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut tb = TabBar::new(vec!["Home".into(), "Settings".into()]);
            tb.active = 0;
            let (_, h) = tb.measure(&ctx, 200, 50);
            tb.draw(&mut ctx, 0, 0, 200, h).unwrap();
        }
        // Underline style draws a fill_rect for the active tab highlight
        // plus the bottom border line.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_underline_style() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut tb = TabBar::new(vec!["A".into(), "B".into()]);
            tb.style = TabStyle::Underline;
            tb.draw(&mut ctx, 0, 0, 200, 30).unwrap();
        }
        // Should not panic.
        assert!(backend.draw_text_count() > 0);
    }

    #[test]
    fn draw_filled_style() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut tb = TabBar::new(vec!["A".into(), "B".into()]);
            tb.style = TabStyle::Filled;
            tb.draw(&mut ctx, 0, 0, 200, 30).unwrap();
        }
        // Should not panic; fill_rect emitted for the filled tab background.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_pill_style() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut tb = TabBar::new(vec!["A".into(), "B".into()]);
            tb.style = TabStyle::Pill;
            tb.draw(&mut ctx, 0, 0, 200, 30).unwrap();
        }
        // Should not panic; fill_rect emitted for the pill background.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_single_tab_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let tb = TabBar::new(vec!["Only".into()]);
            tb.draw(&mut ctx, 0, 0, 200, 30).unwrap();
        }
        assert!(backend.draw_text_count() > 0);
    }

    #[test]
    fn draw_empty_tabs_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let tb = TabBar::new(Vec::new());
            tb.draw(&mut ctx, 0, 0, 200, 30).unwrap();
        }
        // Empty tabs should produce no draw calls.
        assert_eq!(backend.fill_rect_count(), 0);
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn draw_tab_labels() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let tb = TabBar::new(vec!["Home".into(), "Settings".into(), "About".into()]);
            tb.draw(&mut ctx, 0, 0, 300, 30).unwrap();
        }
        assert!(backend.has_text("Home"));
        assert!(backend.has_text("Settings"));
        assert!(backend.has_text("About"));
    }

    fn tabs() -> Vec<String> {
        vec!["A".into(), "B".into(), "C".into()]
    }

    #[test]
    fn each_state_has_distinct_fill_active_and_inactive() {
        for style in [TabStyle::Underline, TabStyle::Filled, TabStyle::Pill] {
            // `target` = hovered tab: 0 is the active tab, 1 inactive.
            for target in [0usize, 1] {
                crate::test_utils::assert_states_distinct(|st, ctx| {
                    let mut tb = TabBar::new(tabs());
                    tb.style = style;
                    tb.hovered =
                        matches!(st, WidgetState::Hover | WidgetState::Pressed).then_some(target);
                    tb.pressed = st == WidgetState::Pressed;
                    tb.disabled = st == WidgetState::Disabled;
                    tb.draw(ctx, 0, 0, 90, 20).unwrap();
                });
            }
        }
    }

    #[test]
    fn resting_active_fill_is_accent() {
        let theme = Theme::dark();
        let fills = crate::test_utils::fill_colors_of(|ctx| {
            TabBar::new(tabs()).draw(ctx, 0, 0, 90, 20).unwrap();
        });
        assert!(fills.contains(&theme.accent));
        assert!(!fills.contains(&WidgetStateColors::surface_bg(&theme, WidgetState::Hover)));
    }

    #[test]
    fn tab_at_matches_draw_layout() {
        let tb = TabBar::new(tabs());
        // 100 / 3 = 33 rem 1: tab 0 spans 0..34, tab 1 34..67, tab 2 67..100.
        assert_eq!(tb.tab_at(0, 100), Some(0));
        assert_eq!(tb.tab_at(33, 100), Some(0));
        assert_eq!(tb.tab_at(34, 100), Some(1));
        assert_eq!(tb.tab_at(66, 100), Some(1));
        assert_eq!(tb.tab_at(67, 100), Some(2));
        assert_eq!(tb.tab_at(99, 100), Some(2));
        assert_eq!(tb.tab_at(100, 100), None);
        assert_eq!(tb.tab_at(-1, 100), None);
    }
}

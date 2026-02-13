//! TabBar widget.

use crate::context::DrawContext;
use crate::layout;
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
}

impl TabBar {
    /// Create a new tab bar.
    pub fn new(tabs: Vec<String>) -> Self {
        Self {
            tabs,
            active: 0,
            style: TabStyle::Underline,
        }
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
        let tab_w = w / self.tabs.len() as u32;
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);

        for (i, tab) in self.tabs.iter().enumerate() {
            let tx = x + (i as u32 * tab_w) as i32;
            let active = i == self.active;

            match self.style {
                TabStyle::Underline => {
                    if active {
                        ctx.backend
                            .fill_rect(tx, y + h as i32 - 2, tab_w, 2, ctx.theme.accent)?;
                    }
                },
                TabStyle::Filled => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            tab_w - 4,
                            h - 4,
                            ctx.theme.border_radius_sm,
                            ctx.theme.accent,
                        )?;
                    }
                },
                TabStyle::Pill => {
                    if active {
                        ctx.backend.fill_rounded_rect(
                            tx + 2,
                            y + 2,
                            tab_w - 4,
                            h - 4,
                            (h - 4) as u16 / 2,
                            ctx.theme.accent,
                        )?;
                    }
                },
            }

            let text_w = ctx.backend.measure_text(tab, fs);
            let label_x = tx + layout::center(tab_w, text_w);
            let label_y = y + layout::center(h, text_h);
            let color = if active {
                match self.style {
                    TabStyle::Underline => ctx.theme.accent,
                    TabStyle::Filled | TabStyle::Pill => ctx.theme.text_on_accent,
                }
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

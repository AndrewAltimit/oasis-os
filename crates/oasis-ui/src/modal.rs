//! Modal dialog widget.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A modal dialog with title, body text, and action buttons.
pub struct Modal {
    /// Dialog title.
    pub title: String,
    /// Dialog body/message text.
    pub body: String,
    /// Action button labels (e.g. \["Cancel", "OK"\]).
    pub buttons: Vec<String>,
    /// Index of the currently focused button.
    pub focused_button: usize,
    /// Whether to draw the semi-transparent backdrop overlay.
    pub show_backdrop: bool,
}

impl Modal {
    /// Create a new modal dialog.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            buttons: vec!["OK".into()],
            focused_button: 0,
            show_backdrop: true,
        }
    }

    /// Create a confirmation dialog with Cancel and OK buttons.
    pub fn confirm(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            buttons: vec!["Cancel".into(), "OK".into()],
            focused_button: 1,
            ..Self::new(title, body)
        }
    }

    /// Focus the next button (wrapping).
    pub fn focus_next(&mut self) {
        if !self.buttons.is_empty() {
            self.focused_button = (self.focused_button + 1) % self.buttons.len();
        }
    }

    /// Focus the previous button (wrapping).
    pub fn focus_prev(&mut self) {
        if !self.buttons.is_empty() {
            self.focused_button = if self.focused_button == 0 {
                self.buttons.len() - 1
            } else {
                self.focused_button - 1
            };
        }
    }

    /// Return the label of the currently focused button.
    pub fn focused_label(&self) -> Option<&str> {
        self.buttons.get(self.focused_button).map(String::as_str)
    }

    /// Fixed modal width for the 480x272 viewport.
    const MODAL_WIDTH: u32 = 280;

    /// Padding inside the modal panel.
    const PADDING: i32 = 12;

    /// Button height.
    const BUTTON_H: u32 = 22;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let m = Modal::new("Title", "Body");
        assert_eq!(m.title, "Title");
        assert_eq!(m.body, "Body");
        assert_eq!(m.buttons, vec!["OK"]);
        assert_eq!(m.focused_button, 0);
        assert!(m.show_backdrop);
    }

    #[test]
    fn confirm_has_two_buttons() {
        let m = Modal::confirm("Delete?", "Are you sure?");
        assert_eq!(m.buttons.len(), 2);
        assert_eq!(m.buttons[0], "Cancel");
        assert_eq!(m.buttons[1], "OK");
        assert_eq!(m.focused_button, 1); // OK focused by default
    }

    #[test]
    fn focus_next_wraps() {
        let mut m = Modal::confirm("T", "B");
        assert_eq!(m.focused_button, 1);
        m.focus_next();
        assert_eq!(m.focused_button, 0); // wraps
        m.focus_next();
        assert_eq!(m.focused_button, 1);
    }

    #[test]
    fn focus_prev_wraps() {
        let mut m = Modal::confirm("T", "B");
        m.focused_button = 0;
        m.focus_prev();
        assert_eq!(m.focused_button, 1); // wraps
    }

    #[test]
    fn focus_next_empty_noop() {
        let mut m = Modal::new("T", "B");
        m.buttons.clear();
        m.focus_next();
        assert_eq!(m.focused_button, 0);
    }

    #[test]
    fn focused_label_returns_correct() {
        let m = Modal::confirm("T", "B");
        assert_eq!(m.focused_label(), Some("OK"));
    }

    #[test]
    fn focused_label_empty_returns_none() {
        let mut m = Modal::new("T", "B");
        m.buttons.clear();
        assert_eq!(m.focused_label(), None);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_viewport_size() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let m = Modal::new("T", "B");
        let (w, h) = m.measure(&ctx, 480, 272);
        // Modal returns full viewport size (backdrop fills it).
        assert_eq!(w, 480);
        assert_eq!(h, 272);
    }

    #[test]
    fn draw_shows_title_and_body() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::new("Warning", "Something happened");
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        assert!(backend.has_text("Warning"));
        assert!(backend.has_text("Something happened"));
    }

    #[test]
    fn draw_shows_buttons() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::confirm("Delete?", "Are you sure?");
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        assert!(backend.has_text("Cancel"));
        assert!(backend.has_text("OK"));
    }

    #[test]
    fn draw_backdrop_emits_fill_rect() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::new("T", "B");
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        // Backdrop + modal panel + button bg = multiple fill_rects
        assert!(backend.fill_rect_count() > 2);
    }

    #[test]
    fn draw_no_backdrop() {
        let theme = Theme::dark();
        let mut backend_with = MockBackend::new();
        let mut backend_without = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend_with, &theme);
            let m = Modal::new("T", "B");
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        {
            let mut ctx = DrawContext::new(&mut backend_without, &theme);
            let mut m = Modal::new("T", "B");
            m.show_backdrop = false;
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        // Without backdrop, fewer fill_rect calls.
        assert!(backend_without.fill_rect_count() < backend_with.fill_rect_count());
    }

    #[test]
    fn draw_empty_buttons_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut m = Modal::new("T", "B");
            m.buttons.clear();
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        assert!(backend.has_text("T"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        for theme in [
            Theme::dark(),
            Theme::light(),
            Theme::classic(),
            Theme::high_contrast(),
        ] {
            let mut backend = MockBackend::new();
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::confirm("Test", "Body");
            m.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
    }
}

impl Widget for Modal {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32) {
        // Modal occupies the full viewport (backdrop fills everything).
        (available_w, available_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let pad = Self::PADDING;
        let fs_title = ctx.theme.font_size_lg;
        let fs_body = ctx.theme.font_size_md;
        let title_h = ctx.backend.measure_text_height(fs_title);
        let body_h = ctx.backend.measure_text_height(fs_body);
        let radius = ctx.theme.border_radius_lg;

        // Compute modal height from content.
        let content_h =
            pad as u32 + title_h + 4 + body_h + pad as u32 + Self::BUTTON_H + pad as u32;
        let mw = Self::MODAL_WIDTH;
        let mh = content_h;
        let mx = x + layout::center(w, mw);
        let my = y + layout::center(h, mh);

        // -- Backdrop --
        if self.show_backdrop {
            ctx.backend.fill_rect(x, y, w, h, ctx.theme.overlay)?;
        }

        // -- Modal panel --
        ctx.theme
            .shadow_modal
            .draw(ctx.backend, mx, my, mw, mh, radius)?;
        ctx.backend
            .fill_rounded_rect(mx, my, mw, mh, radius, ctx.theme.surface)?;
        ctx.backend
            .stroke_rounded_rect(mx, my, mw, mh, radius, 1, ctx.theme.border_subtle)?;

        let mut cy = my + pad;

        // -- Title --
        ctx.backend
            .draw_text(&self.title, mx + pad, cy, fs_title, ctx.theme.text_primary)?;
        cy += title_h as i32 + 4;

        // -- Body --
        let body_w = mw.saturating_sub(pad as u32 * 2);
        ctx.backend.draw_text_wrapped(
            &self.body,
            mx + pad,
            cy,
            fs_body,
            ctx.theme.text_secondary,
            body_w,
            0,
        )?;
        cy += body_h as i32 + pad;

        // -- Buttons (right-aligned row) --
        if !self.buttons.is_empty() {
            let btn_gap = 6i32;
            let btn_pad_h = 12u32;
            // Compute total buttons width.
            let total_btn_w: u32 = self
                .buttons
                .iter()
                .map(|b| ctx.backend.measure_text(b, fs_body) + btn_pad_h)
                .sum::<u32>()
                + (self.buttons.len().saturating_sub(1) as u32 * btn_gap as u32);

            let mut bx = mx + mw as i32 - pad - total_btn_w as i32;
            let btn_radius = ctx.theme.border_radius_md;

            for (i, label) in self.buttons.iter().enumerate() {
                let tw = ctx.backend.measure_text(label, fs_body);
                let bw = tw + btn_pad_h;
                let is_focused = i == self.focused_button;

                let bg = if is_focused {
                    ctx.theme.accent
                } else {
                    ctx.theme.button_bg
                };
                let fg = if is_focused {
                    ctx.theme.text_on_accent
                } else {
                    ctx.theme.text_primary
                };

                ctx.backend
                    .fill_rounded_rect(bx, cy, bw, Self::BUTTON_H, btn_radius, bg)?;

                let text_ty = cy + layout::center(Self::BUTTON_H, body_h);
                let text_tx = bx + layout::center(bw, tw);
                ctx.backend
                    .draw_text(label, text_tx, text_ty, fs_body, fg)?;

                bx += bw as i32 + btn_gap;
            }
        }

        Ok(())
    }
}

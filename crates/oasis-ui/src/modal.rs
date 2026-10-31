//! Modal dialog widget with input blocking and button presets.

use crate::context::DrawContext;
use crate::focus::{FocusDir, step_wrapping};
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Result of dismissing a modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalResult {
    /// No action taken yet (modal still open).
    Pending,
    /// User confirmed (pressed OK / primary action).
    Confirmed,
    /// User cancelled (pressed Cancel / dismissed).
    Cancelled,
    /// User pressed a custom button at the given index.
    Custom(usize),
}

/// Preset button configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalButtons {
    /// Single "OK" button.
    Ok,
    /// "Cancel" and "OK" buttons.
    OkCancel,
    /// "No" and "Yes" buttons.
    YesNo,
    /// "Cancel", "No", and "Yes" buttons.
    YesNoCancel,
    /// Custom: caller sets buttons manually.
    Custom,
}

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
    /// Whether this modal blocks input to underlying widgets.
    pub blocks_input: bool,
    /// Current dismissal result (Pending while open).
    pub result: ModalResult,
    /// Which button preset was used.
    pub preset: ModalButtons,
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
            blocks_input: true,
            result: ModalResult::Pending,
            preset: ModalButtons::Ok,
        }
    }

    /// Create a confirmation dialog with Cancel and OK buttons.
    pub fn confirm(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            buttons: vec!["Cancel".into(), "OK".into()],
            focused_button: 1,
            preset: ModalButtons::OkCancel,
            ..Self::new(title, body)
        }
    }

    /// Create a Yes/No dialog.
    pub fn yes_no(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            buttons: vec!["No".into(), "Yes".into()],
            focused_button: 1,
            preset: ModalButtons::YesNo,
            ..Self::new(title, body)
        }
    }

    /// Create a Yes/No/Cancel dialog.
    pub fn yes_no_cancel(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            buttons: vec!["Cancel".into(), "No".into(), "Yes".into()],
            focused_button: 2,
            preset: ModalButtons::YesNoCancel,
            ..Self::new(title, body)
        }
    }

    /// Create a dialog with custom button labels.
    pub fn custom(title: impl Into<String>, body: impl Into<String>, buttons: Vec<String>) -> Self {
        let focused = buttons.len().saturating_sub(1);
        Self {
            buttons,
            focused_button: focused,
            preset: ModalButtons::Custom,
            ..Self::new(title, body)
        }
    }

    /// Focus the next button (wrapping).
    pub fn focus_next(&mut self) {
        self.step_focus(FocusDir::Next);
    }

    /// Focus the previous button (wrapping).
    pub fn focus_prev(&mut self) {
        self.step_focus(FocusDir::Prev);
    }

    fn step_focus(&mut self, dir: FocusDir) {
        let current = Some(self.focused_button);
        if let Some(i) = step_wrapping(current, self.buttons.len(), dir, |_| true) {
            self.focused_button = i;
        }
    }

    /// Return the label of the currently focused button.
    pub fn focused_label(&self) -> Option<&str> {
        self.buttons.get(self.focused_button).map(String::as_str)
    }

    /// Activate the currently focused button and set the result.
    ///
    /// Returns the `ModalResult` for the pressed button.
    pub fn activate(&mut self) -> ModalResult {
        // `buttons` is pub: a focus index left past the end by a shorter
        // custom button list activates the last button, not a phantom one.
        let index = self
            .focused_button
            .min(self.buttons.len().saturating_sub(1));
        let result = self.result_for_index(index);
        self.result = result;
        result
    }

    /// Dismiss the modal as cancelled (e.g. Escape key).
    pub fn dismiss(&mut self) -> ModalResult {
        self.result = ModalResult::Cancelled;
        ModalResult::Cancelled
    }

    /// Whether this modal is still open (no result yet).
    pub fn is_open(&self) -> bool {
        self.result == ModalResult::Pending
    }

    /// Whether this modal should consume all input events,
    /// preventing them from reaching widgets underneath.
    pub fn should_block_input(&self) -> bool {
        self.blocks_input && self.is_open()
    }

    /// Map a button index to a `ModalResult` based on the preset.
    fn result_for_index(&self, index: usize) -> ModalResult {
        match self.preset {
            ModalButtons::Ok => ModalResult::Confirmed,
            ModalButtons::OkCancel => {
                if index == 0 {
                    ModalResult::Cancelled
                } else {
                    ModalResult::Confirmed
                }
            },
            ModalButtons::YesNo => {
                if index == 0 {
                    ModalResult::Cancelled
                } else {
                    ModalResult::Confirmed
                }
            },
            ModalButtons::YesNoCancel => match index {
                0 => ModalResult::Cancelled,
                1 => ModalResult::Custom(1),
                _ => ModalResult::Confirmed,
            },
            ModalButtons::Custom => ModalResult::Custom(index),
        }
    }

    /// Fixed modal width for the 480x272 viewport.
    const MODAL_WIDTH: u32 = 280;

    /// Padding inside the modal panel.
    const PADDING: i32 = 12;

    /// Button height.
    const BUTTON_H: u32 = 22;
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

        // Compute modal height from content: the body is as tall as its
        // wrapped lines (it used to be sized for one line, so longer
        // messages spilled over the buttons and out of the panel).
        let mw = Self::MODAL_WIDTH;
        let body_w = mw.saturating_sub(pad as u32 * 2);
        let body_lines = wrapped_line_count(ctx, &self.body, fs_body, body_w);
        let body_total_h = body_h * body_lines;
        let content_h =
            pad as u32 + title_h + 4 + body_total_h + pad as u32 + Self::BUTTON_H + pad as u32;
        let mh = content_h.min(h);
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
        ctx.frame(
            (mx, my, mw, mh),
            ctx.theme.surface,
            ctx.theme.border_subtle,
            radius,
        )?;

        let mut cy = my + pad;

        // -- Title --
        ctx.backend
            .draw_text(&self.title, mx + pad, cy, fs_title, ctx.theme.text_primary)?;
        cy += title_h as i32 + 4;

        // -- Body --
        ctx.backend.draw_text_wrapped(
            &self.body,
            mx + pad,
            cy,
            fs_body,
            ctx.theme.text_secondary,
            body_w,
            0,
        )?;
        cy += body_total_h as i32 + pad;

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
                    ctx.theme.button_text
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

/// Lines `SdiText::draw_text_wrapped` breaks `text` into at `max_width`
/// (the same greedy word wrap, so the measured and drawn heights agree);
/// at least one.
fn wrapped_line_count(ctx: &DrawContext<'_>, text: &str, font_size: u16, max_width: u32) -> u32 {
    let mut lines = 0u32;
    for line in text.split('\n') {
        let mut current = String::new();
        let mut any = false;
        for word in line.split_whitespace() {
            any = true;
            let test = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if ctx.backend.measure_text(&test, font_size) > max_width && !current.is_empty() {
                lines += 1;
                current = word.to_string();
            } else {
                current = test;
            }
        }
        // A blank line still takes one line; a non-empty one ends with its
        // last partial line.
        if !any || !current.is_empty() {
            lines += 1;
        }
    }
    lines.max(1)
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
        assert!(m.blocks_input);
        assert_eq!(m.result, ModalResult::Pending);
        assert_eq!(m.preset, ModalButtons::Ok);
    }

    #[test]
    fn confirm_has_two_buttons() {
        let m = Modal::confirm("Delete?", "Are you sure?");
        assert_eq!(m.buttons.len(), 2);
        assert_eq!(m.buttons[0], "Cancel");
        assert_eq!(m.buttons[1], "OK");
        assert_eq!(m.focused_button, 1);
        assert_eq!(m.preset, ModalButtons::OkCancel);
    }

    #[test]
    fn yes_no_dialog() {
        let m = Modal::yes_no("Save?", "Save changes?");
        assert_eq!(m.buttons.len(), 2);
        assert_eq!(m.buttons[0], "No");
        assert_eq!(m.buttons[1], "Yes");
        assert_eq!(m.focused_button, 1);
        assert_eq!(m.preset, ModalButtons::YesNo);
    }

    #[test]
    fn yes_no_cancel_dialog() {
        let m = Modal::yes_no_cancel("Save?", "Changes?");
        assert_eq!(m.buttons.len(), 3);
        assert_eq!(m.buttons[0], "Cancel");
        assert_eq!(m.buttons[1], "No");
        assert_eq!(m.buttons[2], "Yes");
        assert_eq!(m.focused_button, 2);
        assert_eq!(m.preset, ModalButtons::YesNoCancel);
    }

    #[test]
    fn custom_buttons() {
        let m = Modal::custom(
            "Pick",
            "Choose one",
            vec!["A".into(), "B".into(), "C".into()],
        );
        assert_eq!(m.buttons.len(), 3);
        assert_eq!(m.focused_button, 2);
        assert_eq!(m.preset, ModalButtons::Custom);
    }

    #[test]
    fn custom_empty_buttons() {
        let m = Modal::custom("T", "B", vec![]);
        assert!(m.buttons.is_empty());
        assert_eq!(m.focused_button, 0);
    }

    /// Replacing a custom modal's buttons with fewer left `focused_button`
    /// past the end: `activate` reported a button that no longer exists.
    #[test]
    fn stale_focused_button_activates_the_last_button() {
        let mut m = Modal::custom("T", "B", vec!["a".into(), "b".into(), "c".into()]);
        m.focused_button = 2;
        m.buttons.truncate(1);
        assert_eq!(m.activate(), ModalResult::Custom(0));
        m.focus_prev();
        assert_eq!(m.focused_button, 0);
    }

    #[test]
    fn focus_next_wraps() {
        let mut m = Modal::confirm("T", "B");
        assert_eq!(m.focused_button, 1);
        m.focus_next();
        assert_eq!(m.focused_button, 0);
        m.focus_next();
        assert_eq!(m.focused_button, 1);
    }

    #[test]
    fn focus_prev_wraps() {
        let mut m = Modal::confirm("T", "B");
        m.focused_button = 0;
        m.focus_prev();
        assert_eq!(m.focused_button, 1);
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

    #[test]
    fn activate_ok_dialog() {
        let mut m = Modal::new("T", "B");
        let result = m.activate();
        assert_eq!(result, ModalResult::Confirmed);
        assert!(!m.is_open());
    }

    #[test]
    fn activate_confirm_ok() {
        let mut m = Modal::confirm("T", "B");
        // focused_button == 1 (OK)
        let result = m.activate();
        assert_eq!(result, ModalResult::Confirmed);
    }

    #[test]
    fn activate_confirm_cancel() {
        let mut m = Modal::confirm("T", "B");
        m.focused_button = 0; // Cancel
        let result = m.activate();
        assert_eq!(result, ModalResult::Cancelled);
    }

    #[test]
    fn activate_yes_no_yes() {
        let mut m = Modal::yes_no("T", "B");
        let result = m.activate(); // focused on Yes (1)
        assert_eq!(result, ModalResult::Confirmed);
    }

    #[test]
    fn activate_yes_no_no() {
        let mut m = Modal::yes_no("T", "B");
        m.focused_button = 0; // No
        let result = m.activate();
        assert_eq!(result, ModalResult::Cancelled);
    }

    #[test]
    fn activate_custom_button() {
        let mut m = Modal::custom("T", "B", vec!["A".into(), "B".into()]);
        m.focused_button = 0;
        let result = m.activate();
        assert_eq!(result, ModalResult::Custom(0));
    }

    #[test]
    fn dismiss_sets_cancelled() {
        let mut m = Modal::confirm("T", "B");
        let result = m.dismiss();
        assert_eq!(result, ModalResult::Cancelled);
        assert!(!m.is_open());
    }

    #[test]
    fn is_open_while_pending() {
        let m = Modal::new("T", "B");
        assert!(m.is_open());
    }

    #[test]
    fn should_block_input_default() {
        let m = Modal::new("T", "B");
        assert!(m.should_block_input());
    }

    #[test]
    fn should_block_input_after_dismiss() {
        let mut m = Modal::new("T", "B");
        m.dismiss();
        assert!(!m.should_block_input());
    }

    #[test]
    fn should_block_input_disabled() {
        let mut m = Modal::new("T", "B");
        m.blocks_input = false;
        assert!(!m.should_block_input());
    }

    #[test]
    fn modal_result_debug() {
        for r in [
            ModalResult::Pending,
            ModalResult::Confirmed,
            ModalResult::Cancelled,
            ModalResult::Custom(0),
        ] {
            let _ = format!("{r:?}");
        }
    }

    #[test]
    fn modal_buttons_debug() {
        for b in [
            ModalButtons::Ok,
            ModalButtons::OkCancel,
            ModalButtons::YesNo,
            ModalButtons::YesNoCancel,
            ModalButtons::Custom,
        ] {
            let _ = format!("{b:?}");
        }
    }

    #[test]
    fn yes_no_cancel_activate_each() {
        // Cancel (index 0)
        let mut m = Modal::yes_no_cancel("T", "B");
        m.focused_button = 0;
        assert_eq!(m.activate(), ModalResult::Cancelled);

        // No (index 1)
        let mut m = Modal::yes_no_cancel("T", "B");
        m.focused_button = 1;
        assert_eq!(m.activate(), ModalResult::Custom(1));

        // Yes (index 2)
        let mut m = Modal::yes_no_cancel("T", "B");
        m.focused_button = 2;
        assert_eq!(m.activate(), ModalResult::Confirmed);
    }

    // -- Draw / measure tests using RecordingBackend --

    use crate::context::DrawContext;
    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_viewport_size() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        let m = Modal::new("T", "B");
        let (w, h) = m.measure(&ctx, 480, 272);
        assert_eq!(w, 480);
        assert_eq!(h, 272);
    }

    #[test]
    fn draw_shows_title_and_body() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::new("Warning", "Something happened");
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.has_text("Warning"));
        assert!(backend.has_text("Something happened"));
    }

    #[test]
    fn draw_shows_buttons() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::confirm("Delete?", "Sure?");
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.has_text("Cancel"));
        assert!(backend.has_text("OK"));
    }

    #[test]
    fn draw_backdrop_emits_fill_rect() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::new("T", "B");
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.fill_rect_count() > 2);
    }

    #[test]
    fn draw_no_backdrop() {
        let theme = Theme::dark();
        let mut backend_with = RecordingBackend::default();
        let mut backend_without = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend_with, &theme);
            let m = Modal::new("T", "B");
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        {
            let mut ctx = DrawContext::new(&mut backend_without, &theme);
            let mut m = Modal::new("T", "B");
            m.show_backdrop = false;
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend_without.fill_rect_count() < backend_with.fill_rect_count());
    }

    #[test]
    fn long_body_pushes_the_buttons_below_its_last_line() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let body = "word ".repeat(80);
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::new("T", body.trim());
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        let texts = backend.text_positions();
        let last_body_y = texts
            .iter()
            .filter(|t| t.0.starts_with("word"))
            .map(|t| t.2)
            .max()
            .unwrap();
        let ok_y = texts.iter().find(|t| t.0 == "OK").unwrap().2;
        assert!(texts.iter().filter(|t| t.0.starts_with("word")).count() > 1);
        assert!(
            ok_y > last_body_y,
            "button at {ok_y} overlaps body line at {last_body_y}"
        );
    }

    #[test]
    fn draw_empty_buttons_no_panic() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut m = Modal::new("T", "B");
            m.buttons.clear();
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.has_text("T"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let m = Modal::confirm("Test", "Body");
            m.draw(ctx, 0, 0, 480, 272).ok();
        });
    }

    #[test]
    fn draw_yes_no_shows_buttons() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::yes_no("Save?", "Save changes?");
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.has_text("No"));
        assert!(backend.has_text("Yes"));
    }

    #[test]
    fn draw_custom_buttons_shows_all() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let m = Modal::custom("Pick", "Choose", vec!["A".into(), "B".into(), "C".into()]);
            m.draw(&mut ctx, 0, 0, 480, 272).ok();
        }
        assert!(backend.has_text("A"));
        assert!(backend.has_text("B"));
        assert!(backend.has_text("C"));
    }
}

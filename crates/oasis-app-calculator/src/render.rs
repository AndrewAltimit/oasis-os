//! Windowed rendering for the Calculator: display panel, key grid and
//! history pane.
//!
//! All geometry comes from [`CalcLayout`] so drawing and hit-testing
//! agree; all colors come from the active theme via [`CalcColors`].

use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_ui::button::{Button as UiButton, ButtonState, ButtonStyle};
use oasis_ui::{DrawContext, Widget};

use crate::keypad::{CalcLayout, HISTORY_ROW_H, KEYS, KeyGroup, Rect};
use crate::{CalculatorApp, format_number};

/// Calculator color roles, derived from the active theme. Skins can
/// override any slot via `[app_themes.calculator]` in theme.toml.
#[derive(Debug, Clone)]
pub struct CalcColors {
    /// Content background.
    pub bg: Color,
    /// Display panel background.
    pub display_bg: Color,
    /// Display panel border.
    pub display_border: Color,
    /// Main (large) display text.
    pub display_text: Color,
    /// Secondary display text (expression echo, memory flag).
    pub dim_text: Color,
    /// Error message text.
    pub error_text: Color,
    /// History pane background.
    pub history_bg: Color,
    /// History pane header background / text.
    pub history_header_bg: Color,
    pub history_header_text: Color,
    /// History entry text.
    pub history_text: Color,
    /// Highlight behind the entry currently recalled.
    pub history_sel_bg: Color,
    pub history_sel_text: Color,
}

impl CalcColors {
    /// Build colors from the active theme, honouring per-app overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color("calculator", key).unwrap_or(default);
        let ui = &at.ui_theme;
        Self {
            bg: c("bg", at.app.bg),
            display_bg: c("display_bg", ui.input_bg),
            display_border: c("display_border", ui.input_border),
            display_text: c("display_text", ui.text_primary),
            dim_text: c("dim_text", ui.text_secondary),
            error_text: c("error_text", ui.error),
            history_bg: c("history_bg", ui.surface),
            history_header_bg: c("history_header_bg", at.app.title_bar_bg),
            history_header_text: c("history_header_text", at.app.title_bar_text),
            history_text: c("history_text", ui.text_primary),
            history_sel_bg: c("history_sel_bg", at.app.selected_bg),
            history_sel_text: c("history_sel_text", at.app.selected_text),
        }
    }
}

/// Key cap style for a key group.
fn style_for(group: KeyGroup) -> ButtonStyle {
    match group {
        KeyGroup::Equals => ButtonStyle::Primary,
        KeyGroup::Digit | KeyGroup::Edit => ButtonStyle::Secondary,
        KeyGroup::Operator | KeyGroup::Memory => ButtonStyle::Outline,
    }
}

/// Keep the tail of `text` that fits in `max_w` pixels, prefixed with
/// `..` when anything was cut (numbers grow to the left, so the end is
/// the interesting part).
fn fit_tail(backend: &dyn SdiBackend, text: &str, font: u16, max_w: u32) -> String {
    if backend.measure_text(text, font) <= max_w {
        return text.to_string();
    }
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    while start < chars.len() {
        let candidate = format!("..{}", &text[chars[start].0..]);
        if backend.measure_text(&candidate, font) <= max_w {
            return candidate;
        }
        start += 1;
    }
    String::new()
}

impl CalculatorApp {
    /// Draw the whole calculator UI into the content rect.
    pub(crate) fn draw_calculator(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = CalcColors::from_theme(at);
        let l = CalcLayout::compute(cx, cy, cw, ch);
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;
        self.draw_display(&l, backend, at, &colors)?;
        self.draw_keys(&l, backend, at)?;
        if let Some(h) = l.history {
            self.draw_history(h, l.history_rows(), backend, at, &colors)?;
        }
        // Age the pressed-key flash by one frame.
        if let Some((key, frames)) = self.flash.get() {
            self.flash.set((frames > 1).then(|| (key, frames - 1)));
        }
        Ok(())
    }

    fn draw_display(
        &self,
        l: &CalcLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &CalcColors,
    ) -> oasis_types::error::Result<()> {
        let d = l.display;
        let radius = at.ui_theme.border_radius_md;
        backend.fill_rounded_rect(d.x, d.y, d.w, d.h, radius, colors.display_bg)?;
        backend.stroke_rounded_rect(d.x, d.y, d.w, d.h, radius, 1, colors.display_border)?;

        let inner_w = d.w.saturating_sub(10);
        let small = at.font_hint;
        let big = at.ui_theme.font_size_xl;

        // Top line: the expression being typed or the one just evaluated,
        // plus a memory flag on the left.
        let top = if !self.input_buffer.is_empty() {
            self.last_result
                .map(|r| format!("Ans = {}", format_number(r)))
                .unwrap_or_default()
        } else if let Some(last) = self.history.last()
            && self.last_result.is_some()
        {
            format!("{} =", last.expression)
        } else {
            String::new()
        };
        let mut text_left = d.x + 5;
        if self.memory != 0.0 {
            backend.draw_text("M", text_left, d.y + 3, small, colors.dim_text)?;
            text_left += backend.measure_text("M ", small) as i32;
        }
        if !top.is_empty() {
            let avail = (d.x + d.w as i32 - 5 - text_left).max(0) as u32;
            let top = fit_tail(backend, &top, small, avail);
            let tw = backend.measure_text(&top, small) as i32;
            let tx = d.x + d.w as i32 - 5 - tw;
            backend.draw_text(&top, tx, d.y + 3, small, colors.dim_text)?;
        }

        // Main line: error, the live input, or the current result.
        let (main, color, font) = if let Some(err) = &self.error_message {
            (err.clone(), colors.error_text, small)
        } else if self.input_buffer.is_empty() {
            (self.display.clone(), colors.display_text, big)
        } else {
            (self.input_buffer.clone(), colors.display_text, big)
        };
        let main = fit_tail(backend, &main, font, inner_w);
        let mw = backend.measure_text(&main, font) as i32;
        let mh = backend.measure_text_height(font) as i32;
        let my = d.y + d.h as i32 - mh - 4;
        backend.draw_text(&main, d.x + d.w as i32 - 5 - mw, my, font, color)
    }

    fn draw_keys(
        &self,
        l: &CalcLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let flashed = self.flash.get().map(|(k, _)| k);
        let mut ctx = DrawContext::new(backend, &at.ui_theme);
        for (i, def) in KEYS.iter().enumerate() {
            let r = l.key_rect(i);
            let mut button = UiButton::new(def.key.label());
            button.style = style_for(def.key.group());
            if flashed == Some(i) {
                button.state = ButtonState::Pressed;
            }
            button.focused = self.cursor_visible && self.cursor == i;
            button.draw(&mut ctx, r.x, r.y, r.w, r.h)?;
        }
        Ok(())
    }

    fn draw_history(
        &self,
        h: Rect,
        rows: usize,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &CalcColors,
    ) -> oasis_types::error::Result<()> {
        let font = at.font_hint;
        backend.fill_rect(h.x, h.y, h.w, h.h, colors.history_bg)?;
        backend.stroke_rect(h.x, h.y, h.w, h.h, 1, colors.display_border)?;
        backend.fill_rect(h.x, h.y, h.w, HISTORY_ROW_H, colors.history_header_bg)?;
        backend.draw_text(
            "History",
            h.x + 4,
            h.y + 2,
            font,
            colors.history_header_text,
        )?;
        if self.history.is_empty() {
            let y = h.y + HISTORY_ROW_H as i32 + 2;
            return backend.draw_text("(empty)", h.x + 4, y, font, colors.dim_text);
        }
        let text_w = h.w.saturating_sub(8);
        // Newest first.
        for (row, idx) in (0..self.history.len()).rev().take(rows).enumerate() {
            let e = &self.history[idx];
            let y = h.y + ((row as u32 + 1) * HISTORY_ROW_H) as i32;
            let color = if self.recall_index == Some(idx) {
                backend.fill_rect(h.x + 1, y, h.w - 2, HISTORY_ROW_H, colors.history_sel_bg)?;
                colors.history_sel_text
            } else {
                colors.history_text
            };
            let line = format!("{} = {}", e.expression, format_number(e.result));
            let line = fit_tail(backend, &line, font, text_w);
            backend.draw_text(&line, h.x + 4, y + 2, font, color)?;
        }
        Ok(())
    }
}

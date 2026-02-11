//! InputField widget: text input with cursor.

use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout;
use crate::ui::widget::Widget;

/// Text input field with cursor and optional placeholder.
pub struct InputField {
    pub text: String,
    pub placeholder: String,
    pub cursor_pos: usize,
    pub selection: Option<(usize, usize)>,
    pub focused: bool,
    pub password_mode: bool,
}

impl InputField {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            placeholder: String::new(),
            cursor_pos: 0,
            selection: None,
            focused: false,
            password_mode: false,
        }
    }

    /// Display text (masked if password mode).
    fn display_text(&self) -> String {
        if self.password_mode {
            "*".repeat(self.text.len())
        } else {
            self.text.clone()
        }
    }

    /// Insert a character at the cursor position.
    pub fn insert(&mut self, ch: char) {
        let byte_pos = self
            .text
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert(byte_pos, ch);
        self.cursor_pos += 1;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            let byte_pos = self
                .text
                .char_indices()
                .nth(self.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            if byte_pos < self.text.len() {
                let ch_len = self.text[byte_pos..]
                    .chars()
                    .next()
                    .map_or(0, |c| c.len_utf8());
                self.text.drain(byte_pos..byte_pos + ch_len);
            }
        }
    }
}

impl Default for InputField {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for InputField {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let h = ctx.backend.measure_text_height(ctx.theme.font_size_md) + 8;
        (available_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let radius = ctx.theme.border_radius_md;

        // Background.
        ctx.backend
            .fill_rounded_rect(x, y, w, h, radius, ctx.theme.input_bg)?;

        // Border.
        let bc = if self.focused {
            ctx.theme.input_border_focus
        } else {
            ctx.theme.input_border
        };
        ctx.backend.stroke_rounded_rect(x, y, w, h, radius, 1, bc)?;

        // Text or placeholder.
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let ty = y + layout::center(h, text_h);
        let tx = x + 4;
        let max_w = w.saturating_sub(8);

        if self.text.is_empty() {
            ctx.backend.draw_text_ellipsis(
                &self.placeholder,
                tx,
                ty,
                fs,
                ctx.theme.text_disabled,
                max_w,
            )?;
        } else {
            let display = self.display_text();
            ctx.backend
                .draw_text_ellipsis(&display, tx, ty, fs, ctx.theme.text_primary, max_w)?;

            // Cursor.
            if self.focused {
                let before = &display[..display
                    .char_indices()
                    .nth(self.cursor_pos)
                    .map(|(i, _)| i)
                    .unwrap_or(display.len())];
                let cursor_x = tx + ctx.backend.measure_text(before, fs) as i32;
                ctx.backend
                    .fill_rect(cursor_x, ty, 1, text_h, ctx.theme.text_primary)?;
            }
        }
        Ok(())
    }
}

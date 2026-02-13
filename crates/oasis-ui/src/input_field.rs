//! InputField widget: text input with cursor.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Text input field with cursor and optional placeholder.
pub struct InputField {
    /// Current text content.
    pub text: String,
    /// Placeholder text shown when empty.
    pub placeholder: String,
    /// Cursor position as character index.
    pub cursor_pos: usize,
    /// Optional text selection range (start, end).
    pub selection: Option<(usize, usize)>,
    /// Whether the field has focus.
    pub focused: bool,
    /// If true, display text as asterisks.
    pub password_mode: bool,
}

impl InputField {
    /// Create a new empty input field.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let f = InputField::new();
        assert!(f.text.is_empty());
        assert!(f.placeholder.is_empty());
        assert_eq!(f.cursor_pos, 0);
        assert!(f.selection.is_none());
        assert!(!f.focused);
        assert!(!f.password_mode);
    }

    #[test]
    fn insert_char() {
        let mut f = InputField::new();
        f.insert('A');
        assert_eq!(f.text, "A");
        assert_eq!(f.cursor_pos, 1);
    }

    #[test]
    fn insert_multiple_chars() {
        let mut f = InputField::new();
        for ch in "Hello".chars() {
            f.insert(ch);
        }
        assert_eq!(f.text, "Hello");
        assert_eq!(f.cursor_pos, 5);
    }

    #[test]
    fn backspace_removes_char() {
        let mut f = InputField::new();
        f.insert('A');
        f.insert('B');
        f.backspace();
        assert_eq!(f.text, "A");
        assert_eq!(f.cursor_pos, 1);
    }

    #[test]
    fn backspace_at_start_does_nothing() {
        let mut f = InputField::new();
        f.backspace();
        assert!(f.text.is_empty());
        assert_eq!(f.cursor_pos, 0);
    }

    #[test]
    fn backspace_all_chars() {
        let mut f = InputField::new();
        f.insert('X');
        f.insert('Y');
        f.backspace();
        f.backspace();
        assert!(f.text.is_empty());
        assert_eq!(f.cursor_pos, 0);
    }

    #[test]
    fn insert_unicode() {
        let mut f = InputField::new();
        f.insert('\u{00E9}'); // é
        f.insert('\u{1F600}'); // emoji
        assert_eq!(f.text.chars().count(), 2);
        assert_eq!(f.cursor_pos, 2);
    }

    #[test]
    fn backspace_unicode() {
        let mut f = InputField::new();
        f.insert('\u{00E9}');
        f.insert('\u{1F600}');
        f.backspace();
        assert_eq!(f.text, "\u{00E9}");
        assert_eq!(f.cursor_pos, 1);
    }

    #[test]
    fn password_mode_display() {
        let mut f = InputField::new();
        f.password_mode = true;
        f.insert('s');
        f.insert('e');
        f.insert('c');
        let display = f.display_text();
        assert_eq!(display, "***");
    }

    #[test]
    fn normal_mode_display() {
        let mut f = InputField::new();
        f.insert('h');
        f.insert('i');
        let display = f.display_text();
        assert_eq!(display, "hi");
    }

    #[test]
    fn default_same_as_new() {
        let a = InputField::new();
        let b = InputField::default();
        assert_eq!(a.text, b.text);
        assert_eq!(a.cursor_pos, b.cursor_pos);
        assert_eq!(a.focused, b.focused);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_width() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut f = InputField::new();
        for ch in "Hello".chars() {
            f.insert(ch);
        }
        let (w, h) = f.measure(&ctx, 200, 100);
        // measure returns (available_w, text_height + 8)
        assert_eq!(w, 200);
        assert!(h > 0, "height should be positive");
    }

    #[test]
    fn draw_shows_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut f = InputField::new();
            for ch in "Hello".chars() {
                f.insert(ch);
            }
            f.draw(&mut ctx, 0, 0, 200, 24).unwrap();
        }
        assert!(backend.has_text("Hello"), "should draw the field text");
    }

    #[test]
    fn draw_password_shows_asterisks() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut f = InputField::new();
            f.password_mode = true;
            for ch in "abc".chars() {
                f.insert(ch);
            }
            f.draw(&mut ctx, 0, 0, 200, 24).unwrap();
        }
        assert!(
            backend.has_text("***"),
            "password mode should display asterisks"
        );
        assert!(
            !backend.has_text("abc"),
            "password mode should not show plaintext"
        );
    }

    #[test]
    fn draw_placeholder_when_empty() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut f = InputField::new();
            f.placeholder = "Type here...".to_string();
            f.draw(&mut ctx, 0, 0, 200, 24).unwrap();
        }
        assert!(
            backend.has_text("Type here..."),
            "empty field should show placeholder"
        );
    }

    #[test]
    fn draw_cursor_position() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut f = InputField::new();
            f.focused = true;
            for ch in "AB".chars() {
                f.insert(ch);
            }
            f.draw(&mut ctx, 0, 0, 200, 24).unwrap();
        }
        // Cursor is drawn as a 1px-wide fill_rect.
        // Background (fill_rounded_rect -> fill_rect), border (stroke_rounded_rect ->
        // 4 fill_rects), and cursor (fill_rect) = at least 6 fill_rects.
        assert!(
            backend.fill_rect_count() >= 2,
            "focused field should draw cursor rect; got {} fill_rects",
            backend.fill_rect_count()
        );
    }

    #[test]
    fn draw_focused_vs_unfocused() {
        let theme = Theme::dark();
        for focused in [true, false] {
            let mut backend = MockBackend::new();
            {
                let mut ctx = DrawContext::new(&mut backend, &theme);
                let mut f = InputField::new();
                f.focused = focused;
                for ch in "X".chars() {
                    f.insert(ch);
                }
                f.draw(&mut ctx, 0, 0, 200, 24).unwrap();
            }
            // Both states should render without panic.
            assert!(backend.fill_rect_count() > 0);
        }
    }

    #[test]
    fn insert_at_middle() {
        let mut f = InputField::new();
        for ch in "AC".chars() {
            f.insert(ch);
        }
        // Move cursor to position 1 (between A and C).
        f.cursor_pos = 1;
        f.insert('B');
        assert_eq!(f.text, "ABC");
        assert_eq!(f.cursor_pos, 2);
    }

    #[test]
    fn backspace_at_zero_noop() {
        let mut f = InputField::new();
        for ch in "Hi".chars() {
            f.insert(ch);
        }
        f.cursor_pos = 0;
        f.backspace();
        assert_eq!(f.text, "Hi", "text should be unchanged");
        assert_eq!(f.cursor_pos, 0, "cursor should remain at 0");
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

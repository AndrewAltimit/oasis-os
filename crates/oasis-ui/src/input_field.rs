//! InputField widget: text input with cursor.

use crate::context::DrawContext;
use crate::layout;
use crate::states::{WidgetState, WidgetStateColors};
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
    /// Whether the field is disabled (non-interactive).
    pub disabled: bool,
    /// Whether the pointer is over the field.
    pub hovered: bool,
    /// Whether the field is being pressed (pointer down on it).
    pub pressed: bool,
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
            disabled: false,
            hovered: false,
            pressed: false,
        }
    }

    /// Resolved interaction state (Disabled > Pressed > Hover > Normal).
    pub fn state(&self) -> WidgetState {
        WidgetState::from_flags(self.hovered, self.pressed, self.disabled)
    }

    /// Display text (masked if password mode): one `*` per character,
    /// so the cursor's character index lines up with the mask.
    fn display_text(&self) -> String {
        if self.password_mode {
            "*".repeat(self.char_count())
        } else {
            self.text.clone()
        }
    }

    /// Number of characters (not bytes) in the text.
    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// The cursor, clamped to the text (`cursor_pos` is a public field).
    fn cursor(&self) -> usize {
        self.cursor_pos.min(self.char_count())
    }

    /// Byte offset of character index `char_idx` (or the end).
    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Insert a character at the cursor position.
    pub fn insert(&mut self, ch: char) {
        if self.disabled {
            return;
        }
        let cursor = self.cursor();
        let byte_pos = self.byte_at(cursor);
        self.text.insert(byte_pos, ch);
        self.cursor_pos = cursor + 1;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.disabled {
            return;
        }
        let cursor = self.cursor();
        if cursor > 0 {
            self.cursor_pos = cursor - 1;
            let start = self.byte_at(cursor - 1);
            let end = self.byte_at(cursor);
            self.text.drain(start..end);
        }
    }

    /// Delete the character after the cursor (the Delete key).
    pub fn delete(&mut self) {
        if self.disabled {
            return;
        }
        let cursor = self.cursor();
        let (start, end) = (self.byte_at(cursor), self.byte_at(cursor + 1));
        self.text.drain(start..end);
        self.cursor_pos = cursor;
    }

    /// Move the cursor one character left (stops at the start).
    pub fn move_left(&mut self) {
        self.cursor_pos = self.cursor().saturating_sub(1);
    }

    /// Move the cursor one character right (stops at the end).
    pub fn move_right(&mut self) {
        self.cursor_pos = (self.cursor() + 1).min(self.char_count());
    }

    /// Move the cursor to the start of the text.
    pub fn move_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move the cursor to the end of the text.
    pub fn move_end(&mut self) {
        self.cursor_pos = self.char_count();
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

        // Background and border.
        let state = self.state();
        let bc = if state.is_disabled() {
            WidgetStateColors::border(ctx.theme, state)
        } else if self.focused {
            ctx.theme.input_border_focus
        } else {
            ctx.theme.input_border
        };
        ctx.frame(
            (x, y, w, h),
            WidgetStateColors::input_bg(ctx.theme, state),
            bc,
            radius,
        )?;

        // Keyboard focus ring (in addition to the focus-colored border).
        if self.focused && !self.disabled {
            crate::focus::FocusStyle::from_theme(ctx.theme).draw(ctx.backend, x, y, w, h)?;
        }

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
        } else if !self.focused {
            let display = self.display_text();
            let fg = WidgetStateColors::content_text(ctx.theme, state);
            ctx.backend
                .draw_text_ellipsis(&display, tx, ty, fs, fg, max_w)?;
        } else {
            // Focused: show the text around the cursor rather than an
            // ellipsis, scrolling left just enough to keep the cursor
            // inside the field, and clip to the text area.
            let display = self.display_text();
            let fg = WidgetStateColors::content_text(ctx.theme, state);
            let cursor_byte = display
                .char_indices()
                .nth(self.cursor())
                .map_or(display.len(), |(i, _)| i);
            let before_w = ctx.backend.measure_text(&display[..cursor_byte], fs);
            let scroll = before_w.saturating_sub(max_w.saturating_sub(1)) as i32;
            ctx.backend.push_clip_rect(tx, y, max_w, h)?;
            let drawn = ctx.backend.draw_text(&display, tx - scroll, ty, fs, fg);
            ctx.backend.pop_clip_rect()?;
            drawn?;
            if max_w > 0 {
                let cursor_x = tx + before_w as i32 - scroll;
                ctx.backend
                    .fill_rect(cursor_x, ty, 1, text_h, ctx.theme.text_primary)?;
            }
        }
        Ok(())
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

    // -- Draw / measure tests using RecordingBackend --

    use crate::context::DrawContext;
    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_width() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
        let mut backend = RecordingBackend::default();
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
            let mut backend = RecordingBackend::default();
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
    fn disabled_blocks_insert() {
        let mut f = InputField::new();
        f.disabled = true;
        f.insert('A');
        assert!(f.text.is_empty());
    }

    #[test]
    fn disabled_blocks_backspace() {
        let mut f = InputField::new();
        f.insert('A');
        f.disabled = true;
        f.backspace();
        assert_eq!(f.text, "A");
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

    #[test]
    fn each_state_has_distinct_fill() {
        crate::test_utils::assert_states_distinct(|st, ctx| {
            let mut f = InputField::new();
            f.hovered = st == WidgetState::Hover;
            f.pressed = st == WidgetState::Pressed;
            f.disabled = st == WidgetState::Disabled;
            f.draw(ctx, 0, 0, 100, 20).unwrap();
        });
    }

    #[test]
    fn resting_fill_is_input_bg() {
        let theme = Theme::dark();
        let fills = crate::test_utils::fill_colors_of(|ctx| {
            InputField::new().draw(ctx, 0, 0, 100, 20).unwrap();
        });
        assert_eq!(fills.first(), Some(&theme.input_bg));
    }

    #[test]
    fn password_mask_counts_characters_not_bytes() {
        let mut f = InputField::new();
        f.password_mode = true;
        for ch in "h\u{e9}\u{1f600}".chars() {
            f.insert(ch);
        }
        assert_eq!(f.display_text(), "***");
    }

    #[test]
    fn stale_cursor_is_clamped_before_editing() {
        let mut f = InputField::new();
        f.text = "abc".into();
        f.cursor_pos = 99;
        f.insert('d');
        assert_eq!((f.text.as_str(), f.cursor_pos), ("abcd", 4));
        f.cursor_pos = 99;
        f.backspace();
        assert_eq!((f.text.as_str(), f.cursor_pos), ("abc", 3));
    }

    #[test]
    fn cursor_movement_and_delete() {
        let mut f = InputField::new();
        for ch in "h\u{e9}llo".chars() {
            f.insert(ch);
        }
        f.move_home();
        f.move_right();
        f.delete(); // removes the two-byte e-acute
        assert_eq!(f.text, "hllo");
        f.move_end();
        f.delete(); // nothing after the cursor
        assert_eq!((f.text.as_str(), f.cursor_pos), ("hllo", 4));
        f.move_left();
        f.backspace();
        assert_eq!((f.text.as_str(), f.cursor_pos), ("hlo", 2));
        f.move_home();
        f.move_left();
        assert_eq!(f.cursor_pos, 0);
    }

    #[test]
    fn focused_cursor_stays_inside_a_long_field() {
        let theme = crate::theme::Theme::dark();
        let mut backend = crate::test_utils::RecordingBackend::default();
        let mut f = InputField::new();
        f.focused = true;
        for ch in "a very long line of text that overflows".chars() {
            f.insert(ch);
        }
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            f.draw(&mut ctx, 10, 0, 60, 20).unwrap();
        }
        // The 1 px cursor is the only text_primary-colored 1-wide fill.
        let cursor_x = backend
            .commands()
            .iter()
            .rev()
            .find_map(|c| match c {
                oasis_test_backend::DrawCommand::FillRect { x, w: 1, color, .. }
                    if *color == theme.text_primary =>
                {
                    Some(*x)
                },
                _ => None,
            })
            .expect("cursor drawn");
        assert!((14..14 + 52).contains(&cursor_x), "cursor at {cursor_x}");
    }
}

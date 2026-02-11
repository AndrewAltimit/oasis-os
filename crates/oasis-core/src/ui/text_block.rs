//! Text block widget: multiline text with wrapping, truncation, alignment.

use crate::backend::Color;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout::HAlign;
use crate::ui::widget::Widget;

/// A block of text with optional wrapping and alignment.
pub struct TextBlock {
    pub text: String,
    pub font_size: u16,
    pub color: Option<Color>,
    pub max_lines: Option<u32>,
    pub align: HAlign,
    pub line_height: u32,
}

impl TextBlock {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_size: 0, // 0 = use theme default.
            color: None,
            max_lines: None,
            align: HAlign::Left,
            line_height: 0,
        }
    }

    fn effective_font_size(&self, ctx: &DrawContext<'_>) -> u16 {
        if self.font_size > 0 {
            self.font_size
        } else {
            ctx.theme.font_size_md
        }
    }
}

impl Widget for TextBlock {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = self.effective_font_size(ctx);
        let lh = if self.line_height > 0 {
            self.line_height
        } else {
            ctx.backend.measure_text_height(fs)
        };

        let mut lines = 0u32;
        let mut max_w = 0u32;
        for line in self.text.split('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.is_empty() {
                lines += 1;
                continue;
            }
            let mut current = String::new();
            for word in words {
                let test = if current.is_empty() {
                    word.to_string()
                } else {
                    format!("{current} {word}")
                };
                if ctx.backend.measure_text(&test, fs) > available_w && !current.is_empty() {
                    max_w = max_w.max(ctx.backend.measure_text(&current, fs));
                    lines += 1;
                    current = word.to_string();
                } else {
                    current = test;
                }
            }
            if !current.is_empty() {
                max_w = max_w.max(ctx.backend.measure_text(&current, fs));
                lines += 1;
            }
        }
        if let Some(ml) = self.max_lines {
            lines = lines.min(ml);
        }
        (max_w, lines * lh)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, _h: u32) -> Result<()> {
        let fs = self.effective_font_size(ctx);
        let color = self.color.unwrap_or(ctx.theme.text_primary);
        let lh = if self.line_height > 0 {
            self.line_height
        } else {
            ctx.backend.measure_text_height(fs)
        };
        ctx.backend
            .draw_text_wrapped(&self.text, x, y, fs, color, w, lh)?;
        Ok(())
    }
}

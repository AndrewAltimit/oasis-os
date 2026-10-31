//! Text block widget: multiline text with wrapping, truncation, alignment.

use crate::context::DrawContext;
use crate::layout::{self, HAlign};
use crate::widget::Widget;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

/// A block of text with optional wrapping and alignment.
pub struct TextBlock {
    /// Text content.
    pub text: String,
    /// Font size (0 = use theme default).
    pub font_size: u16,
    /// Optional text color override.
    pub color: Option<Color>,
    /// Maximum lines before truncation.
    pub max_lines: Option<u32>,
    /// Horizontal text alignment.
    pub align: HAlign,
    /// Line height in pixels (0 = use font height).
    pub line_height: u32,
}

impl TextBlock {
    /// Create a new text block.
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

    fn effective_line_height(&self, ctx: &DrawContext<'_>, font_size: u16) -> u32 {
        if self.line_height > 0 {
            self.line_height
        } else {
            ctx.backend.measure_text_height(font_size)
        }
    }

    /// The text word-wrapped to `max_w` pixels (explicit `\n` breaks kept,
    /// a word wider than `max_w` on a line of its own), capped at
    /// `max_lines`. `measure` and `draw` share it, so the drawn lines are
    /// exactly the measured ones (`draw` used to wrap separately and
    /// ignore both `max_lines` and `align`).
    fn lines(&self, backend: &dyn SdiBackend, font_size: u16, max_w: u32) -> Vec<String> {
        let cap = self
            .max_lines
            .map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let mut out = Vec::new();
        for line in self.text.split('\n') {
            let mut current = String::new();
            for word in line.split_whitespace() {
                let candidate = if current.is_empty() {
                    word.to_string()
                } else {
                    format!("{current} {word}")
                };
                if !current.is_empty() && backend.measure_text(&candidate, font_size) > max_w {
                    out.push(std::mem::replace(&mut current, word.to_string()));
                } else {
                    current = candidate;
                }
            }
            out.push(current);
            if out.len() >= cap {
                break;
            }
        }
        out.truncate(cap);
        out
    }
}

impl Widget for TextBlock {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = self.effective_font_size(ctx);
        let lh = self.effective_line_height(ctx, fs);
        let lines = self.lines(&*ctx.backend, fs, available_w);
        let max_w = lines
            .iter()
            .map(|l| ctx.backend.measure_text(l, fs))
            .max()
            .unwrap_or(0);
        let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        (max_w, count.saturating_mul(lh))
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, _h: u32) -> Result<()> {
        let fs = self.effective_font_size(ctx);
        let color = self.color.unwrap_or(ctx.theme.text_primary);
        let lh = i64::from(self.effective_line_height(ctx, fs));
        for (i, line) in self.lines(&*ctx.backend, fs, w).iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let ly = i64::from(y) + i as i64 * lh;
            let Ok(ly) = i32::try_from(ly) else {
                break;
            };
            let lw = ctx.backend.measure_text(line, fs);
            let lx = x.saturating_add(layout::align_x(w, lw, self.align));
            ctx.backend.draw_text(line, lx, ly, fs, color)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;

    fn drawn(tb: &TextBlock, w: u32) -> Vec<(String, i32, i32)> {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tb.draw(&mut ctx, 0, 0, w, 500).unwrap();
        }
        backend
            .text_positions()
            .into_iter()
            .map(|(t, x, y, _)| (t.to_string(), x, y))
            .collect()
    }

    #[test]
    fn draw_honors_max_lines_and_matches_measure() {
        let mut tb = TextBlock::new("one\ntwo\nthree\nfour");
        tb.max_lines = Some(2);
        let lines = drawn(&tb, 200);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1].0, "two");
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        let lh = ctx.backend.measure_text_height(theme.font_size_md);
        assert_eq!(tb.measure(&ctx, 200, 500).1, 2 * lh);
    }

    #[test]
    fn draw_honors_alignment() {
        let mut tb = TextBlock::new("hi");
        tb.align = HAlign::Right;
        let (_, x, _) = drawn(&tb, 200).remove(0);
        assert!(x > 100, "right-aligned text drawn at x={x}");
        tb.align = HAlign::Center;
        let (_, cx, _) = drawn(&tb, 200).remove(0);
        assert!(cx > 0 && cx < x);
    }

    #[test]
    fn wraps_words_and_keeps_blank_lines() {
        let tb = TextBlock::new("aaa bbb\n\nccc");
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let w = {
            let ctx = DrawContext::new(&mut backend, &theme);
            ctx.backend.measure_text("aaa", theme.font_size_md)
        };
        let lines = drawn(&tb, w);
        let texts: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(texts, ["aaa", "bbb", "ccc"]);
        // The blank line still takes its row: "ccc" is on the fourth line.
        assert!(lines[2].2 > lines[1].2 + (lines[1].2 - lines[0].2));
    }
}

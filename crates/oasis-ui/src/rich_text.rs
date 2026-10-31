//! RichText widget: simple formatted text with inline styling.

use crate::context::DrawContext;
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

/// A styled text span with optional formatting.
#[derive(Debug, Clone)]
pub struct Span {
    /// The text content.
    pub text: String,
    /// Whether the text is bold (drawn slightly offset for faux bold).
    pub bold: bool,
    /// Custom color (None = theme default).
    pub color: Option<Color>,
    /// Custom font size (None = theme default).
    pub font_size: Option<u16>,
}

impl Span {
    /// Create a plain text span.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            color: None,
            font_size: None,
        }
    }

    /// Create a bold text span.
    pub fn bold(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: true,
            color: None,
            font_size: None,
        }
    }

    /// Create a colored text span.
    pub fn colored(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            bold: false,
            color: Some(color),
            font_size: None,
        }
    }
}

/// A block of rich text composed of styled spans.
pub struct RichText {
    /// Lines of spans (each inner Vec is one line).
    pub lines: Vec<Vec<Span>>,
}

impl Default for RichText {
    fn default() -> Self {
        Self::new()
    }
}

impl RichText {
    /// Create empty rich text.
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Add a line of spans.
    pub fn add_line(&mut self, spans: Vec<Span>) {
        self.lines.push(spans);
    }

    /// Add a single plain text line.
    pub fn add_plain(&mut self, text: impl Into<String>) {
        self.lines.push(vec![Span::plain(text)]);
    }

    /// Total number of lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// `(ascent, height)` of one line: its spans share a baseline
    /// `ascent` px below the line top (the largest span ascent), and the
    /// line reaches down to the lowest span descent. A line with no spans
    /// has the theme size's metrics. Lines used to be one theme size tall
    /// (so a larger span overlapped the next line) and their spans
    /// top-aligned (so mixed sizes sat on different baselines).
    fn line_metrics(ctx: &DrawContext<'_>, line: &[Span]) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;
        let metrics = |size: u16| {
            let height = ctx.backend.measure_text_height(size);
            (ctx.backend.font_ascent(size).min(height), height)
        };
        if line.is_empty() {
            return metrics(fs);
        }
        let (mut ascent, mut descent) = (0, 0);
        for span in line {
            let (a, h) = metrics(span.font_size.unwrap_or(fs));
            ascent = ascent.max(a);
            descent = descent.max(h - a);
        }
        (ascent, ascent + descent)
    }
}

/// Vertical gap between lines.
const LINE_GAP: u32 = 2;

impl Widget for RichText {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;

        let mut max_w = 0u32;
        for line in &self.lines {
            let mut w = 0u32;
            for span in line {
                let sfs = span.font_size.unwrap_or(fs);
                w += ctx.backend.measure_text(&span.text, sfs);
            }
            max_w = max_w.max(w);
        }

        let h = self
            .lines
            .iter()
            .map(|line| Self::line_metrics(ctx, line).1)
            .sum::<u32>()
            + LINE_GAP * self.lines.len().saturating_sub(1) as u32;

        (max_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, _w: u32, _h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let default_color = ctx.theme.text_primary;

        let mut ly = y;
        for line in &self.lines {
            let (baseline, line_h) = Self::line_metrics(ctx, line);
            let mut lx = x;

            for span in line {
                let sfs = span.font_size.unwrap_or(fs);
                let color = span.color.unwrap_or(default_color);
                // Drop smaller spans so every span sits on the baseline.
                let ascent = ctx
                    .backend
                    .font_ascent(sfs)
                    .min(ctx.backend.measure_text_height(sfs));
                let sy = ly + (baseline - ascent) as i32;

                ctx.backend.draw_text(&span.text, lx, sy, sfs, color)?;

                // Faux bold: draw again offset by 1 pixel.
                if span.bold {
                    ctx.backend.draw_text(&span.text, lx + 1, sy, sfs, color)?;
                }

                lx += ctx.backend.measure_text(&span.text, sfs) as i32;
            }
            ly += (line_h + LINE_GAP) as i32;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_span_grows_its_line_and_pushes_the_next_down() {
        use crate::test_utils::RecordingBackend;
        let theme = crate::theme::Theme::dark();
        let mut backend = RecordingBackend::default();
        let mut rt = RichText::new();
        let mut big = Span::plain("BIG");
        big.font_size = Some(theme.font_size_md * 3);
        rt.add_line(vec![Span::plain("a"), big]);
        rt.add_plain("next");
        let (h_measured, big_h);
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            h_measured = rt.measure(&ctx, 480, 272).1;
            big_h = ctx.backend.measure_text_height(theme.font_size_md * 3);
            rt.draw(&mut ctx, 0, 0, 480, 272).unwrap();
        }
        let next_y = backend
            .text_positions()
            .into_iter()
            .find(|t| t.0 == "next")
            .unwrap()
            .2;
        assert!(
            next_y >= big_h as i32,
            "next line at {next_y} overlaps the {big_h}px span"
        );
        assert!(h_measured >= big_h + LINE_GAP);
    }

    #[test]
    fn mixed_size_spans_share_a_baseline() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let mut rt = RichText::new();
        let mut big = Span::plain("BIG");
        big.font_size = Some(theme.font_size_md * 2);
        rt.add_line(vec![Span::plain("small"), big]);
        let (small_asc, big_asc, line_h, measured_h);
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            small_asc = ctx.backend.font_ascent(theme.font_size_md) as i32;
            big_asc = ctx.backend.font_ascent(theme.font_size_md * 2) as i32;
            line_h = RichText::line_metrics(&ctx, &rt.lines[0]).1;
            measured_h = rt.measure(&ctx, 480, 272).1;
            rt.draw(&mut ctx, 0, 10, 480, 272).unwrap();
        }
        assert!(big_asc > small_asc, "sizes differ in ascent");
        let y_of = |label: &str| {
            backend
                .text_positions()
                .into_iter()
                .find(|t| t.0 == label)
                .unwrap()
                .2
        };
        let (small_y, big_y) = (y_of("small"), y_of("BIG"));
        assert_eq!(small_y + small_asc, big_y + big_asc, "baselines differ");
        assert_eq!(big_y, 10, "the tallest span starts at the line top");
        assert_eq!(measured_h, line_h);
    }

    #[test]
    fn new_empty() {
        let rt = RichText::new();
        assert_eq!(rt.line_count(), 0);
    }

    #[test]
    fn add_plain_line() {
        let mut rt = RichText::new();
        rt.add_plain("Hello");
        assert_eq!(rt.line_count(), 1);
        assert_eq!(rt.lines[0][0].text, "Hello");
        assert!(!rt.lines[0][0].bold);
    }

    #[test]
    fn add_styled_line() {
        let mut rt = RichText::new();
        rt.add_line(vec![
            Span::plain("Hello "),
            Span::bold("world"),
            Span::colored("!", Color::rgb(255, 0, 0)),
        ]);
        assert_eq!(rt.line_count(), 1);
        assert_eq!(rt.lines[0].len(), 3);
        assert!(rt.lines[0][1].bold);
        assert_eq!(rt.lines[0][2].color, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn span_constructors() {
        let p = Span::plain("a");
        assert!(!p.bold);
        assert!(p.color.is_none());

        let b = Span::bold("b");
        assert!(b.bold);

        let c = Span::colored("c", Color::WHITE);
        assert_eq!(c.color, Some(Color::WHITE));
    }

    use crate::context::DrawContext;
    use crate::test_utils::RecordingBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn draw_shows_all_spans() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut rt = RichText::new();
            rt.add_line(vec![Span::plain("Hello "), Span::bold("world")]);
            rt.draw(&mut ctx, 0, 0, 200, 50).unwrap();
        }
        assert!(backend.has_text("Hello "));
        assert!(backend.has_text("world"));
    }

    #[test]
    fn draw_empty_no_panic() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let rt = RichText::new();
            rt.draw(&mut ctx, 0, 0, 200, 50).unwrap();
        }
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn measure_empty_zero() {
        let theme = Theme::dark();
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        let rt = RichText::new();
        let (_, h) = rt.measure(&ctx, 200, 200);
        assert_eq!(h, 0);
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let mut rt = RichText::new();
            rt.add_plain("Test");
            rt.draw(ctx, 0, 0, 200, 50).unwrap();
        });
    }
}

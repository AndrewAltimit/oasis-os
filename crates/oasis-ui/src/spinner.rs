//! Spinner and loading indicator widgets.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Character frames for the text-based spinner animation.
const SPINNER_FRAMES: &[char] = &['|', '/', '-', '\\'];

/// Visual style of the spinner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerStyle {
    /// Rotating character sequence (|, /, -, \).
    Text,
    /// Small dots that pulse in sequence.
    Dots,
    /// Indeterminate sliding bar.
    Bar,
}

/// A loading spinner widget.
///
/// Cycles through animation frames at a configurable speed. When the
/// theme has `reduced_motion` enabled, the spinner displays a static
/// indicator instead of animating.
pub struct Spinner {
    /// Visual style variant.
    pub style: SpinnerStyle,
    /// Accumulated time in milliseconds (drives frame selection).
    pub elapsed_ms: u32,
    /// Time per frame in milliseconds.
    pub frame_duration_ms: u32,
    /// Optional label drawn next to the spinner.
    pub label: Option<String>,
}

impl Spinner {
    /// Create a new spinner with default settings.
    pub fn new() -> Self {
        Self {
            style: SpinnerStyle::Text,
            elapsed_ms: 0,
            frame_duration_ms: 150,
            label: None,
        }
    }

    /// Create a spinner with a label.
    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::new()
        }
    }

    /// Advance the spinner animation by `dt_ms` milliseconds.
    pub fn tick(&mut self, dt_ms: u32) {
        self.elapsed_ms = self.elapsed_ms.wrapping_add(dt_ms);
    }

    /// Current frame index based on elapsed time.
    pub fn frame_index(&self) -> usize {
        let total_frames = self.frame_count();
        if total_frames == 0 {
            return 0;
        }
        let effective_duration = self.frame_duration_ms.max(1);
        ((self.elapsed_ms / effective_duration) as usize) % total_frames
    }

    /// Total number of frames for the current style.
    fn frame_count(&self) -> usize {
        match self.style {
            SpinnerStyle::Text => SPINNER_FRAMES.len(),
            SpinnerStyle::Dots => 3,
            SpinnerStyle::Bar => 8,
        }
    }

    /// Current display character for `Text` style.
    pub fn current_char(&self) -> char {
        SPINNER_FRAMES[self.frame_index() % SPINNER_FRAMES.len()]
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Spinner {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;
        let spinner_w = ctx.backend.measure_text("-", fs) + 4;
        let text_h = ctx.backend.measure_text_height(fs);
        let h = text_h + 4;
        let label_w = if let Some(ref label) = self.label {
            ctx.backend.measure_text(label, fs) + 6
        } else {
            0
        };
        let w = (spinner_w + label_w).min(available_w);
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let ty = y + layout::center(h, text_h);

        match self.style {
            SpinnerStyle::Text => {
                let ch = if ctx.theme.reduced_motion {
                    '*' // Static indicator when motion is reduced.
                } else {
                    self.current_char()
                };
                let s = ch.to_string();
                ctx.backend.draw_text(&s, x + 2, ty, fs, ctx.theme.accent)?;

                if let Some(ref label) = self.label {
                    let char_w = ctx.backend.measure_text(&s, fs);
                    let lx = x + 2 + char_w as i32 + 6;
                    ctx.backend
                        .draw_text(label, lx, ty, fs, ctx.theme.text_secondary)?;
                }
            },
            SpinnerStyle::Dots => {
                let dot_size = 4u32;
                let gap = 3u32;
                let frame = if ctx.theme.reduced_motion {
                    // Show all dots equally when motion reduced.
                    usize::MAX
                } else {
                    self.frame_index()
                };
                let dot_y = y + layout::center(h, dot_size);
                for i in 0..3u32 {
                    let dot_x = x + 2 + (i * (dot_size + gap)) as i32;
                    let alpha = if ctx.theme.reduced_motion {
                        180u8
                    } else if i as usize == frame {
                        255u8
                    } else {
                        80u8
                    };
                    let color = ctx.theme.accent.with_alpha(alpha);
                    ctx.backend
                        .fill_rect(dot_x, dot_y, dot_size, dot_size, color)?;
                }

                if let Some(ref label) = self.label {
                    let dots_w = 3 * dot_size + 2 * gap + 4;
                    let lx = x + 2 + dots_w as i32 + 4;
                    ctx.backend
                        .draw_text(label, lx, ty, fs, ctx.theme.text_secondary)?;
                }
            },
            SpinnerStyle::Bar => {
                let bar_h = 4u32.min(h);
                let bar_y = y + layout::center(h, bar_h);
                let bar_w = w.saturating_sub(4);

                // Track.
                ctx.backend
                    .fill_rect(x + 2, bar_y, bar_w, bar_h, ctx.theme.scrollbar_track)?;

                if ctx.theme.reduced_motion {
                    // Static 25% fill in the center.
                    let fill_w = bar_w / 4;
                    let fill_x = x + 2 + ((bar_w - fill_w) / 2) as i32;
                    ctx.backend
                        .fill_rect(fill_x, bar_y, fill_w, bar_h, ctx.theme.accent)?;
                } else {
                    // Sliding indicator.
                    let fill_w = bar_w / 4;
                    let frame = self.frame_index();
                    let travel = bar_w.saturating_sub(fill_w);
                    let total_frames = self.frame_count();
                    let pos = if total_frames > 0 {
                        (travel * frame as u32) / total_frames as u32
                    } else {
                        0
                    };
                    ctx.backend.fill_rect(
                        x + 2 + pos as i32,
                        bar_y,
                        fill_w,
                        bar_h,
                        ctx.theme.accent,
                    )?;
                }
            },
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let s = Spinner::new();
        assert_eq!(s.style, SpinnerStyle::Text);
        assert_eq!(s.elapsed_ms, 0);
        assert_eq!(s.frame_duration_ms, 150);
        assert!(s.label.is_none());
    }

    #[test]
    fn default_same_as_new() {
        let s = Spinner::default();
        assert_eq!(s.style, SpinnerStyle::Text);
        assert_eq!(s.elapsed_ms, 0);
    }

    #[test]
    fn with_label_sets_label() {
        let s = Spinner::with_label("Loading...");
        assert_eq!(s.label.as_deref(), Some("Loading..."));
    }

    #[test]
    fn tick_advances_time() {
        let mut s = Spinner::new();
        s.tick(100);
        assert_eq!(s.elapsed_ms, 100);
        s.tick(50);
        assert_eq!(s.elapsed_ms, 150);
    }

    #[test]
    fn frame_index_zero_initially() {
        let s = Spinner::new();
        assert_eq!(s.frame_index(), 0);
    }

    #[test]
    fn frame_index_advances_with_time() {
        let mut s = Spinner::new();
        // frame_duration_ms = 150, 4 frames for Text style.
        s.tick(150);
        assert_eq!(s.frame_index(), 1);
        s.tick(150);
        assert_eq!(s.frame_index(), 2);
        s.tick(150);
        assert_eq!(s.frame_index(), 3);
    }

    #[test]
    fn frame_index_wraps_around() {
        let mut s = Spinner::new();
        // 4 frames * 150ms = 600ms for a full cycle.
        s.tick(600);
        assert_eq!(s.frame_index(), 0);
        s.tick(150);
        assert_eq!(s.frame_index(), 1);
    }

    #[test]
    fn current_char_cycles() {
        let mut s = Spinner::new();
        assert_eq!(s.current_char(), '|');
        s.tick(150);
        assert_eq!(s.current_char(), '/');
        s.tick(150);
        assert_eq!(s.current_char(), '-');
        s.tick(150);
        assert_eq!(s.current_char(), '\\');
        s.tick(150);
        assert_eq!(s.current_char(), '|');
    }

    #[test]
    fn dots_style_frame_count() {
        let mut s = Spinner::new();
        s.style = SpinnerStyle::Dots;
        // 3 frames for dots.
        assert_eq!(s.frame_index(), 0);
        s.tick(150);
        assert_eq!(s.frame_index(), 1);
        s.tick(150);
        assert_eq!(s.frame_index(), 2);
        s.tick(150);
        assert_eq!(s.frame_index(), 0);
    }

    #[test]
    fn bar_style_frame_count() {
        let mut s = Spinner::new();
        s.style = SpinnerStyle::Bar;
        // 8 frames for bar.
        s.tick(150 * 7);
        assert_eq!(s.frame_index(), 7);
        s.tick(150);
        assert_eq!(s.frame_index(), 0);
    }

    #[test]
    fn zero_frame_duration_no_panic() {
        let mut s = Spinner::new();
        s.frame_duration_ms = 0;
        // Should not panic or divide by zero.
        assert_eq!(s.frame_index(), 0);
        s.tick(100);
        let _ = s.frame_index();
    }

    #[test]
    fn style_variants_distinct() {
        assert_ne!(SpinnerStyle::Text, SpinnerStyle::Dots);
        assert_ne!(SpinnerStyle::Dots, SpinnerStyle::Bar);
        assert_ne!(SpinnerStyle::Text, SpinnerStyle::Bar);
    }

    #[test]
    fn tick_wrapping() {
        let mut s = Spinner::new();
        s.elapsed_ms = u32::MAX - 10;
        s.tick(20); // Should wrap around.
        assert_eq!(s.elapsed_ms, 9);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_returns_nonzero() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let s = Spinner::new();
        let (w, h) = s.measure(&ctx, 200, 100);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn measure_with_label_wider() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let no_label = Spinner::new();
        let with_label = Spinner::with_label("Loading...");
        let (w1, _) = no_label.measure(&ctx, 200, 100);
        let (w2, _) = with_label.measure(&ctx, 200, 100);
        assert!(w2 > w1);
    }

    #[test]
    fn draw_text_style_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let s = Spinner::new();
            s.draw(&mut ctx, 0, 0, 40, 20).unwrap();
        }
        assert!(backend.draw_text_count() > 0);
    }

    #[test]
    fn draw_text_style_with_label() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let s = Spinner::with_label("Wait");
            s.draw(&mut ctx, 0, 0, 100, 20).unwrap();
        }
        assert!(backend.has_text("Wait"));
    }

    #[test]
    fn draw_dots_style_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::new();
            s.style = SpinnerStyle::Dots;
            s.draw(&mut ctx, 0, 0, 60, 20).unwrap();
        }
        // Dots emit fill_rect calls.
        assert!(backend.fill_rect_count() >= 3);
    }

    #[test]
    fn draw_dots_with_label() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::with_label("Connecting");
            s.style = SpinnerStyle::Dots;
            s.draw(&mut ctx, 0, 0, 150, 20).unwrap();
        }
        assert!(backend.has_text("Connecting"));
    }

    #[test]
    fn draw_bar_style_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::new();
            s.style = SpinnerStyle::Bar;
            s.draw(&mut ctx, 0, 0, 100, 20).unwrap();
        }
        // Bar draws at least track + fill.
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_reduced_motion_text() {
        let mut theme = Theme::dark();
        theme.reduced_motion = true;
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let s = Spinner::new();
            s.draw(&mut ctx, 0, 0, 40, 20).unwrap();
        }
        // Should draw '*' character instead of animated frame.
        assert!(backend.has_text("*"));
    }

    #[test]
    fn draw_reduced_motion_dots() {
        let mut theme = Theme::dark();
        theme.reduced_motion = true;
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::new();
            s.style = SpinnerStyle::Dots;
            s.draw(&mut ctx, 0, 0, 60, 20).unwrap();
        }
        // Should still draw 3 dots (all at equal alpha).
        assert!(backend.fill_rect_count() >= 3);
    }

    #[test]
    fn draw_reduced_motion_bar() {
        let mut theme = Theme::dark();
        theme.reduced_motion = true;
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::new();
            s.style = SpinnerStyle::Bar;
            s.draw(&mut ctx, 0, 0, 100, 20).unwrap();
        }
        // Should draw track + static fill.
        assert!(backend.fill_rect_count() >= 2);
    }

    #[test]
    fn draw_bar_at_different_frames() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut s = Spinner::new();
            s.style = SpinnerStyle::Bar;
            s.tick(300); // Advance 2 frames.
            s.draw(&mut ctx, 0, 0, 100, 20).unwrap();
        }
        assert!(backend.fill_rect_count() >= 2);
    }
}

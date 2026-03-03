//! Tooltip widget: hover-activated text overlay.
//!
//! Tooltips appear near a target widget after a configurable hover
//! delay. They position themselves to stay within the viewport,
//! preferring to appear below the target.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Preferred position of the tooltip relative to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipPosition {
    /// Display above the target.
    Above,
    /// Display below the target.
    Below,
    /// Display to the left of the target.
    Left,
    /// Display to the right of the target.
    Right,
}

/// Current visibility state of the tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipState {
    /// Tooltip is hidden.
    Hidden,
    /// Hover has started; waiting for the delay to elapse.
    Waiting,
    /// Tooltip is visible.
    Visible,
}

/// A tooltip overlay with configurable delay and positioning.
pub struct Tooltip {
    /// Tooltip text content.
    pub text: String,
    /// Preferred position relative to the target.
    pub position: TooltipPosition,
    /// Current visibility state.
    pub state: TooltipState,
    /// Delay in milliseconds before showing after hover starts.
    pub delay_ms: u32,
    /// Accumulated hover time in milliseconds.
    pub elapsed_ms: u32,
    /// Horizontal padding inside the tooltip.
    pub pad_h: u32,
    /// Vertical padding inside the tooltip.
    pub pad_v: u32,
    /// Gap between the tooltip and the target.
    pub gap: u32,
}

impl Tooltip {
    /// Create a new tooltip with default delay and positioning.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            position: TooltipPosition::Below,
            delay_ms: 500,
            state: TooltipState::Hidden,
            elapsed_ms: 0,
            pad_h: 6,
            pad_v: 3,
            gap: 4,
        }
    }

    /// Set the preferred position.
    pub fn with_position(mut self, pos: TooltipPosition) -> Self {
        self.position = pos;
        self
    }

    /// Set the hover delay in milliseconds.
    pub fn with_delay(mut self, ms: u32) -> Self {
        self.delay_ms = ms;
        self
    }

    /// Notify that the cursor has entered the target area.
    pub fn on_hover_start(&mut self) {
        if self.state == TooltipState::Hidden {
            self.state = TooltipState::Waiting;
            self.elapsed_ms = 0;
        }
    }

    /// Notify that the cursor has left the target area.
    pub fn on_hover_end(&mut self) {
        self.state = TooltipState::Hidden;
        self.elapsed_ms = 0;
    }

    /// Advance the tooltip timer by `dt_ms` milliseconds.
    ///
    /// If the accumulated time exceeds the delay, the tooltip
    /// becomes visible.
    pub fn tick(&mut self, dt_ms: u32) {
        if self.state == TooltipState::Waiting {
            self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
            if self.elapsed_ms >= self.delay_ms {
                self.state = TooltipState::Visible;
            }
        }
    }

    /// Whether the tooltip should be drawn.
    pub fn is_visible(&self) -> bool {
        self.state == TooltipState::Visible
    }

    /// Compute the tooltip rectangle given an anchor and
    /// viewport bounds. The tooltip is clamped to stay within
    /// `(0, 0, vw, vh)`.
    pub fn compute_rect(
        &self,
        ctx: &DrawContext<'_>,
        anchor: &TooltipAnchor,
    ) -> (i32, i32, u32, u32) {
        let target_x = anchor.target_x;
        let target_y = anchor.target_y;
        let target_w = anchor.target_w;
        let target_h = anchor.target_h;
        let viewport_w = anchor.viewport_w;
        let viewport_h = anchor.viewport_h;
        let fs = ctx.theme.font_size_xs;
        let text_w = ctx.backend.measure_text(&self.text, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        let tw = text_w + self.pad_h * 2;
        let th = text_h + self.pad_v * 2;
        let gap = self.gap as i32;

        // Compute ideal position based on preference.
        let (mut x, mut y) = match self.position {
            TooltipPosition::Below => {
                let x = target_x + layout::center(target_w, tw);
                let y = target_y + target_h as i32 + gap;
                (x, y)
            },
            TooltipPosition::Above => {
                let x = target_x + layout::center(target_w, tw);
                let y = target_y - th as i32 - gap;
                (x, y)
            },
            TooltipPosition::Right => {
                let x = target_x + target_w as i32 + gap;
                let y = target_y + layout::center(target_h, th);
                (x, y)
            },
            TooltipPosition::Left => {
                let x = target_x - tw as i32 - gap;
                let y = target_y + layout::center(target_h, th);
                (x, y)
            },
        };

        // Clamp to viewport bounds.
        x = x.clamp(0, (viewport_w as i32 - tw as i32).max(0));
        y = y.clamp(0, (viewport_h as i32 - th as i32).max(0));

        (x, y, tw, th)
    }

    /// Draw the tooltip at the computed position.
    ///
    /// This is a convenience method that computes the rect and
    /// draws in one call.
    pub fn draw_at(&self, ctx: &mut DrawContext<'_>, anchor: &TooltipAnchor) -> Result<()> {
        if !self.is_visible() || self.text.is_empty() {
            return Ok(());
        }
        let (tx, ty, tw, th) = self.compute_rect(ctx, anchor);
        self.draw(ctx, tx, ty, tw, th)
    }
}

/// Describes the target widget position and viewport bounds
/// for tooltip placement calculations.
#[derive(Debug, Clone, Copy)]
pub struct TooltipAnchor {
    /// X position of the target widget.
    pub target_x: i32,
    /// Y position of the target widget.
    pub target_y: i32,
    /// Width of the target widget.
    pub target_w: u32,
    /// Height of the target widget.
    pub target_h: u32,
    /// Viewport width for edge clamping.
    pub viewport_w: u32,
    /// Viewport height for edge clamping.
    pub viewport_h: u32,
}

impl TooltipAnchor {
    /// Create a new tooltip anchor.
    pub fn new(
        target_x: i32,
        target_y: i32,
        target_w: u32,
        target_h: u32,
        viewport_w: u32,
        viewport_h: u32,
    ) -> Self {
        Self {
            target_x,
            target_y,
            target_w,
            target_h,
            viewport_w,
            viewport_h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a `TooltipAnchor` concisely.
    fn anchor(tx: i32, ty: i32, tw: u32, th: u32, vw: u32, vh: u32) -> TooltipAnchor {
        TooltipAnchor::new(tx, ty, tw, th, vw, vh)
    }

    // -- State machine tests --

    #[test]
    fn new_defaults() {
        let t = Tooltip::new("Help text");
        assert_eq!(t.text, "Help text");
        assert_eq!(t.position, TooltipPosition::Below);
        assert_eq!(t.state, TooltipState::Hidden);
        assert_eq!(t.delay_ms, 500);
        assert_eq!(t.elapsed_ms, 0);
        assert_eq!(t.pad_h, 6);
        assert_eq!(t.pad_v, 3);
        assert_eq!(t.gap, 4);
    }

    #[test]
    fn with_position() {
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Above);
        assert_eq!(t.position, TooltipPosition::Above);
    }

    #[test]
    fn with_delay() {
        let t = Tooltip::new("Tip").with_delay(250);
        assert_eq!(t.delay_ms, 250);
    }

    #[test]
    fn hover_start_transitions_to_waiting() {
        let mut t = Tooltip::new("Tip");
        t.on_hover_start();
        assert_eq!(t.state, TooltipState::Waiting);
        assert_eq!(t.elapsed_ms, 0);
    }

    #[test]
    fn hover_start_while_waiting_is_idempotent() {
        let mut t = Tooltip::new("Tip");
        t.on_hover_start();
        t.tick(100);
        t.on_hover_start(); // should not reset elapsed
        assert_eq!(t.elapsed_ms, 100);
    }

    #[test]
    fn hover_end_resets_to_hidden() {
        let mut t = Tooltip::new("Tip");
        t.on_hover_start();
        t.tick(600);
        assert_eq!(t.state, TooltipState::Visible);
        t.on_hover_end();
        assert_eq!(t.state, TooltipState::Hidden);
        assert_eq!(t.elapsed_ms, 0);
    }

    #[test]
    fn tick_accumulates_time() {
        let mut t = Tooltip::new("Tip").with_delay(100);
        t.on_hover_start();
        t.tick(40);
        assert_eq!(t.state, TooltipState::Waiting);
        assert_eq!(t.elapsed_ms, 40);
        t.tick(40);
        assert_eq!(t.state, TooltipState::Waiting);
        assert_eq!(t.elapsed_ms, 80);
        t.tick(40);
        assert_eq!(t.state, TooltipState::Visible);
        assert_eq!(t.elapsed_ms, 120);
    }

    #[test]
    fn tick_does_nothing_when_hidden() {
        let mut t = Tooltip::new("Tip");
        t.tick(1000);
        assert_eq!(t.state, TooltipState::Hidden);
        assert_eq!(t.elapsed_ms, 0);
    }

    #[test]
    fn tick_does_nothing_when_visible() {
        let mut t = Tooltip::new("Tip").with_delay(50);
        t.on_hover_start();
        t.tick(100);
        assert_eq!(t.state, TooltipState::Visible);
        let before = t.elapsed_ms;
        t.tick(100);
        assert_eq!(t.elapsed_ms, before);
    }

    #[test]
    fn is_visible_states() {
        let mut t = Tooltip::new("Tip");
        assert!(!t.is_visible());
        t.on_hover_start();
        assert!(!t.is_visible());
        t.tick(600);
        assert!(t.is_visible());
        t.on_hover_end();
        assert!(!t.is_visible());
    }

    #[test]
    fn zero_delay_shows_immediately() {
        let mut t = Tooltip::new("Instant").with_delay(0);
        t.on_hover_start();
        t.tick(0);
        assert_eq!(t.state, TooltipState::Visible);
    }

    #[test]
    fn position_variants_debug() {
        for pos in [
            TooltipPosition::Above,
            TooltipPosition::Below,
            TooltipPosition::Left,
            TooltipPosition::Right,
        ] {
            let _ = format!("{pos:?}");
        }
    }

    #[test]
    fn state_variants_debug() {
        for state in [
            TooltipState::Hidden,
            TooltipState::Waiting,
            TooltipState::Visible,
        ] {
            let _ = format!("{state:?}");
        }
    }

    #[test]
    fn from_string_type() {
        let t = Tooltip::new(String::from("dynamic"));
        assert_eq!(t.text, "dynamic");
    }

    #[test]
    fn elapsed_saturates() {
        let mut t = Tooltip::new("Tip").with_delay(u32::MAX);
        t.on_hover_start();
        t.tick(u32::MAX);
        // Should not overflow.
        assert_eq!(t.elapsed_ms, u32::MAX);
    }

    // -- Positioning and drawing tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn compute_rect_below() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Test").with_position(TooltipPosition::Below);
        let (x, y, w, h) = t.compute_rect(&ctx, &anchor(100, 50, 40, 20, 480, 272));
        // Should be below the target.
        assert!(y >= 50 + 20);
        assert!(w > 0);
        assert!(h > 0);
        // Horizontally centered on target.
        let center_target = 100 + 20; // target center x
        let center_tooltip = x + w as i32 / 2;
        assert!(
            (center_target - center_tooltip).abs() <= 1,
            "tooltip x={x}, w={w} should center on target \
             center={center_target}"
        );
    }

    #[test]
    fn compute_rect_above() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Above);
        let (_x, y, _w, h) = t.compute_rect(&ctx, &anchor(100, 100, 40, 20, 480, 272));
        // Should be above the target.
        assert!(y + h as i32 <= 100);
    }

    #[test]
    fn compute_rect_left() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Left);
        let (x, _y, w, _h) = t.compute_rect(&ctx, &anchor(200, 100, 40, 20, 480, 272));
        assert!(x + w as i32 <= 200);
    }

    #[test]
    fn compute_rect_right() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Right);
        let (x, _y, _w, _h) = t.compute_rect(&ctx, &anchor(100, 100, 40, 20, 480, 272));
        assert!(x >= 100 + 40);
    }

    #[test]
    fn compute_rect_clamps_right_edge() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Long tooltip text here");
        let (x, _y, w, _h) = t.compute_rect(&ctx, &anchor(460, 100, 20, 20, 480, 272));
        assert!(
            x + w as i32 <= 480,
            "tooltip right edge ({}) should not exceed \
             viewport width (480)",
            x + w as i32
        );
    }

    #[test]
    fn compute_rect_clamps_bottom_edge() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Below);
        let (_x, y, _w, h) = t.compute_rect(&ctx, &anchor(100, 260, 40, 20, 480, 272));
        assert!(
            y + h as i32 <= 272,
            "tooltip bottom ({}) should not exceed viewport \
             height (272)",
            y + h as i32
        );
    }

    #[test]
    fn compute_rect_clamps_top_edge() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Above);
        let (_x, y, _w, _h) = t.compute_rect(&ctx, &anchor(100, 0, 40, 20, 480, 272));
        assert!(y >= 0, "tooltip y ({y}) should not go negative");
    }

    #[test]
    fn compute_rect_clamps_left_edge() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Tip").with_position(TooltipPosition::Left);
        let (x, _y, _w, _h) = t.compute_rect(&ctx, &anchor(0, 100, 10, 20, 480, 272));
        assert!(x >= 0, "tooltip x ({x}) should not go negative");
    }

    #[test]
    fn measure_returns_tooltip_dimensions() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Hello");
        let (w, h) = t.measure(&ctx, 480, 272);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn draw_visible_emits_fill_and_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = Tooltip::new("Info");
            t.state = TooltipState::Visible;
            t.draw(&mut ctx, 10, 10, 40, 16).ok();
        }
        assert!(
            backend.fill_rect_count() > 0,
            "visible tooltip should draw background"
        );
        assert!(backend.has_text("Info"), "visible tooltip should draw text");
    }

    #[test]
    fn draw_hidden_emits_nothing() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Tooltip::new("Secret");
            t.draw(&mut ctx, 10, 10, 40, 16).ok();
        }
        assert_eq!(
            backend.fill_rect_count(),
            0,
            "hidden tooltip should not draw anything"
        );
        assert!(
            !backend.has_text("Secret"),
            "hidden tooltip should not draw text"
        );
    }

    #[test]
    fn draw_at_convenience_visible() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = Tooltip::new("Help");
            t.state = TooltipState::Visible;
            t.draw_at(&mut ctx, &anchor(100, 50, 40, 20, 480, 272)).ok();
        }
        assert!(backend.has_text("Help"));
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_at_convenience_hidden() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Tooltip::new("Nope");
            t.draw_at(&mut ctx, &anchor(100, 50, 40, 20, 480, 272)).ok();
        }
        assert!(!backend.has_text("Nope"));
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn draw_at_empty_text_no_output() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = Tooltip::new("");
            t.state = TooltipState::Visible;
            t.draw_at(&mut ctx, &anchor(100, 50, 40, 20, 480, 272)).ok();
        }
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let mut t = Tooltip::new("Theme test");
            t.state = TooltipState::Visible;
            t.draw(ctx, 0, 0, 80, 16).ok();
        });
    }

    #[test]
    fn draw_all_positions_no_panic() {
        let theme = Theme::dark();
        for pos in [
            TooltipPosition::Above,
            TooltipPosition::Below,
            TooltipPosition::Left,
            TooltipPosition::Right,
        ] {
            let mut backend = MockBackend::new();
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = Tooltip::new("Pos test").with_position(pos);
            t.state = TooltipState::Visible;
            t.draw_at(&mut ctx, &anchor(200, 100, 40, 20, 480, 272))
                .ok();
        }
    }

    #[test]
    fn compute_rect_tiny_viewport() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let t = Tooltip::new("Too big for viewport");
        let (x, y, _w, _h) = t.compute_rect(&ctx, &anchor(0, 0, 10, 10, 20, 20));
        assert!(x >= 0);
        assert!(y >= 0);
    }
}

impl Widget for Tooltip {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let fs = ctx.theme.font_size_xs;
        let text_w = ctx.backend.measure_text(&self.text, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        (text_w + self.pad_h * 2, text_h + self.pad_v * 2)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        if !self.is_visible() || self.text.is_empty() {
            return Ok(());
        }

        let radius = ctx.theme.border_radius_sm;
        let fs = ctx.theme.font_size_xs;

        // Shadow.
        ctx.theme
            .shadow_tooltip
            .draw(ctx.backend, x, y, w, h, radius)?;

        // Background.
        ctx.backend
            .fill_rounded_rect(x, y, w, h, radius, ctx.theme.tooltip_bg)?;

        // Border.
        ctx.backend
            .stroke_rounded_rect(x, y, w, h, radius, 1, ctx.theme.border_subtle)?;

        // Text.
        let text_w = ctx.backend.measure_text(&self.text, fs);
        let text_h = ctx.backend.measure_text_height(fs);
        let tx = x + layout::center(w, text_w);
        let ty = y + layout::center(h, text_h);
        ctx.backend
            .draw_text(&self.text, tx, ty, fs, ctx.theme.tooltip_text)?;

        Ok(())
    }
}

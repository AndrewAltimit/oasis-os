//! Toast notification widget for ephemeral feedback messages.
//!
//! Provides a self-contained [`ToastStack`] widget that renders stacked toast
//! notifications from a chosen screen corner. Each toast slides in from the
//! edge and auto-dismisses after a configurable TTL (in frames).
//!
//! This is the oasis-ui widget-level toast. It does NOT duplicate the
//! `oasis-core` `ToastManager` which operates on SDI objects; instead it
//! renders via the [`Widget`] trait and [`DrawContext`].

use std::collections::VecDeque;

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::backend::Color;
use oasis_types::error::Result;

/// Toast severity level, mapped to theme status colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    /// Informational message (theme.info).
    Info,
    /// Success/positive feedback (theme.success).
    Success,
    /// Warning/caution (theme.warning).
    Warning,
    /// Error/failure (theme.error).
    Error,
}

/// Screen corner where the toast stack is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastPosition {
    /// Top-right corner (default).
    TopRight,
    /// Top-left corner.
    TopLeft,
    /// Bottom-right corner.
    BottomRight,
    /// Bottom-left corner.
    BottomLeft,
}

/// Duration (in frames) of the slide-in entrance animation.
const ENTRANCE_FRAMES: u32 = 10;

/// Horizontal and vertical margin from the container edge.
const MARGIN: i32 = 8;

/// Vertical gap between stacked toasts.
const GAP: i32 = 4;

/// Horizontal padding inside a toast.
const PAD_H: u32 = 8;

/// Vertical padding inside a toast.
const PAD_V: u32 = 4;

/// A single toast notification.
///
/// # Example
///
/// ```ignore
/// let toast = Toast::new("Saved!", ToastLevel::Success, 120);
/// assert!(!toast.is_expired());
/// ```
#[derive(Debug, Clone)]
pub struct Toast {
    /// Message text displayed in the toast.
    pub message: String,
    /// Severity level determining background color.
    pub level: ToastLevel,
    /// Frames remaining before auto-dismiss.
    pub ttl: u32,
    /// Total initial TTL (for computing expiry progress).
    pub total_ttl: u32,
    /// Entrance animation progress (0.0 = hidden, 1.0 = fully visible).
    pub entrance_progress: f32,
}

impl Toast {
    /// Create a new toast with the given message, level, and lifetime.
    pub fn new(message: impl Into<String>, level: ToastLevel, ttl: u32) -> Self {
        Self {
            message: message.into(),
            level,
            ttl,
            total_ttl: ttl,
            entrance_progress: 0.0,
        }
    }

    /// Whether this toast has expired (zero TTL remaining).
    pub fn is_expired(&self) -> bool {
        self.ttl == 0
    }

    /// Expiry progress from 0.0 (full lifetime remaining) to 1.0 (expired).
    pub fn progress(&self) -> f32 {
        if self.total_ttl == 0 {
            return 1.0;
        }
        1.0 - (self.ttl as f32 / self.total_ttl as f32)
    }

    /// Return the theme background color for this toast's level.
    fn bg_color(&self, theme: &crate::theme::Theme) -> Color {
        match self.level {
            ToastLevel::Info => theme.info,
            ToastLevel::Success => theme.success,
            ToastLevel::Warning => theme.warning,
            ToastLevel::Error => theme.error,
        }
    }
}

/// A stack of toast notifications rendered from a screen corner.
///
/// Manages a bounded queue of toasts, advancing their animations and TTLs
/// each frame via [`tick`](Self::tick).
///
/// # Example
///
/// ```ignore
/// let mut stack = ToastStack::new()
///     .with_position(ToastPosition::TopRight);
/// stack.show("Changes saved", ToastLevel::Success, 120);
/// stack.show("Warning!", ToastLevel::Warning, 180);
/// // Each frame:
/// stack.tick(); // decrements TTLs, removes expired toasts
/// ```
#[derive(Debug)]
pub struct ToastStack {
    /// Active and queued toasts (front = oldest).
    toasts: VecDeque<Toast>,
    /// Screen corner to anchor the stack.
    position: ToastPosition,
    /// Maximum number of toasts rendered simultaneously.
    max_visible: usize,
    /// Maximum total queue size (visible + waiting).
    max_queued: usize,
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastStack {
    /// Create a new empty toast stack with default settings.
    ///
    /// Defaults: position = `TopRight`, max_visible = 4, max_queued = 8.
    pub fn new() -> Self {
        Self {
            toasts: VecDeque::new(),
            position: ToastPosition::TopRight,
            max_visible: 4,
            max_queued: 8,
        }
    }

    /// Set the anchor position for the toast stack.
    pub fn with_position(mut self, position: ToastPosition) -> Self {
        self.position = position;
        self
    }

    /// Enqueue a new toast notification.
    ///
    /// If the queue exceeds `max_queued`, the oldest toast is dropped.
    pub fn show(&mut self, message: impl Into<String>, level: ToastLevel, ttl: u32) {
        self.toasts.push_back(Toast::new(message, level, ttl));
        while self.toasts.len() > self.max_queued {
            self.toasts.pop_front();
        }
    }

    /// Advance all toasts by one frame.
    ///
    /// Decrements TTLs, removes expired toasts, and advances entrance
    /// animations for visible toasts.
    pub fn tick(&mut self) {
        for toast in &mut self.toasts {
            toast.ttl = toast.ttl.saturating_sub(1);
            if toast.entrance_progress < 1.0 {
                toast.entrance_progress =
                    (toast.entrance_progress + 1.0 / ENTRANCE_FRAMES as f32).min(1.0);
            }
        }
        self.toasts.retain(|t| !t.is_expired());
    }

    /// Remove all toasts.
    pub fn clear(&mut self) {
        self.toasts.clear();
    }

    /// Number of toasts currently in the queue.
    pub fn count(&self) -> usize {
        self.toasts.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }
}

impl Widget for ToastStack {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        if self.toasts.is_empty() {
            return (0, 0);
        }

        let fs = ctx.theme.font_size_md;
        let visible = self.toasts.len().min(self.max_visible);

        // Width: widest toast message + padding.
        let max_text_w = self
            .toasts
            .iter()
            .take(self.max_visible)
            .map(|t| ctx.backend.measure_text(&t.message, fs))
            .max()
            .unwrap_or(0);
        let w = max_text_w + PAD_H * 2;

        // Height: stacked toast heights + gaps.
        let text_h = ctx.backend.measure_text_height(fs);
        let toast_h = text_h + PAD_V * 2;
        let total_h = visible as u32 * toast_h
            + visible.saturating_sub(1) as u32 * GAP as u32
            + MARGIN as u32 * 2;

        (w + MARGIN as u32 * 2, total_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        if self.toasts.is_empty() {
            return Ok(());
        }

        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let toast_h = text_h + PAD_V * 2;
        let text_color = ctx.theme.text_on_accent;
        let radius = ctx.theme.border_radius_md;
        let shadow = &ctx.theme.shadow_tooltip;

        // Determine toast width (fill available space minus margins).
        let toast_w = w.saturating_sub(MARGIN as u32 * 2);

        // Iterate over the most recent `max_visible` toasts.
        let visible: Vec<&Toast> = self
            .toasts
            .iter()
            .rev()
            .take(self.max_visible)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let anchors_top = matches!(
            self.position,
            ToastPosition::TopRight | ToastPosition::TopLeft
        );
        let anchors_right = matches!(
            self.position,
            ToastPosition::TopRight | ToastPosition::BottomRight
        );

        for (i, toast) in visible.iter().enumerate() {
            // Compute the resting X position.
            let rest_x = if anchors_right {
                x + w as i32 - MARGIN - toast_w as i32
            } else {
                x + MARGIN
            };

            // Slide-in offset from the anchor edge.
            let slide_offset = ((1.0 - toast.entrance_progress) * toast_w as f32) as i32;
            let tx = if anchors_right {
                rest_x + slide_offset
            } else {
                rest_x - slide_offset
            };

            // Compute Y position: stack from the anchored edge.
            let ty = if anchors_top {
                y + MARGIN + i as i32 * (toast_h as i32 + GAP)
            } else {
                y + h as i32 - MARGIN - (i as i32 + 1) * (toast_h as i32 + GAP) + GAP
            };

            let bg = toast.bg_color(ctx.theme);

            // Shadow behind the toast.
            shadow.draw(ctx.backend, tx, ty, toast_w, toast_h, radius)?;

            // Background rounded rect.
            ctx.backend
                .fill_rounded_rect(tx, ty, toast_w, toast_h, radius, bg)?;

            // Centered message text.
            let text_w = ctx.backend.measure_text(&toast.message, fs);
            let text_x = tx + layout::center(toast_w, text_w);
            let text_y = ty + layout::center(toast_h, text_h);
            ctx.backend
                .draw_text(&toast.message, text_x, text_y, fs, text_color)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DrawContext;
    use crate::test_utils::{self, MockBackend};
    use crate::theme::Theme;
    use crate::widget::Widget;

    // -- Toast creation --

    #[test]
    fn toast_new() {
        let t = Toast::new("hello", ToastLevel::Info, 60);
        assert_eq!(t.message, "hello");
        assert_eq!(t.level, ToastLevel::Info);
        assert_eq!(t.ttl, 60);
        assert_eq!(t.total_ttl, 60);
        assert!((t.entrance_progress - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn toast_is_expired() {
        let t = Toast::new("msg", ToastLevel::Error, 0);
        assert!(t.is_expired());
        let t2 = Toast::new("msg", ToastLevel::Error, 1);
        assert!(!t2.is_expired());
    }

    #[test]
    fn toast_progress_full_ttl() {
        let t = Toast::new("msg", ToastLevel::Success, 60);
        assert!((t.progress() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn toast_progress_half() {
        let mut t = Toast::new("msg", ToastLevel::Warning, 100);
        t.ttl = 50;
        assert!((t.progress() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn toast_progress_expired() {
        let mut t = Toast::new("msg", ToastLevel::Info, 60);
        t.ttl = 0;
        assert!((t.progress() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn toast_progress_zero_total_ttl() {
        let t = Toast::new("msg", ToastLevel::Info, 0);
        assert!((t.progress() - 1.0).abs() < f32::EPSILON);
    }

    // -- ToastStack creation and builder --

    #[test]
    fn stack_new_is_empty() {
        let stack = ToastStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.count(), 0);
    }

    #[test]
    fn stack_with_position() {
        let stack = ToastStack::new().with_position(ToastPosition::BottomLeft);
        assert_eq!(stack.position, ToastPosition::BottomLeft);
    }

    #[test]
    fn stack_default_equals_new() {
        let a = ToastStack::new();
        let b = ToastStack::default();
        assert_eq!(a.count(), b.count());
        assert_eq!(a.position, b.position);
        assert_eq!(a.max_visible, b.max_visible);
        assert_eq!(a.max_queued, b.max_queued);
    }

    // -- show / tick / expire --

    #[test]
    fn show_adds_toast() {
        let mut stack = ToastStack::new();
        stack.show("test", ToastLevel::Info, 60);
        assert_eq!(stack.count(), 1);
        assert!(!stack.is_empty());
    }

    #[test]
    fn tick_decrements_ttl() {
        let mut stack = ToastStack::new();
        stack.show("test", ToastLevel::Info, 10);
        stack.tick();
        assert_eq!(stack.toasts[0].ttl, 9);
    }

    #[test]
    fn tick_removes_expired() {
        let mut stack = ToastStack::new();
        stack.show("test", ToastLevel::Info, 2);
        stack.tick(); // ttl = 1
        assert_eq!(stack.count(), 1);
        stack.tick(); // ttl = 0 -> removed
        assert_eq!(stack.count(), 0);
    }

    #[test]
    fn tick_advances_entrance() {
        let mut stack = ToastStack::new();
        stack.show("test", ToastLevel::Success, 60);
        assert!((stack.toasts[0].entrance_progress - 0.0).abs() < f32::EPSILON);
        stack.tick();
        assert!(stack.toasts[0].entrance_progress > 0.0);
    }

    #[test]
    fn tick_entrance_caps_at_one() {
        let mut stack = ToastStack::new();
        stack.show("test", ToastLevel::Success, 120);
        for _ in 0..100 {
            stack.tick();
        }
        assert!((stack.toasts[0].entrance_progress - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn queue_overflow_drops_oldest() {
        let mut stack = ToastStack::new();
        // max_queued defaults to 8
        for i in 0..12 {
            stack.show(format!("toast {i}"), ToastLevel::Info, 120);
        }
        assert_eq!(stack.count(), 8);
        // Oldest should have been dropped; newest should remain.
        assert_eq!(stack.toasts.back().unwrap().message, "toast 11");
        assert_eq!(stack.toasts.front().unwrap().message, "toast 4");
    }

    #[test]
    fn clear_removes_all() {
        let mut stack = ToastStack::new();
        stack.show("a", ToastLevel::Info, 60);
        stack.show("b", ToastLevel::Error, 60);
        stack.clear();
        assert!(stack.is_empty());
        assert_eq!(stack.count(), 0);
    }

    // -- Position variants --

    #[test]
    fn all_positions_constructible() {
        let positions = [
            ToastPosition::TopRight,
            ToastPosition::TopLeft,
            ToastPosition::BottomRight,
            ToastPosition::BottomLeft,
        ];
        for pos in positions {
            let stack = ToastStack::new().with_position(pos);
            assert_eq!(stack.position, pos);
        }
    }

    // -- Widget draw across all themes --

    #[test]
    fn draw_all_themes() {
        test_utils::test_draw_all_themes(|ctx| {
            let mut stack = ToastStack::new();
            stack.show("Info toast", ToastLevel::Info, 60);
            stack.show("Success toast", ToastLevel::Success, 60);
            stack.show("Warning toast", ToastLevel::Warning, 60);
            stack.show("Error toast", ToastLevel::Error, 60);
            // Advance entrance so they are partially visible.
            for _ in 0..5 {
                stack.tick();
            }
            stack.draw(ctx, 0, 0, 300, 200).unwrap();
        });
    }

    #[test]
    fn draw_empty_stack_no_calls() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let stack = ToastStack::new();
            stack.draw(&mut ctx, 0, 0, 300, 200).unwrap();
        }
        assert_eq!(backend.fill_rect_count(), 0);
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn draw_emits_text_and_rect() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut stack = ToastStack::new();
            stack.show("Hello", ToastLevel::Info, 60);
            // Advance entrance to make it visible.
            for _ in 0..ENTRANCE_FRAMES {
                stack.tick();
            }
            stack.draw(&mut ctx, 0, 0, 300, 200).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
        assert!(backend.has_text("Hello"));
    }

    #[test]
    fn measure_empty() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let stack = ToastStack::new();
        let (w, h) = stack.measure(&ctx, 400, 300);
        assert_eq!(w, 0);
        assert_eq!(h, 0);
    }

    #[test]
    fn measure_nonempty() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut stack = ToastStack::new();
        stack.show("A toast message", ToastLevel::Info, 60);
        let (w, h) = stack.measure(&ctx, 400, 300);
        assert!(w > 0);
        assert!(h > 0);
    }

    // -- Level enum coverage --

    #[test]
    fn toast_level_debug() {
        assert_eq!(format!("{:?}", ToastLevel::Info), "Info");
        assert_eq!(format!("{:?}", ToastLevel::Success), "Success");
        assert_eq!(format!("{:?}", ToastLevel::Warning), "Warning");
        assert_eq!(format!("{:?}", ToastLevel::Error), "Error");
    }

    #[test]
    fn toast_level_eq() {
        assert_eq!(ToastLevel::Info, ToastLevel::Info);
        assert_ne!(ToastLevel::Info, ToastLevel::Error);
    }

    #[test]
    fn toast_position_debug() {
        assert_eq!(format!("{:?}", ToastPosition::TopRight), "TopRight");
        assert_eq!(format!("{:?}", ToastPosition::TopLeft), "TopLeft");
        assert_eq!(format!("{:?}", ToastPosition::BottomRight), "BottomRight");
        assert_eq!(format!("{:?}", ToastPosition::BottomLeft), "BottomLeft");
    }

    // -- bg_color mapping --

    #[test]
    fn bg_color_maps_level_to_theme() {
        let theme = Theme::dark();
        let info = Toast::new("i", ToastLevel::Info, 1);
        let success = Toast::new("s", ToastLevel::Success, 1);
        let warning = Toast::new("w", ToastLevel::Warning, 1);
        let error = Toast::new("e", ToastLevel::Error, 1);
        assert_eq!(info.bg_color(&theme), theme.info);
        assert_eq!(success.bg_color(&theme), theme.success);
        assert_eq!(warning.bg_color(&theme), theme.warning);
        assert_eq!(error.bg_color(&theme), theme.error);
    }
}

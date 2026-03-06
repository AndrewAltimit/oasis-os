//! Shared layout calculations for app content areas.
//!
//! Extracts the repeated title_h / line_h / usable_h / max_visible
//! calculations used across multiple rendering paths in `AppRunner`.

use oasis_skin::ActiveTheme;

/// Pre-computed layout dimensions for an app's content area.
///
/// Removes duplication from the 3+ sites in `AppRunner` that compute
/// the same values from `ActiveTheme`.
pub struct AppLayout {
    /// Height of the app title bar (pixels).
    pub title_h: u32,
    /// Height of a single content line (pixels, minimum 1).
    pub line_h: u32,
    /// Total usable height for content below the title bar.
    pub usable_h: u32,
    /// Maximum number of visible lines in the content area.
    pub max_visible: usize,
    /// X offset for content (left padding).
    pub content_x: i32,
    /// Y offset where content starts (below title bar).
    pub content_y: i32,
    /// Width available for content.
    pub content_w: u32,
}

impl AppLayout {
    /// Compute layout from the active theme.
    ///
    /// The `padding` parameter is the combined vertical padding between
    /// title bar, content area, and bottom bar edges (typically 14).
    pub fn compute(at: &ActiveTheme, padding: u32) -> Self {
        let title_h = at.app.title_bar_height;
        let line_h = at.terminal_line_height.max(1);
        let usable_h = at
            .screen_h
            .saturating_sub(title_h)
            .saturating_sub(at.statusbar_height)
            .saturating_sub(at.bottombar_height)
            .saturating_sub(padding);
        let max_visible = (usable_h / line_h).max(1) as usize;
        let content_x = 8;
        let content_y = (title_h + 4) as i32;
        let content_w = at.screen_w.saturating_sub(16);

        Self {
            title_h,
            line_h,
            usable_h,
            max_visible,
            content_x,
            content_y,
            content_w,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_theme() -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.screen_w = 480;
        at.screen_h = 272;
        at.app.title_bar_height = 20;
        at.terminal_line_height = 12;
        at.statusbar_height = 16;
        at.bottombar_height = 16;
        at
    }

    #[test]
    fn basic_layout() {
        let at = mock_theme();
        let layout = AppLayout::compute(&at, 14);
        assert_eq!(layout.title_h, 20);
        assert_eq!(layout.line_h, 12);
        // usable_h = 272 - 20 - 16 - 16 - 14 = 206
        assert_eq!(layout.usable_h, 206);
        // max_visible = 206 / 12 = 17
        assert_eq!(layout.max_visible, 17);
        assert_eq!(layout.content_x, 8);
        assert_eq!(layout.content_y, 24); // title_h + 4
        assert_eq!(layout.content_w, 464); // 480 - 16
    }

    #[test]
    fn zero_line_height_clamps_to_one() {
        let mut at = mock_theme();
        at.terminal_line_height = 0;
        let layout = AppLayout::compute(&at, 14);
        assert_eq!(layout.line_h, 1);
        assert!(layout.max_visible >= 1);
    }

    #[test]
    fn tiny_screen_no_panic() {
        let mut at = mock_theme();
        at.screen_h = 50;
        at.app.title_bar_height = 20;
        at.statusbar_height = 16;
        at.bottombar_height = 16;
        let layout = AppLayout::compute(&at, 14);
        // saturating_sub prevents underflow
        assert_eq!(layout.usable_h, 0);
        assert_eq!(layout.max_visible, 1); // clamped to 1
    }

    #[test]
    fn large_screen() {
        let mut at = mock_theme();
        at.screen_w = 1024;
        at.screen_h = 768;
        let layout = AppLayout::compute(&at, 14);
        assert!(layout.max_visible > 50);
        assert_eq!(layout.content_w, 1008); // 1024 - 16
    }
}

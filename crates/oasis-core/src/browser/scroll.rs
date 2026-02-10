//! Viewport and scroll offset management.

/// Scroll amounts for different input types.
pub const SCROLL_LINE: i32 = 24;
pub const SCROLL_PAGE_FRACTION: f32 = 0.9;
pub const SCROLL_WHEEL: i32 = 48;
pub const SCROLL_ACCELERATION: f32 = 1.5;
pub const MAX_VELOCITY: f32 = 200.0;

/// Friction applied each tick to decelerate smooth scrolling.
const FRICTION: f32 = 0.85;

/// Velocity below this threshold snaps to zero.
const VELOCITY_EPSILON: f32 = 0.5;

/// Scroll state for the browser viewport.
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Current vertical scroll offset in pixels.
    pub scroll_y: i32,
    /// Total content height (from layout).
    pub content_height: i32,
    /// Visible viewport height (from window content area).
    pub viewport_height: i32,
    /// Scroll velocity for smooth scrolling (pixels per frame).
    scroll_velocity: f32,
    /// Whether smooth scrolling is enabled.
    smooth: bool,
}

impl ScrollState {
    pub fn new(viewport_height: i32, smooth: bool) -> Self {
        Self {
            scroll_y: 0,
            content_height: 0,
            viewport_height,
            scroll_velocity: 0.0,
            smooth,
        }
    }

    /// Scroll up by one line.
    pub fn scroll_up(&mut self) {
        if self.smooth {
            self.scroll_velocity -= SCROLL_LINE as f32;
            self.scroll_velocity = self.scroll_velocity.max(-MAX_VELOCITY);
        } else {
            self.scroll_y -= SCROLL_LINE;
            self.clamp();
        }
    }

    /// Scroll down by one line.
    pub fn scroll_down(&mut self) {
        if self.smooth {
            self.scroll_velocity += SCROLL_LINE as f32;
            self.scroll_velocity = self.scroll_velocity.min(MAX_VELOCITY);
        } else {
            self.scroll_y += SCROLL_LINE;
            self.clamp();
        }
    }

    /// Scroll up by one page.
    pub fn page_up(&mut self) {
        let amount = (self.viewport_height as f32 * SCROLL_PAGE_FRACTION) as i32;
        if self.smooth {
            self.scroll_velocity -= amount as f32;
            self.scroll_velocity = self.scroll_velocity.max(-MAX_VELOCITY);
        } else {
            self.scroll_y -= amount;
            self.clamp();
        }
    }

    /// Scroll down by one page.
    pub fn page_down(&mut self) {
        let amount = (self.viewport_height as f32 * SCROLL_PAGE_FRACTION) as i32;
        if self.smooth {
            self.scroll_velocity += amount as f32;
            self.scroll_velocity = self.scroll_velocity.min(MAX_VELOCITY);
        } else {
            self.scroll_y += amount;
            self.clamp();
        }
    }

    /// Scroll by a mouse wheel notch.
    pub fn wheel_scroll(&mut self, delta: i32) {
        if self.smooth {
            self.scroll_velocity += delta as f32 * SCROLL_WHEEL as f32;
            self.scroll_velocity = self.scroll_velocity.clamp(-MAX_VELOCITY, MAX_VELOCITY);
        } else {
            self.scroll_y += delta * SCROLL_WHEEL;
            self.clamp();
        }
    }

    /// Scroll to an absolute position.
    pub fn scroll_to(&mut self, y: i32) {
        self.scroll_velocity = 0.0;
        self.scroll_y = y;
        self.clamp();
    }

    /// Scroll to make a specific y-coordinate visible.
    /// Centers it in the viewport if it is offscreen.
    pub fn scroll_to_visible(&mut self, target_y: i32, target_height: i32) {
        let visible_top = self.scroll_y;
        let visible_bottom = self.scroll_y + self.viewport_height;

        if target_y >= visible_top && target_y + target_height <= visible_bottom {
            // Already visible, nothing to do.
            return;
        }

        // Center the target in the viewport.
        let center = target_y + target_height / 2;
        self.scroll_y = center - self.viewport_height / 2;
        self.scroll_velocity = 0.0;
        self.clamp();
    }

    /// Scroll to top of document.
    pub fn scroll_to_top(&mut self) {
        self.scroll_velocity = 0.0;
        self.scroll_y = 0;
    }

    /// Scroll to bottom of document.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_velocity = 0.0;
        self.scroll_y = self.max_scroll();
    }

    /// Update content height (after layout).
    pub fn set_content_height(&mut self, height: i32) {
        self.content_height = height;
        self.clamp();
    }

    /// Update viewport height (after window resize).
    pub fn set_viewport_height(&mut self, height: i32) {
        self.viewport_height = height;
        self.clamp();
    }

    /// Tick the smooth scroll animation. Returns true if still
    /// animating.
    pub fn tick(&mut self) -> bool {
        if !self.smooth {
            return false;
        }

        if self.scroll_velocity.abs() < VELOCITY_EPSILON {
            self.scroll_velocity = 0.0;
            return false;
        }

        self.scroll_y += self.scroll_velocity as i32;
        self.scroll_velocity *= FRICTION;
        self.clamp();

        // If clamped to boundary, stop velocity.
        if self.scroll_y == 0 || self.scroll_y == self.max_scroll() {
            self.scroll_velocity = 0.0;
        }

        self.scroll_velocity.abs() >= VELOCITY_EPSILON
    }

    /// Get the maximum scroll offset.
    pub fn max_scroll(&self) -> i32 {
        (self.content_height - self.viewport_height).max(0)
    }

    /// Clamp scroll_y to valid range [0, max_scroll].
    fn clamp(&mut self) {
        let max = self.max_scroll();
        if self.scroll_y < 0 {
            self.scroll_y = 0;
        } else if self.scroll_y > max {
            self.scroll_y = max;
        }
    }

    /// Get scroll percentage (0.0 to 1.0) for scrollbar rendering.
    pub fn scroll_fraction(&self) -> f32 {
        let max = self.max_scroll();
        if max == 0 {
            0.0
        } else {
            self.scroll_y as f32 / max as f32
        }
    }

    /// Is at top?
    pub fn at_top(&self) -> bool {
        self.scroll_y == 0
    }

    /// Is at bottom?
    pub fn at_bottom(&self) -> bool {
        self.scroll_y >= self.max_scroll()
    }

    /// Reset scroll state (for new page load).
    pub fn reset(&mut self) {
        self.scroll_y = 0;
        self.scroll_velocity = 0.0;
        self.content_height = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_down_increments() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(1000);
        s.scroll_down();
        assert_eq!(s.scroll_y, SCROLL_LINE);
    }

    #[test]
    fn scroll_up_clamped_at_zero() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(1000);
        s.scroll_up();
        assert_eq!(s.scroll_y, 0);
    }

    #[test]
    fn page_down_scrolls_by_viewport_fraction() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(1000);
        s.page_down();
        let expected = (200.0 * SCROLL_PAGE_FRACTION) as i32;
        assert_eq!(s.scroll_y, expected);
    }

    #[test]
    fn scroll_clamped_to_max() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(300);
        // Max scroll = 300 - 200 = 100
        s.scroll_to(500);
        assert_eq!(s.scroll_y, 100);
    }

    #[test]
    fn max_scroll_calculation() {
        let s = ScrollState::new(200, false);
        // Content not set yet, so max_scroll = 0.
        assert_eq!(s.max_scroll(), 0);

        let mut s2 = ScrollState::new(200, false);
        s2.set_content_height(500);
        assert_eq!(s2.max_scroll(), 300);
    }

    #[test]
    fn scroll_to_visible_centers_offscreen() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(1000);

        // Target at y=600 with height 20 is offscreen.
        s.scroll_to_visible(600, 20);
        // Center: 600 + 10 = 610, so scroll_y = 610 - 100 = 510.
        assert_eq!(s.scroll_y, 510);
    }

    #[test]
    fn reset_clears_scroll() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(1000);
        s.scroll_to(300);
        assert_eq!(s.scroll_y, 300);

        s.reset();
        assert_eq!(s.scroll_y, 0);
        assert_eq!(s.content_height, 0);
    }

    #[test]
    fn scroll_fraction_calculation() {
        let mut s = ScrollState::new(200, false);
        s.set_content_height(400);
        // max_scroll = 200
        assert_eq!(s.scroll_fraction(), 0.0);

        s.scroll_to(100);
        assert!((s.scroll_fraction() - 0.5).abs() < 0.001);

        s.scroll_to(200);
        assert!((s.scroll_fraction() - 1.0).abs() < 0.001);
    }
}

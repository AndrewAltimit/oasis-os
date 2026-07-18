//! Keyboard focus navigation for widget groups.
//!
//! `FocusRing` tracks which widget index has focus and handles
//! directional navigation (next/prev/wrapping). Widgets query the
//! ring to determine their visual focus state.
//!
//! `FocusManager` builds on `FocusRing` to provide Tab/Shift-Tab
//! keyboard cycling, skip-disabled item logic, and visual focus
//! indicator drawing through `FocusStyle`.
//!
//! `SpatialFocusManager` adds spatial (arrow-key) navigation and
//! tab-index ordering over a set of `FocusableItem`s with bounds.
//! It tracks whether focus was activated via keyboard (showing a
//! focus ring) or hidden by mouse interaction.

/// Direction of focus movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    /// Move focus to the next widget.
    Next,
    /// Move focus to the previous widget.
    Prev,
}

/// Tracks keyboard focus across a group of widgets.
#[derive(Debug, Clone)]
pub struct FocusRing {
    /// Total number of focusable items.
    count: usize,
    /// Index of the currently focused item.
    focused: usize,
    /// Whether wrap-around is enabled.
    pub wrap: bool,
}

impl FocusRing {
    /// Create a new focus ring with the given number of items.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            focused: 0,
            wrap: true,
        }
    }

    /// Get the current focused index.
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Return whether the given index has focus.
    pub fn is_focused(&self, index: usize) -> bool {
        self.focused == index
    }

    /// Set focus to a specific index (clamped to bounds).
    pub fn set_focused(&mut self, index: usize) {
        if self.count > 0 {
            self.focused = index.min(self.count - 1);
        }
    }

    /// Update the total number of focusable items.
    ///
    /// If the current focus index exceeds the new count, it is clamped.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        if count > 0 {
            self.focused = self.focused.min(count - 1);
        } else {
            self.focused = 0;
        }
    }

    /// Total number of focusable items.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Move focus in the given direction.
    ///
    /// Returns the new focused index.
    pub fn move_focus(&mut self, dir: FocusDir) -> usize {
        if self.count == 0 {
            return 0;
        }
        match dir {
            FocusDir::Next => {
                if self.focused + 1 < self.count {
                    self.focused += 1;
                } else if self.wrap {
                    self.focused = 0;
                }
            },
            FocusDir::Prev => {
                if self.focused > 0 {
                    self.focused -= 1;
                } else if self.wrap {
                    self.focused = self.count - 1;
                }
            },
        }
        self.focused
    }

    /// Move focus to the first item.
    pub fn focus_first(&mut self) {
        self.focused = 0;
    }

    /// Move focus to the last item.
    pub fn focus_last(&mut self) {
        if self.count > 0 {
            self.focused = self.count - 1;
        }
    }
}

use oasis_types::backend::Color;

/// Visual style for focus indicators drawn around focused widgets.
#[derive(Debug, Clone, Copy)]
pub struct FocusStyle {
    /// Border color for the focus indicator.
    pub color: Color,
    /// Border width in pixels.
    pub width: u16,
    /// Corner radius for the focus border.
    pub radius: u16,
    /// Gap between the widget edge and the focus border.
    pub offset: i32,
}

impl FocusStyle {
    /// Create a focus style from a theme's accent color.
    pub fn from_accent(accent: Color) -> Self {
        Self {
            color: accent,
            width: 1,
            radius: 2,
            offset: 1,
        }
    }

    /// Create a focus style from a theme, honoring the skin-provided
    /// `focus_ring_*` overrides.
    ///
    /// When the theme sets no focus ring fields (the default), this is
    /// exactly [`FocusStyle::from_accent`] on the theme's accent color,
    /// so skins that don't author `[geometry] focus_ring_*` render
    /// identically to before.
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        let mut style = Self::from_accent(theme.accent);
        if let Some(color) = theme.focus_ring_color {
            style.color = color;
        }
        if let Some(width) = theme.focus_ring_width {
            style.width = width;
        }
        if let Some(offset) = theme.focus_ring_offset {
            style.offset = offset;
        }
        style
    }

    /// Compute the focus indicator rectangle given a widget rect.
    pub fn indicator_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        let off = self.offset;
        let delta = off.saturating_mul(2);
        (
            x.saturating_sub(off),
            y.saturating_sub(off),
            w.saturating_add_signed(delta),
            h.saturating_add_signed(delta),
        )
    }

    /// Draw the focus indicator around a widget rectangle.
    pub fn draw(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        let (fx, fy, fw, fh) = self.indicator_rect(x, y, w, h);
        backend.stroke_rounded_rect(fx, fy, fw, fh, self.radius, self.width, self.color)
    }
}

/// Keyboard navigation action derived from input events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAction {
    /// Tab key: move to the next focusable widget.
    TabNext,
    /// Shift+Tab: move to the previous focusable widget.
    TabPrev,
    /// Activate/confirm the focused widget (Enter/Cross).
    Activate,
    /// Home key: focus the first widget.
    Home,
    /// End key: focus the last widget.
    End,
}

/// Manages keyboard focus across a set of widgets, with support
/// for Tab/Shift-Tab cycling, disabled-item skipping, and visual
/// focus indicators.
#[derive(Debug, Clone)]
pub struct FocusManager {
    /// The underlying focus ring.
    ring: FocusRing,
    /// Per-item enabled state. Items marked `false` are skipped
    /// during Tab navigation.
    enabled: Vec<bool>,
    /// Whether keyboard focus is currently active. When false,
    /// no widget shows a focus indicator.
    pub active: bool,
}

impl FocusManager {
    /// Create a new focus manager with `count` focusable items,
    /// all initially enabled.
    pub fn new(count: usize) -> Self {
        Self {
            ring: FocusRing::new(count),
            enabled: vec![true; count],
            active: false,
        }
    }

    /// Get the current focused index.
    pub fn focused(&self) -> usize {
        self.ring.focused()
    }

    /// Whether the given index currently has visible focus.
    pub fn has_focus(&self, index: usize) -> bool {
        self.active && self.ring.is_focused(index)
    }

    /// Total number of managed items.
    pub fn count(&self) -> usize {
        self.ring.count()
    }

    /// Access the underlying `FocusRing`.
    pub fn ring(&self) -> &FocusRing {
        &self.ring
    }

    /// Mutably access the underlying `FocusRing`.
    pub fn ring_mut(&mut self) -> &mut FocusRing {
        &mut self.ring
    }

    /// Set whether a specific item is enabled for focus.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) {
        if index < self.enabled.len() {
            self.enabled[index] = enabled;
        }
    }

    /// Whether a specific item is enabled.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Update the item count. Grows or shrinks the enabled list.
    pub fn set_count(&mut self, count: usize) {
        self.ring.set_count(count);
        self.enabled.resize(count, true);
    }

    /// Process a focus action and return the new focused index.
    ///
    /// Skips disabled items when navigating. If all items are
    /// disabled, focus does not move.
    pub fn handle_action(&mut self, action: FocusAction) -> usize {
        if self.ring.count() == 0 {
            return 0;
        }
        self.active = true;
        match action {
            FocusAction::TabNext => self.move_skip(FocusDir::Next),
            FocusAction::TabPrev => self.move_skip(FocusDir::Prev),
            FocusAction::Activate => self.ring.focused(),
            FocusAction::Home => {
                self.ring.focus_first();
                self.skip_to_enabled(FocusDir::Next)
            },
            FocusAction::End => {
                self.ring.focus_last();
                self.skip_to_enabled(FocusDir::Prev)
            },
        }
    }

    /// Move focus in the given direction, skipping disabled items.
    fn move_skip(&mut self, dir: FocusDir) -> usize {
        let start = self.ring.focused();
        self.ring.move_focus(dir);
        // Skip disabled items, but stop if we wrap all the way
        // around to avoid infinite loops.
        let mut attempts = 0;
        while !self.is_enabled(self.ring.focused()) && attempts < self.ring.count() {
            self.ring.move_focus(dir);
            attempts += 1;
        }
        // If everything is disabled, go back to start.
        if attempts >= self.ring.count() {
            self.ring.set_focused(start);
        }
        self.ring.focused()
    }

    /// From the current position, skip to the nearest enabled
    /// item in the given direction.
    fn skip_to_enabled(&mut self, dir: FocusDir) -> usize {
        if self.is_enabled(self.ring.focused()) {
            return self.ring.focused();
        }
        let start = self.ring.focused();
        let mut attempts = 0;
        while !self.is_enabled(self.ring.focused()) && attempts < self.ring.count() {
            self.ring.move_focus(dir);
            attempts += 1;
        }
        if attempts >= self.ring.count() {
            self.ring.set_focused(start);
        }
        self.ring.focused()
    }

    /// Set focus to a specific index (must be enabled).
    pub fn set_focused(&mut self, index: usize) {
        if index < self.enabled.len() && self.enabled[index] {
            self.ring.set_focused(index);
            self.active = true;
        }
    }

    /// Deactivate keyboard focus (no visible indicator).
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    /// Draw the focus indicator around a widget if it has focus.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_indicator(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        index: usize,
        style: &FocusStyle,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        if self.has_focus(index) {
            style.draw(backend, x, y, w, h)?;
        }
        Ok(())
    }

    /// Draw the focus indicator around a widget if it has focus,
    /// styled from the theme (honors skin `focus_ring_*` overrides).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_indicator_themed(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        index: usize,
        theme: &crate::theme::Theme,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        self.draw_indicator(backend, index, &FocusStyle::from_theme(theme), x, y, w, h)
    }
}

// ── Spatial / keyboard-only navigation ──────────────────────────

/// Direction for spatial and sequential focus navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    /// Move to the next item in tab order.
    Next,
    /// Move to the previous item in tab order.
    Previous,
    /// Move focus upward (spatial).
    Up,
    /// Move focus downward (spatial).
    Down,
    /// Move focus leftward (spatial).
    Left,
    /// Move focus rightward (spatial).
    Right,
}

/// Axis-aligned rectangle for focus hit-testing and spatial
/// navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl Rect {
    /// Create a new rectangle.
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Horizontal centre.
    pub fn cx(&self) -> i32 {
        self.x.saturating_add(self.w as i32 / 2)
    }

    /// Vertical centre.
    pub fn cy(&self) -> i32 {
        self.y.saturating_add(self.h as i32 / 2)
    }
}

/// A focusable UI element registered with [`SpatialFocusManager`].
#[derive(Debug, Clone)]
pub struct FocusableItem {
    /// Unique identifier for this item.
    pub id: String,
    /// Tab order index. Lower values receive focus first.
    /// Negative values are skipped during sequential navigation.
    pub tab_index: i32,
    /// Bounding rectangle (screen coordinates).
    pub bounds: Rect,
}

/// Result of processing an input event through the focus system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEvent {
    /// Focus moved to a new item (index into the items list).
    Moved(usize),
    /// The currently focused item should be activated.
    Activate,
    /// No focus-relevant action occurred.
    None,
}

/// Manages keyboard-only navigation over a set of spatially-placed
/// focusable items, with tab-index ordering and arrow-key spatial
/// movement.
///
/// Tracks whether the focus ring should be visible (keyboard mode)
/// or hidden (mouse mode).
#[derive(Debug, Clone)]
pub struct SpatialFocusManager {
    items: Vec<FocusableItem>,
    /// Index into `items` of the currently focused element, or
    /// `None` if nothing is focused.
    focused: Option<usize>,
    /// `true` when the last interaction was keyboard-driven,
    /// meaning the focus ring indicator should be drawn.
    ring_visible: bool,
    /// Whether shift is held (for Shift+Tab detection).
    shift_held: bool,
}

impl SpatialFocusManager {
    /// Create an empty spatial focus manager.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            focused: None,
            ring_visible: false,
            shift_held: false,
        }
    }

    /// Register a focusable item. Returns its index.
    pub fn add_item(&mut self, item: FocusableItem) -> usize {
        let idx = self.items.len();
        self.items.push(item);
        idx
    }

    /// Replace all items at once.
    pub fn set_items(&mut self, items: Vec<FocusableItem>) {
        self.items = items;
        // Clamp or clear focus.
        if let Some(idx) = self.focused
            && idx >= self.items.len()
        {
            self.focused = if self.items.is_empty() {
                None
            } else {
                Some(self.items.len() - 1)
            };
        }
    }

    /// Number of registered items.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Currently focused index (if any).
    pub fn focused_index(&self) -> Option<usize> {
        self.focused
    }

    /// Currently focused item (if any).
    pub fn focused_item(&self) -> Option<&FocusableItem> {
        self.focused.and_then(|i| self.items.get(i))
    }

    /// Whether the focus ring indicator should be rendered.
    ///
    /// Returns `true` after keyboard navigation, `false` after
    /// mouse/pointer interaction.
    pub fn focus_ring_visible(&self) -> bool {
        self.ring_visible
    }

    /// Mark the focus ring as hidden (e.g. after mouse click).
    pub fn hide_focus_ring(&mut self) {
        self.ring_visible = false;
    }

    /// Mark the focus ring as visible (e.g. after Tab press).
    pub fn show_focus_ring(&mut self) {
        self.ring_visible = true;
    }

    /// Move focus in the given direction.
    ///
    /// `Next`/`Previous` follow tab-index order.
    /// `Up`/`Down`/`Left`/`Right` use spatial proximity.
    ///
    /// Returns the new focused index, or `None` if no items exist.
    pub fn navigate(&mut self, direction: FocusDirection) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        self.ring_visible = true;

        let new_idx = match direction {
            FocusDirection::Next => self.next_tab_index(),
            FocusDirection::Previous => self.prev_tab_index(),
            FocusDirection::Up
            | FocusDirection::Down
            | FocusDirection::Left
            | FocusDirection::Right => Some(self.spatial_move(direction)),
        };
        if let Some(idx) = new_idx {
            self.focused = Some(idx);
        }
        self.focused
    }

    /// Process an `InputEvent` and return what happened.
    ///
    /// Handles Tab, Shift+Tab, arrow keys, Enter/Space (Confirm),
    /// and pointer events (hide focus ring).
    pub fn handle_input(&mut self, event: &oasis_types::input::InputEvent) -> FocusEvent {
        use oasis_types::input::{Button, InputEvent};
        match event {
            // -- Sequential navigation --
            InputEvent::TextInput('\t') => {
                if self.shift_held {
                    self.nav_event(FocusDirection::Previous)
                } else {
                    self.nav_event(FocusDirection::Next)
                }
            },

            // -- Spatial navigation --
            InputEvent::ButtonPress(Button::Up) => self.nav_event(FocusDirection::Up),
            InputEvent::ButtonPress(Button::Down) => self.nav_event(FocusDirection::Down),
            InputEvent::ButtonPress(Button::Left) => self.nav_event(FocusDirection::Left),
            InputEvent::ButtonPress(Button::Right) => self.nav_event(FocusDirection::Right),

            // -- Activation --
            InputEvent::ButtonPress(Button::Confirm) => {
                if self.focused.is_some() {
                    self.ring_visible = true;
                    FocusEvent::Activate
                } else {
                    FocusEvent::None
                }
            },

            // -- Shift tracking (Select = Shift on PSP) --
            InputEvent::ButtonPress(Button::Select) => {
                self.shift_held = true;
                FocusEvent::None
            },
            InputEvent::ButtonRelease(Button::Select) => {
                self.shift_held = false;
                FocusEvent::None
            },

            // -- Mouse hides focus ring --
            InputEvent::PointerClick { .. } | InputEvent::CursorMove { .. } => {
                self.ring_visible = false;
                FocusEvent::None
            },

            _ => FocusEvent::None,
        }
    }

    /// Set focus to a specific index directly.
    pub fn set_focused(&mut self, index: usize) {
        if index < self.items.len() {
            self.focused = Some(index);
        }
    }

    /// Clear focus entirely.
    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    // ── Internal helpers ────────────────────────────────────────

    fn nav_event(&mut self, dir: FocusDirection) -> FocusEvent {
        match self.navigate(dir) {
            Some(idx) => FocusEvent::Moved(idx),
            None => FocusEvent::None,
        }
    }

    /// Items sorted by tab_index then registration order, skipping
    /// negative tab_index values.
    fn tab_order(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].tab_index >= 0)
            .collect();
        indices.sort_by_key(|&i| (self.items[i].tab_index, i));
        indices
    }

    fn next_tab_index(&self) -> Option<usize> {
        let order = self.tab_order();
        if order.is_empty() {
            return None;
        }
        Some(match self.focused {
            Some(cur) => match order.iter().position(|&i| i == cur) {
                Some(pos) => order[(pos + 1) % order.len()],
                None => order[0],
            },
            None => order[0],
        })
    }

    fn prev_tab_index(&self) -> Option<usize> {
        let order = self.tab_order();
        if order.is_empty() {
            return None;
        }
        Some(match self.focused {
            Some(cur) => match order.iter().position(|&i| i == cur) {
                Some(0) => order[order.len() - 1],
                Some(pos) => order[pos - 1],
                None => order[order.len() - 1],
            },
            None => order[order.len() - 1],
        })
    }

    fn spatial_move(&self, dir: FocusDirection) -> usize {
        let cur = match self.focused {
            Some(i) => i,
            None => return 0,
        };
        let cur_bounds = &self.items[cur].bounds;
        let cx = cur_bounds.cx();
        let cy = cur_bounds.cy();

        let mut best: Option<(i64, usize)> = None;

        for (i, item) in self.items.iter().enumerate() {
            if i == cur {
                continue;
            }
            let ix = item.bounds.cx();
            let iy = item.bounds.cy();
            let dx = ix as i64 - cx as i64;
            let dy = iy as i64 - cy as i64;

            // Filter candidates by direction.
            let valid = match dir {
                FocusDirection::Up => dy < 0,
                FocusDirection::Down => dy > 0,
                FocusDirection::Left => dx < 0,
                FocusDirection::Right => dx > 0,
                _ => false,
            };
            if !valid {
                continue;
            }

            // Distance: heavily weight the perpendicular axis so
            // we prefer items roughly aligned.
            let cost = match dir {
                FocusDirection::Up | FocusDirection::Down => dy.abs() + dx.abs() * 3,
                FocusDirection::Left | FocusDirection::Right => dx.abs() + dy.abs() * 3,
                _ => dx.abs() + dy.abs(),
            };

            if best.is_none() || cost < best.as_ref().map_or(i64::MAX, |b| b.0) {
                best = Some((cost, i));
            }
        }

        best.map_or(cur, |b| b.1)
    }
}

impl Default for SpatialFocusManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Draw a focus ring indicator around the given bounds.
///
/// This is a convenience function that widgets can call when they
/// detect they are focused. Uses `FocusStyle` internally. For a ring
/// that honors skin-authored `focus_ring_*` theme fields, use
/// [`draw_focus_ring_themed`].
pub fn draw_focus_ring(
    backend: &mut dyn oasis_types::backend::SdiBackend,
    bounds: &Rect,
    color: Color,
) -> oasis_types::error::Result<()> {
    let style = FocusStyle {
        color,
        width: 2,
        radius: 3,
        offset: 2,
    };
    style.draw(backend, bounds.x, bounds.y, bounds.w, bounds.h)
}

/// Draw a focus ring indicator styled by the theme.
///
/// Uses the theme's `focus_ring_*` overrides when the skin sets them.
/// Unset fields fall back to [`draw_focus_ring`]'s visuals (accent
/// color, 2 px stroke, offset 2), so skins without focus ring theming
/// render identically to before.
pub fn draw_focus_ring_themed(
    backend: &mut dyn oasis_types::backend::SdiBackend,
    bounds: &Rect,
    theme: &crate::theme::Theme,
) -> oasis_types::error::Result<()> {
    let style = FocusStyle {
        color: theme.focus_ring_color.unwrap_or(theme.accent),
        width: theme.focus_ring_width.unwrap_or(2),
        radius: 3,
        offset: theme.focus_ring_offset.unwrap_or(2),
    };
    style.draw(backend, bounds.x, bounds.y, bounds.w, bounds.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let f = FocusRing::new(5);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 5);
        assert!(f.wrap);
    }

    #[test]
    fn new_empty() {
        let f = FocusRing::new(0);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 0);
    }

    #[test]
    fn is_focused() {
        let f = FocusRing::new(3);
        assert!(f.is_focused(0));
        assert!(!f.is_focused(1));
        assert!(!f.is_focused(2));
    }

    #[test]
    fn set_focused_clamps() {
        let mut f = FocusRing::new(3);
        f.set_focused(10);
        assert_eq!(f.focused(), 2); // clamped to count-1
    }

    #[test]
    fn set_focused_valid() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        assert_eq!(f.focused(), 2);
    }

    #[test]
    fn set_focused_empty_noop() {
        let mut f = FocusRing::new(0);
        f.set_focused(5);
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn move_next_wraps() {
        let mut f = FocusRing::new(3);
        assert_eq!(f.move_focus(FocusDir::Next), 1);
        assert_eq!(f.move_focus(FocusDir::Next), 2);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps
    }

    #[test]
    fn move_prev_wraps() {
        let mut f = FocusRing::new(3);
        assert_eq!(f.move_focus(FocusDir::Prev), 2); // wraps from 0
        assert_eq!(f.move_focus(FocusDir::Prev), 1);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn move_next_no_wrap() {
        let mut f = FocusRing::new(3);
        f.wrap = false;
        f.set_focused(2);
        assert_eq!(f.move_focus(FocusDir::Next), 2); // stays at end
    }

    #[test]
    fn move_prev_no_wrap() {
        let mut f = FocusRing::new(3);
        f.wrap = false;
        assert_eq!(f.move_focus(FocusDir::Prev), 0); // stays at start
    }

    #[test]
    fn move_focus_empty_returns_zero() {
        let mut f = FocusRing::new(0);
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn move_focus_single_item() {
        let mut f = FocusRing::new(1);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps to same
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn focus_first_and_last() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.focus_first();
        assert_eq!(f.focused(), 0);
        f.focus_last();
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn focus_last_empty() {
        let mut f = FocusRing::new(0);
        f.focus_last();
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn set_count_clamps_focused() {
        let mut f = FocusRing::new(5);
        f.set_focused(4);
        f.set_count(3);
        assert_eq!(f.focused(), 2); // clamped
        assert_eq!(f.count(), 3);
    }

    #[test]
    fn set_count_to_zero() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.set_count(0);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 0);
    }

    #[test]
    fn set_count_grow_preserves_focus() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        f.set_count(10);
        assert_eq!(f.focused(), 2); // unchanged
    }

    #[test]
    fn focus_dir_debug() {
        assert_eq!(format!("{:?}", FocusDir::Next), "Next");
        assert_eq!(format!("{:?}", FocusDir::Prev), "Prev");
    }

    #[test]
    fn focus_ring_clone() {
        let f = FocusRing::new(3);
        let f2 = f.clone();
        assert_eq!(f.focused(), f2.focused());
        assert_eq!(f.count(), f2.count());
    }

    #[test]
    fn sequential_navigation() {
        let mut f = FocusRing::new(4);
        // Forward through all items
        for expected in 1..=3 {
            assert_eq!(f.move_focus(FocusDir::Next), expected);
        }
        // Backward through all items
        for expected in (0..=2).rev() {
            assert_eq!(f.move_focus(FocusDir::Prev), expected);
        }
    }

    // -- Additional focus traversal tests --

    #[test]
    fn wrap_around_full_cycle_next() {
        let mut f = FocusRing::new(3);
        // 0 -> 1 -> 2 -> 0 -> 1 -> 2 -> 0
        let expected = [1, 2, 0, 1, 2, 0];
        for &e in &expected {
            assert_eq!(f.move_focus(FocusDir::Next), e);
        }
    }

    #[test]
    fn wrap_around_full_cycle_prev() {
        let mut f = FocusRing::new(3);
        // 0 -> 2 -> 1 -> 0 -> 2 -> 1 -> 0
        let expected = [2, 1, 0, 2, 1, 0];
        for &e in &expected {
            assert_eq!(f.move_focus(FocusDir::Prev), e);
        }
    }

    #[test]
    fn no_wrap_next_boundary_stays() {
        let mut f = FocusRing::new(5);
        f.wrap = false;
        f.set_focused(4);
        // At the end, Next should not move.
        assert_eq!(f.move_focus(FocusDir::Next), 4);
        assert_eq!(f.move_focus(FocusDir::Next), 4);
    }

    #[test]
    fn no_wrap_prev_boundary_stays() {
        let mut f = FocusRing::new(5);
        f.wrap = false;
        f.set_focused(0);
        // At the beginning, Prev should not move.
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn set_count_increase() {
        let mut f = FocusRing::new(3);
        f.set_focused(2);
        f.set_count(10);
        assert_eq!(f.count(), 10);
        assert_eq!(f.focused(), 2); // unchanged
    }

    #[test]
    fn set_count_decrease_clamps_focus() {
        let mut f = FocusRing::new(10);
        f.set_focused(9);
        f.set_count(5);
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn set_count_to_one() {
        let mut f = FocusRing::new(10);
        f.set_focused(5);
        f.set_count(1);
        assert_eq!(f.focused(), 0);
        assert_eq!(f.count(), 1);
    }

    #[test]
    fn focus_first_from_middle() {
        let mut f = FocusRing::new(10);
        f.set_focused(5);
        f.focus_first();
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn focus_last_from_first() {
        let mut f = FocusRing::new(10);
        f.focus_last();
        assert_eq!(f.focused(), 9);
    }

    #[test]
    fn is_focused_after_movement() {
        let mut f = FocusRing::new(5);
        assert!(f.is_focused(0));
        f.move_focus(FocusDir::Next);
        assert!(!f.is_focused(0));
        assert!(f.is_focused(1));
    }

    #[test]
    fn set_focused_to_zero() {
        let mut f = FocusRing::new(5);
        f.set_focused(3);
        f.set_focused(0);
        assert_eq!(f.focused(), 0);
    }

    #[test]
    fn single_item_wrap_next_prev() {
        let mut f = FocusRing::new(1);
        f.wrap = true;
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn single_item_no_wrap_next_prev() {
        let mut f = FocusRing::new(1);
        f.wrap = false;
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn two_items_alternating() {
        let mut f = FocusRing::new(2);
        assert_eq!(f.move_focus(FocusDir::Next), 1);
        assert_eq!(f.move_focus(FocusDir::Next), 0);
        assert_eq!(f.move_focus(FocusDir::Prev), 1);
        assert_eq!(f.move_focus(FocusDir::Prev), 0);
    }

    #[test]
    fn focus_dir_equality() {
        assert_eq!(FocusDir::Next, FocusDir::Next);
        assert_eq!(FocusDir::Prev, FocusDir::Prev);
        assert_ne!(FocusDir::Next, FocusDir::Prev);
    }

    #[test]
    fn large_focus_ring() {
        let mut f = FocusRing::new(1000);
        f.set_focused(999);
        assert_eq!(f.focused(), 999);
        assert_eq!(f.move_focus(FocusDir::Next), 0); // wraps
    }

    #[test]
    fn set_focused_exact_max() {
        let mut f = FocusRing::new(5);
        f.set_focused(4); // max valid
        assert_eq!(f.focused(), 4);
    }

    #[test]
    fn set_focused_way_over_max() {
        let mut f = FocusRing::new(5);
        f.set_focused(1000);
        assert_eq!(f.focused(), 4);
    }

    // -- FocusStyle tests --

    #[test]
    fn focus_style_from_accent() {
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        assert_eq!(s.color, Color::rgb(80, 160, 255));
        assert_eq!(s.width, 1);
        assert_eq!(s.radius, 2);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn focus_style_from_theme_unset_matches_from_accent() {
        // A theme without focus_ring_* overrides must produce exactly
        // the accent-derived style (pixel-identical no-override path).
        let theme = crate::theme::Theme::dark();
        let themed = FocusStyle::from_theme(&theme);
        let derived = FocusStyle::from_accent(theme.accent);
        assert_eq!(themed.color, derived.color);
        assert_eq!(themed.width, derived.width);
        assert_eq!(themed.radius, derived.radius);
        assert_eq!(themed.offset, derived.offset);
    }

    #[test]
    fn focus_style_from_theme_honors_overrides() {
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgba(255, 0, 255, 160));
        theme.focus_ring_width = Some(3);
        theme.focus_ring_offset = Some(4);
        let s = FocusStyle::from_theme(&theme);
        assert_eq!(s.color, Color::rgba(255, 0, 255, 160));
        assert_eq!(s.width, 3);
        assert_eq!(s.offset, 4);
        // Radius stays at the accent-derived default.
        assert_eq!(s.radius, 2);
    }

    #[test]
    fn focus_style_from_theme_partial_override() {
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgb(0, 255, 0));
        let s = FocusStyle::from_theme(&theme);
        assert_eq!(s.color, Color::rgb(0, 255, 0));
        // Unset fields keep the accent-derived defaults.
        assert_eq!(s.width, 1);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn draw_focus_ring_themed_calls_backend() {
        use crate::test_utils::MockBackend;
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgb(255, 128, 0));
        let mut backend = MockBackend::new();
        let bounds = Rect::new(10, 20, 80, 40);
        draw_focus_ring_themed(&mut backend, &bounds, &theme).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn manager_draw_indicator_themed() {
        use crate::test_utils::MockBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let theme = crate::theme::Theme::dark();
        let mut backend = MockBackend::new();
        fm.draw_indicator_themed(&mut backend, 0, &theme, 10, 20, 80, 30)
            .ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn focus_style_indicator_rect() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 2,
            radius: 4,
            offset: 2,
        };
        let (fx, fy, fw, fh) = s.indicator_rect(10, 20, 100, 50);
        assert_eq!(fx, 8);
        assert_eq!(fy, 18);
        assert_eq!(fw, 104);
        assert_eq!(fh, 54);
    }

    #[test]
    fn focus_style_indicator_rect_zero_offset() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 1,
            radius: 0,
            offset: 0,
        };
        let r = s.indicator_rect(5, 10, 80, 30);
        assert_eq!(r, (5, 10, 80, 30));
    }

    #[test]
    fn focus_style_draw_calls_backend() {
        use crate::test_utils::MockBackend;
        let s = FocusStyle::from_accent(Color::rgb(0, 255, 0));
        let mut backend = MockBackend::new();
        s.draw(&mut backend, 10, 20, 100, 50).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    // -- FocusManager tests --

    #[test]
    fn manager_new_defaults() {
        let fm = FocusManager::new(5);
        assert_eq!(fm.count(), 5);
        assert_eq!(fm.focused(), 0);
        assert!(!fm.active);
    }

    #[test]
    fn manager_new_empty() {
        let fm = FocusManager::new(0);
        assert_eq!(fm.count(), 0);
        assert_eq!(fm.focused(), 0);
    }

    #[test]
    fn manager_has_focus_inactive() {
        let fm = FocusManager::new(3);
        assert!(!fm.has_focus(0));
    }

    #[test]
    fn manager_has_focus_active() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        assert!(fm.has_focus(0));
        assert!(!fm.has_focus(1));
    }

    #[test]
    fn manager_tab_next() {
        let mut fm = FocusManager::new(3);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert!(fm.active);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 2);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 0);
    }

    #[test]
    fn manager_tab_prev() {
        let mut fm = FocusManager::new(3);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 2);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 0);
    }

    #[test]
    fn manager_activate() {
        let mut fm = FocusManager::new(3);
        fm.handle_action(FocusAction::TabNext);
        let idx = fm.handle_action(FocusAction::Activate);
        assert_eq!(idx, 1);
    }

    #[test]
    fn manager_home() {
        let mut fm = FocusManager::new(5);
        fm.handle_action(FocusAction::TabNext);
        fm.handle_action(FocusAction::TabNext);
        assert_eq!(fm.handle_action(FocusAction::Home), 0);
    }

    #[test]
    fn manager_end() {
        let mut fm = FocusManager::new(5);
        assert_eq!(fm.handle_action(FocusAction::End), 4);
    }

    #[test]
    fn manager_skip_disabled_next() {
        let mut fm = FocusManager::new(4);
        fm.set_enabled(1, false);
        fm.set_enabled(2, false);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 3);
    }

    #[test]
    fn manager_skip_disabled_prev() {
        let mut fm = FocusManager::new(4);
        fm.set_enabled(2, false);
        fm.set_enabled(3, false);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
    }

    #[test]
    fn manager_all_disabled_stays() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(0, false);
        fm.set_enabled(1, false);
        fm.set_enabled(2, false);
        let before = fm.focused();
        fm.handle_action(FocusAction::TabNext);
        assert_eq!(fm.focused(), before);
    }

    #[test]
    fn manager_set_count_grows() {
        let mut fm = FocusManager::new(3);
        fm.set_count(5);
        assert_eq!(fm.count(), 5);
        assert!(fm.is_enabled(3));
        assert!(fm.is_enabled(4));
    }

    #[test]
    fn manager_set_count_shrinks() {
        let mut fm = FocusManager::new(5);
        fm.ring_mut().set_focused(4);
        fm.set_count(3);
        assert_eq!(fm.count(), 3);
        assert_eq!(fm.focused(), 2);
    }

    #[test]
    fn manager_set_focused_enabled() {
        let mut fm = FocusManager::new(5);
        fm.set_focused(3);
        assert_eq!(fm.focused(), 3);
        assert!(fm.active);
    }

    #[test]
    fn manager_set_focused_disabled_rejected() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(3, false);
        fm.set_focused(3);
        assert_eq!(fm.focused(), 0);
    }

    #[test]
    fn manager_deactivate() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        fm.deactivate();
        assert!(!fm.active);
        assert!(!fm.has_focus(0));
    }

    #[test]
    fn manager_is_enabled_out_of_bounds() {
        let fm = FocusManager::new(3);
        assert!(!fm.is_enabled(10));
    }

    #[test]
    fn manager_set_enabled_out_of_bounds_noop() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(10, false);
        assert_eq!(fm.count(), 3);
    }

    #[test]
    fn manager_ring_access() {
        let fm = FocusManager::new(5);
        assert_eq!(fm.ring().count(), 5);
    }

    #[test]
    fn manager_ring_mut_access() {
        let mut fm = FocusManager::new(5);
        fm.ring_mut().set_focused(3);
        assert_eq!(fm.focused(), 3);
    }

    #[test]
    fn manager_home_skips_disabled_first() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(0, false);
        assert_eq!(fm.handle_action(FocusAction::Home), 1);
    }

    #[test]
    fn manager_end_skips_disabled_last() {
        let mut fm = FocusManager::new(5);
        fm.set_enabled(4, false);
        assert_eq!(fm.handle_action(FocusAction::End), 3);
    }

    #[test]
    fn manager_draw_indicator_active() {
        use crate::test_utils::MockBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 0, &s, 10, 20, 80, 30).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn manager_draw_indicator_wrong_index() {
        use crate::test_utils::MockBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 1, &s, 10, 20, 80, 30).ok();
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn manager_draw_indicator_inactive() {
        use crate::test_utils::MockBackend;
        let fm = FocusManager::new(3);
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        let mut backend = MockBackend::new();
        fm.draw_indicator(&mut backend, 0, &s, 10, 20, 80, 30).ok();
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn manager_empty_handle_action() {
        let mut fm = FocusManager::new(0);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 0);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 0);
        assert_eq!(fm.handle_action(FocusAction::Home), 0);
        assert_eq!(fm.handle_action(FocusAction::End), 0);
    }

    #[test]
    fn focus_action_debug() {
        for a in [
            FocusAction::TabNext,
            FocusAction::TabPrev,
            FocusAction::Activate,
            FocusAction::Home,
            FocusAction::End,
        ] {
            let _ = format!("{a:?}");
        }
    }

    #[test]
    fn focus_action_equality() {
        assert_eq!(FocusAction::TabNext, FocusAction::TabNext);
        assert_ne!(FocusAction::TabNext, FocusAction::TabPrev);
    }

    #[test]
    fn manager_single_enabled_item() {
        let mut fm = FocusManager::new(3);
        fm.set_enabled(0, false);
        fm.set_enabled(2, false);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert_eq!(fm.handle_action(FocusAction::TabNext), 1);
        assert_eq!(fm.handle_action(FocusAction::TabPrev), 1);
    }

    #[test]
    fn manager_tab_full_cycle() {
        let mut fm = FocusManager::new(4);
        let mut visited = Vec::new();
        for _ in 0..4 {
            let idx = fm.handle_action(FocusAction::TabNext);
            visited.push(idx);
        }
        assert_eq!(visited, vec![1, 2, 3, 0]);
    }

    #[test]
    fn manager_clone() {
        let mut fm = FocusManager::new(3);
        fm.active = true;
        fm.set_enabled(1, false);
        let fm2 = fm.clone();
        assert_eq!(fm.focused(), fm2.focused());
        assert_eq!(fm.active, fm2.active);
        assert_eq!(fm.is_enabled(1), fm2.is_enabled(1));
    }

    // ── FocusDirection tests ────────────────────────────────────

    #[test]
    fn focus_direction_all_variants() {
        let dirs = [
            FocusDirection::Next,
            FocusDirection::Previous,
            FocusDirection::Up,
            FocusDirection::Down,
            FocusDirection::Left,
            FocusDirection::Right,
        ];
        for d in &dirs {
            let _ = format!("{d:?}");
        }
        assert_ne!(FocusDirection::Next, FocusDirection::Up);
        assert_eq!(FocusDirection::Left, FocusDirection::Left);
    }

    // ── Rect tests ──────────────────────────────────────────────

    #[test]
    fn rect_center() {
        let r = Rect::new(10, 20, 100, 50);
        assert_eq!(r.cx(), 60);
        assert_eq!(r.cy(), 45);
    }

    #[test]
    fn rect_zero_size_center() {
        let r = Rect::new(5, 5, 0, 0);
        assert_eq!(r.cx(), 5);
        assert_eq!(r.cy(), 5);
    }

    // ── SpatialFocusManager tests ───────────────────────────────

    fn make_items() -> Vec<FocusableItem> {
        // 2x2 grid:
        //  [A(0,0)]  [B(100,0)]
        //  [C(0,100)] [D(100,100)]
        vec![
            FocusableItem {
                id: "A".into(),
                tab_index: 0,
                bounds: Rect::new(0, 0, 80, 40),
            },
            FocusableItem {
                id: "B".into(),
                tab_index: 1,
                bounds: Rect::new(100, 0, 80, 40),
            },
            FocusableItem {
                id: "C".into(),
                tab_index: 2,
                bounds: Rect::new(0, 100, 80, 40),
            },
            FocusableItem {
                id: "D".into(),
                tab_index: 3,
                bounds: Rect::new(100, 100, 80, 40),
            },
        ]
    }

    #[test]
    fn spatial_new_empty() {
        let sm = SpatialFocusManager::new();
        assert_eq!(sm.item_count(), 0);
        assert_eq!(sm.focused_index(), None);
        assert!(!sm.focus_ring_visible());
    }

    #[test]
    fn spatial_default_is_new() {
        let sm = SpatialFocusManager::default();
        assert_eq!(sm.item_count(), 0);
    }

    #[test]
    fn spatial_add_and_focus() {
        let mut sm = SpatialFocusManager::new();
        for item in make_items() {
            sm.add_item(item);
        }
        assert_eq!(sm.item_count(), 4);
        assert_eq!(sm.focused_index(), None);

        sm.navigate(FocusDirection::Next);
        assert_eq!(sm.focused_index(), Some(0));
        assert!(sm.focus_ring_visible());
    }

    #[test]
    fn spatial_tab_order_sequential() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        // Tab through: None->A->B->C->D->A
        assert_eq!(sm.navigate(FocusDirection::Next), Some(0));
        assert_eq!(sm.focused_item().map(|i| i.id.as_str()), Some("A"));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(1));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(2));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(3));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(0));
    }

    #[test]
    fn spatial_tab_order_reverse() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        // Previous from None -> last in tab order (D)
        assert_eq!(sm.navigate(FocusDirection::Previous), Some(3));
        assert_eq!(sm.navigate(FocusDirection::Previous), Some(2));
        assert_eq!(sm.navigate(FocusDirection::Previous), Some(1));
        assert_eq!(sm.navigate(FocusDirection::Previous), Some(0));
        // Wraps back to D
        assert_eq!(sm.navigate(FocusDirection::Previous), Some(3));
    }

    #[test]
    fn spatial_arrow_right() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(0); // A at (0,0)

        // Right from A -> B at (100,0)
        assert_eq!(sm.navigate(FocusDirection::Right), Some(1));
    }

    #[test]
    fn spatial_arrow_down() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(0); // A at (0,0)

        // Down from A -> C at (0,100)
        assert_eq!(sm.navigate(FocusDirection::Down), Some(2));
    }

    #[test]
    fn spatial_arrow_left() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(1); // B at (100,0)

        // Left from B -> A at (0,0)
        assert_eq!(sm.navigate(FocusDirection::Left), Some(0));
    }

    #[test]
    fn spatial_arrow_up() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(2); // C at (0,100)

        // Up from C -> A at (0,0)
        assert_eq!(sm.navigate(FocusDirection::Up), Some(0));
    }

    #[test]
    fn spatial_no_candidate_stays() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(0); // A at top-left

        // Up from A -> nothing above, stays at A
        assert_eq!(sm.navigate(FocusDirection::Up), Some(0));
        // Left from A -> nothing to left, stays at A
        assert_eq!(sm.navigate(FocusDirection::Left), Some(0));
    }

    #[test]
    fn spatial_focus_ring_visibility_toggle() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        assert!(!sm.focus_ring_visible());

        // Tab shows focus ring
        sm.navigate(FocusDirection::Next);
        assert!(sm.focus_ring_visible());

        // Mouse hides it
        sm.hide_focus_ring();
        assert!(!sm.focus_ring_visible());

        // Keyboard shows it again
        sm.show_focus_ring();
        assert!(sm.focus_ring_visible());
    }

    #[test]
    fn spatial_handle_input_tab() {
        use oasis_types::input::InputEvent;
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        let evt = InputEvent::TextInput('\t');
        assert_eq!(sm.handle_input(&evt), FocusEvent::Moved(0));
        assert_eq!(sm.handle_input(&evt), FocusEvent::Moved(1));
    }

    #[test]
    fn spatial_handle_input_shift_tab() {
        use oasis_types::input::{Button, InputEvent};
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        // Press Select (= Shift)
        sm.handle_input(&InputEvent::ButtonPress(Button::Select));
        // Shift+Tab goes Previous
        let evt = InputEvent::TextInput('\t');
        assert_eq!(sm.handle_input(&evt), FocusEvent::Moved(3));
        assert_eq!(sm.handle_input(&evt), FocusEvent::Moved(2));
        // Release Select
        sm.handle_input(&InputEvent::ButtonRelease(Button::Select));
        // Now Tab goes Next
        assert_eq!(sm.handle_input(&evt), FocusEvent::Moved(3));
    }

    #[test]
    fn spatial_handle_input_arrows() {
        use oasis_types::input::{Button, InputEvent};
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(0);

        let r = sm.handle_input(&InputEvent::ButtonPress(Button::Right));
        assert_eq!(r, FocusEvent::Moved(1)); // A -> B

        let r = sm.handle_input(&InputEvent::ButtonPress(Button::Down));
        assert_eq!(r, FocusEvent::Moved(3)); // B -> D
    }

    #[test]
    fn spatial_handle_input_activate() {
        use oasis_types::input::{Button, InputEvent};
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(2);

        let r = sm.handle_input(&InputEvent::ButtonPress(Button::Confirm));
        assert_eq!(r, FocusEvent::Activate);
    }

    #[test]
    fn spatial_handle_input_activate_none_focused() {
        use oasis_types::input::{Button, InputEvent};
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        // No focus set
        let r = sm.handle_input(&InputEvent::ButtonPress(Button::Confirm));
        assert_eq!(r, FocusEvent::None);
    }

    #[test]
    fn spatial_handle_input_mouse_hides_ring() {
        use oasis_types::input::InputEvent;
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());

        // Tab to show ring
        sm.handle_input(&InputEvent::TextInput('\t'));
        assert!(sm.focus_ring_visible());

        // Mouse click hides ring
        sm.handle_input(&InputEvent::PointerClick { x: 50, y: 50 });
        assert!(!sm.focus_ring_visible());
    }

    #[test]
    fn spatial_negative_tab_index_skipped() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(vec![
            FocusableItem {
                id: "A".into(),
                tab_index: 0,
                bounds: Rect::new(0, 0, 40, 40),
            },
            FocusableItem {
                id: "skip".into(),
                tab_index: -1,
                bounds: Rect::new(50, 0, 40, 40),
            },
            FocusableItem {
                id: "B".into(),
                tab_index: 1,
                bounds: Rect::new(100, 0, 40, 40),
            },
        ]);

        // Tab: A -> B -> A (skips "skip")
        assert_eq!(sm.navigate(FocusDirection::Next), Some(0));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(2));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(0));
    }

    #[test]
    fn spatial_set_items_clamps_focus() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(3);
        assert_eq!(sm.focused_index(), Some(3));

        // Shrink to 2 items -> focus clamped to 1
        sm.set_items(make_items()[..2].to_vec());
        assert_eq!(sm.focused_index(), Some(1));
    }

    #[test]
    fn spatial_set_items_empty_clears_focus() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(2);
        sm.set_items(vec![]);
        assert_eq!(sm.focused_index(), None);
    }

    #[test]
    fn spatial_clear_focus() {
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        sm.set_focused(1);
        sm.clear_focus();
        assert_eq!(sm.focused_index(), None);
    }

    #[test]
    fn spatial_navigate_empty_returns_none() {
        let mut sm = SpatialFocusManager::new();
        assert_eq!(sm.navigate(FocusDirection::Next), None);
        assert_eq!(sm.navigate(FocusDirection::Up), None);
    }

    #[test]
    fn spatial_custom_tab_order() {
        let mut sm = SpatialFocusManager::new();
        // Register in reverse tab order
        sm.set_items(vec![
            FocusableItem {
                id: "C".into(),
                tab_index: 2,
                bounds: Rect::new(0, 0, 40, 40),
            },
            FocusableItem {
                id: "A".into(),
                tab_index: 0,
                bounds: Rect::new(50, 0, 40, 40),
            },
            FocusableItem {
                id: "B".into(),
                tab_index: 1,
                bounds: Rect::new(100, 0, 40, 40),
            },
        ]);

        // Tab order should be A(1) -> B(2) -> C(0)
        assert_eq!(sm.navigate(FocusDirection::Next), Some(1));
        assert_eq!(sm.focused_item().map(|i| i.id.as_str()), Some("A"));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(2));
        assert_eq!(sm.focused_item().map(|i| i.id.as_str()), Some("B"));
        assert_eq!(sm.navigate(FocusDirection::Next), Some(0));
        assert_eq!(sm.focused_item().map(|i| i.id.as_str()), Some("C"));
    }

    #[test]
    fn spatial_diagonal_prefers_aligned() {
        // Items: A at (0,0), B at (200,10), C at (10,200)
        // From A, Right should pick B (mostly horizontal)
        // From A, Down should pick C (mostly vertical)
        let mut sm = SpatialFocusManager::new();
        sm.set_items(vec![
            FocusableItem {
                id: "A".into(),
                tab_index: 0,
                bounds: Rect::new(0, 0, 40, 40),
            },
            FocusableItem {
                id: "B".into(),
                tab_index: 1,
                bounds: Rect::new(200, 10, 40, 40),
            },
            FocusableItem {
                id: "C".into(),
                tab_index: 2,
                bounds: Rect::new(10, 200, 40, 40),
            },
        ]);

        sm.set_focused(0);
        assert_eq!(sm.navigate(FocusDirection::Right), Some(1));

        sm.set_focused(0);
        assert_eq!(sm.navigate(FocusDirection::Down), Some(2));
    }

    #[test]
    fn draw_focus_ring_calls_backend() {
        use crate::test_utils::MockBackend;
        let mut backend = MockBackend::new();
        let bounds = Rect::new(10, 20, 80, 40);
        draw_focus_ring(&mut backend, &bounds, Color::rgb(0, 120, 255)).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn focus_event_variants() {
        assert_eq!(FocusEvent::Moved(0), FocusEvent::Moved(0));
        assert_ne!(FocusEvent::Moved(0), FocusEvent::Moved(1));
        assert_eq!(FocusEvent::Activate, FocusEvent::Activate);
        assert_ne!(FocusEvent::Activate, FocusEvent::None);
        let _ = format!("{:?}", FocusEvent::None);
    }
}

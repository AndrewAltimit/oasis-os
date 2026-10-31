//! [`SpatialFocusManager`]: tab-index order plus arrow-key spatial moves.

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
        // `w / 2` always fits an i32 (`w as i32 / 2` went negative for
        // widths past i32::MAX).
        self.x.saturating_add((self.w / 2) as i32)
    }

    /// Vertical centre.
    pub fn cy(&self) -> i32 {
        self.y.saturating_add((self.h / 2) as i32)
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
    /// Handles Tab / Shift+Tab (the [`InputEvent::Tab`] /
    /// [`InputEvent::ShiftTab`] events every keyboard backend emits, plus a
    /// raw `'\t'` character with the held Select button acting as Shift on
    /// gamepads), arrow keys, Confirm, and pointer events (hide the focus
    /// ring).
    ///
    /// [`InputEvent::Tab`]: oasis_types::input::InputEvent::Tab
    /// [`InputEvent::ShiftTab`]: oasis_types::input::InputEvent::ShiftTab
    pub fn handle_input(&mut self, event: &oasis_types::input::InputEvent) -> FocusEvent {
        use oasis_types::input::{Button, InputEvent};
        match event {
            // -- Sequential navigation --
            InputEvent::Tab => self.nav_event(FocusDirection::Next),
            InputEvent::ShiftTab => self.nav_event(FocusDirection::Previous),
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
        // Nothing focused yet: an arrow key lands on the first item in
        // tab order (not blindly on index 0, which may be unfocusable).
        let Some(cur) = self.focused else {
            return self.tab_order().first().copied().unwrap_or(0);
        };
        let cur_bounds = &self.items[cur].bounds;
        let cx = cur_bounds.cx();
        let cy = cur_bounds.cy();

        let mut best: Option<(i64, usize)> = None;

        for (i, item) in self.items.iter().enumerate() {
            // Negative tab_index items are not keyboard-focusable: arrow
            // keys used to land on them although Tab skips them.
            if i == cur || item.tab_index < 0 {
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

#[cfg(test)]
mod tests {
    use crate::focus::*;
    #[allow(unused_imports)]
    use oasis_types::backend::Color;
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
    fn spatial_handle_input_tab_events_from_keyboard_backends() {
        use oasis_types::input::InputEvent;
        let mut sm = SpatialFocusManager::new();
        sm.set_items(make_items());
        assert_eq!(sm.handle_input(&InputEvent::Tab), FocusEvent::Moved(0));
        assert_eq!(sm.handle_input(&InputEvent::Tab), FocusEvent::Moved(1));
        assert_eq!(sm.handle_input(&InputEvent::ShiftTab), FocusEvent::Moved(0));
        assert_eq!(sm.handle_input(&InputEvent::ShiftTab), FocusEvent::Moved(3));
        assert!(sm.focus_ring_visible());
    }

    #[test]
    fn spatial_arrow_with_nothing_focused_skips_untabbable_first_item() {
        let mut sm = SpatialFocusManager::new();
        let mut items = make_items();
        items[0].tab_index = -1;
        sm.set_items(items);
        assert_eq!(sm.navigate(FocusDirection::Down), Some(1));
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
        // Arrow keys skip it too (Right from A used to land on "skip").
        assert_eq!(sm.navigate(FocusDirection::Right), Some(2));
        assert_eq!(sm.navigate(FocusDirection::Left), Some(0));
    }

    #[test]
    fn rect_center_handles_huge_sizes() {
        let r = Rect::new(0, 0, u32::MAX, u32::MAX);
        assert_eq!((r.cx(), r.cy()), (i32::MAX, i32::MAX));
        assert_eq!(Rect::new(-10, 4, 20, 2).cx(), 0);
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
    fn focus_event_variants() {
        assert_eq!(FocusEvent::Moved(0), FocusEvent::Moved(0));
        assert_ne!(FocusEvent::Moved(0), FocusEvent::Moved(1));
        assert_eq!(FocusEvent::Activate, FocusEvent::Activate);
        assert_ne!(FocusEvent::Activate, FocusEvent::None);
        let _ = format!("{:?}", FocusEvent::None);
    }
}

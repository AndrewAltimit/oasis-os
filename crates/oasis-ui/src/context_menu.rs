//! Context menu widget: right-click popup with actions, separators, and submenus.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A single entry in a context menu.
#[derive(Debug, Clone)]
pub enum MenuItem {
    /// A clickable action.
    Action {
        /// Display label for this action.
        label: String,
        /// Whether this action is currently enabled.
        enabled: bool,
    },
    /// A horizontal separator line between groups.
    Separator,
    /// A nested submenu.
    Submenu {
        /// Display label for this submenu.
        label: String,
        /// Child items shown when the submenu is opened.
        items: Vec<MenuItem>,
    },
}

impl MenuItem {
    /// Create an enabled action item.
    pub fn action(label: impl Into<String>) -> Self {
        Self::Action {
            label: label.into(),
            enabled: true,
        }
    }

    /// Create a disabled action item.
    pub fn action_disabled(label: impl Into<String>) -> Self {
        Self::Action {
            label: label.into(),
            enabled: false,
        }
    }

    /// Create a separator.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Create a submenu.
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            items,
        }
    }

    /// Whether this item is selectable (not a separator, and enabled if action).
    fn is_selectable(&self) -> bool {
        match self {
            Self::Action { enabled, .. } => *enabled,
            Self::Separator => false,
            Self::Submenu { .. } => true,
        }
    }
}

/// Horizontal padding inside the menu panel.
const PAD_H: i32 = 6;
/// Vertical padding above the first item and below the last.
const PAD_V: i32 = 4;
/// Height of a separator line.
const SEPARATOR_HEIGHT: u32 = 7;
/// Submenu arrow indicator.
const SUBMENU_ARROW: &str = "\u{25B6}";

/// A popup context menu that renders a list of `MenuItem`s at a screen position.
pub struct ContextMenu {
    /// Items to display in the menu.
    pub items: Vec<MenuItem>,
    /// Screen X position of the menu's top-left corner.
    pub x: i32,
    /// Screen Y position of the menu's top-left corner.
    pub y: i32,
    /// Whether the menu is currently shown.
    pub visible: bool,
    /// Index of the currently highlighted item, if any.
    pub selected_index: Option<usize>,
    /// Index of the currently open submenu, if any.
    pub open_submenu: Option<usize>,
}

impl ContextMenu {
    /// Create a new context menu with the given items, initially hidden.
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            x: 0,
            y: 0,
            visible: false,
            selected_index: None,
            open_submenu: None,
        }
    }

    /// Show the menu at the given screen position.
    pub fn show(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.visible = true;
        self.selected_index = None;
        self.open_submenu = None;
    }

    /// Hide the menu and reset selection state.
    pub fn hide(&mut self) {
        self.visible = false;
        self.selected_index = None;
        self.open_submenu = None;
    }

    /// Whether the menu is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Return the currently selected item, if any.
    pub fn selected_item(&self) -> Option<&MenuItem> {
        self.selected_index.and_then(|i| self.items.get(i))
    }

    /// Move the selection to the previous selectable item, wrapping around.
    pub fn navigate_up(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let start = match self.selected_index {
            Some(0) | None => self.items.len() - 1,
            Some(i) => i - 1,
        };
        self.selected_index = Some(self.find_selectable_backward(start));
    }

    /// Move the selection to the next selectable item, wrapping around.
    pub fn navigate_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let start = match self.selected_index {
            None => 0,
            Some(i) => (i + 1) % self.items.len(),
        };
        self.selected_index = Some(self.find_selectable_forward(start));
    }

    /// Activate the currently selected item.
    ///
    /// Returns `Some(flat_action_index)` if an enabled `Action` was activated,
    /// where the index counts only `Action` items (skipping separators and
    /// submenus). Returns `None` if nothing is selected, the selected item is
    /// disabled, or it is a separator.
    ///
    /// If the selected item is a `Submenu`, the submenu is toggled open and
    /// `None` is returned.
    pub fn activate(&mut self) -> Option<usize> {
        let idx = self.selected_index?;
        let item = self.items.get(idx)?;
        match item {
            MenuItem::Action { enabled: true, .. } => {
                // Compute the flat action index: count only Action items before this one.
                let action_idx = self
                    .items
                    .iter()
                    .take(idx)
                    .filter(|it| matches!(it, MenuItem::Action { .. }))
                    .count();
                Some(action_idx)
            },
            MenuItem::Action { enabled: false, .. } | MenuItem::Separator => None,
            MenuItem::Submenu { .. } => {
                self.open_submenu = if self.open_submenu == Some(idx) {
                    None
                } else {
                    Some(idx)
                };
                None
            },
        }
    }

    /// Check if a click at `(mx, my)` is outside the menu bounds.
    ///
    /// Returns `true` if the click is outside (caller should dismiss the menu),
    /// `false` if it is inside.
    pub fn dismiss_on_click(&self, ctx: &DrawContext<'_>, mx: i32, my: i32) -> bool {
        if !self.visible {
            return false;
        }
        let (w, h) = self.compute_size(ctx);
        !(mx >= self.x && mx < self.x + w as i32 && my >= self.y && my < self.y + h as i32)
    }

    // -- Internal helpers --

    /// Compute the width and total height of the menu panel.
    fn compute_size(&self, ctx: &DrawContext<'_>) -> (u32, u32) {
        let fs = ctx.theme.font_size_md;
        let row_h = self.row_height(ctx);

        // Width: find the widest label + padding.
        let max_label_w = self
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { label, .. } | MenuItem::Submenu { label, .. } => {
                    ctx.backend.measure_text(label, fs)
                },
                MenuItem::Separator => 0,
            })
            .max()
            .unwrap_or(0);
        // Extra space for the submenu arrow if any submenus exist.
        let arrow_w = if self
            .items
            .iter()
            .any(|it| matches!(it, MenuItem::Submenu { .. }))
        {
            ctx.backend.measure_text(SUBMENU_ARROW, fs) + PAD_H as u32
        } else {
            0
        };
        let w = max_label_w + PAD_H as u32 * 2 + arrow_w;

        // Height: sum up row heights.
        let h = self
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => SEPARATOR_HEIGHT,
                _ => row_h,
            })
            .sum::<u32>()
            + PAD_V as u32 * 2;

        (w.max(40), h)
    }

    /// Height of a single non-separator row.
    fn row_height(&self, ctx: &DrawContext<'_>) -> u32 {
        ctx.backend.measure_text_height(ctx.theme.font_size_md) + 6
    }

    /// Starting from `start`, search forward (wrapping) for a selectable item.
    fn find_selectable_forward(&self, start: usize) -> usize {
        let n = self.items.len();
        for offset in 0..n {
            let i = (start + offset) % n;
            if self.items[i].is_selectable() {
                return i;
            }
        }
        // Fallback: nothing selectable, return start.
        start
    }

    /// Starting from `start`, search backward (wrapping) for a selectable item.
    fn find_selectable_backward(&self, start: usize) -> usize {
        let n = self.items.len();
        for offset in 0..n {
            let i = (start + n - offset) % n;
            if self.items[i].is_selectable() {
                return i;
            }
        }
        start
    }
}

impl Widget for ContextMenu {
    fn measure(&self, ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        self.compute_size(ctx)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        if !self.visible {
            return Ok(());
        }

        let radius = ctx.theme.border_radius_md;
        let fs = ctx.theme.font_size_md;
        let row_h = self.row_height(ctx);
        let text_h = ctx.backend.measure_text_height(fs);
        let ty_off = layout::center(row_h, text_h);

        // Shadow.
        ctx.theme
            .shadow_dropdown
            .draw(ctx.backend, x, y, w, h, radius)?;

        // Surface background.
        ctx.backend
            .fill_rounded_rect(x, y, w, h, radius, ctx.theme.surface)?;

        // Border.
        ctx.backend
            .stroke_rounded_rect(x, y, w, h, radius, 1, ctx.theme.border_subtle)?;

        // Draw each item.
        let mut cy = y + PAD_V;
        for (i, item) in self.items.iter().enumerate() {
            match item {
                MenuItem::Action { label, enabled } => {
                    // Highlight selected row.
                    if self.selected_index == Some(i) {
                        ctx.backend.fill_rect(
                            x + 1,
                            cy,
                            w.saturating_sub(2),
                            row_h,
                            ctx.theme.accent_subtle,
                        )?;
                    }

                    let color = if *enabled {
                        ctx.theme.text_primary
                    } else {
                        ctx.theme.text_disabled
                    };
                    ctx.backend.draw_text_ellipsis(
                        label,
                        x + PAD_H,
                        cy + ty_off,
                        fs,
                        color,
                        w.saturating_sub(PAD_H as u32 * 2),
                    )?;

                    cy += row_h as i32;
                },
                MenuItem::Separator => {
                    let sep_y = cy + SEPARATOR_HEIGHT as i32 / 2;
                    ctx.backend.draw_line(
                        x + PAD_H,
                        sep_y,
                        x + w as i32 - PAD_H,
                        sep_y,
                        1,
                        ctx.theme.border_subtle,
                    )?;
                    cy += SEPARATOR_HEIGHT as i32;
                },
                MenuItem::Submenu { label, .. } => {
                    // Highlight selected row.
                    if self.selected_index == Some(i) {
                        ctx.backend.fill_rect(
                            x + 1,
                            cy,
                            w.saturating_sub(2),
                            row_h,
                            ctx.theme.accent_subtle,
                        )?;
                    }

                    ctx.backend.draw_text_ellipsis(
                        label,
                        x + PAD_H,
                        cy + ty_off,
                        fs,
                        ctx.theme.text_primary,
                        w.saturating_sub(PAD_H as u32 * 3),
                    )?;

                    // Submenu arrow on the right side.
                    let arrow_w = ctx.backend.measure_text(SUBMENU_ARROW, fs);
                    ctx.backend.draw_text(
                        SUBMENU_ARROW,
                        x + w as i32 - arrow_w as i32 - PAD_H,
                        cy + ty_off,
                        fs,
                        ctx.theme.text_secondary,
                    )?;

                    cy += row_h as i32;
                },
            }
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

    /// Build a sample menu with mixed items.
    fn sample_items() -> Vec<MenuItem> {
        vec![
            MenuItem::action("Cut"),
            MenuItem::action("Copy"),
            MenuItem::action_disabled("Paste"),
            MenuItem::separator(),
            MenuItem::submenu(
                "More",
                vec![MenuItem::action("Sub A"), MenuItem::action("Sub B")],
            ),
        ]
    }

    // -- Construction --

    #[test]
    fn new_creates_hidden_menu() {
        let menu = ContextMenu::new(sample_items());
        assert!(!menu.is_visible());
        assert_eq!(menu.items.len(), 5);
        assert!(menu.selected_index.is_none());
        assert!(menu.open_submenu.is_none());
    }

    // -- Show / Hide --

    #[test]
    fn show_makes_visible_at_position() {
        let mut menu = ContextMenu::new(sample_items());
        menu.show(100, 200);
        assert!(menu.is_visible());
        assert_eq!(menu.x, 100);
        assert_eq!(menu.y, 200);
        assert!(menu.selected_index.is_none());
    }

    #[test]
    fn hide_clears_state() {
        let mut menu = ContextMenu::new(sample_items());
        menu.show(50, 60);
        menu.navigate_down();
        menu.hide();
        assert!(!menu.is_visible());
        assert!(menu.selected_index.is_none());
        assert!(menu.open_submenu.is_none());
    }

    // -- Navigation --

    #[test]
    fn navigate_down_selects_first_item() {
        let mut menu = ContextMenu::new(sample_items());
        menu.navigate_down();
        assert_eq!(menu.selected_index, Some(0));
    }

    #[test]
    fn navigate_down_skips_separator() {
        let mut menu = ContextMenu::new(sample_items());
        // Items: 0=Cut, 1=Copy, 2=Paste(disabled), 3=Separator, 4=More(submenu)
        // Navigate to index 2 (Paste disabled), then down should skip separator
        // to index 4 (More).
        menu.selected_index = Some(2);
        menu.navigate_down();
        // Index 3 is separator -> skip to 4
        assert_eq!(menu.selected_index, Some(4));
    }

    #[test]
    fn navigate_down_skips_disabled_and_separator() {
        let items = vec![
            MenuItem::action("A"),
            MenuItem::separator(),
            MenuItem::action_disabled("B"),
            MenuItem::action("C"),
        ];
        let mut menu = ContextMenu::new(items);
        menu.selected_index = Some(0);
        menu.navigate_down();
        // Should skip separator (1) and disabled (2), land on C (3).
        assert_eq!(menu.selected_index, Some(3));
    }

    #[test]
    fn navigate_up_wraps_to_last() {
        let mut menu = ContextMenu::new(sample_items());
        // From nothing, up should wrap to last selectable item.
        menu.navigate_up();
        // Last item is index 4 (submenu "More"), which is selectable.
        assert_eq!(menu.selected_index, Some(4));
    }

    #[test]
    fn navigate_up_skips_separator() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(4);
        menu.navigate_up();
        // Index 3 is separator -> skip. Index 2 is disabled -> skip.
        // Index 1 is "Copy" (enabled) -> select.
        assert_eq!(menu.selected_index, Some(1));
    }

    #[test]
    fn navigate_empty_menu() {
        let mut menu = ContextMenu::new(vec![]);
        menu.navigate_down();
        assert!(menu.selected_index.is_none());
        menu.navigate_up();
        assert!(menu.selected_index.is_none());
    }

    // -- Activate --

    #[test]
    fn activate_returns_action_index() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(0); // "Cut"
        let result = menu.activate();
        assert_eq!(result, Some(0));
    }

    #[test]
    fn activate_second_action() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(1); // "Copy"
        let result = menu.activate();
        assert_eq!(result, Some(1));
    }

    #[test]
    fn activate_disabled_returns_none() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(2); // "Paste" (disabled)
        let result = menu.activate();
        assert!(result.is_none());
    }

    #[test]
    fn activate_separator_returns_none() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(3); // Separator
        let result = menu.activate();
        assert!(result.is_none());
    }

    #[test]
    fn activate_submenu_toggles_open() {
        let mut menu = ContextMenu::new(sample_items());
        menu.selected_index = Some(4); // "More" submenu
        let result = menu.activate();
        assert!(result.is_none());
        assert_eq!(menu.open_submenu, Some(4));
        // Activate again to close.
        let result = menu.activate();
        assert!(result.is_none());
        assert!(menu.open_submenu.is_none());
    }

    #[test]
    fn activate_nothing_selected() {
        let mut menu = ContextMenu::new(sample_items());
        let result = menu.activate();
        assert!(result.is_none());
    }

    // -- Dismiss on click --

    #[test]
    fn dismiss_outside_click() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut menu = ContextMenu::new(sample_items());
        menu.show(100, 100);
        // Click far outside.
        assert!(menu.dismiss_on_click(&ctx, 0, 0));
    }

    #[test]
    fn no_dismiss_inside_click() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let mut menu = ContextMenu::new(sample_items());
        menu.show(10, 10);
        // Click inside the menu area.
        assert!(!menu.dismiss_on_click(&ctx, 15, 15));
    }

    #[test]
    fn dismiss_when_hidden_returns_false() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let menu = ContextMenu::new(sample_items());
        assert!(!menu.dismiss_on_click(&ctx, 0, 0));
    }

    // -- Measure --

    #[test]
    fn measure_returns_positive_dimensions() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let menu = ContextMenu::new(sample_items());
        let (w, h) = menu.measure(&ctx, 480, 272);
        assert!(w > 0, "menu width should be positive");
        assert!(h > 0, "menu height should be positive");
    }

    #[test]
    fn measure_empty_menu() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let menu = ContextMenu::new(vec![]);
        let (w, h) = menu.measure(&ctx, 480, 272);
        // Minimum width + top/bottom padding.
        assert!(w >= 40);
        assert!(h >= PAD_V as u32 * 2);
    }

    // -- Draw tests --

    #[test]
    fn draw_visible_emits_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut menu = ContextMenu::new(sample_items());
            menu.visible = true;
            let (w, h) = menu.compute_size(&ctx);
            menu.draw(&mut ctx, 10, 10, w, h).unwrap();
        }
        assert!(backend.has_text("Cut"));
        assert!(backend.has_text("Copy"));
        assert!(backend.has_text("Paste"));
        assert!(backend.has_text("More"));
        assert!(backend.has_text(SUBMENU_ARROW));
    }

    #[test]
    fn draw_hidden_emits_nothing() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let menu = ContextMenu::new(sample_items());
            menu.draw(&mut ctx, 10, 10, 100, 100).unwrap();
        }
        assert!(!backend.has_text("Cut"));
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn draw_with_selection_highlights_row() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut menu = ContextMenu::new(sample_items());
            menu.visible = true;
            menu.selected_index = Some(1);
            let (w, h) = menu.compute_size(&ctx);
            menu.draw(&mut ctx, 0, 0, w, h).unwrap();
        }
        // At least one fill_rect for the selection highlight.
        assert!(backend.fill_rect_count() > 0);
        assert!(backend.has_text("Copy"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        test_utils::test_draw_all_themes(|ctx| {
            let mut menu = ContextMenu::new(sample_items());
            menu.visible = true;
            menu.selected_index = Some(0);
            menu.draw(ctx, 0, 0, 150, 120).unwrap();
        });
    }

    // -- MenuItem helpers --

    #[test]
    fn menu_item_constructors() {
        let a = MenuItem::action("Open");
        assert!(matches!(a, MenuItem::Action { label, enabled: true } if label == "Open"));

        let d = MenuItem::action_disabled("Delete");
        assert!(matches!(d, MenuItem::Action { label, enabled: false } if label == "Delete"));

        let s = MenuItem::separator();
        assert!(matches!(s, MenuItem::Separator));

        let sub = MenuItem::submenu("Sub", vec![MenuItem::action("X")]);
        assert!(
            matches!(sub, MenuItem::Submenu { label, items } if label == "Sub" && items.len() == 1)
        );
    }

    #[test]
    fn is_selectable_logic() {
        assert!(MenuItem::action("A").is_selectable());
        assert!(!MenuItem::action_disabled("B").is_selectable());
        assert!(!MenuItem::separator().is_selectable());
        assert!(MenuItem::submenu("C", vec![]).is_selectable());
    }

    // -- selected_item --

    #[test]
    fn selected_item_returns_correct_item() {
        let mut menu = ContextMenu::new(sample_items());
        assert!(menu.selected_item().is_none());
        menu.selected_index = Some(1);
        let item = menu.selected_item().unwrap();
        assert!(matches!(item, MenuItem::Action { label, .. } if label == "Copy"));
    }
}

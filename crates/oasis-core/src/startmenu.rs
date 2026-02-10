//! Start menu popup -- a PSIX-style overlay toggled by a bottom-bar button.
//!
//! Displays a 2-column grid of app categories anchored above the bottom bar.
//! The start button is always visible in Dashboard mode; the popup appears
//! when toggled open.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_rounded_fill, ensure_text, hide_objects};
use crate::theme;

// -- Layout constants ---------------------------------------------------------

/// Start button position and size on the bottom bar.
const BTN_X: i32 = 4;
const BTN_Y: i32 = theme::BOTTOMBAR_Y + 3;
const BTN_W: u32 = 48;
const BTN_H: u32 = 18;

/// Menu panel geometry.
const MENU_X: i32 = 2;
const MENU_W: u32 = 200;
const MENU_COLS: usize = 2;
const ITEM_ROW_H: i32 = 22;
const ICON_SIZE: u32 = 14;
const PAD_TOP: i32 = 8;
const PAD_BOTTOM: i32 = 8;
const PAD_LEFT: i32 = 8;
const COL_W: i32 = (MENU_W as i32 - PAD_LEFT * 2) / MENU_COLS as i32;

/// Z-order for menu objects (above bars at 900, below cursor).
const Z_MENU: i32 = 950;
/// Z-order for the start button (sits on the bottom bar layer).
const Z_BUTTON: i32 = 903;

/// Maximum items supported (for SDI object naming).
const MAX_ITEMS: usize = 12;

// -- Types --------------------------------------------------------------------

/// Action returned when a menu item is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartMenuAction {
    /// Launch an app by title (matched against `AppEntry.title`).
    LaunchApp(String),
    /// Switch to the terminal.
    OpenTerminal,
    /// Run a terminal command.
    RunCommand(String),
    /// Exit the application.
    Exit,
    /// No action.
    None,
}

/// A single item in the start menu grid.
#[derive(Debug, Clone)]
pub struct StartMenuItem {
    pub label: String,
    pub action: StartMenuAction,
    pub color: Color,
}

/// Runtime state for the start menu.
#[derive(Debug)]
pub struct StartMenuState {
    /// Whether the menu popup is currently visible.
    pub open: bool,
    /// Menu items displayed in the popup.
    pub items: Vec<StartMenuItem>,
    /// Currently selected item index.
    pub selected: usize,
    /// Number of columns (always 2).
    cols: usize,
    /// Computed menu panel height.
    menu_h: u32,
    /// Computed menu panel Y position.
    menu_y: i32,
}

impl StartMenuState {
    /// Create a new start menu with the given items.
    pub fn new(items: Vec<StartMenuItem>) -> Self {
        let rows = items.len().div_ceil(MENU_COLS);
        let menu_h = (PAD_TOP + rows as i32 * ITEM_ROW_H + PAD_BOTTOM) as u32;
        let menu_y = theme::BOTTOMBAR_Y - menu_h as i32 - 2;
        Self {
            open: false,
            items,
            selected: 0,
            cols: MENU_COLS,
            menu_h,
            menu_y,
        }
    }

    /// Default set of start menu items.
    pub fn default_items() -> Vec<StartMenuItem> {
        vec![
            StartMenuItem {
                label: "Games".to_string(),
                action: StartMenuAction::LaunchApp("File Manager".to_string()),
                color: Color::rgb(70, 130, 180),
            },
            StartMenuItem {
                label: "Music".to_string(),
                action: StartMenuAction::LaunchApp("Music Player".to_string()),
                color: Color::rgb(60, 179, 113),
            },
            StartMenuItem {
                label: "Video".to_string(),
                action: StartMenuAction::LaunchApp("Photo Viewer".to_string()),
                color: Color::rgb(218, 165, 32),
            },
            StartMenuItem {
                label: "Photos".to_string(),
                action: StartMenuAction::LaunchApp("Photo Viewer".to_string()),
                color: Color::rgb(186, 85, 211),
            },
            StartMenuItem {
                label: "Settings".to_string(),
                action: StartMenuAction::LaunchApp("Settings".to_string()),
                color: Color::rgb(100, 149, 237),
            },
            StartMenuItem {
                label: "Exit".to_string(),
                action: StartMenuAction::Exit,
                color: Color::rgb(205, 92, 92),
            },
        ]
    }

    /// Toggle the menu open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.selected = 0;
        }
    }

    /// Close the menu.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Handle D-pad / Confirm / Cancel input when menu is open.
    ///
    /// Returns an action if an item was activated, or `None`.
    pub fn handle_input(&mut self, button: &Button) -> StartMenuAction {
        if !self.open {
            return StartMenuAction::None;
        }
        let rows = self.items.len().div_ceil(self.cols);
        let row = self.selected / self.cols;
        let col = self.selected % self.cols;

        match button {
            Button::Up => {
                if row > 0 {
                    self.selected -= self.cols;
                }
            },
            Button::Down => {
                let new_idx = self.selected + self.cols;
                if new_idx < self.items.len() {
                    self.selected = new_idx;
                }
            },
            Button::Left => {
                if col > 0 {
                    self.selected -= 1;
                }
            },
            Button::Right => {
                if col + 1 < self.cols && self.selected + 1 < self.items.len() {
                    self.selected += 1;
                }
            },
            Button::Confirm => {
                let action = self.items[self.selected].action.clone();
                self.close();
                return action;
            },
            Button::Cancel => {
                self.close();
            },
            _ => {},
        }
        let _ = rows; // suppress unused warning
        StartMenuAction::None
    }

    /// Test whether a pointer click hits the start button.
    pub fn hit_test_button(&self, x: i32, y: i32) -> bool {
        x >= BTN_X && x < BTN_X + BTN_W as i32 && y >= BTN_Y && y < BTN_Y + BTN_H as i32
    }

    /// Test whether a pointer click hits a menu item. Returns the action if so.
    pub fn hit_test_item(&self, x: i32, y: i32) -> Option<StartMenuAction> {
        if !self.open {
            return None;
        }
        // Check if within menu panel.
        if x < MENU_X
            || x >= MENU_X + MENU_W as i32
            || y < self.menu_y
            || y >= self.menu_y + self.menu_h as i32
        {
            return None;
        }
        // Determine which item was clicked.
        let rel_y = y - self.menu_y - PAD_TOP;
        let rel_x = x - MENU_X - PAD_LEFT;
        if rel_y < 0 || rel_x < 0 {
            return None;
        }
        let row = rel_y / ITEM_ROW_H;
        let col = rel_x / COL_W;
        let idx = row as usize * self.cols + col as usize;
        if idx < self.items.len() {
            Some(self.items[idx].action.clone())
        } else {
            None
        }
    }

    /// Test whether a click is inside the open menu panel (for consuming clicks).
    pub fn hit_test_panel(&self, x: i32, y: i32) -> bool {
        self.open
            && x >= MENU_X
            && x < MENU_X + MENU_W as i32
            && y >= self.menu_y
            && y < self.menu_y + self.menu_h as i32
    }

    /// Update SDI objects for the start button and (when open) the popup.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // -- Start button (always visible) --
        self.update_button_sdi(sdi, at);

        // -- Menu popup (only when open) --
        if self.open {
            self.update_menu_sdi(sdi, at);
        } else {
            self.hide_menu_sdi(sdi);
        }
    }

    /// Hide all start menu SDI objects (button + popup).
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_objects(sdi, &["start_btn_bg", "start_btn_text"]);
        self.hide_menu_sdi(sdi);
    }

    // -- Private SDI helpers --------------------------------------------------

    fn update_button_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Button background pill.
        if !sdi.contains("start_btn_bg") {
            let obj = sdi.create("start_btn_bg");
            obj.overlay = true;
            obj.z = Z_BUTTON;
        }
        if let Ok(obj) = sdi.get_mut("start_btn_bg") {
            obj.x = BTN_X;
            obj.y = BTN_Y;
            obj.w = BTN_W;
            obj.h = BTN_H;
            obj.color = at.sm_button_bg;
            obj.visible = true;
            obj.border_radius = Some(BTN_H as u16 / 2);
        }

        // Button text.
        if !sdi.contains("start_btn_text") {
            let obj = sdi.create("start_btn_text");
            obj.overlay = true;
            obj.z = Z_BUTTON + 1;
        }
        if let Ok(obj) = sdi.get_mut("start_btn_text") {
            obj.x = BTN_X + 6;
            obj.y = BTN_Y + 5;
            obj.font_size = theme::FONT_SMALL;
            obj.text = Some("START".to_string());
            obj.text_color = at.sm_button_text;
            obj.visible = true;
        }
    }

    fn update_menu_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Panel background.
        if !sdi.contains("sm_bg") {
            let obj = sdi.create("sm_bg");
            obj.overlay = true;
            obj.z = Z_MENU;
        }
        if let Ok(obj) = sdi.get_mut("sm_bg") {
            obj.x = MENU_X;
            obj.y = self.menu_y;
            obj.w = MENU_W;
            obj.h = self.menu_h;
            obj.color = at.sm_panel_bg;
            obj.visible = true;
            obj.border_radius = Some(at.sm_panel_border_radius);
            obj.shadow_level = Some(at.sm_panel_shadow_level);
            obj.gradient_top = at.sm_panel_gradient_top;
            obj.gradient_bottom = at.sm_panel_gradient_bottom;
        }

        // Panel border.
        if !sdi.contains("sm_border") {
            let obj = sdi.create("sm_border");
            obj.overlay = true;
            obj.z = Z_MENU + 1;
        }
        if let Ok(obj) = sdi.get_mut("sm_border") {
            obj.x = MENU_X;
            obj.y = self.menu_y;
            obj.w = MENU_W;
            obj.h = self.menu_h;
            obj.color = Color::rgba(0, 0, 0, 0); // transparent fill
            obj.visible = true;
            obj.border_radius = Some(at.sm_panel_border_radius);
            obj.stroke_width = Some(1);
            obj.stroke_color = Some(at.sm_panel_border);
        }

        // Selection highlight.
        let sel_row = self.selected / self.cols;
        let sel_col = self.selected % self.cols;
        let hl_x = MENU_X + PAD_LEFT + sel_col as i32 * COL_W;
        let hl_y = self.menu_y + PAD_TOP + sel_row as i32 * ITEM_ROW_H;
        ensure_rounded_fill(
            sdi,
            "sm_highlight",
            hl_x,
            hl_y,
            COL_W as u32 - 2,
            ITEM_ROW_H as u32 - 2,
            at.sm_highlight_color,
            at.sm_panel_border_radius,
        );
        if let Ok(obj) = sdi.get_mut("sm_highlight") {
            obj.z = Z_MENU + 2;
        }

        // Items: icon placeholder + label.
        for (i, item) in self.items.iter().enumerate().take(MAX_ITEMS) {
            let row = i / self.cols;
            let col = i % self.cols;
            let ix = MENU_X + PAD_LEFT + col as i32 * COL_W + 2;
            let iy = self.menu_y
                + PAD_TOP
                + row as i32 * ITEM_ROW_H
                + (ITEM_ROW_H - ICON_SIZE as i32) / 2;

            // Icon placeholder (colored square).
            let icon_name = format!("sm_item_icon_{i}");
            ensure_rounded_fill(sdi, &icon_name, ix, iy, ICON_SIZE, ICON_SIZE, item.color, 2);
            if let Ok(obj) = sdi.get_mut(&icon_name) {
                obj.z = Z_MENU + 3;
            }

            // Text label.
            let label_name = format!("sm_item_label_{i}");
            let text_color = if i == self.selected {
                at.sm_item_text_active
            } else {
                at.sm_item_text
            };
            ensure_text(
                sdi,
                &label_name,
                ix + ICON_SIZE as i32 + 4,
                iy + 3,
                theme::FONT_SMALL,
                text_color,
            );
            if let Ok(obj) = sdi.get_mut(&label_name) {
                obj.text = Some(item.label.clone());
                obj.text_color = text_color;
                obj.z = Z_MENU + 3;
            }
        }

        // Hide unused item slots.
        for i in self.items.len()..MAX_ITEMS {
            for prefix in &["sm_item_icon_", "sm_item_label_"] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }

    fn hide_menu_sdi(&self, sdi: &mut SdiRegistry) {
        hide_objects(sdi, &["sm_bg", "sm_border", "sm_highlight"]);
        for i in 0..MAX_ITEMS {
            for prefix in &["sm_item_icon_", "sm_item_label_"] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_items_count() {
        let items = StartMenuState::default_items();
        assert_eq!(items.len(), 6);
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        assert!(!sm.open);
        sm.toggle();
        assert!(sm.open);
        assert_eq!(sm.selected, 0);
        sm.toggle();
        assert!(!sm.open);
    }

    #[test]
    fn dpad_navigation_2col() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        sm.selected = 0;

        // Right moves to col 1.
        sm.handle_input(&Button::Right);
        assert_eq!(sm.selected, 1);

        // Down moves to next row.
        sm.handle_input(&Button::Down);
        assert_eq!(sm.selected, 3);

        // Left moves back to col 0.
        sm.handle_input(&Button::Left);
        assert_eq!(sm.selected, 2);

        // Up moves to previous row.
        sm.handle_input(&Button::Up);
        assert_eq!(sm.selected, 0);
    }

    #[test]
    fn confirm_returns_action_and_closes() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        sm.selected = 5; // Exit item
        let action = sm.handle_input(&Button::Confirm);
        assert_eq!(action, StartMenuAction::Exit);
        assert!(!sm.open);
    }

    #[test]
    fn cancel_closes_menu() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        let action = sm.handle_input(&Button::Cancel);
        assert_eq!(action, StartMenuAction::None);
        assert!(!sm.open);
    }

    #[test]
    fn hit_test_button() {
        let sm = StartMenuState::new(StartMenuState::default_items());
        assert!(sm.hit_test_button(BTN_X + 1, BTN_Y + 1));
        assert!(!sm.hit_test_button(300, 100));
    }

    #[test]
    fn hit_test_item_when_closed() {
        let sm = StartMenuState::new(StartMenuState::default_items());
        assert!(sm.hit_test_item(MENU_X + 10, sm.menu_y + 10).is_none());
    }

    #[test]
    fn hit_test_item_when_open() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        // Click on first item area.
        let y = sm.menu_y + PAD_TOP + 2;
        let x = MENU_X + PAD_LEFT + 2;
        let action = sm.hit_test_item(x, y);
        assert!(action.is_some());
    }

    #[test]
    fn update_sdi_creates_button_objects() {
        let sm = StartMenuState::new(StartMenuState::default_items());
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        sm.update_sdi(&mut sdi, &at);
        assert!(sdi.contains("start_btn_bg"));
        assert!(sdi.contains("start_btn_text"));
        // Menu should be hidden.
        assert!(!sdi.contains("sm_bg") || !sdi.get("sm_bg").unwrap().visible);
    }

    #[test]
    fn update_sdi_shows_menu_when_open() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        sm.update_sdi(&mut sdi, &at);
        assert!(sdi.get("sm_bg").unwrap().visible);
        assert!(sdi.get("sm_border").unwrap().visible);
        assert!(sdi.get("sm_highlight").unwrap().visible);
        assert!(sdi.contains("sm_item_icon_0"));
        assert!(sdi.contains("sm_item_label_0"));
    }

    #[test]
    fn navigation_clamps_at_boundaries() {
        let mut sm = StartMenuState::new(StartMenuState::default_items());
        sm.open = true;
        sm.selected = 0;
        // Up at row 0 should stay.
        sm.handle_input(&Button::Up);
        assert_eq!(sm.selected, 0);
        // Left at col 0 should stay.
        sm.handle_input(&Button::Left);
        assert_eq!(sm.selected, 0);
    }

    #[test]
    fn menu_geometry() {
        let sm = StartMenuState::new(StartMenuState::default_items());
        // 6 items in 2 cols = 3 rows.
        let expected_h = (PAD_TOP + 3 * ITEM_ROW_H + PAD_BOTTOM) as u32;
        assert_eq!(sm.menu_h, expected_h);
        assert!(sm.menu_y < theme::BOTTOMBAR_Y);
    }
}

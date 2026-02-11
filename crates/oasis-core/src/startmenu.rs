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

/// Start button X position on the bottom bar.
const BTN_X: i32 = 4;
/// Menu panel X position.
const MENU_X: i32 = 2;
/// Padding inside the menu panel.
const PAD_TOP: i32 = 8;
const PAD_BOTTOM: i32 = 8;
const PAD_LEFT: i32 = 8;

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
    /// Snapshot of the active theme for layout calculations.
    at: ActiveTheme,
    /// Computed menu panel height.
    menu_h: u32,
    /// Computed menu panel Y position.
    menu_y: i32,
    /// Y position of the start button.
    btn_y: i32,
    /// Header height (0 if no header).
    header_h: u32,
    /// Footer height (0 if no footer).
    footer_h: u32,
}

impl StartMenuState {
    /// Create a new start menu with the given items (uses default theme).
    pub fn new(items: Vec<StartMenuItem>) -> Self {
        Self::new_with_theme(items, &ActiveTheme::default())
    }

    /// Create a new start menu with geometry derived from the active theme.
    pub fn new_with_theme(items: Vec<StartMenuItem>, at: &ActiveTheme) -> Self {
        let cols = at.sm_columns.max(1);
        let rows = items.len().div_ceil(cols);
        let header_h = if at.sm_header_text.is_some() && at.sm_header_height > 0 {
            at.sm_header_height
        } else {
            0
        };
        let footer_h = if at.sm_footer_enabled && at.sm_footer_height > 0 {
            at.sm_footer_height
        } else {
            0
        };
        let menu_h = header_h
            + (PAD_TOP + rows as i32 * at.sm_item_row_height + PAD_BOTTOM) as u32
            + footer_h;
        let bar_y = (theme::SCREEN_H - at.bottombar_height) as i32;
        let btn_y = bar_y + 3;
        let menu_y = bar_y - menu_h as i32 - 2;
        Self {
            open: false,
            items,
            selected: 0,
            at: at.clone(),
            menu_h,
            menu_y,
            btn_y,
            header_h,
            footer_h,
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
        let cols = self.at.sm_columns.max(1);
        let row = self.selected / cols;
        let col = self.selected % cols;

        match button {
            Button::Up => {
                if row > 0 {
                    self.selected -= cols;
                }
            },
            Button::Down => {
                let new_idx = self.selected + cols;
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
                if col + 1 < cols && self.selected + 1 < self.items.len() {
                    self.selected += 1;
                }
            },
            Button::Confirm => {
                if let Some(item) = self.items.get(self.selected) {
                    let action = item.action.clone();
                    self.close();
                    return action;
                }
            },
            Button::Cancel => {
                self.close();
            },
            _ => {},
        }
        StartMenuAction::None
    }

    /// Test whether a pointer click hits the start button.
    pub fn hit_test_button(&self, x: i32, y: i32) -> bool {
        let btn_w = self.at.sm_button_width;
        let btn_h = self.at.sm_button_height;
        x >= BTN_X && x < BTN_X + btn_w as i32 && y >= self.btn_y && y < self.btn_y + btn_h as i32
    }

    /// Test whether a pointer click hits a menu item. Returns the action if so.
    pub fn hit_test_item(&self, x: i32, y: i32) -> Option<StartMenuAction> {
        if !self.open {
            return None;
        }
        let menu_w = self.at.sm_panel_width;
        // Check if within menu panel.
        if x < MENU_X
            || x >= MENU_X + menu_w as i32
            || y < self.menu_y
            || y >= self.menu_y + self.menu_h as i32
        {
            return None;
        }
        // Items start after header.
        let items_top = self.menu_y + self.header_h as i32 + PAD_TOP;
        let rel_y = y - items_top;
        let rel_x = x - MENU_X - PAD_LEFT;
        if rel_y < 0 || rel_x < 0 {
            return None;
        }
        let cols = self.at.sm_columns.max(1);
        let col_w = (menu_w as i32 - PAD_LEFT * 2) / cols as i32;
        if col_w <= 0 || self.at.sm_item_row_height <= 0 {
            return None;
        }
        let row = rel_y / self.at.sm_item_row_height;
        let col = rel_x / col_w;
        let idx = row as usize * cols + col as usize;
        if idx < self.items.len() {
            Some(self.items[idx].action.clone())
        } else {
            None
        }
    }

    /// Test whether a click is inside the open menu panel (for consuming clicks).
    pub fn hit_test_panel(&self, x: i32, y: i32) -> bool {
        let menu_w = self.at.sm_panel_width;
        self.open
            && x >= MENU_X
            && x < MENU_X + menu_w as i32
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
        let btn_w = at.sm_button_width;
        let btn_h = at.sm_button_height;
        let radius = if at.sm_button_shape == "rect" {
            Some(2u16)
        } else {
            Some(btn_h as u16 / 2)
        };

        // Button background.
        if !sdi.contains("start_btn_bg") {
            let obj = sdi.create("start_btn_bg");
            obj.overlay = true;
            obj.z = Z_BUTTON;
        }
        if let Ok(obj) = sdi.get_mut("start_btn_bg") {
            obj.x = BTN_X;
            obj.y = self.btn_y;
            obj.w = btn_w;
            obj.h = btn_h;
            obj.color = at.sm_button_bg;
            obj.visible = true;
            obj.border_radius = radius;
            obj.gradient_top = at.sm_button_gradient_top;
            obj.gradient_bottom = at.sm_button_gradient_bottom;
        }

        // Button text.
        if !sdi.contains("start_btn_text") {
            let obj = sdi.create("start_btn_text");
            obj.overlay = true;
            obj.z = Z_BUTTON + 1;
        }
        if let Ok(obj) = sdi.get_mut("start_btn_text") {
            let char_w = at.font_small.max(8) as i32 / 8 * 8;
            let text_w = at.sm_button_label.len() as i32 * char_w;
            obj.x = BTN_X + (btn_w as i32 - text_w) / 2;
            obj.y = self.btn_y + (btn_h as i32 - at.font_small as i32) / 2;
            obj.font_size = at.font_small;
            obj.text = Some(at.sm_button_label.clone());
            obj.text_color = at.sm_button_text;
            obj.visible = true;
        }
    }

    fn update_menu_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let menu_w = at.sm_panel_width;
        let cols = at.sm_columns.max(1);
        let col_w = ((menu_w as i32 - PAD_LEFT * 2) / cols as i32).max(1);
        let item_row_h = at.sm_item_row_height.max(1);
        let icon_size = at.sm_item_icon_size;
        let items_top = self.menu_y + self.header_h as i32;

        // Panel background.
        if !sdi.contains("sm_bg") {
            let obj = sdi.create("sm_bg");
            obj.overlay = true;
            obj.z = Z_MENU;
        }
        if let Ok(obj) = sdi.get_mut("sm_bg") {
            obj.x = MENU_X;
            obj.y = self.menu_y;
            obj.w = menu_w;
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
            obj.w = menu_w;
            obj.h = self.menu_h;
            obj.color = Color::rgba(0, 0, 0, 0); // transparent fill
            obj.visible = true;
            obj.border_radius = Some(at.sm_panel_border_radius);
            obj.stroke_width = Some(1);
            obj.stroke_color = Some(at.sm_panel_border);
        }

        // Header (if configured).
        if let Some(ref header_text) = at.sm_header_text
            && self.header_h > 0
        {
            if !sdi.contains("sm_header_bg") {
                let obj = sdi.create("sm_header_bg");
                obj.overlay = true;
                obj.z = Z_MENU + 1;
            }
            if let Ok(obj) = sdi.get_mut("sm_header_bg") {
                obj.x = MENU_X;
                obj.y = self.menu_y;
                obj.w = menu_w;
                obj.h = self.header_h;
                obj.color = at.sm_header_bg;
                obj.visible = true;
                // No border_radius: only the panel bg rounds the corners.
                obj.border_radius = None;
            }
            if !sdi.contains("sm_header_text") {
                let obj = sdi.create("sm_header_text");
                obj.overlay = true;
                obj.z = Z_MENU + 2;
            }
            if let Ok(obj) = sdi.get_mut("sm_header_text") {
                obj.x = MENU_X + PAD_LEFT;
                obj.y = self.menu_y + (self.header_h as i32 - at.font_small as i32) / 2;
                obj.font_size = at.font_small;
                obj.text = Some(header_text.clone());
                obj.text_color = at.sm_header_text_color;
                obj.visible = true;
            }
        }

        // Selection highlight.
        let sel_row = self.selected / cols;
        let sel_col = self.selected % cols;
        let hl_x = MENU_X + PAD_LEFT + sel_col as i32 * col_w;
        let hl_y = items_top + PAD_TOP + sel_row as i32 * item_row_h;
        ensure_rounded_fill(
            sdi,
            "sm_highlight",
            hl_x,
            hl_y,
            (col_w as u32).saturating_sub(2),
            (item_row_h as u32).saturating_sub(2),
            at.sm_highlight_color,
            at.sm_panel_border_radius,
        );
        if let Ok(obj) = sdi.get_mut("sm_highlight") {
            obj.z = Z_MENU + 2;
        }

        // Items: icon placeholder + label.
        for (i, item) in self.items.iter().enumerate().take(MAX_ITEMS) {
            let row = i / cols;
            let col = i % cols;
            let ix = MENU_X + PAD_LEFT + col as i32 * col_w + 2;
            let iy =
                items_top + PAD_TOP + row as i32 * item_row_h + (item_row_h - icon_size as i32) / 2;

            // Icon placeholder (colored square).
            let icon_name = format!("sm_item_icon_{i}");
            ensure_rounded_fill(sdi, &icon_name, ix, iy, icon_size, icon_size, item.color, 2);
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
                ix + icon_size as i32 + 4,
                iy + (icon_size as i32 - at.font_small as i32) / 2,
                at.font_small,
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

        // Footer (if configured).
        if at.sm_footer_enabled && self.footer_h > 0 {
            if !sdi.contains("sm_footer_bg") {
                let obj = sdi.create("sm_footer_bg");
                obj.overlay = true;
                obj.z = Z_MENU + 1;
            }
            if let Ok(obj) = sdi.get_mut("sm_footer_bg") {
                obj.x = MENU_X;
                obj.y = self.menu_y + self.menu_h as i32 - self.footer_h as i32;
                obj.w = menu_w;
                obj.h = self.footer_h;
                obj.color = at.sm_footer_bg;
                obj.visible = true;
                // No border_radius: only the panel bg rounds the corners.
                obj.border_radius = None;
            }
            if !sdi.contains("sm_footer_text") {
                let obj = sdi.create("sm_footer_text");
                obj.overlay = true;
                obj.z = Z_MENU + 2;
            }
            if let Ok(obj) = sdi.get_mut("sm_footer_text") {
                obj.x = MENU_X + PAD_LEFT;
                obj.y = self.menu_y + self.menu_h as i32 - self.footer_h as i32
                    + (self.footer_h as i32 - at.font_small as i32) / 2;
                obj.font_size = at.font_small;
                obj.text = Some("Log Off  Shut Down".to_string());
                obj.text_color = at.sm_footer_text_color;
                obj.visible = true;
            }
        }
    }

    fn hide_menu_sdi(&self, sdi: &mut SdiRegistry) {
        hide_objects(
            sdi,
            &[
                "sm_bg",
                "sm_border",
                "sm_highlight",
                "sm_header_bg",
                "sm_header_text",
                "sm_footer_bg",
                "sm_footer_text",
            ],
        );
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
        assert!(sm.hit_test_button(BTN_X + 1, sm.btn_y + 1));
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
        // Click on first item area (items start after header).
        let y = sm.menu_y + sm.header_h as i32 + PAD_TOP + 2;
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
        let at = ActiveTheme::default();
        let sm = StartMenuState::new(StartMenuState::default_items());
        // 6 items in 2 cols = 3 rows, default item_row_height = 22.
        let expected_h = (PAD_TOP + 3 * at.sm_item_row_height + PAD_BOTTOM) as u32;
        assert_eq!(sm.menu_h, expected_h);
        let bar_y = (theme::SCREEN_H - at.bottombar_height) as i32;
        assert!(sm.menu_y < bar_y);
    }
}

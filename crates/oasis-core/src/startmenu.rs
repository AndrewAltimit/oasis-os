//! Start menu popup -- a PSIX-style overlay toggled by a bottom-bar button.
//!
//! Displays a 2-column grid of app categories anchored above the bottom bar.
//! The start button is always visible in Dashboard mode; the popup appears
//! when toggled open.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_border, ensure_rounded_fill, ensure_text, hide_objects};
use crate::transition::ease_out_cubic;
use oasis_types::bitmap_font::glyph_advance_scaled;
use oasis_types::color::{darken, with_alpha};

/// Pixel width of `s` rendered at `font_size`.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

// -- Layout constants ---------------------------------------------------------
// BTN_X and MENU_X are now themed via at.menu.button_x / at.menu.panel_x.

/// Z-order for menu objects (above bars at 900, below cursor).
const Z_MENU: i32 = 950;
/// Z-order for the start button (sits on the bottom bar layer).
const Z_BUTTON: i32 = 903;

/// Maximum items supported (for SDI object naming).
const MAX_ITEMS: usize = 12;
/// Maximum separator rows.
const MAX_SEP_ROWS: usize = 6;

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
    /// Currently selected item index (drives Enter / keyboard activation).
    pub selected: usize,
    /// Item index currently under the mouse pointer, if any. Drives the
    /// visual highlight; cleared when the cursor leaves the panel.
    pub hovered: Option<usize>,
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
    /// Animation progress: 0.0 = closed, 1.0 = fully open.
    anim_progress: f32,
    /// Target state for animation.
    anim_target_open: bool,
    /// Smooth selection row position (lerps toward target row).
    visual_row: f32,
    /// Smooth selection column position (lerps toward target column).
    visual_col: f32,
}

impl StartMenuState {
    /// Create a new start menu with the given items (uses default theme).
    pub fn new(items: Vec<StartMenuItem>) -> Self {
        Self::new_with_theme(items, &ActiveTheme::default())
    }

    /// Create a new start menu with geometry derived from the active theme.
    pub fn new_with_theme(items: Vec<StartMenuItem>, at: &ActiveTheme) -> Self {
        let cols = at.menu.columns.max(1);
        let rows = items.len().div_ceil(cols);
        let header_h = if at.menu.header_text.is_some() && at.menu.header_height > 0 {
            at.menu.header_height
        } else {
            0
        };
        let footer_h = if at.menu.footer_enabled && at.menu.footer_height > 0 {
            at.menu.footer_height
        } else {
            0
        };
        let pad = at.menu.pad_inner;
        let menu_h =
            header_h + (pad + rows as i32 * at.menu.item_row_height + pad) as u32 + footer_h;
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let btn_y = bar_y + 3;
        let menu_y = bar_y - menu_h as i32 - 2;
        Self {
            open: false,
            items,
            selected: 0,
            hovered: None,
            at: at.clone(),
            menu_h,
            menu_y,
            btn_y,
            header_h,
            footer_h,
            anim_progress: 0.0,
            anim_target_open: false,
            visual_row: 0.0,
            visual_col: 0.0,
        }
    }

    /// Default set of start menu items with colors derived from the theme.
    pub fn default_items(at: &ActiveTheme) -> Vec<StartMenuItem> {
        let colors = &at.menu.item_colors;
        let color = |idx: usize| -> Color {
            colors
                .get(idx)
                .copied()
                .unwrap_or(at.menu.item_fallback_color)
        };
        vec![
            StartMenuItem {
                label: "Games".to_string(),
                action: StartMenuAction::LaunchApp("File Manager".to_string()),
                color: color(0),
            },
            StartMenuItem {
                label: "Music".to_string(),
                action: StartMenuAction::LaunchApp("Music Player".to_string()),
                color: color(1),
            },
            StartMenuItem {
                label: "Video".to_string(),
                action: StartMenuAction::LaunchApp("Photo Viewer".to_string()),
                color: color(2),
            },
            StartMenuItem {
                label: "Photos".to_string(),
                action: StartMenuAction::LaunchApp("Photo Viewer".to_string()),
                color: color(3),
            },
            StartMenuItem {
                label: "Settings".to_string(),
                action: StartMenuAction::LaunchApp("Settings".to_string()),
                color: color(4),
            },
            StartMenuItem {
                label: "Exit".to_string(),
                action: StartMenuAction::Exit,
                color: color(5),
            },
        ]
    }

    /// Toggle the menu open/closed (with animation).
    pub fn toggle(&mut self) {
        if self.anim_target_open {
            // Start closing.
            self.anim_target_open = false;
            // Keep `open = true` so SDI still renders during close anim.
        } else {
            // Start opening.
            self.anim_target_open = true;
            self.open = true;
            self.selected = 0;
        }
    }

    /// Close the menu (with animation).
    pub fn close(&mut self) {
        self.anim_target_open = false;
        self.hovered = None;
        // Keep `open = true` so SDI still renders during close anim.
    }

    /// Update the hovered item based on cursor position. Sets `hovered` to
    /// the item index under the cursor, or `None` if the cursor is outside
    /// the panel or over empty space. The highlight only renders while
    /// hovered is `Some`.
    pub fn set_hover(&mut self, x: i32, y: i32) {
        self.hovered = self.cell_index_at(x, y);
    }

    /// Compute the item index for a panel-relative cell hit, or `None`.
    fn cell_index_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let menu_w = self.at.menu.panel_width as i32;
        let menu_x = self.at.menu.panel_x;
        let menu_bottom = self.menu_y + self.menu_h as i32;
        if x < menu_x || x >= menu_x + menu_w || y < self.menu_y || y >= menu_bottom {
            return None;
        }
        // Clamp pad to non-negative so a malformed TOML can't shift items_top
        // above the panel and produce spurious hover indices.
        let pad = self.at.menu.pad_inner.max(0);
        let items_top = self.menu_y + self.header_h as i32 + pad;
        let rel_y = y - items_top;
        let rel_x = x - menu_x - pad;
        if rel_y < 0 || rel_x < 0 {
            return None;
        }
        let cols = self.at.menu.columns.max(1);
        let col_w = (menu_w - pad * 2) / cols as i32;
        if col_w <= 0 || self.at.menu.item_row_height <= 0 {
            return None;
        }
        let row = rel_y / self.at.menu.item_row_height;
        let col = rel_x / col_w;
        if col >= cols as i32 {
            return None;
        }
        let idx = row as usize * cols + col as usize;
        if idx < self.items.len() {
            Some(idx)
        } else {
            None
        }
    }

    /// Advance open/close animation by one frame.
    ///
    /// Lerps `anim_progress` toward the target at 0.15/frame (~7 frames).
    /// When close animation completes, sets `open = false`.
    pub fn tick_animation(&mut self) {
        let target = if self.anim_target_open { 1.0 } else { 0.0 };
        let diff = target - self.anim_progress;
        if diff.abs() < 0.01 {
            self.anim_progress = target;
        } else {
            self.anim_progress += diff * self.at.start_menu_anim_speed;
        }
        // When fully closed, mark as not open.
        if !self.anim_target_open && self.anim_progress <= 0.01 {
            self.anim_progress = 0.0;
            self.open = false;
        }
        // Lerp visual row/col toward current selected position.
        let cols = self.at.menu.columns.max(1);
        let target_row = (self.selected / cols) as f32;
        let target_col = (self.selected % cols) as f32;
        let speed = self.at.start_menu_anim_speed;
        self.visual_row += (target_row - self.visual_row) * speed;
        self.visual_col += (target_col - self.visual_col) * speed;
    }

    /// Whether the menu is logically open (including during close animation).
    pub fn is_visible(&self) -> bool {
        self.open || self.anim_progress > 0.01
    }

    /// Handle D-pad / Confirm / Cancel input when menu is open.
    ///
    /// Returns an action if an item was activated, or `None`.
    /// Input is ignored during close animation.
    pub fn handle_input(&mut self, button: &Button) -> StartMenuAction {
        if !self.open || !self.anim_target_open {
            return StartMenuAction::None;
        }
        let cols = self.at.menu.columns.max(1);
        let row = self.selected / cols;
        let col = self.selected % cols;

        match button {
            Button::Up if row > 0 => {
                self.selected -= cols;
            },
            Button::Down => {
                let new_idx = self.selected + cols;
                if new_idx < self.items.len() {
                    self.selected = new_idx;
                }
            },
            Button::Left if col > 0 => {
                self.selected -= 1;
            },
            Button::Right if col + 1 < cols && self.selected + 1 < self.items.len() => {
                self.selected += 1;
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
        let btn_w = self.at.menu.button_width;
        let btn_h = self.at.menu.button_height;
        let btn_x = self.at.menu.button_x;
        x >= btn_x && x < btn_x + btn_w as i32 && y >= self.btn_y && y < self.btn_y + btn_h as i32
    }

    /// Test whether a pointer click hits a menu item. Returns the action if so.
    pub fn hit_test_item(&self, x: i32, y: i32) -> Option<StartMenuAction> {
        if !self.open {
            return None;
        }
        let menu_w = self.at.menu.panel_width;
        let menu_x = self.at.menu.panel_x;
        // Check if within menu panel.
        if x < menu_x
            || x >= menu_x + menu_w as i32
            || y < self.menu_y
            || y >= self.menu_y + self.menu_h as i32
        {
            return None;
        }
        // Items start after header.
        let pad = self.at.menu.pad_inner;
        let items_top = self.menu_y + self.header_h as i32 + pad;
        let rel_y = y - items_top;
        let rel_x = x - menu_x - pad;
        if rel_y < 0 || rel_x < 0 {
            return None;
        }
        let cols = self.at.menu.columns.max(1);
        let col_w = (menu_w as i32 - pad * 2) / cols as i32;
        if col_w <= 0 || self.at.menu.item_row_height <= 0 {
            return None;
        }
        let row = rel_y / self.at.menu.item_row_height;
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
        let menu_w = self.at.menu.panel_width;
        let menu_x = self.at.menu.panel_x;
        self.open
            && x >= menu_x
            && x < menu_x + menu_w as i32
            && y >= self.menu_y
            && y < self.menu_y + self.menu_h as i32
    }

    /// Update SDI objects for the start button and (when open) the popup.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // -- Start button (always visible) --
        self.update_button_sdi(sdi, at);

        // -- Menu popup (visible when open or animating) --
        if self.is_visible() {
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
        let btn_w = at.menu.button_width;
        let btn_h = at.menu.button_height;
        let radius = if at.menu.button_shape == "rect" {
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
        let btn_x = at.menu.button_x;
        // Darken button when menu is open for a "pressed" look.
        let btn_color = if self.anim_target_open {
            darken(at.menu.button_bg, 0.85)
        } else {
            at.menu.button_bg
        };
        if let Ok(obj) = sdi.get_mut("start_btn_bg") {
            obj.x = btn_x;
            obj.y = self.btn_y;
            obj.w = btn_w;
            obj.h = btn_h;
            obj.color = btn_color;
            obj.visible = true;
            obj.border_radius = radius;
            obj.gradient_top = at.menu.button_gradient_top;
            obj.gradient_bottom = at.menu.button_gradient_bottom;
        }

        // Button text.
        if !sdi.contains("start_btn_text") {
            let obj = sdi.create("start_btn_text");
            obj.overlay = true;
            obj.z = Z_BUTTON + 1;
        }
        if let Ok(obj) = sdi.get_mut("start_btn_text") {
            let char_w = at.font_small.max(8) as i32 / 8 * 8;
            let text_w = at.menu.button_label.len() as i32 * char_w;
            obj.x = btn_x + (btn_w as i32 - text_w) / 2;
            obj.y = self.btn_y + (btn_h as i32 - at.font_small as i32) / 2;
            obj.font_size = at.font_small;
            obj.text = Some(at.menu.button_label.clone());
            obj.text_color = at.menu.button_text;
            obj.visible = true;
        }
    }

    fn update_menu_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let menu_w = at.menu.panel_width;
        let menu_x = at.menu.panel_x;
        let cols = at.menu.columns.max(1);
        let pad = at.menu.pad_inner;
        let col_w = ((menu_w as i32 - pad * 2) / cols as i32).max(1);
        let item_row_h = at.menu.item_row_height.max(1);
        let icon_size = at.menu.item_icon_size;

        // Animation: eased progress controls alpha and vertical offset.
        let eased = ease_out_cubic(self.anim_progress);
        let y_offset = ((1.0 - eased) * 10.0) as i32;
        let alpha_scale = eased;
        let scale_alpha = |c: Color| -> Color { with_alpha(c, (c.a as f32 * alpha_scale) as u8) };

        let items_top = self.menu_y + self.header_h as i32 + y_offset;

        // Panel background.
        if !sdi.contains("sm_bg") {
            let obj = sdi.create("sm_bg");
            obj.overlay = true;
            obj.z = Z_MENU;
        }
        if let Ok(obj) = sdi.get_mut("sm_bg") {
            obj.x = menu_x;
            obj.y = self.menu_y + y_offset;
            obj.w = menu_w;
            obj.h = self.menu_h;
            obj.color = scale_alpha(at.menu.panel_bg);
            obj.visible = true;
            obj.border_radius = Some(at.menu.panel_border_radius);
            obj.shadow_level = Some(at.menu.panel_shadow_level);
            obj.gradient_top = at.menu.panel_gradient_top.map(scale_alpha);
            obj.gradient_bottom = at.menu.panel_gradient_bottom.map(scale_alpha);
        }

        // Panel border.
        if !sdi.contains("sm_border") {
            let obj = sdi.create("sm_border");
            obj.overlay = true;
            obj.z = Z_MENU + 1;
        }
        if let Ok(obj) = sdi.get_mut("sm_border") {
            obj.x = menu_x;
            obj.y = self.menu_y + y_offset;
            obj.w = menu_w;
            obj.h = self.menu_h;
            obj.color = Color::rgba(0, 0, 0, 0); // transparent fill
            obj.visible = true;
            obj.border_radius = Some(at.menu.panel_border_radius);
            obj.stroke_width = Some(1);
            obj.stroke_color = Some(scale_alpha(at.menu.panel_border));
        }

        // Header (if configured).
        if let Some(ref header_text) = at.menu.header_text
            && self.header_h > 0
        {
            if !sdi.contains("sm_header_bg") {
                let obj = sdi.create("sm_header_bg");
                obj.overlay = true;
                obj.z = Z_MENU + 1;
            }
            if let Ok(obj) = sdi.get_mut("sm_header_bg") {
                obj.x = menu_x;
                obj.y = self.menu_y + y_offset;
                obj.w = menu_w;
                obj.h = self.header_h;
                obj.color = scale_alpha(at.menu.header_bg);
                obj.visible = true;
                obj.border_radius = None;
            }
            if !sdi.contains("sm_header_text") {
                let obj = sdi.create("sm_header_text");
                obj.overlay = true;
                obj.z = Z_MENU + 2;
            }
            if let Ok(obj) = sdi.get_mut("sm_header_text") {
                obj.x = menu_x + pad;
                obj.y = self.menu_y + y_offset + (self.header_h as i32 - at.font_small as i32) / 2;
                obj.font_size = at.font_small;
                obj.text = Some(header_text.clone());
                obj.text_color = scale_alpha(at.menu.header_text_color);
                obj.visible = true;
            }
        }

        // Layout mode: "tiles" puts the icon on top with a centered label
        // (Activities-overview / Launchpad style). Anything else keeps the
        // legacy icon-left, label-right list layout (Win95 / XP style).
        let is_tiles = at.menu.layout_mode == "tiles";

        // Hover highlight: only render when the cursor is over an item.
        // No selection-driven highlight — opening the menu shows nothing
        // until the user actually points at something.
        if let Some(idx) = self.hovered.filter(|i| *i < self.items.len()) {
            let row = idx / cols;
            let col = idx % cols;
            let hl_inset = if is_tiles { 4 } else { 0 };
            let hl_x = menu_x + pad + col as i32 * col_w + hl_inset;
            let hl_y = items_top + pad + row as i32 * item_row_h + hl_inset;
            let hl_w = (col_w as u32).saturating_sub((hl_inset * 2 + 2) as u32);
            let hl_h = (item_row_h as u32).saturating_sub((hl_inset * 2 + 2) as u32);
            let hl_radius = if is_tiles {
                12
            } else {
                at.menu.panel_border_radius
            };
            ensure_rounded_fill(
                sdi,
                "sm_highlight",
                hl_x,
                hl_y,
                hl_w,
                hl_h,
                scale_alpha(at.menu.highlight_color),
                hl_radius,
            );
            if let Ok(obj) = sdi.get_mut("sm_highlight") {
                obj.z = Z_MENU + 2;
                obj.visible = true;
            }
        } else if let Ok(obj) = sdi.get_mut("sm_highlight") {
            obj.visible = false;
        }

        // Items: icon + label.
        for (i, item) in self.items.iter().enumerate().take(MAX_ITEMS) {
            let row = i / cols;
            let col = i % cols;
            let cell_x = menu_x + pad + col as i32 * col_w;
            let cell_y = items_top + pad + row as i32 * item_row_h;

            let (ix, iy, label_x, label_y) = if is_tiles {
                // Icon centered horizontally, near the top of the cell.
                // Clamp to non-negative so an icon configured larger than the
                // tile cell does not draw left of the cell boundary.
                let ix = cell_x + ((col_w - icon_size as i32) / 2).max(0);
                let icon_top_pad = 10;
                let iy = cell_y + icon_top_pad;
                let tw = text_px(&item.label, at.font_small);
                // Clamp to non-negative so labels wider than the tile don't
                // draw to the left of the cell boundary (potentially into an
                // adjacent tile or off-screen).
                let label_x = cell_x + ((col_w - tw) / 2).max(0);
                let label_y = iy + icon_size as i32 + 6;
                (ix, iy, label_x, label_y)
            } else {
                // Legacy: icon on the left, label on the right.
                let ix = cell_x + 2;
                let iy = cell_y + (item_row_h - icon_size as i32) / 2;
                let label_x = ix + icon_size as i32 + 4;
                let label_y = iy + (icon_size as i32 - at.font_small as i32) / 2;
                (ix, iy, label_x, label_y)
            };

            // Icon: rounded fill. Tiles mode renders flat tiles with a
            // generous corner radius (Adwaita-style); the legacy list
            // layout keeps the small 2px rounding it always had.
            let icon_name = format!("sm_item_icon_{i}");
            let icon_color = scale_alpha(item.color);
            let icon_radius = if is_tiles { 10 } else { 2 };
            ensure_rounded_fill(
                sdi,
                &icon_name,
                ix,
                iy,
                icon_size,
                icon_size,
                icon_color,
                icon_radius,
            );
            if let Ok(obj) = sdi.get_mut(&icon_name) {
                obj.z = Z_MENU + 3;
                // Clear any gradient left from a previous frame so the tile
                // stays a flat single shade.
                obj.gradient_top = None;
                obj.gradient_bottom = None;
            }

            // Text label.
            let label_name = format!("sm_item_label_{i}");
            let text_color = if i == self.selected {
                scale_alpha(at.menu.item_text_active)
            } else {
                scale_alpha(at.menu.item_text)
            };
            ensure_text(
                sdi,
                &label_name,
                label_x,
                label_y,
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

        // Item separators between rows.
        let rows = self.items.len().div_ceil(cols);
        if at.menu.item_separator && rows > 1 {
            for row in 1..rows.min(MAX_SEP_ROWS) {
                let sep_y = items_top + pad + row as i32 * item_row_h;
                ensure_border(
                    sdi,
                    &format!("sm_sep_{row}"),
                    menu_x + pad,
                    sep_y,
                    menu_w - pad as u32 * 2,
                    1,
                    scale_alpha(at.menu.item_separator_color),
                );
                if let Ok(obj) = sdi.get_mut(&format!("sm_sep_{row}")) {
                    obj.z = Z_MENU + 2;
                    obj.overlay = true;
                }
            }
        }
        // Hide unused separators.
        let start_hide = if at.menu.item_separator && rows > 1 {
            rows
        } else {
            1
        };
        for row in start_hide..MAX_SEP_ROWS {
            let name = format!("sm_sep_{row}");
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }

        // Footer (if configured).
        if at.menu.footer_enabled && self.footer_h > 0 {
            if !sdi.contains("sm_footer_bg") {
                let obj = sdi.create("sm_footer_bg");
                obj.overlay = true;
                obj.z = Z_MENU + 1;
            }
            if let Ok(obj) = sdi.get_mut("sm_footer_bg") {
                obj.x = menu_x;
                obj.y = self.menu_y + self.menu_h as i32 - self.footer_h as i32 + y_offset;
                obj.w = menu_w;
                obj.h = self.footer_h;
                obj.color = scale_alpha(at.menu.footer_bg);
                obj.visible = true;
                obj.border_radius = None;
            }
            if !sdi.contains("sm_footer_text") {
                let obj = sdi.create("sm_footer_text");
                obj.overlay = true;
                obj.z = Z_MENU + 2;
            }
            if let Ok(obj) = sdi.get_mut("sm_footer_text") {
                obj.x = menu_x + pad;
                obj.y = self.menu_y + self.menu_h as i32 - self.footer_h as i32
                    + y_offset
                    + (self.footer_h as i32 - at.font_small as i32) / 2;
                obj.font_size = at.font_small;
                obj.text = Some(at.menu.footer_text.clone());
                obj.text_color = scale_alpha(at.menu.footer_text_color);
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
        for row in 1..MAX_SEP_ROWS {
            let name = format!("sm_sep_{row}");
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_items_count() {
        let items = StartMenuState::default_items(&ActiveTheme::default());
        assert_eq!(items.len(), 6);
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        assert!(!sm.open);
        sm.toggle();
        assert!(sm.open);
        assert!(sm.anim_target_open);
        assert_eq!(sm.selected, 0);
        sm.toggle();
        // Menu stays visually open during close animation.
        assert!(sm.open);
        assert!(!sm.anim_target_open);
        // After enough ticks, menu fully closes.
        for _ in 0..50 {
            sm.tick_animation();
        }
        assert!(!sm.open);
    }

    #[test]
    fn dpad_navigation_2col() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
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
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
        sm.selected = 5; // Exit item
        let action = sm.handle_input(&Button::Confirm);
        assert_eq!(action, StartMenuAction::Exit);
        // Close animation started -- menu stays open until animation completes.
        assert!(!sm.anim_target_open);
        for _ in 0..50 {
            sm.tick_animation();
        }
        assert!(!sm.open);
    }

    #[test]
    fn cancel_closes_menu() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
        let action = sm.handle_input(&Button::Cancel);
        assert_eq!(action, StartMenuAction::None);
        assert!(!sm.anim_target_open);
        for _ in 0..50 {
            sm.tick_animation();
        }
        assert!(!sm.open);
    }

    #[test]
    fn hit_test_button() {
        let sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        assert!(sm.hit_test_button(sm.at.menu.button_x + 1, sm.btn_y + 1));
        assert!(!sm.hit_test_button(300, 100));
    }

    #[test]
    fn hit_test_item_when_closed() {
        let sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        assert!(
            sm.hit_test_item(sm.at.menu.panel_x + 10, sm.menu_y + 10)
                .is_none()
        );
    }

    #[test]
    fn hit_test_item_when_open() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        // Click on first item area (items start after header).
        let pad = sm.at.menu.pad_inner;
        let y = sm.menu_y + sm.header_h as i32 + pad + 2;
        let x = sm.at.menu.panel_x + pad + 2;
        let action = sm.hit_test_item(x, y);
        assert!(action.is_some());
    }

    #[test]
    fn update_sdi_creates_button_objects() {
        let sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
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
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
        // Simulate hover so the highlight is rendered.
        sm.hovered = Some(0);
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
    fn highlight_hidden_without_hover() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
        // No hover set: highlight should not be visible.
        let mut sdi = SdiRegistry::new();
        sm.update_sdi(&mut sdi, &ActiveTheme::default());
        // Either not created, or created and hidden.
        if let Ok(obj) = sdi.get("sm_highlight") {
            assert!(!obj.visible);
        }
    }

    #[test]
    fn navigation_clamps_at_boundaries() {
        let mut sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        sm.open = true;
        sm.anim_target_open = true;
        sm.anim_progress = 1.0;
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
        let sm = StartMenuState::new(StartMenuState::default_items(&ActiveTheme::default()));
        // 6 items in 2 cols = 3 rows, default item_row_height = 22.
        let pad = at.menu.pad_inner;
        let expected_h = (pad + 3 * at.menu.item_row_height + pad) as u32;
        assert_eq!(sm.menu_h, expected_h);
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        assert!(sm.menu_y < bar_y);
    }
}

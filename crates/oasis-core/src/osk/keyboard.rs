//! On-screen keyboard state and rendering.

use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;

/// Keyboard input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OskMode {
    /// Lowercase letters.
    Alpha,
    /// Uppercase letters.
    AlphaUpper,
    /// Numbers and symbols.
    NumSymbol,
}

/// Configuration for the on-screen keyboard layout.
#[derive(Debug, Clone)]
pub struct OskConfig {
    /// Grid columns.
    pub cols: usize,
    /// Screen position (top-left).
    pub x: i32,
    pub y: i32,
    /// Cell size in pixels.
    pub cell_w: u32,
    pub cell_h: u32,
    /// Title displayed above the keyboard.
    pub title: String,
}

impl Default for OskConfig {
    fn default() -> Self {
        Self {
            cols: 10,
            x: 20,
            y: 100,
            cell_w: 40,
            cell_h: 32,
            title: "Input".to_string(),
        }
    }
}

/// Character grids for each mode.
const ALPHA_LOWER: &[char] = &[
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', ' ', '.', ',', '!',
];

const ALPHA_UPPER: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', ' ', '.', ',', '!',
];

const NUM_SYMBOL: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '@', '#', '$', '%', '&', '*', '(', ')', '-',
    '_', '=', '+', '[', ']', '{', '}', '/', '\\', ':', ';',
];

/// Runtime state for the software on-screen keyboard.
#[derive(Debug)]
pub struct OskState {
    pub config: OskConfig,
    /// Current keyboard mode.
    pub mode: OskMode,
    /// Cursor position in the character grid.
    pub cursor: usize,
    /// The text buffer being edited.
    pub buffer: String,
    /// Whether the OSK is currently active/visible.
    pub active: bool,
    /// Whether the user confirmed or cancelled (`None` = still editing).
    pub result: Option<bool>,
}

impl OskState {
    /// Create a new OSK with the given config and initial text.
    pub fn new(config: OskConfig, initial: &str) -> Self {
        Self {
            config,
            mode: OskMode::Alpha,
            cursor: 0,
            buffer: initial.to_string(),
            active: true,
            result: None,
        }
    }

    /// Get the character grid for the current mode.
    fn chars(&self) -> &'static [char] {
        match self.mode {
            OskMode::Alpha => ALPHA_LOWER,
            OskMode::AlphaUpper => ALPHA_UPPER,
            OskMode::NumSymbol => NUM_SYMBOL,
        }
    }

    /// Number of rows in the current grid.
    pub fn rows(&self) -> usize {
        self.chars().len().div_ceil(self.config.cols)
    }

    /// Handle a button press. Returns `true` if the OSK consumed the input.
    pub fn handle_input(&mut self, button: &Button) -> bool {
        if !self.active {
            return false;
        }

        let chars = self.chars();
        let cols = self.config.cols;
        let len = chars.len();

        match button {
            Button::Right => {
                self.cursor = (self.cursor + 1) % len;
            },
            Button::Left => {
                if self.cursor == 0 {
                    self.cursor = len - 1;
                } else {
                    self.cursor -= 1;
                }
            },
            Button::Down => {
                let next = self.cursor + cols;
                if next < len {
                    self.cursor = next;
                }
            },
            Button::Up => {
                if self.cursor >= cols {
                    self.cursor -= cols;
                }
            },
            Button::Confirm => {
                // Type the selected character.
                if self.cursor < len {
                    self.buffer.push(chars[self.cursor]);
                }
            },
            Button::Square => {
                // Backspace.
                self.buffer.pop();
            },
            Button::Triangle => {
                // Cycle mode.
                self.mode = match self.mode {
                    OskMode::Alpha => OskMode::AlphaUpper,
                    OskMode::AlphaUpper => OskMode::NumSymbol,
                    OskMode::NumSymbol => OskMode::Alpha,
                };
                // Clamp cursor to new grid size.
                let new_len = self.chars().len();
                if self.cursor >= new_len {
                    self.cursor = new_len - 1;
                }
            },
            Button::Start => {
                // Confirm input.
                self.result = Some(true);
                self.active = false;
            },
            Button::Cancel => {
                // Cancel input.
                self.result = Some(false);
                self.active = false;
            },
            _ => return false,
        }
        true
    }

    /// Get the confirmed text, if the user pressed Start.
    pub fn confirmed_text(&self) -> Option<&str> {
        match self.result {
            Some(true) => Some(&self.buffer),
            _ => None,
        }
    }

    /// Whether the user cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.result == Some(false)
    }

    /// Render the OSK to SDI objects.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry) {
        let chars = self.chars();
        let cols = self.config.cols;
        let rows = self.rows();

        // Background.
        let bg_name = "osk_bg";
        if !sdi.contains(bg_name) {
            sdi.create(bg_name);
        }
        if let Ok(obj) = sdi.get_mut(bg_name) {
            obj.x = self.config.x - 4;
            obj.y = self.config.y - 24;
            obj.w = (cols as u32) * self.config.cell_w + 8;
            obj.h = (rows as u32) * self.config.cell_h + 48;
            obj.color = Color::rgba(20, 20, 40, 220);
            obj.visible = self.active;
        }

        // Title.
        let title_name = "osk_title";
        if !sdi.contains(title_name) {
            sdi.create(title_name);
        }
        if let Ok(obj) = sdi.get_mut(title_name) {
            obj.text = Some(self.config.title.clone());
            obj.x = self.config.x;
            obj.y = self.config.y - 20;
            obj.font_size = 12;
            obj.text_color = Color::WHITE;
            obj.w = 0;
            obj.h = 0;
            obj.visible = self.active;
        }

        // Input buffer display.
        let buf_name = "osk_buffer";
        if !sdi.contains(buf_name) {
            sdi.create(buf_name);
        }
        if let Ok(obj) = sdi.get_mut(buf_name) {
            obj.text = Some(format!("{}|", self.buffer));
            obj.x = self.config.x;
            obj.y = self.config.y + (rows as i32) * self.config.cell_h as i32 + 4;
            obj.font_size = 12;
            obj.text_color = Color::rgb(100, 200, 255);
            obj.w = 0;
            obj.h = 0;
            obj.visible = self.active;
        }

        // Character grid cells.
        for (i, &ch) in chars.iter().enumerate() {
            let name = format!("osk_key_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let col = (i % cols) as i32;
                let row = (i / cols) as i32;
                obj.x = self.config.x + col * self.config.cell_w as i32;
                obj.y = self.config.y + row * self.config.cell_h as i32;
                obj.w = self.config.cell_w - 2;
                obj.h = self.config.cell_h - 2;
                obj.text = Some(ch.to_string());
                obj.font_size = 14;
                obj.text_color = Color::WHITE;
                obj.visible = self.active;

                if i == self.cursor {
                    obj.color = Color::rgb(60, 100, 180);
                } else {
                    obj.color = Color::rgb(40, 40, 60);
                }
            }
        }

        // Mode indicator.
        let mode_name = "osk_mode";
        if !sdi.contains(mode_name) {
            sdi.create(mode_name);
        }
        if let Ok(obj) = sdi.get_mut(mode_name) {
            let mode_text = match self.mode {
                OskMode::Alpha => "abc",
                OskMode::AlphaUpper => "ABC",
                OskMode::NumSymbol => "123",
            };
            obj.text = Some(format!("[{mode_text}] Triangle=mode Start=OK Cancel=back"));
            obj.x = self.config.x;
            obj.y = self.config.y + (rows as i32) * self.config.cell_h as i32 + 20;
            obj.font_size = 10;
            obj.text_color = Color::rgb(150, 150, 180);
            obj.w = 0;
            obj.h = 0;
            obj.visible = self.active;
        }
    }

    /// Hide all OSK-related SDI objects.
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        let osk_names = ["osk_bg", "osk_title", "osk_buffer", "osk_mode"];
        for name in &osk_names {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..self.chars().len() {
            let name = format!("osk_key_{i}");
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
    fn keyboard_default_config() {
        let config = OskConfig::default();
        assert_eq!(config.cols, 10);
        assert_eq!(config.x, 20);
        assert_eq!(config.y, 100);
        assert_eq!(config.cell_w, 40);
        assert_eq!(config.cell_h, 32);
        assert_eq!(config.title, "Input");
    }

    #[test]
    fn keyboard_creation_with_initial_text() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "Hello");
        assert_eq!(osk.buffer, "Hello");
        assert_eq!(osk.mode, OskMode::Alpha);
        assert_eq!(osk.cursor, 0);
        assert!(osk.active);
        assert_eq!(osk.result, None);
    }

    #[test]
    fn keyboard_creation_empty() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        assert_eq!(osk.buffer, "");
        assert!(osk.active);
    }

    #[test]
    fn mode_cycling_alpha_to_upper() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert_eq!(osk.mode, OskMode::Alpha);
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::AlphaUpper);
    }

    #[test]
    fn mode_cycling_upper_to_numsymbol() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::AlphaUpper;
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::NumSymbol);
    }

    #[test]
    fn mode_cycling_numsymbol_to_alpha() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::NumSymbol;
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::Alpha);
    }

    #[test]
    fn mode_cycling_full_cycle() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert_eq!(osk.mode, OskMode::Alpha);
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::AlphaUpper);
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::NumSymbol);
        osk.handle_input(&Button::Triangle);
        assert_eq!(osk.mode, OskMode::Alpha);
    }

    #[test]
    fn cursor_move_right() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert_eq!(osk.cursor, 0);
        osk.handle_input(&Button::Right);
        assert_eq!(osk.cursor, 1);
        osk.handle_input(&Button::Right);
        assert_eq!(osk.cursor, 2);
    }

    #[test]
    fn cursor_move_left() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 5;
        osk.handle_input(&Button::Left);
        assert_eq!(osk.cursor, 4);
        osk.handle_input(&Button::Left);
        assert_eq!(osk.cursor, 3);
    }

    #[test]
    fn cursor_move_right_wraps_around() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        let len = ALPHA_LOWER.len();
        osk.cursor = len - 1;
        osk.handle_input(&Button::Right);
        assert_eq!(osk.cursor, 0);
    }

    #[test]
    fn cursor_move_left_wraps_around() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert_eq!(osk.cursor, 0);
        osk.handle_input(&Button::Left);
        assert_eq!(osk.cursor, ALPHA_LOWER.len() - 1);
    }

    #[test]
    fn cursor_move_down() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert_eq!(osk.cursor, 0);
        osk.handle_input(&Button::Down);
        assert_eq!(osk.cursor, 10); // One row down (cols=10).
    }

    #[test]
    fn cursor_move_down_stays_in_bounds() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        let len = ALPHA_LOWER.len();
        osk.cursor = len - 1; // Last character.
        let prev = osk.cursor;
        osk.handle_input(&Button::Down);
        assert_eq!(osk.cursor, prev); // Should not move.
    }

    #[test]
    fn cursor_move_up() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 15;
        osk.handle_input(&Button::Up);
        assert_eq!(osk.cursor, 5);
    }

    #[test]
    fn cursor_move_up_clamps_at_top() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 5; // First row.
        osk.handle_input(&Button::Up);
        assert_eq!(osk.cursor, 5); // Should not move.
    }

    #[test]
    fn character_selection_adds_to_buffer() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 0; // 'a' in ALPHA_LOWER.
        osk.handle_input(&Button::Confirm);
        assert_eq!(osk.buffer, "a");
    }

    #[test]
    fn character_selection_multiple() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 0; // 'a'
        osk.handle_input(&Button::Confirm);
        osk.cursor = 1; // 'b'
        osk.handle_input(&Button::Confirm);
        osk.cursor = 2; // 'c'
        osk.handle_input(&Button::Confirm);
        assert_eq!(osk.buffer, "abc");
    }

    #[test]
    fn character_selection_uppercase() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::AlphaUpper;
        osk.cursor = 0; // 'A'
        osk.handle_input(&Button::Confirm);
        assert_eq!(osk.buffer, "A");
    }

    #[test]
    fn character_selection_numbers() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::NumSymbol;
        osk.cursor = 0; // '0'
        osk.handle_input(&Button::Confirm);
        assert_eq!(osk.buffer, "0");
    }

    #[test]
    fn backspace_removes_character() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "abc");
        osk.handle_input(&Button::Square);
        assert_eq!(osk.buffer, "ab");
    }

    #[test]
    fn backspace_on_empty_buffer() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.handle_input(&Button::Square);
        assert_eq!(osk.buffer, "");
    }

    #[test]
    fn confirm_input_with_start() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "test");
        osk.handle_input(&Button::Start);
        assert_eq!(osk.result, Some(true));
        assert!(!osk.active);
        assert_eq!(osk.confirmed_text(), Some("test"));
    }

    #[test]
    fn cancel_input() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "test");
        osk.handle_input(&Button::Cancel);
        assert_eq!(osk.result, Some(false));
        assert!(!osk.active);
        assert!(osk.is_cancelled());
        assert_eq!(osk.confirmed_text(), None);
    }

    #[test]
    fn confirmed_text_before_confirmation() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "test");
        assert_eq!(osk.confirmed_text(), None);
    }

    #[test]
    fn is_cancelled_before_input() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        assert!(!osk.is_cancelled());
    }

    #[test]
    fn handle_input_returns_true_when_active() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        assert!(osk.handle_input(&Button::Right));
    }

    #[test]
    fn handle_input_returns_false_when_inactive() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.active = false;
        assert!(!osk.handle_input(&Button::Right));
    }

    #[test]
    fn handle_input_ignores_unrecognized_button() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        let prev_cursor = osk.cursor;
        assert!(!osk.handle_input(&Button::Select));
        assert_eq!(osk.cursor, prev_cursor);
    }

    #[test]
    fn mode_change_clamps_cursor() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 29; // Last position in ALPHA_LOWER (30 chars).
        osk.handle_input(&Button::Triangle); // Switch to AlphaUpper (same size).
        assert_eq!(osk.cursor, 29);
        // If grids were different sizes, cursor would clamp.
    }

    #[test]
    fn rows_calculation() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        // ALPHA_LOWER has 30 chars, cols=10, so rows = 3.
        assert_eq!(osk.rows(), 3);
    }

    #[test]
    fn update_sdi_creates_background() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        assert!(sdi.contains("osk_bg"));
        assert!(sdi.get("osk_bg").unwrap().visible);
    }

    #[test]
    fn update_sdi_creates_title() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        assert!(sdi.contains("osk_title"));
        let obj = sdi.get("osk_title").unwrap();
        assert_eq!(obj.text, Some("Input".to_string()));
    }

    #[test]
    fn update_sdi_creates_buffer_display() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "test");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        assert!(sdi.contains("osk_buffer"));
        let obj = sdi.get("osk_buffer").unwrap();
        assert_eq!(obj.text, Some("test|".to_string()));
    }

    #[test]
    fn update_sdi_creates_character_cells() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        for i in 0..ALPHA_LOWER.len() {
            let name = format!("osk_key_{i}");
            assert!(sdi.contains(&name));
        }
    }

    #[test]
    fn update_sdi_highlights_selected_key() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.cursor = 5;
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        let selected = sdi.get("osk_key_5").unwrap();
        let unselected = sdi.get("osk_key_0").unwrap();
        assert_ne!(selected.color, unselected.color);
    }

    #[test]
    fn update_sdi_shows_mode_indicator() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        assert!(sdi.contains("osk_mode"));
        let obj = sdi.get("osk_mode").unwrap();
        assert!(obj.text.as_ref().unwrap().contains("abc"));
    }

    #[test]
    fn update_sdi_mode_indicator_uppercase() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::AlphaUpper;
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        let obj = sdi.get("osk_mode").unwrap();
        assert!(obj.text.as_ref().unwrap().contains("ABC"));
    }

    #[test]
    fn update_sdi_mode_indicator_numsymbol() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.mode = OskMode::NumSymbol;
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        let obj = sdi.get("osk_mode").unwrap();
        assert!(obj.text.as_ref().unwrap().contains("123"));
    }

    #[test]
    fn hide_sdi_hides_all_objects() {
        let config = OskConfig::default();
        let osk = OskState::new(config, "");
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        osk.hide_sdi(&mut sdi);
        assert!(!sdi.get("osk_bg").unwrap().visible);
        assert!(!sdi.get("osk_title").unwrap().visible);
        assert!(!sdi.get("osk_buffer").unwrap().visible);
        assert!(!sdi.get("osk_mode").unwrap().visible);
        for i in 0..ALPHA_LOWER.len() {
            let name = format!("osk_key_{i}");
            assert!(!sdi.get(&name).unwrap().visible);
        }
    }

    #[test]
    fn update_sdi_when_inactive() {
        let config = OskConfig::default();
        let mut osk = OskState::new(config, "");
        osk.active = false;
        let mut sdi = SdiRegistry::new();
        osk.update_sdi(&mut sdi);
        assert!(!sdi.get("osk_bg").unwrap().visible);
    }
}

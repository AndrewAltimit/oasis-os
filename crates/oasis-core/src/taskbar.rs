//! Desktop taskbar -- shows a button for each open window.
//!
//! Sits directly above the bottom bar in Desktop mode. Each button represents
//! an open window; clicking focuses, minimizes, or restores the window.
//! Buttons resize dynamically to fit all open windows.

use oasis_types::bitmap_font::glyph_advance_scaled;

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_border, ensure_rounded_fill, ensure_text, hide_objects};

/// Measure the pixel width of a text string using proportional glyph metrics.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

/// Truncate `title` so its rendered width fits within `max_px`, appending "..."
/// if truncated.
fn truncate_title(title: &str, font_size: u16, max_px: i32) -> String {
    let full_w = text_px(title, font_size);
    if full_w <= max_px {
        return title.to_string();
    }
    let ellipsis_w = text_px("...", font_size);
    let target = max_px - ellipsis_w;
    if target <= 0 {
        return "...".to_string();
    }
    let mut w = 0i32;
    let mut end = 0;
    for (i, c) in title.char_indices() {
        let cw = glyph_advance_scaled(c, font_size) as i32;
        if w + cw > target {
            break;
        }
        w += cw;
        end = i + c.len_utf8();
    }
    format!("{}...", &title[..end])
}

/// Maximum number of taskbar buttons we create SDI objects for.
const MAX_BUTTONS: usize = 16;

/// Preferred button width in pixels.
const PREFERRED_WIDTH: i32 = 120;

/// Minimum button width in pixels.
const MIN_WIDTH: i32 = 40;

/// Horizontal padding inside each button (text inset from button edge).
const BUTTON_PAD: i32 = 4;

/// Gap between buttons.
const BUTTON_GAP: i32 = 1;

/// Active indicator underline height.
const INDICATOR_H: u32 = 2;

/// A cached taskbar button for hit testing.
#[derive(Debug, Clone)]
struct TaskbarButton {
    window_id: String,
    x: i32,
    width: u32,
}

/// Runtime state for the desktop taskbar.
#[derive(Debug)]
pub struct Taskbar {
    /// Cached button rects from last `update_sdi` call (for hit testing).
    buttons: Vec<TaskbarButton>,
    /// Index of the currently hovered button (for visual feedback).
    hover_index: Option<usize>,
    /// Number of SDI button objects created (for cleanup).
    sdi_count: usize,
}

impl Taskbar {
    /// Create a new empty taskbar.
    pub fn new() -> Self {
        Self {
            buttons: Vec::new(),
            hover_index: None,
            sdi_count: 0,
        }
    }

    /// Compute the Y position of the taskbar's top edge.
    pub fn bar_y(at: &ActiveTheme) -> i32 {
        (at.screen_h - at.bottombar_height - at.taskbar_height) as i32
    }

    /// Synchronize SDI objects to reflect the current window list.
    ///
    /// `windows` should be the WM's window list in z-order. `active_id` is
    /// the currently focused window id (if any).
    pub fn update_sdi(
        &mut self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        windows: &[crate::wm::window::Window],
        active_id: Option<&str>,
    ) {
        if at.taskbar_height == 0 || windows.is_empty() {
            self.hide_sdi(sdi);
            return;
        }

        let bar_y = Self::bar_y(at);
        let bar_h = at.taskbar_height;
        let screen_w = at.screen_w;
        let font = at.font_small;
        let text_y = bar_y + (bar_h as i32 - font as i32) / 2;

        // Background bar.
        if !sdi.contains("taskbar_bg") {
            let obj = sdi.create("taskbar_bg");
            obj.x = 0;
            obj.y = bar_y;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.color = at.taskbar_bg;
            obj.overlay = true;
            obj.z = 895;
        }
        if let Ok(obj) = sdi.get_mut("taskbar_bg") {
            obj.color = at.taskbar_bg;
            obj.y = bar_y;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.visible = true;
            obj.gradient_top = at.taskbar_gradient_top;
            obj.gradient_bottom = at.taskbar_gradient_bottom;
        }

        // Top separator line.
        ensure_border(
            sdi,
            "taskbar_sep",
            0,
            bar_y,
            screen_w,
            1,
            at.taskbar_separator,
        );

        // Compute button layout.
        let count = windows.len().min(MAX_BUTTONS);
        let available = screen_w as i32;
        let total_gaps = if count > 1 {
            (count as i32 - 1) * BUTTON_GAP
        } else {
            0
        };
        let per_button = if count > 0 {
            ((available - total_gaps) / count as i32).clamp(MIN_WIDTH, PREFERRED_WIDTH)
        } else {
            PREFERRED_WIDTH
        };

        // Build button list and create SDI objects.
        self.buttons.clear();
        let mut cx = 0i32;
        for (i, win) in windows.iter().take(MAX_BUTTONS).enumerate() {
            let btn_w = if i == count - 1 {
                // Last button takes remaining space to avoid rounding gaps.
                (available - cx).max(MIN_WIDTH)
            } else {
                per_button
            };

            let is_active = active_id == Some(win.id.as_str());
            let is_minimized = win.state == crate::wm::window::WindowState::Minimized;
            let is_hovered = self.hover_index == Some(i);

            let btn_color = if is_hovered {
                at.taskbar_btn_hover
            } else if is_active {
                at.taskbar_btn_active
            } else if is_minimized {
                at.taskbar_btn_minimized
            } else {
                at.taskbar_btn_inactive
            };

            // Button background.
            let bg_name = format!("taskbar_btn_{i}");
            ensure_rounded_fill(
                sdi,
                &bg_name,
                cx,
                bar_y + 1, // below separator
                btn_w as u32,
                bar_h - 1,
                btn_color,
                0,
            );

            // Button text (truncated to fit).
            let text_name = format!("taskbar_btn_{i}_text");
            let max_text_w = btn_w - BUTTON_PAD * 2;
            let label = truncate_title(&win.title, font, max_text_w);
            ensure_text(
                sdi,
                &text_name,
                cx + BUTTON_PAD,
                text_y,
                font,
                at.taskbar_text_color,
            );
            if let Ok(obj) = sdi.get_mut(&text_name) {
                obj.text = Some(label);
            }

            // Active indicator underline.
            let ind_name = format!("taskbar_btn_{i}_ind");
            if is_active {
                ensure_rounded_fill(
                    sdi,
                    &ind_name,
                    cx + 2,
                    bar_y + bar_h as i32 - INDICATOR_H as i32,
                    (btn_w - 4).max(0) as u32,
                    INDICATOR_H,
                    at.taskbar_indicator,
                    1,
                );
            } else if let Ok(obj) = sdi.get_mut(&ind_name) {
                obj.visible = false;
            }

            self.buttons.push(TaskbarButton {
                window_id: win.id.to_string(),
                x: cx,
                width: btn_w as u32,
            });

            cx += btn_w + BUTTON_GAP;
        }

        // Hide any leftover buttons from a previous frame with more windows.
        for i in count..self.sdi_count {
            for suffix in &["", "_text", "_ind"] {
                let name = format!("taskbar_btn_{i}{suffix}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
        self.sdi_count = count;
    }

    /// Hide all taskbar SDI objects.
    pub fn hide_sdi(&mut self, sdi: &mut SdiRegistry) {
        hide_objects(sdi, &["taskbar_bg", "taskbar_sep"]);
        for i in 0..self.sdi_count {
            for suffix in &["", "_text", "_ind"] {
                let name = format!("taskbar_btn_{i}{suffix}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
        self.buttons.clear();
    }

    /// Hit test: returns the window id if a taskbar button was clicked.
    pub fn hit_test(&self, x: i32, y: i32, at: &ActiveTheme) -> Option<&str> {
        let bar_y = Self::bar_y(at);
        let bar_h = at.taskbar_height as i32;
        if y < bar_y || y >= bar_y + bar_h {
            return None;
        }
        for btn in &self.buttons {
            if x >= btn.x && x < btn.x + btn.width as i32 {
                return Some(&btn.window_id);
            }
        }
        None
    }

    /// Update hover state based on cursor position.
    pub fn set_hover(&mut self, x: i32, y: i32, at: &ActiveTheme) {
        let bar_y = Self::bar_y(at);
        let bar_h = at.taskbar_height as i32;
        if y < bar_y || y >= bar_y + bar_h {
            self.hover_index = None;
            return;
        }
        self.hover_index = self
            .buttons
            .iter()
            .position(|btn| x >= btn.x && x < btn.x + btn.width as i32);
    }
}

impl Default for Taskbar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active_theme::ActiveTheme;
    use crate::sdi::SdiRegistry;
    use crate::wm::window::{Window, WindowConfig, WindowState, WindowType, WmTheme};

    fn make_window(id: &str, title: &str, state: WindowState) -> Window {
        let config = WindowConfig {
            id: id.to_string(),
            title: title.to_string(),
            x: None,
            y: None,
            width: 200,
            height: 150,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        let mut win = Window::new(&config, 10, 10, &WmTheme::default());
        win.state = state;
        win
    }

    #[test]
    fn empty_windows_hides_taskbar() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_sdi(&mut sdi, &at, &[], None);
        assert!(taskbar.buttons.is_empty());
    }

    #[test]
    fn single_window_creates_button() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "File Manager", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"));

        assert_eq!(taskbar.buttons.len(), 1);
        assert_eq!(taskbar.buttons[0].window_id, "app1");
        assert!(sdi.contains("taskbar_bg"));
        assert!(sdi.contains("taskbar_btn_0"));
        assert!(sdi.contains("taskbar_btn_0_text"));
    }

    #[test]
    fn multiple_windows_resize_buttons() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows: Vec<Window> = (0..5)
            .map(|i| make_window(&format!("app{i}"), &format!("App {i}"), WindowState::Normal))
            .collect();
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"));

        assert_eq!(taskbar.buttons.len(), 5);
        // All buttons should fit within screen width.
        let last = &taskbar.buttons[4];
        assert!(last.x + last.width as i32 <= at.screen_w as i32);
    }

    #[test]
    fn hit_test_returns_correct_window() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![
            make_window("app1", "First", WindowState::Normal),
            make_window("app2", "Second", WindowState::Normal),
        ];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"));

        let bar_y = Taskbar::bar_y(&at);
        // Click on first button.
        assert_eq!(taskbar.hit_test(5, bar_y + 5, &at), Some("app1"));
        // Click on second button.
        let btn2_x = taskbar.buttons[1].x + 5;
        assert_eq!(taskbar.hit_test(btn2_x, bar_y + 5, &at), Some("app2"));
    }

    #[test]
    fn hit_test_outside_returns_none() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"));

        // Click above taskbar.
        assert_eq!(taskbar.hit_test(5, 0, &at), None);
        // Click below taskbar.
        assert_eq!(taskbar.hit_test(5, at.screen_h as i32, &at), None);
    }

    #[test]
    fn hide_sdi_clears_buttons() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"));
        assert!(!taskbar.buttons.is_empty());

        taskbar.hide_sdi(&mut sdi);
        assert!(taskbar.buttons.is_empty());
    }

    #[test]
    fn hover_state_updates() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![
            make_window("app1", "First", WindowState::Normal),
            make_window("app2", "Second", WindowState::Normal),
        ];
        taskbar.update_sdi(&mut sdi, &at, &windows, None);

        let bar_y = Taskbar::bar_y(&at);
        taskbar.set_hover(5, bar_y + 5, &at);
        assert_eq!(taskbar.hover_index, Some(0));

        let btn2_x = taskbar.buttons[1].x + 5;
        taskbar.set_hover(btn2_x, bar_y + 5, &at);
        assert_eq!(taskbar.hover_index, Some(1));

        // Move outside.
        taskbar.set_hover(5, 0, &at);
        assert_eq!(taskbar.hover_index, None);
    }

    #[test]
    fn truncate_title_short_unchanged() {
        let result = truncate_title("Hi", 8, 200);
        assert_eq!(result, "Hi");
    }

    #[test]
    fn truncate_title_long_gets_ellipsis() {
        let result = truncate_title("A Very Long Window Title That Won't Fit", 8, 50);
        assert!(result.ends_with("..."));
        assert!(text_px(&result, 8) <= 50);
    }

    #[test]
    fn leftover_buttons_hidden_on_window_close() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();

        // Start with 3 windows.
        let windows: Vec<Window> = (0..3)
            .map(|i| make_window(&format!("app{i}"), &format!("App {i}"), WindowState::Normal))
            .collect();
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"));
        assert_eq!(taskbar.sdi_count, 3);

        // Close one window (now 2).
        let windows: Vec<Window> = (0..2)
            .map(|i| make_window(&format!("app{i}"), &format!("App {i}"), WindowState::Normal))
            .collect();
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"));
        assert_eq!(taskbar.sdi_count, 2);
        // The third button should be hidden.
        assert!(!sdi.get("taskbar_btn_2").unwrap().visible);
    }

    #[test]
    fn zero_taskbar_height_hides() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let mut at = ActiveTheme::default();
        at.taskbar_height = 0;
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"));
        assert!(taskbar.buttons.is_empty());
    }

    #[test]
    fn default_trait() {
        let taskbar = Taskbar::default();
        assert!(taskbar.buttons.is_empty());
        assert_eq!(taskbar.hover_index, None);
    }
}

//! Desktop taskbar -- shows a button for each open window.
//!
//! Renders inline within the bottom bar, to the right of the start button.
//! Each button represents an open window; clicking focuses, minimizes, or
//! restores the window. Buttons resize dynamically to fit all open windows.
//!
//! Also displays a virtual desktop indicator (e.g. "< 1/4 >") in the
//! right portion of the bottom bar when more than one desktop exists.

use oasis_types::bitmap_font::glyph_advance_scaled;

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_rounded_fill, ensure_text, hide_objects};

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

/// Width of the desktop prev/next arrow buttons.
const DESKTOP_ARROW_W: i32 = 14;

/// Horizontal gap between the desktop indicator and the right edge.
const DESKTOP_RIGHT_MARGIN: i32 = 4;

/// Result of a desktop indicator hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopHit {
    /// The "previous desktop" arrow was clicked.
    Prev,
    /// The "next desktop" arrow was clicked.
    Next,
}

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
    /// Cached bar Y position for hit testing.
    bar_y: i32,
    /// Cached bar height for hit testing.
    bar_h: u32,
    /// Cached start X for hit testing.
    start_x: i32,
    /// Stable ordering of window IDs (insertion order, not z-order).
    /// Windows are appended when first seen and removed when closed.
    order: Vec<String>,
    /// Cached X position of the "prev desktop" arrow for hit testing.
    desktop_prev_x: i32,
    /// Cached X position of the "next desktop" arrow for hit testing.
    desktop_next_x: i32,
    /// Whether the desktop indicator is currently visible (>1 desktop).
    desktop_indicator_visible: bool,
}

impl Taskbar {
    /// Create a new empty taskbar.
    pub fn new() -> Self {
        Self {
            buttons: Vec::new(),
            hover_index: None,
            sdi_count: 0,
            bar_y: 0,
            bar_h: 0,
            start_x: 0,
            order: Vec::new(),
            desktop_prev_x: 0,
            desktop_next_x: 0,
            desktop_indicator_visible: false,
        }
    }

    /// Compute the starting X offset for taskbar buttons (after the start
    /// button, or 0 when start menu is disabled).
    pub fn taskbar_start_x(at: &ActiveTheme, has_start_menu: bool) -> i32 {
        if has_start_menu {
            at.menu.button_x + at.menu.button_width as i32 + 4
        } else {
            4
        }
    }

    /// Synchronize SDI objects to reflect the current window list.
    ///
    /// `windows` should be the WM's window list in z-order. `active_id` is
    /// the currently focused window id (if any). `has_start_menu` controls
    /// whether buttons start after the start button.
    pub fn update_sdi(
        &mut self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        windows: &[crate::wm::window::Window],
        active_id: Option<&str>,
        has_start_menu: bool,
    ) {
        if windows.is_empty() {
            self.hide_sdi(sdi);
            self.order.clear();
            return;
        }

        // Maintain stable insertion order: add new windows, remove closed ones.
        let current_ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        // Remove closed windows.
        self.order.retain(|id| current_ids.contains(&id.as_str()));
        // Append newly opened windows (preserving existing order).
        for id in &current_ids {
            if !self.order.iter().any(|o| o == id) {
                self.order.push(id.to_string());
            }
        }

        // Render inline within the bottom bar.
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let bar_h = at.bottombar_height;
        let font = at.font_small;
        let text_y = bar_y + (bar_h as i32 - font as i32) / 2;

        // Cache for hit testing.
        self.bar_y = bar_y;
        self.bar_h = bar_h;

        let start_x = Self::taskbar_start_x(at, has_start_menu);
        self.start_x = start_x;

        // Right edge: leave room for media tabs (roughly right third).
        let max_x = at.screen_w as i32 * 2 / 3;

        // Compute button layout within available region.
        let count = self.order.len().min(MAX_BUTTONS);
        let available = (max_x - start_x).max(MIN_WIDTH);
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

        // Build button list and create SDI objects (stable order).
        self.buttons.clear();
        let mut cx = start_x;
        for (i, win_id) in self.order.iter().take(MAX_BUTTONS).enumerate() {
            let win = match windows.iter().find(|w| w.id == *win_id) {
                Some(w) => w,
                None => continue,
            };
            // All buttons same width; capped at PREFERRED_WIDTH.
            let btn_w = per_button;

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

            // Button background (rendered on top of bottom bar).
            let bg_name = format!("taskbar_btn_{i}");
            ensure_rounded_fill(
                sdi,
                &bg_name,
                cx,
                bar_y + 2,
                btn_w as u32,
                bar_h.saturating_sub(4),
                btn_color,
                2,
            );
            if let Ok(obj) = sdi.get_mut(&bg_name) {
                obj.z = 901;
                obj.overlay = true;
            }

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
                obj.z = 902;
                obj.overlay = true;
            }

            // Active indicator underline.
            let ind_name = format!("taskbar_btn_{i}_ind");
            if is_active {
                ensure_rounded_fill(
                    sdi,
                    &ind_name,
                    cx + 2,
                    bar_y + bar_h as i32 - INDICATOR_H as i32 - 1,
                    (btn_w - 4).max(0) as u32,
                    INDICATOR_H,
                    at.taskbar_indicator,
                    1,
                );
                if let Ok(obj) = sdi.get_mut(&ind_name) {
                    obj.z = 902;
                    obj.overlay = true;
                }
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
        // Legacy cleanup: hide old separate-bar objects if they exist.
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
        self.order.clear();
        self.hide_desktop_indicator(sdi);
    }

    /// Update the virtual desktop indicator SDI objects.
    ///
    /// Shows "< N/M >" in the right side of the bottom bar when more
    /// than one desktop exists. `active` is the 0-based current desktop
    /// index and `count` is the total number of desktops.
    pub fn update_desktop_indicator(
        &mut self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        active: usize,
        count: usize,
    ) {
        if count <= 1 {
            self.hide_desktop_indicator(sdi);
            return;
        }

        self.desktop_indicator_visible = true;

        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let bar_h = at.bottombar_height;

        // Ensure hit test geometry is set even without window buttons.
        self.bar_y = bar_y;
        self.bar_h = bar_h;
        let font = at.font_small;

        let label = format!("{}/{count}", active + 1);
        let label_w = text_px(&label, font);

        // Total width: [<] [label] [>] with small gaps.
        let total_w = DESKTOP_ARROW_W + 2 + label_w + 2 + DESKTOP_ARROW_W;
        let right_edge = at.screen_w as i32 - DESKTOP_RIGHT_MARGIN;
        let base_x = right_edge - total_w;

        let text_y = bar_y + (bar_h as i32 - font as i32) / 2;

        // Prev arrow button "<".
        let prev_x = base_x;
        self.desktop_prev_x = prev_x;
        ensure_rounded_fill(
            sdi,
            "desktop_ind_prev",
            prev_x,
            bar_y + 2,
            DESKTOP_ARROW_W as u32,
            bar_h.saturating_sub(4),
            at.taskbar_btn_inactive,
            2,
        );
        if let Ok(obj) = sdi.get_mut("desktop_ind_prev") {
            obj.z = 901;
            obj.overlay = true;
        }
        ensure_text(
            sdi,
            "desktop_ind_prev_text",
            prev_x + 3,
            text_y,
            font,
            at.taskbar_text_color,
        );
        if let Ok(obj) = sdi.get_mut("desktop_ind_prev_text") {
            obj.text = Some("<".to_string());
            obj.z = 902;
            obj.overlay = true;
        }

        // Label "N/M".
        let label_x = prev_x + DESKTOP_ARROW_W + 2;
        ensure_text(
            sdi,
            "desktop_ind_label",
            label_x,
            text_y,
            font,
            at.taskbar_text_color,
        );
        if let Ok(obj) = sdi.get_mut("desktop_ind_label") {
            obj.text = Some(label);
            obj.z = 902;
            obj.overlay = true;
        }

        // Next arrow button ">".
        let next_x = label_x + label_w + 2;
        self.desktop_next_x = next_x;
        ensure_rounded_fill(
            sdi,
            "desktop_ind_next",
            next_x,
            bar_y + 2,
            DESKTOP_ARROW_W as u32,
            bar_h.saturating_sub(4),
            at.taskbar_btn_inactive,
            2,
        );
        if let Ok(obj) = sdi.get_mut("desktop_ind_next") {
            obj.z = 901;
            obj.overlay = true;
        }
        ensure_text(
            sdi,
            "desktop_ind_next_text",
            next_x + 3,
            text_y,
            font,
            at.taskbar_text_color,
        );
        if let Ok(obj) = sdi.get_mut("desktop_ind_next_text") {
            obj.text = Some(">".to_string());
            obj.z = 902;
            obj.overlay = true;
        }
    }

    /// Hide the desktop indicator SDI objects.
    fn hide_desktop_indicator(&mut self, sdi: &mut SdiRegistry) {
        self.desktop_indicator_visible = false;
        hide_objects(
            sdi,
            &[
                "desktop_ind_prev",
                "desktop_ind_prev_text",
                "desktop_ind_label",
                "desktop_ind_next",
                "desktop_ind_next_text",
            ],
        );
    }

    /// Hit test the desktop indicator arrows.
    ///
    /// Returns `Some(DesktopHit::Prev)` or `Some(DesktopHit::Next)` if a
    /// desktop arrow was clicked, `None` otherwise.
    pub fn desktop_hit_test(&self, x: i32, y: i32) -> Option<DesktopHit> {
        if !self.desktop_indicator_visible {
            return None;
        }
        if y < self.bar_y || y >= self.bar_y + self.bar_h as i32 {
            return None;
        }
        if x >= self.desktop_prev_x && x < self.desktop_prev_x + DESKTOP_ARROW_W {
            return Some(DesktopHit::Prev);
        }
        if x >= self.desktop_next_x && x < self.desktop_next_x + DESKTOP_ARROW_W {
            return Some(DesktopHit::Next);
        }
        None
    }

    /// Hit test: returns the window id if a taskbar button was clicked.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<&str> {
        if self.buttons.is_empty() {
            return None;
        }
        if y < self.bar_y || y >= self.bar_y + self.bar_h as i32 {
            return None;
        }
        // Only hit taskbar buttons in their X range.
        if x < self.start_x {
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
    pub fn set_hover(&mut self, x: i32, y: i32) {
        if self.buttons.is_empty()
            || y < self.bar_y
            || y >= self.bar_y + self.bar_h as i32
            || x < self.start_x
        {
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
        taskbar.update_sdi(&mut sdi, &at, &[], None, false);
        assert!(taskbar.buttons.is_empty());
    }

    #[test]
    fn single_window_creates_button() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "File Manager", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);

        assert_eq!(taskbar.buttons.len(), 1);
        assert_eq!(taskbar.buttons[0].window_id, "app1");
        // No separate background bar -- buttons render inline.
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
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"), false);

        assert_eq!(taskbar.buttons.len(), 5);
        // All buttons should fit within the available region.
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
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);

        let bar_y = taskbar.bar_y;
        // Click on first button.
        let btn1_x = taskbar.buttons[0].x + 2;
        assert_eq!(taskbar.hit_test(btn1_x, bar_y + 5), Some("app1"));
        // Click on second button.
        let btn2_x = taskbar.buttons[1].x + 5;
        assert_eq!(taskbar.hit_test(btn2_x, bar_y + 5), Some("app2"));
    }

    #[test]
    fn hit_test_outside_returns_none() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);

        // Click above taskbar.
        assert_eq!(taskbar.hit_test(5, 0), None);
        // Click below taskbar.
        assert_eq!(taskbar.hit_test(5, at.screen_h as i32), None);
    }

    #[test]
    fn hide_sdi_clears_buttons() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);
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
        taskbar.update_sdi(&mut sdi, &at, &windows, None, false);

        let bar_y = taskbar.bar_y;
        let btn1_x = taskbar.buttons[0].x + 2;
        taskbar.set_hover(btn1_x, bar_y + 5);
        assert_eq!(taskbar.hover_index, Some(0));

        let btn2_x = taskbar.buttons[1].x + 5;
        taskbar.set_hover(btn2_x, bar_y + 5);
        assert_eq!(taskbar.hover_index, Some(1));

        // Move outside.
        taskbar.set_hover(5, 0);
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
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"), false);
        assert_eq!(taskbar.sdi_count, 3);

        // Close one window (now 2).
        let windows: Vec<Window> = (0..2)
            .map(|i| make_window(&format!("app{i}"), &format!("App {i}"), WindowState::Normal))
            .collect();
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app0"), false);
        assert_eq!(taskbar.sdi_count, 2);
        // The third button should be hidden.
        assert!(!sdi.get("taskbar_btn_2").unwrap().visible);
    }

    #[test]
    fn start_menu_offset() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];

        // Without start menu: buttons start at x=4.
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);
        assert_eq!(taskbar.buttons[0].x, 4);

        // With start menu: buttons start after start button.
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), true);
        let expected_x = at.menu.button_x + at.menu.button_width as i32 + 4;
        assert_eq!(taskbar.buttons[0].x, expected_x);
    }

    #[test]
    fn default_trait() {
        let taskbar = Taskbar::default();
        assert!(taskbar.buttons.is_empty());
        assert_eq!(taskbar.hover_index, None);
    }

    // ---------------------------------------------------------------
    // Desktop indicator
    // ---------------------------------------------------------------

    #[test]
    fn desktop_indicator_hidden_for_single_desktop() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 0, 1);
        assert!(!taskbar.desktop_indicator_visible);
        assert!(!sdi.contains("desktop_ind_prev"));
    }

    #[test]
    fn desktop_indicator_shown_for_multiple_desktops() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 0, 4);
        assert!(taskbar.desktop_indicator_visible);
        assert!(sdi.contains("desktop_ind_prev"));
        assert!(sdi.contains("desktop_ind_label"));
        assert!(sdi.contains("desktop_ind_next"));
        // Label should show "1/4" (active=0 → display 1).
        let label = sdi.get("desktop_ind_label").unwrap();
        assert_eq!(label.text.as_deref(), Some("1/4"));
    }

    #[test]
    fn desktop_indicator_label_updates() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 2, 4);
        let label = sdi.get("desktop_ind_label").unwrap();
        assert_eq!(label.text.as_deref(), Some("3/4"));
    }

    #[test]
    fn desktop_indicator_hidden_on_hide_sdi() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let windows = vec![make_window("app1", "First", WindowState::Normal)];
        taskbar.update_sdi(&mut sdi, &at, &windows, Some("app1"), false);
        taskbar.update_desktop_indicator(&mut sdi, &at, 0, 4);
        assert!(taskbar.desktop_indicator_visible);

        taskbar.hide_sdi(&mut sdi);
        assert!(!taskbar.desktop_indicator_visible);
    }

    #[test]
    fn desktop_hit_test_prev() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 1, 4);

        let bar_y = taskbar.bar_y;
        let prev_x = taskbar.desktop_prev_x;
        assert_eq!(
            taskbar.desktop_hit_test(prev_x + 2, bar_y + 5),
            Some(DesktopHit::Prev)
        );
    }

    #[test]
    fn desktop_hit_test_next() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 1, 4);

        let bar_y = taskbar.bar_y;
        let next_x = taskbar.desktop_next_x;
        assert_eq!(
            taskbar.desktop_hit_test(next_x + 2, bar_y + 5),
            Some(DesktopHit::Next)
        );
    }

    #[test]
    fn desktop_hit_test_none_when_hidden() {
        let taskbar = Taskbar::new();
        assert_eq!(taskbar.desktop_hit_test(100, 260), None);
    }

    #[test]
    fn desktop_hit_test_miss_between_arrows() {
        let mut taskbar = Taskbar::new();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        taskbar.update_desktop_indicator(&mut sdi, &at, 0, 4);

        let bar_y = taskbar.bar_y;
        // Click between the two arrows (on the label).
        let mid_x = taskbar.desktop_prev_x + DESKTOP_ARROW_W + 5;
        assert_eq!(taskbar.desktop_hit_test(mid_x, bar_y + 5), None);
    }
}

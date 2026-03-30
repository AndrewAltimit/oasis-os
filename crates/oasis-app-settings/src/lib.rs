//! Settings application for OASIS_OS.
//!
//! Provides a categorised settings screen with skin selection, audio
//! configuration, system information, and about/version details. Skin
//! switching is communicated back to the shell via VFS IPC.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_skin::builtin::builtin_names;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// VFS IPC path used to request a skin change.
///
/// The runner watches for requests on this path. The data payload is the
/// skin name to switch to.
pub const SKIN_CHANGE_REQUEST_PATH: &str = "/system/ipc/skin-change";

/// Settings categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Display,
    Audio,
    System,
    About,
}

impl Category {
    const ALL: [Category; 4] = [
        Category::Display,
        Category::Audio,
        Category::System,
        Category::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Display => "Display",
            Category::Audio => "Audio",
            Category::System => "System",
            Category::About => "About",
        }
    }
}

/// Settings application state.
#[derive(Debug)]
pub struct SettingsApp {
    content: ContentState,
    /// Currently selected category.
    category: Category,
    /// Index within the current category's selectable items.
    item_cursor: usize,
    /// Active skin name.
    current_skin: String,
    /// Virtual resolution width.
    width: u32,
    /// Virtual resolution height.
    height: u32,
    /// Backend display name.
    backend_name: String,
    /// Audio volume level (0-100).
    volume: u32,
}

impl SettingsApp {
    /// Create a new settings app.
    pub fn new(path: &str, skin_name: &str, width: u32, height: u32, backend_name: &str) -> Self {
        let mut app = Self {
            content: ContentState::new("Settings", path),
            category: Category::Display,
            item_cursor: 0,
            current_skin: skin_name.to_string(),
            width,
            height,
            backend_name: backend_name.to_string(),
            volume: 80,
        };
        app.refresh_lines();
        app
    }

    /// Build display lines for the current category.
    fn build_lines(&self) -> Vec<String> {
        let sep = "\u{2500}".repeat(36);
        let mut lines = Vec::new();

        // Category tabs header.
        let tabs: Vec<String> = Category::ALL
            .iter()
            .map(|c| {
                if *c == self.category {
                    format!("[{}]", c.label())
                } else {
                    c.label().to_string()
                }
            })
            .collect();
        lines.push(format!("  {}", tabs.join("  ")));
        lines.push(sep.clone());

        match self.category {
            Category::Display => self.build_display_lines(&mut lines),
            Category::Audio => self.build_audio_lines(&mut lines),
            Category::System => self.build_system_lines(&mut lines),
            Category::About => self.build_about_lines(&mut lines),
        }

        lines.push(sep);
        lines.push(String::new());
        lines.push("  [L/R]=Category  [U/D]=Navigate".to_string());
        lines.push("  [Confirm]=Select  [Cancel]=Exit".to_string());
        lines
    }

    /// Build lines for the Display category.
    fn build_display_lines(&self, lines: &mut Vec<String>) {
        lines.push("  Skin Selection".to_string());
        lines.push(String::new());

        let names = builtin_names();
        for (i, name) in names.iter().enumerate() {
            let marker = if *name == self.current_skin { " *" } else { "" };
            let cursor = if i == self.item_cursor { ">" } else { " " };
            lines.push(format!("  {cursor} {name}{marker}"));
        }

        lines.push(String::new());
        lines.push(format!("  Resolution: {} x {}", self.width, self.height));
    }

    /// Build lines for the Audio category.
    fn build_audio_lines(&self, lines: &mut Vec<String>) {
        lines.push("  Audio Settings".to_string());
        lines.push(String::new());

        // Volume bar visualisation.
        let filled = (self.volume / 5) as usize;
        let empty = 20_usize.saturating_sub(filled);
        let bar = format!(
            "  Volume: [{}{}] {}%",
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(empty),
            self.volume
        );
        lines.push(bar);
        lines.push(String::new());
        lines.push("  [U/D] = Adjust volume".to_string());
        lines.push(String::new());
        lines.push("  Audio Output:  Default".to_string());
        lines.push("  Sample Rate:   44100 Hz".to_string());
        lines.push("  Channels:      Stereo".to_string());
    }

    /// Build lines for the System category.
    fn build_system_lines(&self, lines: &mut Vec<String>) {
        lines.push("  System Information".to_string());
        lines.push(String::new());
        lines.push(format!("  Backend:       {}", self.backend_name));
        lines.push(format!("  Resolution:    {} x {}", self.width, self.height));
        lines.push(format!("  Active Skin:   {}", self.current_skin));
        lines.push(format!("  Version:       {}", env!("CARGO_PKG_VERSION")));
        lines.push(String::new());
        lines.push("  VFS:           MemoryVfs".to_string());
        lines.push("  Rust Edition:  2024".to_string());
        lines.push("  MSRV:          1.91.0".to_string());
    }

    /// Build lines for the About category.
    fn build_about_lines(&self, lines: &mut Vec<String>) {
        lines.push("  About OASIS_OS".to_string());
        lines.push(String::new());
        lines.push(format!("  Version:    {}", env!("CARGO_PKG_VERSION")));
        lines.push("  License:    MIT / Unlicense".to_string());
        lines.push("  Crates:     20 workspace crates".to_string());
        lines.push("  Apps:       16 built-in".to_string());
        lines.push("  Skins:      18 built-in".to_string());
        lines.push(String::new());
        lines.push("  An embeddable operating system".to_string());
        lines.push("  framework originally ported from".to_string());
        lines.push("  Inspired by PSP homebrew (PSIX).".to_string());
        lines.push(String::new());
        lines.push("  github.com/AndrewAltimit/oasis-os".to_string());
    }

    /// Rebuild display lines from current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.build_lines();
    }

    /// Number of selectable items in the current category.
    fn item_count(&self) -> usize {
        match self.category {
            Category::Display => builtin_names().len(),
            // Audio has no selectable items (volume is adjusted directly).
            Category::Audio | Category::System | Category::About => 0,
        }
    }

    /// Switch to the next category (right).
    fn next_category(&mut self) {
        let idx = Category::ALL
            .iter()
            .position(|c| *c == self.category)
            .unwrap_or(0);
        let next = (idx + 1) % Category::ALL.len();
        self.category = Category::ALL[next];
        self.item_cursor = 0;
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
    }

    /// Switch to the previous category (left).
    fn prev_category(&mut self) {
        let idx = Category::ALL
            .iter()
            .position(|c| *c == self.category)
            .unwrap_or(0);
        let prev = if idx == 0 {
            Category::ALL.len() - 1
        } else {
            idx - 1
        };
        self.category = Category::ALL[prev];
        self.item_cursor = 0;
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
    }

    /// Handle confirm action in the current category.
    fn handle_confirm(&mut self) {
        if self.category == Category::Display {
            let names = builtin_names();
            if self.item_cursor < names.len() {
                let selected = names[self.item_cursor];
                if selected != self.current_skin {
                    self.current_skin = selected.to_string();
                    self.content.pending_vfs_request =
                        Some((SKIN_CHANGE_REQUEST_PATH.to_string(), selected.to_string()));
                }
                self.refresh_lines();
            }
        }
    }
}

impl App for SettingsApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => AppAction::Exit,

            Button::Left => {
                self.prev_category();
                AppAction::None
            },

            Button::Right => {
                self.next_category();
                AppAction::None
            },

            Button::Up => {
                if self.category == Category::Audio {
                    self.volume = (self.volume + 5).min(100);
                    self.refresh_lines();
                } else {
                    let count = self.item_count();
                    if count > 0 && self.item_cursor > 0 {
                        self.item_cursor -= 1;
                        self.refresh_lines();
                    }
                    self.content.navigate_up();
                }
                AppAction::None
            },

            Button::Down => {
                if self.category == Category::Audio {
                    self.volume = self.volume.saturating_sub(5);
                    self.refresh_lines();
                } else {
                    let count = self.item_count();
                    if count > 0 && self.item_cursor + 1 < count {
                        self.item_cursor += 1;
                        self.refresh_lines();
                    }
                    self.content.navigate_down();
                }
                AppAction::None
            },

            Button::Confirm => {
                self.handle_confirm();
                AppAction::None
            },

            _ => AppAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn make_app() -> SettingsApp {
        SettingsApp::new("/apps/settings", "classic", 480, 272, "SDL3")
    }

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    #[test]
    fn title_and_path() {
        let app = make_app();
        assert_eq!(app.title(), "Settings");
        assert_eq!(app.path(), "/apps/settings");
    }

    #[test]
    fn initial_category_is_display() {
        let app = make_app();
        assert_eq!(app.category, Category::Display);
    }

    #[test]
    fn lines_not_empty() {
        let app = make_app();
        assert!(!app.lines().is_empty());
    }

    #[test]
    fn lines_contain_skin_names() {
        let app = make_app();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("classic")));
        assert!(lines.iter().any(|l| l.contains("balatro")));
    }

    #[test]
    fn current_skin_marked() {
        let app = make_app();
        let lines = app.lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("classic") && l.contains('*')),
            "current skin should be marked with *"
        );
    }

    #[test]
    fn cancel_exits() {
        let vfs = make_vfs();
        let mut app = make_app();
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn right_switches_category() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Audio);
    }

    #[test]
    fn left_wraps_category() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.category, Category::About);
    }

    #[test]
    fn category_cycle_wraps_right() {
        let vfs = make_vfs();
        let mut app = make_app();
        for _ in 0..4 {
            app.handle_input(&Button::Right, &vfs);
        }
        assert_eq!(app.category, Category::Display);
    }

    #[test]
    fn navigate_skins_down() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.item_cursor, 1);
    }

    #[test]
    fn navigate_skins_up_at_top_stays() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn confirm_selects_skin() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Move to second skin and confirm.
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);

        let names = builtin_names();
        assert_eq!(app.current_skin, names[1]);

        // Should have a pending VFS IPC request.
        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.as_ref().unwrap();
        assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
        assert_eq!(data, names[1]);
    }

    #[test]
    fn confirm_same_skin_no_request() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Confirm on current skin (classic, cursor=0).
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn audio_volume_up() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs); // Audio category
        let before = app.volume;
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.volume, (before + 5).min(100));
    }

    #[test]
    fn audio_volume_down() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs); // Audio category
        let before = app.volume;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.volume, before.saturating_sub(5));
    }

    #[test]
    fn audio_volume_clamped_at_100() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs); // Audio
        app.volume = 100;
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.volume, 100);
    }

    #[test]
    fn audio_volume_clamped_at_0() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs); // Audio
        app.volume = 0;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.volume, 0);
    }

    #[test]
    fn system_category_lines() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to System (right twice).
        app.handle_input(&Button::Right, &vfs);
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::System);
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("SDL3")));
        assert!(lines.iter().any(|l| l.contains("480")));
    }

    #[test]
    fn about_category_lines() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to About (right three times).
        for _ in 0..3 {
            app.handle_input(&Button::Right, &vfs);
        }
        assert_eq!(app.category, Category::About);
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("MIT")));
        assert!(lines.iter().any(|l| l.contains("PSP")));
    }

    #[test]
    fn downcast_works() {
        let app = make_app();
        let any = app.as_any();
        assert!(any.downcast_ref::<SettingsApp>().is_some());
    }

    #[test]
    fn no_browse_dir_or_viewing_file() {
        let app = make_app();
        assert!(app.browse_dir().is_none());
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn item_cursor_bounded() {
        let vfs = make_vfs();
        let mut app = make_app();
        let count = builtin_names().len();
        // Navigate down past all skins.
        for _ in 0..count + 5 {
            app.handle_input(&Button::Down, &vfs);
        }
        assert!(app.item_cursor < count);
    }

    #[test]
    fn switching_category_resets_cursor() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        assert!(app.item_cursor > 0);
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn audio_lines_contain_volume() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs); // Audio
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Volume")));
    }
}

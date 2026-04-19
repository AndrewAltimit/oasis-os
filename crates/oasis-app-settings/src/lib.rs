//! Settings application for OASIS_OS.
//!
//! Provides a categorised settings screen with skin selection, resolution
//! switching, audio configuration, and system/about details. Changes are
//! published back to the shell through VFS IPC paths, and the shell applies
//! them live to the running session.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_skin::builtin::builtin_names;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

/// VFS IPC path used to request a skin change.
pub const SKIN_CHANGE_REQUEST_PATH: &str = "/system/ipc/skin-change";

/// VFS IPC path used to request a resolution change.
///
/// The payload is `"WIDTHxHEIGHT"` (e.g. `"1280x720"`).
pub const RESOLUTION_CHANGE_REQUEST_PATH: &str = "/system/ipc/resolution-change";

/// VFS path where the shell publishes the currently active skin name.
pub const SKIN_STATE_PATH: &str = "/system/state/skin";

/// VFS path where the shell publishes the current resolution (`"WxH"`).
pub const RESOLUTION_STATE_PATH: &str = "/system/state/resolution";

/// VFS path where the shell publishes the current backend name.
pub const BACKEND_STATE_PATH: &str = "/system/state/backend";

/// Resolution presets offered by the Settings UI.
///
/// Kept small on purpose so the list fits on a PSP-native screen (480x272)
/// without scrolling. First entry is PSP-native; later entries are common
/// desktop sizes.
pub const RESOLUTION_PRESETS: &[(u32, u32)] = &[
    (480, 272),
    (800, 600),
    (1024, 768),
    (1280, 720),
    (1600, 900),
    (1920, 1080),
];

/// Settings categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Display,
    Resolution,
    Audio,
    System,
    About,
}

impl Category {
    const ALL: [Category; 5] = [
        Category::Display,
        Category::Resolution,
        Category::Audio,
        Category::System,
        Category::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Display => "Display",
            Category::Resolution => "Resolution",
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
    /// Create a new settings app with explicit current values.
    ///
    /// Most callers should prefer [`SettingsApp::from_vfs`], which reads the
    /// shell-published state from well-known VFS paths so the UI reflects the
    /// actually-running skin and resolution.
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
        // Align the cursor with the currently active skin so the highlight
        // starts on the running skin rather than always on the first entry.
        if let Some(idx) = builtin_names().iter().position(|n| *n == app.current_skin) {
            app.item_cursor = idx;
        }
        app.refresh_lines();
        app.sync_content_cursor();
        app
    }

    /// Create a settings app by reading the shell-published runtime state
    /// from the VFS. Falls back to the defaults passed in for any path that
    /// doesn't exist yet (typically only on first boot).
    pub fn from_vfs(
        path: &str,
        vfs: &dyn Vfs,
        default_skin: &str,
        default_w: u32,
        default_h: u32,
        default_backend: &str,
    ) -> Self {
        let skin = read_utf8(vfs, SKIN_STATE_PATH).unwrap_or_else(|| default_skin.to_string());
        let (width, height) = read_utf8(vfs, RESOLUTION_STATE_PATH)
            .and_then(|s| parse_resolution(&s))
            .unwrap_or((default_w, default_h));
        let backend =
            read_utf8(vfs, BACKEND_STATE_PATH).unwrap_or_else(|| default_backend.to_string());
        Self::new(path, &skin, width, height, &backend)
    }

    /// Index into [`RESOLUTION_PRESETS`] for the currently active resolution,
    /// or `None` if no preset matches exactly.
    fn current_resolution_index(&self) -> Option<usize> {
        RESOLUTION_PRESETS
            .iter()
            .position(|(w, h)| *w == self.width && *h == self.height)
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
            Category::Resolution => self.build_resolution_lines(&mut lines),
            Category::Audio => self.build_audio_lines(&mut lines),
            Category::System => self.build_system_lines(&mut lines),
            Category::About => self.build_about_lines(&mut lines),
        }

        lines.push(sep);
        lines.push(String::new());
        lines.push("  [L/R]=Category  [U/D]=Navigate".to_string());
        lines.push("  [Confirm]=Apply  [Cancel]=Exit".to_string());
        lines
    }

    /// Build lines for the Display category.
    fn build_display_lines(&self, lines: &mut Vec<String>) {
        lines.push("  Skin Selection".to_string());
        lines.push(String::new());

        // No embedded cursor marker: `draw_content_windowed` renders the
        // selection `>` from `content.cursor`, which `sync_content_cursor`
        // keeps aligned with `item_cursor`. Including a second marker here
        // would double-draw the prefix and desync on scroll.
        let names = builtin_names();
        for name in names.iter() {
            let marker = if *name == self.current_skin { " *" } else { "" };
            lines.push(format!("   {name}{marker}"));
        }

        lines.push(String::new());
        lines.push(format!("  Resolution: {} x {}", self.width, self.height));
    }

    /// Build lines for the Resolution category.
    fn build_resolution_lines(&self, lines: &mut Vec<String>) {
        lines.push("  Virtual Resolution".to_string());
        lines.push(String::new());

        for (w, h) in RESOLUTION_PRESETS.iter() {
            let active = *w == self.width && *h == self.height;
            let marker = if active { " *" } else { "" };
            let label = preset_label(*w, *h);
            lines.push(format!("   {w}x{h}  {label}{marker}"));
        }

        lines.push(String::new());
        lines.push("  Window + layout resize live.".to_string());
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

    /// Re-read the shell-published state from VFS. Called on each tick so the
    /// UI reflects changes applied by the shell after we posted a request.
    fn sync_from_vfs(&mut self, vfs: &dyn Vfs) {
        let mut changed = false;

        if let Some(skin) = read_utf8(vfs, SKIN_STATE_PATH)
            && skin != self.current_skin
        {
            self.current_skin = skin;
            if self.category == Category::Display
                && let Some(idx) = builtin_names().iter().position(|n| *n == self.current_skin)
            {
                self.item_cursor = idx;
            }
            changed = true;
        }

        if let Some((w, h)) =
            read_utf8(vfs, RESOLUTION_STATE_PATH).and_then(|s| parse_resolution(&s))
            && (w != self.width || h != self.height)
        {
            self.width = w;
            self.height = h;
            if self.category == Category::Resolution
                && let Some(idx) = self.current_resolution_index()
            {
                self.item_cursor = idx;
            }
            changed = true;
        }

        if changed {
            self.refresh_lines();
            self.sync_content_cursor();
        }
    }

    /// Rebuild display lines from current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.build_lines();
    }

    /// First content-line index where selectable items begin in the
    /// Display / Resolution categories. Layout is:
    ///   0 = tab header
    ///   1 = separator
    ///   2 = section header ("Skin Selection" / "Virtual Resolution")
    ///   3 = blank
    ///   4 = first item
    /// Kept in sync with [`Self::build_display_lines`] and
    /// [`Self::build_resolution_lines`], and also used by
    /// [`Self::handle_click`] to map clicks back to item indices.
    const ITEMS_START: usize = 4;

    /// Align `content.cursor` (and scroll, if needed) with the active item
    /// so the single `>` prefix drawn by `draw_content_windowed` lands on
    /// the item `item_cursor` points at. Only meaningful for categories
    /// that actually have a list of selectable items — scrollable text
    /// categories leave content.cursor alone.
    fn sync_content_cursor(&mut self) {
        if !matches!(self.category, Category::Display | Category::Resolution) {
            return;
        }
        let target = Self::ITEMS_START + self.item_cursor;
        let max_visible = self.content.cached_max_visible.max(1);
        if target < self.content.scroll {
            self.content.scroll = target;
        } else if target >= self.content.scroll + max_visible {
            self.content.scroll = target + 1 - max_visible;
        }
        self.content.cursor = target - self.content.scroll;
    }

    /// Number of selectable items in the current category.
    fn item_count(&self) -> usize {
        match self.category {
            Category::Display => builtin_names().len(),
            Category::Resolution => RESOLUTION_PRESETS.len(),
            // No selectable items -- Audio uses Up/Down for volume directly.
            Category::Audio | Category::System | Category::About => 0,
        }
    }

    /// Index of the cursor to start at when entering the given category.
    fn cursor_for_category(&self, c: Category) -> usize {
        match c {
            Category::Display => builtin_names()
                .iter()
                .position(|n| *n == self.current_skin)
                .unwrap_or(0),
            Category::Resolution => self.current_resolution_index().unwrap_or(0),
            _ => 0,
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
        self.item_cursor = self.cursor_for_category(self.category);
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
        self.sync_content_cursor();
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
        self.item_cursor = self.cursor_for_category(self.category);
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
        self.sync_content_cursor();
    }

    /// Handle confirm action in the current category.
    fn handle_confirm(&mut self) {
        match self.category {
            Category::Display => {
                let names = builtin_names();
                if self.item_cursor < names.len() {
                    let selected = names[self.item_cursor];
                    if selected != self.current_skin {
                        // Don't mutate current_skin yet — wait for the shell
                        // to publish the new state back. This makes the UI
                        // accurately reflect whether the swap actually took.
                        self.content.pending_vfs_request =
                            Some((SKIN_CHANGE_REQUEST_PATH.to_string(), selected.to_string()));
                    }
                }
            },
            Category::Resolution => {
                if self.item_cursor < RESOLUTION_PRESETS.len() {
                    let (w, h) = RESOLUTION_PRESETS[self.item_cursor];
                    if w != self.width || h != self.height {
                        self.content.pending_vfs_request = Some((
                            RESOLUTION_CHANGE_REQUEST_PATH.to_string(),
                            format!("{w}x{h}"),
                        ));
                    }
                }
            },
            _ => {},
        }
    }
}

impl App for SettingsApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        // Always check for shell-published state updates first so the display
        // catches up before we interpret the input. This is also what makes
        // the "* currently active" marker flip as soon as the shell applies
        // a pending change.
        self.sync_from_vfs(vfs);

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
                match self.category {
                    Category::Audio => {
                        self.volume = (self.volume + 5).min(100);
                        self.refresh_lines();
                    },
                    Category::Display | Category::Resolution => {
                        if self.item_cursor > 0 {
                            self.item_cursor -= 1;
                            self.refresh_lines();
                            self.sync_content_cursor();
                        }
                    },
                    Category::System | Category::About => {
                        // Plain scrollable text — let the content cursor
                        // drive itself.
                        self.content.navigate_up();
                    },
                }
                AppAction::None
            },

            Button::Down => {
                match self.category {
                    Category::Audio => {
                        self.volume = self.volume.saturating_sub(5);
                        self.refresh_lines();
                    },
                    Category::Display | Category::Resolution => {
                        let count = self.item_count();
                        if count > 0 && self.item_cursor + 1 < count {
                            self.item_cursor += 1;
                            self.refresh_lines();
                            self.sync_content_cursor();
                        }
                    },
                    Category::System | Category::About => {
                        self.content.navigate_down();
                    },
                }
                AppAction::None
            },

            // Triangle (Space on desktop) doubles as Confirm so users who
            // discover Space before Enter still get a working path.
            Button::Confirm | Button::Triangle => {
                self.handle_confirm();
                AppAction::None
            },

            _ => AppAction::None,
        }
    }

    fn handle_click(
        &mut self,
        _lx: i32,
        ly: i32,
        _cw: u32,
        _ch: u32,
        _fullscreen: bool,
    ) -> AppAction {
        // Map the click Y coordinate back to a content-line index using the
        // same constants the renderer uses for `draw_content_windowed`:
        // `title_bar_height + line_idx * line_h`. We don't have the theme
        // here, so fall back to the common desktop values (20px titlebar,
        // 14px line height). Off-by-one clicks still land on the correct
        // row because row heights are uniform.
        const TITLE_BAR_HEIGHT: i32 = 20;
        const LINE_H: i32 = 14;

        let y_in_content = ly - TITLE_BAR_HEIGHT;
        if y_in_content < 0 {
            return AppAction::None;
        }
        let visible_idx = (y_in_content / LINE_H) as usize;
        let line_idx = self.content.scroll + visible_idx;

        if line_idx < Self::ITEMS_START {
            return AppAction::None;
        }
        let item_idx = line_idx - Self::ITEMS_START;

        match self.category {
            Category::Display => {
                let names = builtin_names();
                if item_idx < names.len() {
                    self.item_cursor = item_idx;
                    self.handle_confirm();
                    self.refresh_lines();
                    self.sync_content_cursor();
                }
            },
            Category::Resolution => {
                if item_idx < RESOLUTION_PRESETS.len() {
                    self.item_cursor = item_idx;
                    self.handle_confirm();
                    self.refresh_lines();
                    self.sync_content_cursor();
                }
            },
            _ => {},
        }
        AppAction::None
    }
}

/// Read a VFS file as a trimmed UTF-8 string, returning `None` if missing or
/// not valid UTF-8.
fn read_utf8(vfs: &dyn Vfs, path: &str) -> Option<String> {
    let data = vfs.read(path).ok()?;
    let s = std::str::from_utf8(&data).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse a `"WIDTHxHEIGHT"` string into its components.
fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Friendly label for a resolution preset (aspect ratio / common name).
fn preset_label(w: u32, h: u32) -> &'static str {
    match (w, h) {
        (480, 272) => "(PSP)",
        (800, 600) => "(4:3 SVGA)",
        (1024, 768) => "(4:3 XGA)",
        (1280, 720) => "(16:9 HD)",
        (1600, 900) => "(16:9 HD+)",
        (1920, 1080) => "(16:9 FHD)",
        _ => "",
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
        assert_eq!(app.category, Category::Resolution);
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
        for _ in 0..Category::ALL.len() {
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
        for _ in 0..2 {
            app.handle_input(&Button::Right, &vfs); // Display -> Resolution -> Audio
        }
        assert_eq!(app.category, Category::Audio);
        let before = app.volume;
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.volume, (before + 5).min(100));
    }

    #[test]
    fn audio_volume_down() {
        let vfs = make_vfs();
        let mut app = make_app();
        for _ in 0..2 {
            app.handle_input(&Button::Right, &vfs);
        }
        let before = app.volume;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.volume, before.saturating_sub(5));
    }

    #[test]
    fn audio_volume_clamped_at_100() {
        let vfs = make_vfs();
        let mut app = make_app();
        for _ in 0..2 {
            app.handle_input(&Button::Right, &vfs);
        }
        app.volume = 100;
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.volume, 100);
    }

    #[test]
    fn audio_volume_clamped_at_0() {
        let vfs = make_vfs();
        let mut app = make_app();
        for _ in 0..2 {
            app.handle_input(&Button::Right, &vfs);
        }
        app.volume = 0;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.volume, 0);
    }

    #[test]
    fn system_category_lines() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to System.
        for _ in 0..3 {
            app.handle_input(&Button::Right, &vfs);
        }
        assert_eq!(app.category, Category::System);
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("SDL3")));
        assert!(lines.iter().any(|l| l.contains("480")));
    }

    #[test]
    fn about_category_lines() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to About.
        for _ in 0..4 {
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
    fn switching_category_resets_cursor_to_current_skin() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Move cursor off the active skin.
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        assert!(app.item_cursor > 0);
        // Leave and return -> cursor snaps back to active skin (classic, index 0).
        app.handle_input(&Button::Right, &vfs);
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn audio_lines_contain_volume() {
        let vfs = make_vfs();
        let mut app = make_app();
        for _ in 0..2 {
            app.handle_input(&Button::Right, &vfs);
        }
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Volume")));
    }

    // -- Resolution category --

    #[test]
    fn resolution_category_lists_presets() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Resolution);
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("480x272")));
        assert!(lines.iter().any(|l| l.contains("1920x1080")));
    }

    #[test]
    fn resolution_current_is_marked() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs);
        let lines = app.lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("480x272") && l.contains('*')),
            "current resolution should be marked"
        );
    }

    #[test]
    fn resolution_cursor_starts_on_current() {
        let vfs = make_vfs();
        // Start with a non-first resolution so we know the cursor is really
        // being aligned rather than just defaulting to 0.
        let mut app = SettingsApp::new("/apps/settings", "classic", 1280, 720, "SDL3");
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Resolution);
        // 1280x720 is index 3 in RESOLUTION_PRESETS.
        assert_eq!(app.item_cursor, 3);
    }

    #[test]
    fn confirm_resolution_writes_request() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Display -> Resolution.
        app.handle_input(&Button::Right, &vfs);
        // Move from 480x272 (idx 0) to 1280x720 (idx 3).
        for _ in 0..3 {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);

        let req = app.take_pending_request();
        let (path, data) = req.expect("resolution confirm should post an IPC request");
        assert_eq!(path, RESOLUTION_CHANGE_REQUEST_PATH);
        assert_eq!(data, "1280x720");
    }

    #[test]
    fn confirm_same_resolution_no_request() {
        let vfs = make_vfs();
        let mut app = make_app();
        app.handle_input(&Button::Right, &vfs);
        // Cursor starts on the active resolution, so Confirm should no-op.
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.take_pending_request().is_none());
    }

    // -- VFS-driven construction + sync --

    #[test]
    fn from_vfs_reads_state_paths() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(SKIN_STATE_PATH, b"balatro").unwrap();
        vfs.write(RESOLUTION_STATE_PATH, b"1280x720").unwrap();
        vfs.write(BACKEND_STATE_PATH, b"SDL3").unwrap();

        let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "UNKNOWN");
        assert_eq!(app.current_skin, "balatro");
        assert_eq!((app.width, app.height), (1280, 720));
        assert_eq!(app.backend_name, "SDL3");
    }

    #[test]
    fn from_vfs_falls_back_to_defaults() {
        let vfs = MemoryVfs::new();
        let app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
        assert_eq!(app.current_skin, "classic");
        assert_eq!((app.width, app.height), (480, 272));
    }

    #[test]
    fn sync_picks_up_shell_state_changes() {
        let mut vfs = MemoryVfs::new();
        let mut app = make_app();
        assert_eq!(app.current_skin, "classic");

        // Shell applies a skin swap and publishes the new state.
        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(SKIN_STATE_PATH, b"balatro").unwrap();
        vfs.write(RESOLUTION_STATE_PATH, b"1280x720").unwrap();

        // Any input tick refreshes from VFS.
        app.handle_input(&Button::Up, &vfs);

        assert_eq!(app.current_skin, "balatro");
        assert_eq!((app.width, app.height), (1280, 720));
    }

    // -- Click + alternate-confirm --

    #[test]
    fn space_triggers_confirm() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Move cursor off the active skin.
        app.handle_input(&Button::Down, &vfs);
        // Space maps to Button::Triangle in the SDL input layer — Settings
        // should accept it as an alternate Confirm so users who try Space
        // before discovering Enter still get feedback.
        app.handle_input(&Button::Triangle, &vfs);
        assert!(
            app.take_pending_request().is_some(),
            "Space/Triangle should confirm the selection"
        );
    }

    #[test]
    fn click_on_skin_row_applies() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Items render starting at line 4 (tabs + separator + header + blank).
        // Renderer uses title_bar_height = 20 + line_h = 14, so line 4 lives
        // at y = 20 + 4*14 = 76. Line 5 (second skin, not the active one)
        // lives at y = 90.
        let _action = app.handle_click(10, 90, 400, 220, false);
        let req = app.take_pending_request();
        let (path, data) = req.expect("click on non-active skin should post IPC");
        assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
        // builtin_names()[1] is the second skin (not "classic").
        assert_eq!(data, builtin_names()[1]);
    }

    #[test]
    fn click_on_active_skin_noop() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Line 4 = first skin ("classic"), which is the active one.
        let _action = app.handle_click(10, 76, 400, 220, false);
        assert!(app.take_pending_request().is_none());
        // Double-check we pulled the vfs arg in via sync (no panic).
        let _ = vfs;
    }

    #[test]
    fn click_above_content_area_ignored() {
        let mut app = make_app();
        // Click in the title bar area (y < 20).
        let _action = app.handle_click(10, 5, 400, 220, false);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn click_on_resolution_preset_applies() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to Resolution category.
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Resolution);
        // Click the 4th preset (1280x720, index 3 → content line 4+3=7 →
        // y = 20 + 7*14 = 118).
        let _action = app.handle_click(10, 118, 400, 220, false);
        let req = app.take_pending_request();
        let (path, data) = req.expect("click on non-active preset should post IPC");
        assert_eq!(path, RESOLUTION_CHANGE_REQUEST_PATH);
        assert_eq!(data, "1280x720");
    }

    // -- Cursor sync (no double-`>` markers) --

    #[test]
    fn display_lines_have_no_embedded_cursor_marker() {
        let app = make_app();
        let lines = app.lines();
        // The only `>` in the rendered output should come from
        // `draw_content_windowed`, not from build_display_lines. Verify no
        // skin row contains a `>` character embedded in the text.
        for line in lines.iter() {
            let is_skin_row = builtin_names().iter().any(|n| line.contains(n));
            if is_skin_row {
                assert!(
                    !line.contains('>'),
                    "skin row should not embed a cursor marker: {line:?}"
                );
            }
        }
    }

    #[test]
    fn sync_cursor_lands_on_active_skin_line() {
        // Start with a skin that's not first in the list to make the test
        // meaningful (otherwise cursor=0 and scroll=0 trivially "match").
        let vfs = make_vfs();
        let mut app = SettingsApp::new("/apps/settings", "balatro", 480, 272, "SDL3");
        app.content.cached_max_visible = 13;
        app.sync_content_cursor();
        let balatro_idx = builtin_names()
            .iter()
            .position(|n| *n == "balatro")
            .expect("balatro must be a known skin");
        let expected_line = SettingsApp::ITEMS_START + balatro_idx;
        assert_eq!(
            app.content.scroll + app.content.cursor,
            expected_line,
            "content cursor+scroll must point at the active skin's line",
        );
        // Also make sure navigation updates both in lockstep.
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.scroll + app.content.cursor, expected_line + 1,);
    }

    #[test]
    fn sync_cursor_scrolls_when_item_below_viewport() {
        let mut app = make_app();
        // Simulate a small viewport.
        app.content.cached_max_visible = 5;
        // Walk cursor to a late skin (say index 10) — requires scroll.
        app.item_cursor = 10;
        app.sync_content_cursor();
        let target = SettingsApp::ITEMS_START + 10;
        assert!(
            app.content.scroll > 0,
            "scroll should advance once the cursor moves beyond the viewport"
        );
        assert_eq!(app.content.scroll + app.content.cursor, target);
        assert!(
            app.content.cursor < 5,
            "cursor must stay within visible range"
        );
    }

    #[test]
    fn parse_resolution_ok() {
        assert_eq!(parse_resolution("800x600"), Some((800, 600)));
        assert_eq!(parse_resolution("  1920x1080 "), Some((1920, 1080)));
    }

    #[test]
    fn parse_resolution_bad() {
        assert!(parse_resolution("not a resolution").is_none());
        assert!(parse_resolution("800").is_none());
        assert!(parse_resolution("ax600").is_none());
    }
}

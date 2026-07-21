//! Settings application for OASIS_OS.
//!
//! Provides a categorised settings screen with skin selection, resolution
//! switching, audio configuration, and system/about details. Changes are
//! published back to the shell through VFS IPC paths, and the shell applies
//! them live to the running session.

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_skin::builtin::builtin_names;
use oasis_skin::theme::{contrast_ratio, parse_hex_color};
use oasis_skin::{SkinTheme, SkinVariant, resolve_skin};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

mod colors;

pub use colors::SettingsColors;

/// VFS IPC path used to request a skin change.
pub const SKIN_CHANGE_REQUEST_PATH: &str = "/system/ipc/skin-change";

/// VFS IPC path used to request an in-memory theme preview ("Apply" in the
/// Appearance editor). The payload is a serialized `SkinTheme` TOML document;
/// the shell keeps the current skin's layout/features and swaps only the
/// theme, without writing anything to disk.
pub const SKIN_APPLY_THEME_REQUEST_PATH: &str = "/system/ipc/skin-apply-theme";

/// VFS IPC path used to save the edited theme as a custom skin. The payload
/// is `<name>\n<theme toml>`; the shell writes `skins/<name>/` in the
/// standard directory format and then swaps to it by name.
pub const SKIN_SAVE_CUSTOM_REQUEST_PATH: &str = "/system/ipc/skin-save-custom";

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
    Appearance,
    Resolution,
    Audio,
    System,
    About,
}

impl Category {
    const ALL: [Category; 6] = [
        Category::Display,
        Category::Appearance,
        Category::Resolution,
        Category::Audio,
        Category::System,
        Category::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Display => "Display",
            Category::Appearance => "Appearance",
            Category::Resolution => "Resolution",
            Category::Audio => "Audio",
            Category::System => "System",
            Category::About => "About",
        }
    }
}

/// Labels for the 9 editable base palette colors, in `SkinTheme` order.
const BASE_COLOR_LABELS: [&str; 9] = [
    "Background",
    "Primary",
    "Secondary",
    "Text",
    "Dim Text",
    "Status Bar",
    "Prompt",
    "Output",
    "Error",
];

/// Number of action rows after the 9 color rows in the Appearance list
/// (Apply, Save, and one row per variant).
const APPEARANCE_ACTIONS: usize = 2 + SkinVariant::ALL.len();

/// For each base-color role (indexed like [`BASE_COLOR_LABELS`]), the partner
/// role its contrast is measured against and the WCAG AA ratio it should
/// clear. Surfaces (`Background`, `Status Bar`) are judged by the readability
/// of `Text` drawn on them; foreground roles are judged against `Background`.
/// `Text` itself is held to the 4.5:1 body-text minimum; the rest to 3.0:1.
const CONTRAST_PARTNERS: [(usize, f64); 9] = [
    (3, 4.5), // Background   vs Text
    (0, 3.0), // Primary      vs Background
    (0, 3.0), // Secondary    vs Background
    (0, 4.5), // Text         vs Background
    (0, 3.0), // Dim Text     vs Background
    (3, 4.5), // Status Bar   vs Text
    (0, 3.0), // Prompt       vs Background
    (0, 3.0), // Output       vs Background
    (0, 3.0), // Error        vs Background
];

/// Format a color as `#RRGGBB`.
fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Short label for a base-color role, used in the inline contrast readout.
fn short_role(role: usize) -> &'static str {
    match role {
        0 => "Bg",
        3 => "Text",
        _ => BASE_COLOR_LABELS[role],
    }
}

/// State for the Appearance base-color editor.
#[derive(Debug)]
struct AppearanceState {
    /// The 9 editable base palette colors (order matches
    /// [`BASE_COLOR_LABELS`]).
    colors: [Color; 9],
    /// Which skin the palette was loaded from (reloaded when the shell
    /// publishes a different active skin).
    loaded_for: String,
    /// Active channel while editing a color row (0 = R, 1 = G, 2 = B).
    /// `None` when not in edit mode.
    editing_channel: Option<u8>,
    /// Color value before edit mode was entered, for Cancel-revert.
    edit_backup: Color,
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self {
            colors: [Color::BLACK; 9],
            loaded_for: String::new(),
            editing_channel: None,
            edit_backup: Color::BLACK,
        }
    }
}

impl AppearanceState {
    /// Load the palette from a skin's theme.
    fn load_from_theme(&mut self, theme: &SkinTheme, skin_name: &str) {
        let parse = |s: &str, fallback: Color| parse_hex_color(s).unwrap_or(fallback);
        self.colors = [
            parse(&theme.background, Color::BLACK),
            parse(&theme.primary, Color::rgb(50, 100, 200)),
            parse(&theme.secondary, Color::rgb(80, 80, 80)),
            parse(&theme.text, Color::WHITE),
            parse(&theme.dim_text, Color::rgb(128, 128, 128)),
            parse(&theme.status_bar, Color::rgb(40, 60, 90)),
            parse(&theme.prompt, Color::rgb(0, 255, 0)),
            parse(&theme.output, Color::rgb(204, 204, 204)),
            parse(&theme.error, Color::rgb(255, 68, 68)),
        ];
        self.loaded_for = skin_name.to_string();
        self.editing_channel = None;
    }

    /// Write the palette into a theme's 9 base color fields.
    fn write_to_theme(&self, theme: &mut SkinTheme) {
        theme.background = hex(self.colors[0]);
        theme.primary = hex(self.colors[1]);
        theme.secondary = hex(self.colors[2]);
        theme.text = hex(self.colors[3]);
        theme.dim_text = hex(self.colors[4]);
        theme.status_bar = hex(self.colors[5]);
        theme.prompt = hex(self.colors[6]);
        theme.output = hex(self.colors[7]);
        theme.error = hex(self.colors[8]);
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
    /// Appearance editor state (base palette, edit mode).
    appearance: AppearanceState,
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
            appearance: AppearanceState::default(),
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
            Category::Appearance => self.build_appearance_lines(&mut lines),
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

    /// Build lines for the Appearance category (base-color editor).
    ///
    /// Layout mirrors the other list categories: header at line 2, blank at
    /// line 3, selectable items from [`Self::ITEMS_START`] on. Items are the
    /// 9 base colors followed by the action rows (Apply / Save / variants),
    /// with no gaps so `item_cursor` maps 1:1 to lines.
    fn build_appearance_lines(&self, lines: &mut Vec<String>) {
        lines.push("  Appearance - Base Colors".to_string());
        lines.push(String::new());

        for (i, label) in BASE_COLOR_LABELS.iter().enumerate() {
            let c = self.appearance.colors[i];
            let readout = self.contrast_readout(i);
            if self.item_cursor == i && self.appearance.editing_channel.is_some() {
                let ch = self.appearance.editing_channel.unwrap_or(0);
                let mark = |idx: u8, name: char, v: u8| {
                    if ch == idx {
                        format!("[{name}:{v:3}]")
                    } else {
                        format!(" {name}:{v:3} ")
                    }
                };
                lines.push(format!(
                    "   {label:<11} {} {}{}{} {readout}",
                    hex(c),
                    mark(0, 'R', c.r),
                    mark(1, 'G', c.g),
                    mark(2, 'B', c.b),
                ));
            } else {
                lines.push(format!("   {label:<11} {}  {readout}", hex(c)));
            }
        }

        lines.push("   [ Apply (preview) ]".to_string());
        lines.push(format!("   [ Save as '{}' ]", self.custom_skin_name()));
        for v in SkinVariant::ALL {
            lines.push(format!("   [ Variant: {} ]", v.label()));
        }

        lines.push(String::new());
        if self.appearance.editing_channel.is_some() {
            lines.push("  [U/D]=Value  [L/R]=Channel".to_string());
            lines.push("  [Confirm]=Done  [Cancel]=Revert".to_string());
        } else {
            lines.push("  [Confirm]=Edit color / activate".to_string());
            lines.push("  AA = passes WCAG contrast, low = below".to_string());
        }
    }

    /// Inline contrast readout for a base-color row: the ratio against the
    /// role's sensible partner color (see [`CONTRAST_PARTNERS`]) plus an
    /// `AA` / `low` verdict at the WCAG AA threshold. Recomputed on every
    /// refresh, so it updates live as a channel is stepped.
    fn contrast_readout(&self, role: usize) -> String {
        let (partner, required) = CONTRAST_PARTNERS[role];
        let ratio = contrast_ratio(
            self.appearance.colors[role],
            self.appearance.colors[partner],
        );
        let verdict = if ratio >= required { "AA" } else { "low" };
        format!("vs {} {ratio:.1}:1 {verdict}", short_role(partner))
    }

    /// Name used by "Save as custom skin": `custom-<base>` where `<base>` is
    /// the current skin without any existing `custom-` prefix or variant
    /// suffix, so repeated saves don't stack prefixes.
    fn custom_skin_name(&self) -> String {
        let base = self
            .current_skin
            .strip_prefix("custom-")
            .unwrap_or(&self.current_skin);
        format!("custom-{base}")
    }

    /// Ensure the Appearance palette reflects the currently active skin.
    fn ensure_appearance_palette(&mut self) {
        if self.appearance.loaded_for == self.current_skin {
            return;
        }
        let name = self.current_skin.clone();
        match resolve_skin(&name) {
            Ok(skin) => self.appearance.load_from_theme(&skin.theme, &name),
            Err(_) => {
                let default_theme = SkinTheme::default();
                self.appearance.load_from_theme(&default_theme, &name);
            },
        }
    }

    /// The theme to apply/save: the active skin's theme with the edited
    /// palette written over its 9 base colors.
    fn edited_theme(&self) -> SkinTheme {
        let mut theme = resolve_skin(&self.current_skin)
            .map(|s| s.theme)
            .unwrap_or_default();
        self.appearance.write_to_theme(&mut theme);
        theme
    }

    /// Handle Confirm on an Appearance row.
    fn appearance_confirm(&mut self) {
        if self.item_cursor < BASE_COLOR_LABELS.len() {
            // Enter edit mode on a color row.
            self.appearance.editing_channel = Some(0);
            self.appearance.edit_backup = self.appearance.colors[self.item_cursor];
            self.refresh_lines();
            self.sync_content_cursor();
            return;
        }
        let action = self.item_cursor - BASE_COLOR_LABELS.len();
        match action {
            // Apply (preview): send the edited theme for an in-memory swap.
            0 => {
                if let Ok(toml_doc) = self.edited_theme().to_toml_string() {
                    self.content.pending_vfs_request =
                        Some((SKIN_APPLY_THEME_REQUEST_PATH.to_string(), toml_doc));
                }
            },
            // Save as custom skin: the shell writes skins/<name>/ and swaps.
            1 => {
                if let Ok(toml_doc) = self.edited_theme().to_toml_string() {
                    let payload = format!("{}\n{toml_doc}", self.custom_skin_name());
                    self.content.pending_vfs_request =
                        Some((SKIN_SAVE_CUSTOM_REQUEST_PATH.to_string(), payload));
                }
            },
            // Variant rows: transform the local palette, then auto-preview so
            // the variant is immediately visible (and saveable afterwards).
            n => {
                if let Some(&variant) = SkinVariant::ALL.get(n - 2) {
                    let variant_theme = self.edited_theme().derive_variant(variant);
                    let loaded_for = self.appearance.loaded_for.clone();
                    self.appearance.load_from_theme(&variant_theme, &loaded_for);
                    if let Ok(toml_doc) = variant_theme.to_toml_string() {
                        self.content.pending_vfs_request =
                            Some((SKIN_APPLY_THEME_REQUEST_PATH.to_string(), toml_doc));
                    }
                    self.refresh_lines();
                }
            },
        }
    }

    /// Handle input while a color row is in edit mode. Returns the action to
    /// bubble up (always `None`; Cancel exits edit mode, not the app).
    fn handle_appearance_edit(&mut self, button: &Button) -> AppAction {
        let Some(channel) = self.appearance.editing_channel else {
            return AppAction::None;
        };
        let idx = self.item_cursor.min(BASE_COLOR_LABELS.len() - 1);
        match button {
            Button::Left => {
                self.appearance.editing_channel = Some((channel + 2) % 3);
            },
            Button::Right => {
                self.appearance.editing_channel = Some((channel + 1) % 3);
            },
            Button::Up | Button::Down => {
                let c = &mut self.appearance.colors[idx];
                let field = match channel {
                    0 => &mut c.r,
                    1 => &mut c.g,
                    _ => &mut c.b,
                };
                const STEP: u8 = 8;
                *field = if matches!(button, Button::Up) {
                    field.saturating_add(STEP)
                } else {
                    field.saturating_sub(STEP)
                };
            },
            Button::Confirm | Button::Triangle => {
                self.appearance.editing_channel = None;
            },
            Button::Cancel => {
                self.appearance.colors[idx] = self.appearance.edit_backup;
                self.appearance.editing_channel = None;
            },
            _ => return AppAction::None,
        }
        self.refresh_lines();
        self.sync_content_cursor();
        AppAction::None
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
            if self.category == Category::Appearance {
                // The shell swapped skins under us -- reload the palette so
                // the editor reflects the actually-running theme (unless the
                // swap was our own preview, which keeps the same skin name
                // and therefore doesn't reach this branch).
                self.ensure_appearance_palette();
            }
            if self.category == Category::Display {
                let names = builtin_names();
                if let Some(idx) = names.iter().position(|n| *n == self.current_skin) {
                    self.item_cursor = idx;
                } else {
                    // Shell published a skin not in the builtin list (e.g.
                    // external TOML). Keep the cursor in bounds so navigation
                    // and `handle_confirm` stay safe even if the builtin list
                    // shrinks between syncs.
                    let max = names.len().saturating_sub(1);
                    if self.item_cursor > max {
                        self.item_cursor = max;
                    }
                }
            }
            changed = true;
        }

        if let Some((w, h)) =
            read_utf8(vfs, RESOLUTION_STATE_PATH).and_then(|s| parse_resolution(&s))
            && (w != self.width || h != self.height)
        {
            self.width = w;
            self.height = h;
            if self.category == Category::Resolution {
                if let Some(idx) = self.current_resolution_index() {
                    self.item_cursor = idx;
                } else {
                    let max = RESOLUTION_PRESETS.len().saturating_sub(1);
                    if self.item_cursor > max {
                        self.item_cursor = max;
                    }
                }
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
        if !matches!(
            self.category,
            Category::Display | Category::Appearance | Category::Resolution
        ) {
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
            Category::Appearance => BASE_COLOR_LABELS.len() + APPEARANCE_ACTIONS,
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
        self.enter_category(Category::ALL[next]);
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
        self.enter_category(Category::ALL[prev]);
    }

    /// Common category-switch bookkeeping.
    fn enter_category(&mut self, category: Category) {
        // Leaving the Appearance editor always drops edit mode.
        self.appearance.editing_channel = None;
        self.category = category;
        if category == Category::Appearance {
            self.ensure_appearance_palette();
        }
        self.item_cursor = self.cursor_for_category(category);
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
        self.sync_content_cursor();
    }

    /// Handle confirm action in the current category.
    fn handle_confirm(&mut self) {
        match self.category {
            Category::Appearance => self.appearance_confirm(),
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
            Category::Resolution if self.item_cursor < RESOLUTION_PRESETS.len() => {
                let (w, h) = RESOLUTION_PRESETS[self.item_cursor];
                if w != self.width || h != self.height {
                    self.content.pending_vfs_request = Some((
                        RESOLUTION_CHANGE_REQUEST_PATH.to_string(),
                        format!("{w}x{h}"),
                    ));
                }
            },
            _ => {},
        }
    }

    /// Re-color the generic SDI objects created by `render_app_chrome` /
    /// `render_content_sdi` with the per-app palette. When no
    /// `[app_themes.settings]` overrides exist, [`SettingsColors::from_theme`]
    /// returns exactly the theme values the shared renderer already applied,
    /// so this pass is a visual no-op.
    fn apply_sdi_colors(&self, sdi: &mut SdiRegistry, colors: &SettingsColors) {
        if let Ok(obj) = sdi.get_mut("app_bg") {
            obj.color = colors.bg;
        }
        if let Ok(obj) = sdi.get_mut("app_title_bg") {
            obj.color = colors.title_bar_bg;
        }
        if let Ok(obj) = sdi.get_mut("app_title_text") {
            obj.text_color = colors.title_bar_text;
        }
        if let Ok(obj) = sdi.get_mut("app_sel_bg") {
            obj.color = colors.selected_bg;
        }
        if let Ok(obj) = sdi.get_mut("app_sel_accent") {
            obj.color = colors.selection_accent;
        }
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            obj.text_color = colors.dim_text;
        }
        // Line objects: same 100-object cap as `hide_app_sdi`.
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.text_color = if i == self.content.cursor {
                    colors.selected_text
                } else {
                    colors.text
                };
            }
        }
    }
}

impl App for SettingsApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Same sequence as `impl_content_app_methods!`, followed by a
        // per-app recolor pass driven by `[app_themes.settings]`.
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
        self.apply_sdi_colors(sdi, &SettingsColors::from_theme(at));
    }

    /// Windowed renderer. Mirrors
    /// `oasis_app_core::render::draw_content_windowed` line-for-line, but
    /// sources colors from [`SettingsColors`] so skins can restyle the
    /// Settings window via `[app_themes.settings]`. Keep the layout metrics
    /// in sync with the shared renderer (and with `handle_click`).
    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = SettingsColors::from_theme(at);
        let content = &self.content;

        // Title row. Settings never sets `browse_dir`/`viewing_file`, so the
        // generic renderer's directory suffix is always empty here.
        backend.draw_text(&content.title, cx + 4, cy + 2, 12, colors.title_bar_text)?;

        // Separator.
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            colors.divider,
        )?;

        // Content lines.
        let line_h = at.terminal_line_height.max(12) as i32;
        let max_lines = ((ch as i32 - line_h - 4) / line_h).max(0) as usize;
        let visible = content
            .lines
            .len()
            .saturating_sub(content.scroll)
            .min(max_lines);
        for i in 0..visible {
            let line_idx = content.scroll + i;
            let line = &content.lines[line_idx];
            let prefix = if i == content.cursor { "> " } else { "  " };
            let text = format!("{prefix}{line}");
            let text_color = if i == content.cursor {
                colors.selected_text
            } else {
                colors.text
            };
            let y = cy + at.app.title_bar_height as i32 + i as i32 * line_h;
            backend.draw_text(&text, cx + 4, y, 12, text_color)?;
        }

        // Scroll indicator.
        let scroll_text = if content.lines.len() > max_lines {
            format!(
                "[{}/{}]  Cancel=back",
                content.scroll + 1,
                content.lines.len().saturating_sub(max_lines) + 1,
            )
        } else {
            "Cancel=back".to_string()
        };
        let scroll_y = cy + ch as i32 - 14;
        backend.draw_text(&scroll_text, cx + 4, scroll_y, 10, colors.dim_text)?;

        Ok(())
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.content.pending_vfs_request.take()
    }

    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.content.pending_vfs_request.as_ref()
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        // Always check for shell-published state updates first so the display
        // catches up before we interpret the input. This is also what makes
        // the "* currently active" marker flip as soon as the shell applies
        // a pending change.
        self.sync_from_vfs(vfs);

        // Color-edit mode captures all input (Left/Right cycle channels
        // instead of switching categories; Cancel reverts instead of
        // exiting the app).
        if self.category == Category::Appearance && self.appearance.editing_channel.is_some() {
            return self.handle_appearance_edit(button);
        }

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
                    Category::Display | Category::Appearance | Category::Resolution => {
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
                    Category::Display | Category::Appearance | Category::Resolution => {
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
        // same metrics the renderer uses for `draw_content_windowed`:
        // `title_bar_height + line_idx * line_h`. Both values are cached on
        // `ContentState` by `update_layout` each frame so they stay in sync
        // with the active skin's theme rather than being hardcoded here.
        let title_bar_height = self.content.cached_title_bar_height as i32;
        let line_h = self.content.cached_line_h.max(1) as i32;

        let y_in_content = ly - title_bar_height;
        if y_in_content < 0 {
            return AppAction::None;
        }
        let visible_idx = (y_in_content / line_h) as usize;
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
            Category::Appearance if item_idx < self.item_count() => {
                self.item_cursor = item_idx;
                self.handle_confirm();
                self.refresh_lines();
                self.sync_content_cursor();
            },
            Category::Resolution if item_idx < RESOLUTION_PRESETS.len() => {
                self.item_cursor = item_idx;
                self.handle_confirm();
                self.refresh_lines();
                self.sync_content_cursor();
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
///
/// Shared between the Settings app (for reading state payloads) and the
/// shell runner in `oasis-app` (for dispatching IPC resolution-change
/// requests). Keeping a single copy prevents the two sides from drifting.
pub fn parse_resolution(s: &str) -> Option<(u32, u32)> {
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
        assert_eq!(app.category, Category::Appearance);
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
        for _ in 0..3 {
            // Display -> Appearance -> Resolution -> Audio
            app.handle_input(&Button::Right, &vfs);
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
        for _ in 0..3 {
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
        for _ in 0..3 {
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
        for _ in 0..3 {
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
        for _ in 0..4 {
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
        for _ in 0..5 {
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
        for _ in 0..3 {
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
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Resolution);
        // 1280x720 is index 3 in RESOLUTION_PRESETS.
        assert_eq!(app.item_cursor, 3);
    }

    #[test]
    fn confirm_resolution_writes_request() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Display -> Appearance -> Resolution.
        app.handle_input(&Button::Right, &vfs);
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

    // -- Per-app color theming --

    #[test]
    fn update_sdi_defaults_match_shared_renderer() {
        // Without [app_themes.settings], the recolor pass must leave every
        // object exactly as the shared renderer set it.
        let mut app = make_app();
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        app.update_sdi(&mut sdi, &at);

        let bg = sdi.get("app_bg").expect("app_bg exists");
        assert_eq!(bg.color, at.app.bg);
        let sel = sdi.get("app_sel_bg").expect("app_sel_bg exists");
        assert_eq!(sel.color, at.app.selected_bg);
        // The cursor starts on the active skin row (ITEMS_START + 0), so
        // that line gets selected_text and line 0 gets the normal color.
        let line = sdi.get("app_line_0").expect("app_line_0 exists");
        assert_eq!(line.text_color, at.app.text);
        let cursor_name = format!("app_line_{}", app.content.cursor);
        let cursor_line = sdi.get(&cursor_name).expect("cursor line exists");
        assert_eq!(cursor_line.text_color, at.app.selected_text);
    }

    #[test]
    fn update_sdi_applies_settings_overrides() {
        use oasis_types::backend::Color;

        let mut app = make_app();
        let mut sdi = SdiRegistry::new();
        let mut at = ActiveTheme::default();
        let bg = Color::rgba(1, 2, 3, 255);
        let text = Color::rgba(4, 5, 6, 255);
        let overrides = at.app_themes.entry("settings".to_string()).or_default();
        overrides.insert("bg".to_string(), bg);
        overrides.insert("text".to_string(), text);

        app.update_sdi(&mut sdi, &at);

        let obj = sdi.get("app_bg").expect("app_bg exists");
        assert_eq!(obj.color, bg);
        let line = sdi.get("app_line_1").expect("app_line_1 exists");
        assert_eq!(line.text_color, text);
        // Slots without overrides keep the theme default.
        let title = sdi.get("app_title_bg").expect("app_title_bg exists");
        assert_eq!(title.color, at.app.title_bar_bg);
    }

    // -- Appearance editor --

    /// Navigate a fresh app to the Appearance category.
    fn appearance_app(vfs: &MemoryVfs) -> SettingsApp {
        let mut app = make_app();
        app.handle_input(&Button::Right, vfs);
        assert_eq!(app.category, Category::Appearance);
        app
    }

    #[test]
    fn appearance_lists_base_colors_and_actions() {
        let vfs = make_vfs();
        let app = appearance_app(&vfs);
        let lines = app.lines();
        for label in BASE_COLOR_LABELS {
            assert!(
                lines.iter().any(|l| l.contains(label)),
                "missing color row {label}"
            );
        }
        assert!(lines.iter().any(|l| l.contains("Apply")));
        assert!(lines.iter().any(|l| l.contains("custom-classic")));
        assert!(lines.iter().any(|l| l.contains("Variant: Dark")));
        assert!(lines.iter().any(|l| l.contains("Variant: High Contrast")));
    }

    #[test]
    fn appearance_rows_show_contrast_readout() {
        let vfs = make_vfs();
        let app = appearance_app(&vfs);
        let lines = app.lines();
        // Every base-color row carries a "vs <partner> N.N:1 <verdict>"
        // readout; the Text role is judged against the background.
        let text_row = lines
            .iter()
            .find(|l| l.trim_start().starts_with("Text ") && l.contains(":1"))
            .expect("Text row has a contrast readout");
        assert!(
            text_row.contains("vs Bg"),
            "Text judged vs background: {text_row}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains(" AA") || l.contains(" low")),
            "no contrast verdicts rendered"
        );
    }

    #[test]
    fn contrast_readout_flags_low_contrast() {
        let app = SettingsApp::new("/apps/settings", "classic", 480, 272, "SDL3");
        // Background (role 0) is judged against Text (role 3).
        let readout = app.contrast_readout(0);
        assert!(
            readout.starts_with("vs Text"),
            "unexpected partner: {readout}"
        );
        assert!(
            readout.contains("AA") || readout.contains("low"),
            "missing verdict: {readout}"
        );
    }

    #[test]
    fn appearance_palette_matches_current_skin() {
        let vfs = make_vfs();
        let app = appearance_app(&vfs);
        let classic = resolve_skin("classic").expect("classic resolves");
        let expected = parse_hex_color(&classic.theme.background).expect("valid hex");
        assert_eq!(app.appearance.colors[0], expected);
    }

    #[test]
    fn appearance_edit_adjusts_channel() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        let before = app.appearance.colors[0];
        // Enter edit mode on Background, bump R by one step, commit.
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.appearance.editing_channel.is_some());
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.appearance.editing_channel.is_none());
        assert_eq!(app.appearance.colors[0].r, before.r.saturating_add(8));
        assert_eq!(app.appearance.colors[0].g, before.g);
    }

    #[test]
    fn appearance_edit_cancel_reverts() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        let before = app.appearance.colors[0];
        app.handle_input(&Button::Confirm, &vfs);
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Up, &vfs);
        // Cancel exits edit mode and restores the color (not the app).
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert!(app.appearance.editing_channel.is_none());
        assert_eq!(app.appearance.colors[0], before);
    }

    #[test]
    fn appearance_edit_captures_left_right() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        app.handle_input(&Button::Confirm, &vfs);
        // Left/Right cycle channels instead of switching categories.
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Appearance);
        assert_eq!(app.appearance.editing_channel, Some(1));
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.appearance.editing_channel, Some(0));
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.appearance.editing_channel, Some(2));
    }

    #[test]
    fn appearance_apply_posts_theme_toml() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        // Move to the "Apply" row (first action after the 9 colors).
        for _ in 0..BASE_COLOR_LABELS.len() {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);
        let (path, payload) = app.take_pending_request().expect("apply posts IPC");
        assert_eq!(path, SKIN_APPLY_THEME_REQUEST_PATH);
        let theme = SkinTheme::from_toml_str(&payload).expect("payload is a valid theme");
        assert_eq!(
            parse_hex_color(&theme.background),
            Some(app.appearance.colors[0])
        );
    }

    #[test]
    fn appearance_save_posts_named_payload() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        for _ in 0..BASE_COLOR_LABELS.len() + 1 {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);
        let (path, payload) = app.take_pending_request().expect("save posts IPC");
        assert_eq!(path, SKIN_SAVE_CUSTOM_REQUEST_PATH);
        let (name, theme_toml) = payload.split_once('\n').expect("name line present");
        assert_eq!(name, "custom-classic");
        assert!(SkinTheme::from_toml_str(theme_toml).is_ok());
    }

    #[test]
    fn appearance_variant_transforms_palette_and_previews() {
        let vfs = make_vfs();
        let mut app = appearance_app(&vfs);
        // classic is dark; the Light variant row should flip the background
        // into the light half and immediately post a preview.
        let light_row = BASE_COLOR_LABELS.len() + 2 + 1; // Apply, Save, Dark, Light
        for _ in 0..light_row {
            app.handle_input(&Button::Down, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);
        let (path, _) = app.take_pending_request().expect("variant posts preview");
        assert_eq!(path, SKIN_APPLY_THEME_REQUEST_PATH);
        let bg = app.appearance.colors[0];
        let luma = 0.2126 * bg.r as f32 + 0.7152 * bg.g as f32 + 0.0722 * bg.b as f32;
        assert!(luma > 127.0, "light variant background not light: {bg:?}");
    }

    #[test]
    fn custom_skin_name_does_not_stack_prefix() {
        let app = SettingsApp::new("/apps/settings", "custom-classic", 480, 272, "SDL3");
        assert_eq!(app.custom_skin_name(), "custom-classic");
    }

    #[test]
    fn appearance_palette_reload_on_external_skin_change() {
        let mut vfs = MemoryVfs::new();
        let mut app = appearance_app(&vfs);
        let before = app.appearance.colors[0];
        // Shell swaps to paper (a light skin) behind our back.
        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(SKIN_STATE_PATH, b"paper").unwrap();
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.appearance.loaded_for, "paper");
        assert_ne!(app.appearance.colors[0], before);
    }
}

//! Settings application for OASIS_OS.
//!
//! Provides a categorised settings screen with skin selection, resolution
//! switching, audio volume, interface language, accessibility options and
//! system/about details. Changes are published back to the shell through
//! VFS IPC paths; the shell applies them live, persists them to
//! `/system/settings.toml` and publishes the resulting state under
//! `/system/state/*`, which this app reads back.

use std::cell::Cell;

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_i18n::Locale;
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_skin::builtin::builtin_names;
use oasis_skin::theme::{contrast_ratio, parse_hex_color};
use oasis_skin::{SkinTheme, SkinVariant, resolve_skin};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

mod colors;
mod layout;
mod render;
mod rows;

pub use colors::SettingsColors;

use layout::SettingsLayout;
use rows::{Row, row_of_item, scroll_for};

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

/// VFS IPC path used to set the master volume. Payload: `0`-`100`.
pub const VOLUME_CHANGE_REQUEST_PATH: &str = "/system/ipc/volume";

/// VFS IPC path used to select the UI locale. Payload: a locale code
/// (`"en"`, `"de"`, ...).
pub const LOCALE_CHANGE_REQUEST_PATH: &str = "/system/ipc/locale";

/// VFS IPC path used to set the font scale. Payload: a float such as
/// `"1.25"`.
pub const FONT_SCALE_REQUEST_PATH: &str = "/system/ipc/font-scale";

/// VFS IPC path used to switch reduced motion. Payload: `"1"` or `"0"`.
pub const REDUCED_MOTION_REQUEST_PATH: &str = "/system/ipc/reduced-motion";

/// VFS IPC path for the high-contrast shortcut. Payload: `"on"` (swap to
/// [`HIGH_CONTRAST_SKIN`], remembering the current skin) or `"off"` (swap
/// back to the remembered skin).
pub const HIGH_CONTRAST_REQUEST_PATH: &str = "/system/ipc/high-contrast";

/// VFS path where the shell publishes the currently active skin name.
pub const SKIN_STATE_PATH: &str = "/system/state/skin";

/// VFS path where the shell publishes the current resolution (`"WxH"`).
pub const RESOLUTION_STATE_PATH: &str = "/system/state/resolution";

/// VFS path where the shell publishes the current backend name.
pub const BACKEND_STATE_PATH: &str = "/system/state/backend";

/// VFS path where the shell publishes the master volume (`0`-`100`).
pub const VOLUME_STATE_PATH: &str = "/system/state/volume";

/// VFS path where the shell publishes the selected locale code.
pub const LOCALE_STATE_PATH: &str = "/system/state/locale";

/// VFS path where the shell publishes the font scale.
pub const FONT_SCALE_STATE_PATH: &str = "/system/state/font-scale";

/// VFS path where the shell publishes the reduced-motion switch (`1`/`0`).
pub const REDUCED_MOTION_STATE_PATH: &str = "/system/state/reduced-motion";

/// Built-in skin used by the Accessibility high-contrast shortcut.
pub const HIGH_CONTRAST_SKIN: &str = "highcontrast";

/// Font scale steps offered by the Accessibility slider.
pub const FONT_SCALE_PRESETS: &[f32] = &[0.75, 1.0, 1.25, 1.5];

/// Volume change per Up/Down press.
const VOLUME_STEP: u32 = 5;

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
    Language,
    Accessibility,
    System,
    About,
}

impl Category {
    const ALL: [Category; 8] = [
        Category::Display,
        Category::Appearance,
        Category::Resolution,
        Category::Audio,
        Category::Language,
        Category::Accessibility,
        Category::System,
        Category::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Display => "Display",
            Category::Appearance => "Appearance",
            Category::Resolution => "Resolution",
            Category::Audio => "Audio",
            Category::Language => "Language",
            Category::Accessibility => "Accessibility",
            Category::System => "System",
            Category::About => "About",
        }
    }

    /// Whether Up/Down move a selection cursor over items (as opposed to
    /// adjusting a value or scrolling text).
    fn has_items(self) -> bool {
        !matches!(self, Category::Audio | Category::System | Category::About)
    }
}

/// Selectable items of the Accessibility category.
const A11Y_FONT_SCALE: usize = 0;
const A11Y_HIGH_CONTRAST: usize = 1;
const A11Y_REDUCED_MOTION: usize = 2;
const A11Y_ITEMS: usize = 3;

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

/// Last raw values read from the shell-published preference state paths.
///
/// Preference controls (volume, font scale, reduced motion) update the UI
/// optimistically and post a request; the shell applies it a frame later.
/// Syncing only when a state file *changes* keeps a stale published value
/// from snapping the control back in the meantime.
#[derive(Debug, Default)]
struct SeenState {
    volume: Option<String>,
    locale: Option<String>,
    font_scale: Option<String>,
    reduced_motion: Option<String>,
}

/// Font metrics of the last windowed draw, reused by click hit-testing.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    font_body: u16,
    font_hint: u16,
}

impl Default for Metrics {
    fn default() -> Self {
        let at = ActiveTheme::default();
        Self {
            font_body: at.font_body,
            font_hint: at.font_hint,
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
    /// Selected UI locale.
    locale: Locale,
    /// Font scale multiplier.
    font_scale: f32,
    /// Reduced-motion accessibility switch.
    reduced_motion: bool,
    /// Scroll offset for the text-only categories (System / About) in the
    /// windowed renderer.
    text_scroll: usize,
    /// Last seen shell-published preference state.
    seen: SeenState,
    /// Font metrics of the last windowed draw (click hit-testing).
    metrics: Cell<Metrics>,
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
            locale: Locale::English,
            font_scale: 1.0,
            reduced_motion: false,
            text_scroll: 0,
            seen: SeenState::default(),
            metrics: Cell::new(Metrics::default()),
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
        let mut app = Self::new(path, &skin, width, height, &backend);
        if app.sync_prefs(vfs) {
            app.refresh_lines();
        }
        app
    }

    /// Index into [`RESOLUTION_PRESETS`] for the currently active resolution,
    /// or `None` if no preset matches exactly.
    fn current_resolution_index(&self) -> Option<usize> {
        RESOLUTION_PRESETS
            .iter()
            .position(|(w, h)| *w == self.width && *h == self.height)
    }

    /// Whether the high-contrast shortcut is currently on.
    fn high_contrast(&self) -> bool {
        self.current_skin == HIGH_CONTRAST_SKIN
    }

    // -- Row model ---------------------------------------------------------

    /// Body rows of the current category.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        match self.category {
            Category::Display => self.display_rows(&mut rows),
            Category::Appearance => self.appearance_rows(&mut rows),
            Category::Resolution => self.resolution_rows(&mut rows),
            Category::Audio => self.audio_rows(&mut rows),
            Category::Language => self.language_rows(&mut rows),
            Category::Accessibility => self.accessibility_rows(&mut rows),
            Category::System => self.system_rows(&mut rows),
            Category::About => self.about_rows(&mut rows),
        }
        rows
    }

    /// Build display lines for the current category (fullscreen SDI path
    /// and [`App::lines`]).
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
        lines.extend(self.rows().iter().map(Row::to_line));
        lines.push(sep);
        lines.push(String::new());
        lines.push("  [L/R]=Category  [U/D]=Navigate".to_string());
        lines.push("  [Confirm]=Apply  [Cancel]=Exit".to_string());
        lines
    }

    /// Rows for the Display category.
    fn display_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Skin Selection".to_string()));
        rows.push(Row::Blank);
        for (i, name) in builtin_names().iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: (*name).to_string(),
                active: *name == self.current_skin,
            });
        }
        rows.push(Row::Blank);
        rows.push(Row::Text(format!(
            "Resolution: {} x {}",
            self.width, self.height
        )));
    }

    /// Rows for the Appearance category (base-color editor).
    ///
    /// Items are the 9 base colors followed by the action rows (Apply /
    /// Save / variants), with no gaps so `item_cursor` maps 1:1 to rows.
    fn appearance_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Appearance - Base Colors".to_string()));
        rows.push(Row::Blank);

        for (i, label) in BASE_COLOR_LABELS.iter().enumerate() {
            let c = self.appearance.colors[i];
            let readout = self.contrast_readout(i);
            let text = if self.item_cursor == i && self.appearance.editing_channel.is_some() {
                let ch = self.appearance.editing_channel.unwrap_or(0);
                let mark = |idx: u8, name: char, v: u8| {
                    if ch == idx {
                        format!("[{name}:{v:3}]")
                    } else {
                        format!(" {name}:{v:3} ")
                    }
                };
                format!(
                    "{label:<11} {} {}{}{} {readout}",
                    hex(c),
                    mark(0, 'R', c.r),
                    mark(1, 'G', c.g),
                    mark(2, 'B', c.b),
                )
            } else {
                format!("{label:<11} {}  {readout}", hex(c))
            };
            rows.push(Row::Item {
                item: i,
                label: text,
                active: false,
            });
        }

        let n = BASE_COLOR_LABELS.len();
        let mut actions = vec![
            "[ Apply (preview) ]".to_string(),
            format!("[ Save as '{}' ]", self.custom_skin_name()),
        ];
        actions.extend(
            SkinVariant::ALL
                .iter()
                .map(|v| format!("[ Variant: {} ]", v.label())),
        );
        for (i, label) in actions.into_iter().enumerate() {
            rows.push(Row::Item {
                item: n + i,
                label,
                active: false,
            });
        }

        rows.push(Row::Blank);
        if self.appearance.editing_channel.is_some() {
            rows.push(Row::Text("[U/D]=Value  [L/R]=Channel".to_string()));
            rows.push(Row::Text("[Confirm]=Done  [Cancel]=Revert".to_string()));
        } else {
            rows.push(Row::Text("[Confirm]=Edit color / activate".to_string()));
            rows.push(Row::Text(
                "AA = passes WCAG contrast, low = below".to_string(),
            ));
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
                    self.post(SKIN_APPLY_THEME_REQUEST_PATH, toml_doc);
                }
            },
            // Save as custom skin: the shell writes skins/<name>/ and swaps.
            1 => {
                if let Ok(toml_doc) = self.edited_theme().to_toml_string() {
                    let payload = format!("{}\n{toml_doc}", self.custom_skin_name());
                    self.post(SKIN_SAVE_CUSTOM_REQUEST_PATH, payload);
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
                        self.post(SKIN_APPLY_THEME_REQUEST_PATH, toml_doc);
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

    /// Rows for the Resolution category.
    fn resolution_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Virtual Resolution".to_string()));
        rows.push(Row::Blank);
        for (i, (w, h)) in RESOLUTION_PRESETS.iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: format!("{w}x{h}  {}", preset_label(*w, *h)),
                active: *w == self.width && *h == self.height,
            });
        }
        rows.push(Row::Blank);
        rows.push(Row::Text("Window + layout resize live.".to_string()));
    }

    /// Rows for the Audio category.
    fn audio_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Audio Settings".to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Slider {
            item: 0,
            label: "Volume".to_string(),
            value: self.volume as f32,
            min: 0.0,
            max: 100.0,
            value_text: format!("{}%", self.volume),
        });
        rows.push(Row::Blank);
        rows.push(Row::Text("[U/D] or +/- = Adjust volume".to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text("Audio Output:  Default".to_string()));
        rows.push(Row::Text("Sample Rate:   44100 Hz".to_string()));
        rows.push(Row::Text("Channels:      Stereo".to_string()));
    }

    /// Rows for the Language category.
    fn language_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Interface Language".to_string()));
        rows.push(Row::Blank);
        for (i, locale) in Locale::all().iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: locale_label(*locale),
                active: *locale == self.locale,
            });
        }
        rows.push(Row::Blank);
        if !oasis_i18n::bitmap_font_supports(self.locale) {
            rows.push(Row::Text(
                "Font lacks these glyphs: UI stays English.".to_string(),
            ));
        }
    }

    /// Rows for the Accessibility category.
    fn accessibility_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("Accessibility".to_string()));
        rows.push(Row::Blank);
        let (min, max) = font_scale_range();
        rows.push(Row::Slider {
            item: A11Y_FONT_SCALE,
            label: "Font Scale".to_string(),
            value: self.font_scale,
            min,
            max,
            value_text: format!("{:.0}%", self.font_scale * 100.0),
        });
        rows.push(Row::Toggle {
            item: A11Y_HIGH_CONTRAST,
            label: "High Contrast".to_string(),
            on: self.high_contrast(),
        });
        rows.push(Row::Toggle {
            item: A11Y_REDUCED_MOTION,
            label: "Reduced Motion".to_string(),
            on: self.reduced_motion,
        });
        rows.push(Row::Blank);
        rows.push(Row::Text(
            "[Confirm]=Change  [Square]/-=Smaller".to_string(),
        ));
    }

    /// Rows for the System category.
    fn system_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("System Information".to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text(format!("Backend:       {}", self.backend_name)));
        rows.push(Row::Text(format!(
            "Resolution:    {} x {}",
            self.width, self.height
        )));
        rows.push(Row::Text(format!("Active Skin:   {}", self.current_skin)));
        rows.push(Row::Text(format!(
            "Version:       {}",
            env!("CARGO_PKG_VERSION")
        )));
        rows.push(Row::Blank);
        rows.push(Row::Text("VFS:           MemoryVfs".to_string()));
        rows.push(Row::Text("Rust Edition:  2024".to_string()));
        rows.push(Row::Text("MSRV:          1.91.0".to_string()));
    }

    /// Rows for the About category.
    fn about_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading("About OASIS_OS".to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text(format!(
            "Version:    {}",
            env!("CARGO_PKG_VERSION")
        )));
        for text in [
            "License:    MIT / Unlicense",
            "Crates:     20 workspace crates",
            "Apps:       16 built-in",
            "Skins:      18 built-in",
        ] {
            rows.push(Row::Text(text.to_string()));
        }
        rows.push(Row::Blank);
        for text in [
            "An embeddable operating system",
            "framework originally ported from",
            "Inspired by PSP homebrew (PSIX).",
        ] {
            rows.push(Row::Text(text.to_string()));
        }
        rows.push(Row::Blank);
        rows.push(Row::Text("github.com/AndrewAltimit/oasis-os".to_string()));
    }

    // -- Shell state sync --------------------------------------------------

    /// Re-read the shell-published state from VFS. Called on input and on
    /// every tick so the UI reflects changes applied by the shell after we
    /// posted a request. Returns `true` when anything changed.
    fn sync_from_vfs(&mut self, vfs: &dyn Vfs) -> bool {
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

        changed |= self.sync_prefs(vfs);

        if changed {
            self.refresh_lines();
            self.sync_content_cursor();
        }
        changed
    }

    /// Adopt shell-published preference values that changed since the last
    /// sync (see [`SeenState`]). Returns `true` when anything changed.
    fn sync_prefs(&mut self, vfs: &dyn Vfs) -> bool {
        let mut changed = false;
        if let Some(v) = read_changed(vfs, VOLUME_STATE_PATH, &mut self.seen.volume)
            .and_then(|s| s.parse::<u32>().ok())
        {
            changed |= v.min(100) != self.volume;
            self.volume = v.min(100);
        }
        if let Some(l) = read_changed(vfs, LOCALE_STATE_PATH, &mut self.seen.locale)
            .and_then(|s| Locale::from_code(&s))
        {
            changed |= l != self.locale;
            self.locale = l;
        }
        if let Some(f) = read_changed(vfs, FONT_SCALE_STATE_PATH, &mut self.seen.font_scale)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|f| f.is_finite())
        {
            changed |= (f - self.font_scale).abs() > f32::EPSILON;
            self.font_scale = f;
        }
        if let Some(on) = read_changed(
            vfs,
            REDUCED_MOTION_STATE_PATH,
            &mut self.seen.reduced_motion,
        )
        .map(|s| parse_bool(&s))
        {
            changed |= on != self.reduced_motion;
            self.reduced_motion = on;
        }
        changed
    }

    /// Rebuild display lines from current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.build_lines();
    }

    /// First content-line index where selectable items begin in the list
    /// categories. Layout is:
    ///   0 = tab header
    ///   1 = separator
    ///   2 = section heading
    ///   3 = blank
    ///   4 = first item
    /// Every list category's rows start with heading + blank, and
    /// [`Self::handle_click`] uses this to map fullscreen clicks back to
    /// item indices.
    const ITEMS_START: usize = 4;

    /// Align `content.cursor` (and scroll, if needed) with the active item
    /// so the single `>` prefix drawn by the fullscreen renderer lands on
    /// the item `item_cursor` points at. Only meaningful for categories
    /// that actually have a list of selectable items — scrollable text
    /// categories leave content.cursor alone.
    fn sync_content_cursor(&mut self) {
        if !self.category.has_items() && self.category != Category::Audio {
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
            Category::Language => Locale::all().len(),
            Category::Accessibility => A11Y_ITEMS,
            // The Audio volume slider is the category's only control;
            // Up/Down adjust it directly.
            Category::Audio => 1,
            Category::System | Category::About => 0,
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
            Category::Language => Locale::all()
                .iter()
                .position(|l| *l == self.locale)
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Switch to the next category (right).
    fn next_category(&mut self) {
        let idx = self.category_index();
        let next = (idx + 1) % Category::ALL.len();
        self.enter_category(Category::ALL[next]);
    }

    /// Switch to the previous category (left).
    fn prev_category(&mut self) {
        let idx = self.category_index();
        let prev = if idx == 0 {
            Category::ALL.len() - 1
        } else {
            idx - 1
        };
        self.enter_category(Category::ALL[prev]);
    }

    fn category_index(&self) -> usize {
        Category::ALL
            .iter()
            .position(|c| *c == self.category)
            .unwrap_or(0)
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
        self.text_scroll = 0;
        self.content.scroll = 0;
        self.content.cursor = 0;
        self.refresh_lines();
        self.sync_content_cursor();
    }

    /// Queue an IPC request for the shell.
    fn post(&mut self, path: &str, payload: String) {
        self.content.pending_vfs_request = Some((path.to_string(), payload));
    }

    /// Set the volume (clamped) and ask the shell to apply it.
    fn set_volume(&mut self, volume: u32) {
        let volume = volume.min(100);
        if volume == self.volume {
            return;
        }
        self.volume = volume;
        self.post(VOLUME_CHANGE_REQUEST_PATH, volume.to_string());
        self.refresh_lines();
    }

    /// Set the font scale (clamped to the preset range) and ask the shell
    /// to apply it.
    fn set_font_scale(&mut self, scale: f32) {
        let (min, max) = font_scale_range();
        let scale = scale.clamp(min, max);
        if (scale - self.font_scale).abs() < f32::EPSILON {
            return;
        }
        self.font_scale = scale;
        self.post(FONT_SCALE_REQUEST_PATH, format!("{scale}"));
        self.refresh_lines();
    }

    /// Step the font scale one preset up (`up`) or down, clamping at the
    /// ends. `wrap` wraps from the largest back to the smallest (Confirm
    /// cycles through the presets).
    fn step_font_scale(&mut self, up: bool, wrap: bool) {
        let n = FONT_SCALE_PRESETS.len();
        let current = nearest_preset(self.font_scale);
        let next = if up {
            if current + 1 < n {
                current + 1
            } else if wrap {
                0
            } else {
                current
            }
        } else {
            current.saturating_sub(1)
        };
        self.set_font_scale(FONT_SCALE_PRESETS[next]);
    }

    /// Toggle reduced motion and ask the shell to apply it.
    fn toggle_reduced_motion(&mut self) {
        self.reduced_motion = !self.reduced_motion;
        let payload = if self.reduced_motion { "1" } else { "0" };
        self.post(REDUCED_MOTION_REQUEST_PATH, payload.to_string());
        self.refresh_lines();
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
                        self.post(SKIN_CHANGE_REQUEST_PATH, selected.to_string());
                    }
                }
            },
            Category::Resolution if self.item_cursor < RESOLUTION_PRESETS.len() => {
                let (w, h) = RESOLUTION_PRESETS[self.item_cursor];
                if w != self.width || h != self.height {
                    self.post(RESOLUTION_CHANGE_REQUEST_PATH, format!("{w}x{h}"));
                }
            },
            Category::Language => {
                if let Some(&locale) = Locale::all().get(self.item_cursor)
                    && locale != self.locale
                {
                    // Like skins, wait for the shell to publish the applied
                    // locale before marking it active.
                    self.post(LOCALE_CHANGE_REQUEST_PATH, locale.code().to_string());
                }
            },
            Category::Accessibility => match self.item_cursor {
                A11Y_FONT_SCALE => self.step_font_scale(true, true),
                A11Y_HIGH_CONTRAST => {
                    let payload = if self.high_contrast() { "off" } else { "on" };
                    self.post(HIGH_CONTRAST_REQUEST_PATH, payload.to_string());
                },
                A11Y_REDUCED_MOTION => self.toggle_reduced_motion(),
                _ => {},
            },
            _ => {},
        }
    }

    /// Adjust the current category's slider by one step. Returns `true`
    /// when the current row is a slider.
    fn adjust_slider(&mut self, up: bool) -> bool {
        match self.category {
            Category::Audio => {
                let v = if up {
                    self.volume + VOLUME_STEP
                } else {
                    self.volume.saturating_sub(VOLUME_STEP)
                };
                self.set_volume(v);
                true
            },
            Category::Accessibility if self.item_cursor == A11Y_FONT_SCALE => {
                self.step_font_scale(up, false);
                true
            },
            _ => false,
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

    /// Windowed layout for a content rect of the given size, using the
    /// font metrics of the last draw.
    fn layout(&self, cx: i32, cy: i32, cw: u32, ch: u32) -> SettingsLayout {
        let m = self.metrics.get();
        SettingsLayout::compute(
            cx,
            cy,
            cw,
            ch,
            Category::ALL.len(),
            m.font_body,
            m.font_hint,
        )
    }

    /// First visible body row in the windowed renderer.
    fn body_scroll(&self, rows: &[Row], visible: usize) -> usize {
        let focus = if self.category.has_items() || self.category == Category::Audio {
            row_of_item(rows, self.item_cursor)
        } else {
            None
        };
        scroll_for(rows.len(), visible, focus, self.text_scroll)
    }

    /// Windowed click: tabs switch category, rows select + activate,
    /// slider tracks set the value at the click position.
    fn handle_windowed_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32) {
        let l = self.layout(0, 0, cw, ch);
        if let Some(i) = l.tabs.iter().position(|t| t.contains(lx, ly)) {
            if Category::ALL[i] != self.category {
                self.enter_category(Category::ALL[i]);
            }
            return;
        }
        let Some(vis) = l.row_at(lx, ly) else {
            return;
        };
        let rows = self.rows();
        let scroll = self.body_scroll(&rows, l.visible_rows());
        let Some(row) = rows.get(scroll + vis) else {
            return;
        };
        let Some(item) = row.item() else {
            return;
        };
        if self.category == Category::Appearance && self.appearance.editing_channel.is_some() {
            // A click elsewhere commits the edit in progress.
            self.appearance.editing_channel = None;
        }
        self.item_cursor = item;
        match row {
            Row::Slider { min, max, .. } => {
                let track = l.control_rect(l.row_rect(vis));
                if track.contains(lx, ly) {
                    let frac = (lx - track.x) as f32 / track.w.max(1) as f32;
                    let value = min + (max - min) * frac.clamp(0.0, 1.0);
                    match self.category {
                        Category::Audio => {
                            let v = (value / VOLUME_STEP as f32).round() as u32 * VOLUME_STEP;
                            self.set_volume(v);
                        },
                        _ => self.set_font_scale(FONT_SCALE_PRESETS[nearest_preset(value)]),
                    }
                }
            },
            _ => self.handle_confirm(),
        }
        self.refresh_lines();
        self.sync_content_cursor();
    }

    /// Fullscreen click: map the Y coordinate to a content line using the
    /// shared content renderer's metrics (content starts at the shared
    /// `WINDOWED_TOP_PAD`, rows are `cached_line_h` tall).
    fn handle_fullscreen_click(&mut self, ly: i32) {
        let content_top = oasis_app_core::render::WINDOWED_TOP_PAD as i32;
        let line_h = self.content.cached_line_h.max(1) as i32;
        let y_in_content = ly - content_top;
        if y_in_content < 0 {
            return;
        }
        let line_idx = self.content.scroll + (y_in_content / line_h) as usize;
        if line_idx < Self::ITEMS_START || !self.category.has_items() {
            return;
        }
        let item_idx = line_idx - Self::ITEMS_START;
        if item_idx < self.item_count() {
            self.item_cursor = item_idx;
            self.handle_confirm();
            self.refresh_lines();
            self.sync_content_cursor();
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

    /// Windowed renderer: category tab strip, a row list drawn with
    /// oasis-ui widgets (Slider / Toggle) and a hint footer, all sized from
    /// the active theme's fonts (which carry the user's font scale).
    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        self.metrics.set(Metrics {
            font_body: at.font_body,
            font_hint: at.font_hint,
        });
        self.draw_settings(cx, cy, cw, ch, backend, at)
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

    fn tick(&mut self, _dt_ms: u32, vfs: &dyn Vfs) -> bool {
        self.sync_from_vfs(vfs)
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

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        if mods.has_command() {
            return None;
        }
        let up = match key {
            Key::Char('+' | '=') => true,
            Key::Char('-' | '_') => false,
            _ => return None,
        };
        self.adjust_slider(up).then_some(AppAction::None)
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

            Button::Up | Button::Down => {
                let up = matches!(button, Button::Up);
                match self.category {
                    Category::Audio => {
                        self.adjust_slider(up);
                    },
                    Category::System | Category::About => {
                        // Plain scrollable text — let the content cursor
                        // drive itself (fullscreen) and scroll the windowed
                        // row list.
                        if up {
                            self.content.navigate_up();
                            self.text_scroll = self.text_scroll.saturating_sub(1);
                        } else {
                            self.content.navigate_down();
                            let max = self.rows().len().saturating_sub(1);
                            self.text_scroll = (self.text_scroll + 1).min(max);
                        }
                    },
                    _ => {
                        let count = self.item_count();
                        if up && self.item_cursor > 0 {
                            self.item_cursor -= 1;
                        } else if !up && self.item_cursor + 1 < count {
                            self.item_cursor += 1;
                        } else {
                            return AppAction::None;
                        }
                        self.refresh_lines();
                        self.sync_content_cursor();
                    },
                }
                AppAction::None
            },

            // Square steps the focused slider down (font scale); Confirm
            // steps / cycles it up.
            Button::Square => {
                self.adjust_slider(false);
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

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, fullscreen: bool) -> AppAction {
        if fullscreen {
            self.handle_fullscreen_click(ly);
        } else {
            self.handle_windowed_click(lx, ly, cw, ch);
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

/// Read a state file and return its value only if it differs from the last
/// value seen (updating `seen`).
fn read_changed(vfs: &dyn Vfs, path: &str, seen: &mut Option<String>) -> Option<String> {
    let value = read_utf8(vfs, path)?;
    if seen.as_deref() == Some(value.as_str()) {
        return None;
    }
    *seen = Some(value.clone());
    Some(value)
}

/// Parse a boolean state payload (`1`/`true`/`on`).
fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "true" | "on" | "yes")
}

/// Index of the font-scale preset closest to `scale`.
fn nearest_preset(scale: f32) -> usize {
    FONT_SCALE_PRESETS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (*a - scale)
                .abs()
                .partial_cmp(&(*b - scale).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map_or(1, |(i, _)| i)
}

/// Smallest and largest font-scale preset.
fn font_scale_range() -> (f32, f32) {
    let min = FONT_SCALE_PRESETS.first().copied().unwrap_or(1.0);
    let max = FONT_SCALE_PRESETS.last().copied().unwrap_or(1.0);
    (min, max)
}

/// List label for a locale: its native name when the bitmap font can draw
/// it, otherwise the English name plus the code.
fn locale_label(locale: Locale) -> String {
    if oasis_i18n::bitmap_font_supports(locale) {
        locale.name().to_string()
    } else {
        format!("{} ({})", locale.english_name(), locale.code())
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
        for _ in 0..6 {
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
        for _ in 0..7 {
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

    /// Windowed content size used by the click tests.
    const CW: u32 = 480;
    const CH: u32 = 260;

    /// Content-local point in the middle of the row showing `item`, using
    /// the same layout + scroll the renderer uses. `x_frac` positions the
    /// point across the row (0.0 = left edge, 1.0 = right edge).
    fn item_point(app: &SettingsApp, item: usize, x_frac: f32) -> (i32, i32) {
        let l = app.layout(0, 0, CW, CH);
        let rows = app.rows();
        let scroll = app.body_scroll(&rows, l.visible_rows());
        let row = row_of_item(&rows, item).expect("item has a row");
        let r = l.row_rect(row - scroll);
        (r.x + (r.w as f32 * x_frac) as i32, r.y + r.h as i32 / 2)
    }

    #[test]
    fn click_on_skin_row_applies() {
        let mut app = make_app();
        let (x, y) = item_point(&app, 1, 0.2);
        let _action = app.handle_click(x, y, CW, CH, false);
        let req = app.take_pending_request();
        let (path, data) = req.expect("click on non-active skin should post IPC");
        assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
        // builtin_names()[1] is the second skin (not "classic").
        assert_eq!(data, builtin_names()[1]);
    }

    #[test]
    fn click_on_active_skin_noop() {
        let mut app = make_app();
        // Item 0 = "classic", which is the active skin.
        let (x, y) = item_point(&app, 0, 0.2);
        let _action = app.handle_click(x, y, CW, CH, false);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn click_above_content_area_ignored() {
        let mut app = make_app();
        // Top-left padding above the tab strip: nothing to hit.
        let _action = app.handle_click(1, 1, CW, CH, false);
        assert!(app.take_pending_request().is_none());
        assert_eq!(app.category, Category::Display);
    }

    #[test]
    fn click_on_tab_switches_category() {
        let mut app = make_app();
        let l = app.layout(0, 0, CW, CH);
        let audio = Category::ALL
            .iter()
            .position(|c| *c == Category::Audio)
            .expect("audio tab");
        let t = l.tabs[audio];
        app.handle_click(t.x + 2, t.y + 2, CW, CH, false);
        assert_eq!(app.category, Category::Audio);
    }

    #[test]
    fn click_on_resolution_preset_applies() {
        let vfs = make_vfs();
        let mut app = make_app();
        // Navigate to Resolution category.
        app.handle_input(&Button::Right, &vfs);
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.category, Category::Resolution);
        // Click the 4th preset (1280x720, index 3).
        let (x, y) = item_point(&app, 3, 0.2);
        let _action = app.handle_click(x, y, CW, CH, false);
        let req = app.take_pending_request();
        let (path, data) = req.expect("click on non-active preset should post IPC");
        assert_eq!(path, RESOLUTION_CHANGE_REQUEST_PATH);
        assert_eq!(data, "1280x720");
    }

    #[test]
    fn fullscreen_click_maps_content_lines() {
        let mut app = make_app();
        // Fullscreen keeps the line-based mapping: items start at content
        // line 4; with WINDOWED_TOP_PAD = 4 and line_h = 14, line 5 (the
        // second skin) lives at y = 4 + 5*14 = 74.
        app.handle_click(10, 74, 400, 220, true);
        let (path, data) = app.take_pending_request().expect("fullscreen click posts");
        assert_eq!(path, SKIN_CHANGE_REQUEST_PATH);
        assert_eq!(data, builtin_names()[1]);
    }

    // -- Audio / Language / Accessibility --

    /// Navigate a fresh app to `category`.
    fn app_in(category: Category, vfs: &MemoryVfs) -> SettingsApp {
        let mut app = make_app();
        while app.category != category {
            app.handle_input(&Button::Right, vfs);
        }
        app
    }

    #[test]
    fn volume_change_emits_ipc_request() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Audio, &vfs);
        app.handle_input(&Button::Up, &vfs);
        let (path, data) = app.take_pending_request().expect("volume posts IPC");
        assert_eq!(path, VOLUME_CHANGE_REQUEST_PATH);
        assert_eq!(data, "85");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        let (_, data) = app.take_pending_request().expect("volume posts IPC");
        assert_eq!(data, "75");
    }

    #[test]
    fn volume_at_limit_posts_nothing() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Audio, &vfs);
        app.volume = 100;
        app.handle_input(&Button::Up, &vfs);
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn volume_plus_minus_keys() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Audio, &vfs);
        let consumed = app.handle_key(&Key::Char('-'), Modifiers::default(), &vfs);
        assert_eq!(consumed, Some(AppAction::None));
        assert_eq!(app.volume, 75);
        // Outside slider categories the keys fall through.
        let mut other = make_app();
        assert!(
            other
                .handle_key(&Key::Char('+'), Modifiers::default(), &vfs)
                .is_none()
        );
    }

    #[test]
    fn click_on_volume_slider_sets_value() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Audio, &vfs);
        let l = app.layout(0, 0, CW, CH);
        let rows = app.rows();
        let row = row_of_item(&rows, 0).expect("volume row");
        let track = l.control_rect(l.row_rect(row));
        // Click near the left end of the track -> low volume.
        app.handle_click(track.x + 1, track.y + 2, CW, CH, false);
        assert!(app.volume <= 5, "volume {}", app.volume);
        let (path, _) = app.take_pending_request().expect("slider click posts");
        assert_eq!(path, VOLUME_CHANGE_REQUEST_PATH);
    }

    #[test]
    fn shell_volume_state_is_adopted_once() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(VOLUME_STATE_PATH, b"40").unwrap();
        let mut app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
        assert_eq!(app.volume, 40);
        // Optimistic local change survives a sync while the shell still
        // publishes the old value ...
        app.enter_category(Category::Audio);
        app.handle_input(&Button::Up, &vfs);
        assert!(!app.tick(16, &vfs));
        assert_eq!(app.volume, 45);
        // ... and a new published value is adopted.
        vfs.write(VOLUME_STATE_PATH, b"60").unwrap();
        assert!(app.tick(16, &vfs));
        assert_eq!(app.volume, 60);
    }

    #[test]
    fn language_lists_locales_and_requests_change() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Language, &vfs);
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Deutsch")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("English") && l.contains('*')),
            "active locale marked"
        );
        // English is first; move to the next locale and confirm.
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        let (path, data) = app.take_pending_request().expect("locale posts IPC");
        assert_eq!(path, LOCALE_CHANGE_REQUEST_PATH);
        assert_eq!(data, Locale::all()[1].code());
    }

    #[test]
    fn language_marks_published_locale() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/system").unwrap();
        vfs.mkdir("/system/state").unwrap();
        vfs.write(LOCALE_STATE_PATH, b"fr").unwrap();
        let mut app = SettingsApp::from_vfs("/apps/settings", &vfs, "classic", 480, 272, "SDL3");
        assert_eq!(app.locale, Locale::French);
        app.enter_category(Category::Language);
        let fr = Locale::all()
            .iter()
            .position(|l| *l == Locale::French)
            .expect("fr");
        assert_eq!(app.item_cursor, fr);
    }

    #[test]
    fn accessibility_font_scale_cycles_and_posts() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Accessibility, &vfs);
        assert_eq!(app.item_cursor, A11Y_FONT_SCALE);
        app.handle_input(&Button::Confirm, &vfs);
        let (path, data) = app.take_pending_request().expect("font scale posts");
        assert_eq!(path, FONT_SCALE_REQUEST_PATH);
        assert_eq!(data.parse::<f32>().ok(), Some(1.25));
        // Square steps back down.
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.font_scale, 1.0);
        // Confirm at the largest preset wraps to the smallest.
        app.font_scale = 1.5;
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.font_scale, 0.75);
    }

    #[test]
    fn accessibility_toggles_post_requests() {
        let vfs = make_vfs();
        let mut app = app_in(Category::Accessibility, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        let (path, data) = app.take_pending_request().expect("high contrast posts");
        assert_eq!(path, HIGH_CONTRAST_REQUEST_PATH);
        assert_eq!(data, "on");

        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        let (path, data) = app.take_pending_request().expect("reduced motion posts");
        assert_eq!(path, REDUCED_MOTION_REQUEST_PATH);
        assert_eq!(data, "1");
        assert!(app.reduced_motion);
        assert!(
            app.lines()
                .iter()
                .any(|l| l.contains("Reduced Motion: [ON]"))
        );
    }

    #[test]
    fn high_contrast_toggle_reflects_active_skin() {
        let vfs = make_vfs();
        let mut app = SettingsApp::new("/apps/settings", HIGH_CONTRAST_SKIN, 480, 272, "SDL3");
        app.enter_category(Category::Accessibility);
        assert!(
            app.lines()
                .iter()
                .any(|l| l.contains("High Contrast: [ON]"))
        );
        app.item_cursor = A11Y_HIGH_CONTRAST;
        app.handle_input(&Button::Confirm, &vfs);
        let (_, data) = app.take_pending_request().expect("posts");
        assert_eq!(data, "off");
    }

    #[test]
    fn windowed_draw_uses_widgets_and_theme_fonts() {
        use oasis_test_backend::{DrawCommand, RecordingBackend};

        let vfs = make_vfs();
        let app = app_in(Category::Accessibility, &vfs);
        let mut backend = RecordingBackend::new(CW, CH);
        let mut at = ActiveTheme::default();
        at.font_body = 15;
        app.draw_windowed(0, 0, CW, CH, &mut backend, &at)
            .expect("draw");
        let cmds = backend.commands();
        let texts: Vec<(&str, u16)> = cmds
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText {
                    text, font_size, ..
                } => Some((text.as_str(), *font_size)),
                _ => None,
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|(t, size)| t.contains("Font Scale") && *size == 15),
            "rows use the theme's (scaled) body font: {texts:?}"
        );
        // Toggle thumbs (circles, rasterized to rects by the default
        // `fill_circle`) use the widget theme's thumb color.
        let thumb = at.ui_theme.toggle_thumb;
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                DrawCommand::FillRect { color, .. } if *color == thumb
            )),
            "toggle widgets drawn"
        );
        // The metrics are cached for click hit-testing.
        assert_eq!(app.metrics.get().font_body, 15);
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

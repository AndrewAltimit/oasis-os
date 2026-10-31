//! Settings application for OASIS_OS.
//!
//! Provides a categorised settings screen with skin selection, resolution
//! switching, audio volume, interface language, accessibility options and
//! system/about details. Changes are published back to the shell through
//! VFS IPC paths; the shell applies them live, persists them to
//! `/system/settings.toml` and publishes the resulting state under
//! `/system/state/*`, which this app reads back.

use oasis_app_core::{App, AppAction, AppRequest, ContentState};
use oasis_i18n::{Locale, tr};
use oasis_skin::ActiveTheme;
use oasis_skin::builtin::builtin_names;
use oasis_skin::theme::{contrast_ratio, parse_hex_color};
use oasis_skin::{SkinTheme, SkinVariant, resolve_skin};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

mod appearance;
mod body;
mod category;
mod colors;
mod layout;
mod prefs;
mod render;
mod rows;
#[cfg(test)]
mod tests;

pub use colors::SettingsColors;

use appearance::{APPEARANCE_ACTIONS, AppearanceState, BASE_COLOR_LABELS};
use category::Category;
use layout::SettingsLayout;
use prefs::{SeenState, read_utf8};
use rows::{Row, row_of_item, scroll_for};

// The VFS IPC paths the Settings app posts requests to and reads the host's
// state from, plus the resolution grammar, live in `oasis_types::ipc`
// (shared with the host shell); re-exported here for existing callers.
pub use oasis_types::ipc::{
    BACKEND_STATE_PATH, FONT_SCALE_REQUEST_PATH, FONT_SCALE_STATE_PATH, HIGH_CONTRAST_REQUEST_PATH,
    LOCALE_CHANGE_REQUEST_PATH, LOCALE_STATE_PATH, REDUCED_MOTION_REQUEST_PATH,
    REDUCED_MOTION_STATE_PATH, RESOLUTION_CHANGE_REQUEST_PATH, RESOLUTION_STATE_PATH,
    SKIN_APPLY_THEME_REQUEST_PATH, SKIN_CHANGE_REQUEST_PATH, SKIN_SAVE_CUSTOM_REQUEST_PATH,
    SKIN_STATE_PATH, VOLUME_CHANGE_REQUEST_PATH, VOLUME_STATE_PATH, parse_resolution,
};

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

/// Selectable items of the Accessibility category.
const A11Y_FONT_SCALE: usize = 0;
const A11Y_HIGH_CONTRAST: usize = 1;
const A11Y_REDUCED_MOTION: usize = 2;
const A11Y_ITEMS: usize = 3;

/// Font metrics recorded by `App::layout`, reused by click hit-testing.
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
    /// Font metrics of the last `layout` (click hit-testing).
    metrics: Metrics,
    /// Body rows that fit the content area of the last `layout`.
    visible_rows: usize,
    /// Content-area size recorded by the last `App::layout` (click
    /// hit-testing).
    content_size: (u32, u32),
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
            metrics: Metrics::default(),
            visible_rows: 1,
            content_size: (0, 0),
            appearance: AppearanceState::default(),
        };
        // Align the cursor with the currently active skin so the highlight
        // starts on the running skin rather than always on the first entry.
        if let Some(idx) = builtin_names().iter().position(|n| *n == app.current_skin) {
            app.item_cursor = idx;
        }
        app.refresh_lines();
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

    /// Build display lines for the current category ([`App::lines`]).
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
        lines.push(format!("  {}", tr!("settings.nav_hint")));
        lines.push(format!("  {}", tr!("settings.confirm_hint")));
        lines
    }

    // -- Shell state sync --------------------------------------------------

    /// Rebuild display lines from current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.build_lines();
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
        self.refresh_lines();
    }

    /// Queue an IPC request for the shell.
    fn post(&mut self, path: &str, payload: String) {
        self.content.post_request(path, payload);
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

    /// Windowed layout for a content rect of the given size, using the
    /// font metrics of the last `layout`.
    fn settings_layout(&self, cx: i32, cy: i32, cw: u32, ch: u32) -> SettingsLayout {
        let m = self.metrics;
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
        let l = self.settings_layout(0, 0, cw, ch);
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
    }
}

impl App for SettingsApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
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
        self.draw_settings(cx, cy, cw, ch, backend, at)
    }

    /// The wheel moves the selection / scrolls text, but never changes a
    /// value: on the Audio tab and while editing a palette channel Up/Down
    /// adjust the value, so the wheel is ignored there.
    fn handle_scroll(&mut self, delta: i32, vfs: &dyn Vfs) -> bool {
        if self.category == Category::Audio || self.appearance.editing_channel.is_some() {
            return false;
        }
        let button = if delta < 0 { Button::Up } else { Button::Down };
        let steps = delta.unsigned_abs().min(oasis_app_core::MAX_WHEEL_STEPS);
        for _ in 0..steps {
            self.handle_input(&button, vfs);
        }
        steps > 0
    }

    fn layout(&mut self, cw: u32, ch: u32, at: &ActiveTheme) {
        // The windowed rows are sized from the theme's (font-scaled)
        // fonts; record them for click hit-testing and text scrolling.
        self.metrics = Metrics {
            font_body: at.font_body,
            font_hint: at.font_hint,
        };
        self.content_size = (cw, ch);
        self.visible_rows = self.settings_layout(0, 0, cw, ch).visible_rows();
        let max = self.rows().len().saturating_sub(self.visible_rows);
        self.text_scroll = self.text_scroll.min(max);
    }

    fn drain_requests(&mut self, out: &mut Vec<AppRequest>) {
        out.append(&mut self.content.requests);
    }

    fn tick(&mut self, _dt_ms: u32, vfs: &dyn Vfs) -> bool {
        self.sync_from_vfs(vfs)
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
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
                        // Plain scrollable text: scroll the row list, never
                        // past the last full page (so Up reacts at once).
                        if up {
                            self.text_scroll = self.text_scroll.saturating_sub(1);
                        } else {
                            let max = self.rows().len().saturating_sub(self.visible_rows);
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

    fn handle_click(&mut self, lx: i32, ly: i32, _vfs: &dyn Vfs) -> AppAction {
        let (cw, ch) = self.content_size;
        self.handle_windowed_click(lx, ly, cw, ch);
        AppAction::None
    }
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

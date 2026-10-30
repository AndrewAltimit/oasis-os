//! `skin.toml` manifest and `features.toml` feature gates.

use serde::Deserialize;

/// Top-level skin manifest (`skin.toml`).
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct SkinManifest {
    /// Human-readable skin name.
    pub name: String,
    /// Skin version string (default "1.0").
    #[serde(default = "default_version")]
    pub version: String,
    /// Skin author name.
    #[serde(default)]
    pub author: String,
    /// Short description of the skin.
    #[serde(default)]
    pub description: String,
    /// Virtual screen width in pixels (default 480).
    #[serde(default = "default_width")]
    pub screen_width: u32,
    /// Virtual screen height in pixels (default 272).
    #[serde(default = "default_height")]
    pub screen_height: u32,
    /// Desktop render width override (default: PSP-native 480x272 skins
    /// are upscaled to 1280x720 on desktop; other sizes render as-is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_width: Option<u32>,
    /// Desktop render height override (see `desktop_width`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_height: Option<u32>,
    /// Parent skin to inherit from (built-in name).
    ///
    /// Child skin only needs to specify overrides; non-overridden fields
    /// come from the parent. Max depth 3 to prevent infinite recursion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
}

fn default_version() -> String {
    "1.0".to_string()
}
fn default_width() -> u32 {
    480
}
fn default_height() -> u32 {
    272
}

/// Feature gates controlling which capabilities a skin exposes.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct SkinFeatures {
    /// Whether the dashboard icon grid is shown.
    #[serde(default = "yes")]
    pub dashboard: bool,
    /// Whether the command terminal is accessible.
    #[serde(default = "yes")]
    pub terminal: bool,
    /// Whether the file browser command (ls/cd/cat) is available.
    #[serde(default = "yes")]
    pub file_browser: bool,
    /// Whether the HTML/CSS browser widget is available.
    #[serde(default = "yes")]
    pub browser: bool,
    /// Whether the window manager is active (for Desktop skin).
    #[serde(default)]
    pub window_manager: bool,
    /// Number of dashboard pages (for icon grid skins).
    #[serde(default = "default_pages")]
    pub dashboard_pages: u32,
    /// Icons per page (grid capacity).
    #[serde(default = "default_icons_per_page")]
    pub icons_per_page: u32,
    /// Dashboard icon layout: `"grid"` (uniform cells, default), `"free"`
    /// (desktop-style per-icon positions with column auto-flow and drag &
    /// drop on pointer targets), or `"column"` (opt-in sparse PSIX-style
    /// left column — same interaction semantics as `free`, but auto-flow is
    /// constrained to `free_icon_cols` columns (default 1), icons are
    /// left-aligned instead of centered, and the vertical pitch is larger).
    #[serde(default = "default_icon_layout")]
    pub icon_layout: String,
    /// Cap the free/column auto-flow to this many columns. `None` = as many
    /// as fit the grid width (classic `free` behaviour). In `"column"`
    /// layout this defaults to 1 when unset; in `"free"` layout it applies
    /// only when explicitly set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_icon_cols: Option<u32>,
    /// In free layout, snap dragged icons to a virtual grid on drop
    /// (default true — matches real desktop behaviour).
    #[serde(default = "yes")]
    pub snap_to_grid: bool,
    /// Launch apps on a single pointer click (default true). When false,
    /// the first click only selects the icon and a second click launches.
    #[serde(default = "yes")]
    pub launch_on_single_click: bool,
    /// Draw the skin's own software mouse cursor instead of relying on the
    /// host OS pointer (default false). Themeable via `[cursor]` in
    /// theme.toml.
    #[serde(default)]
    pub software_cursor: bool,
    /// Grid columns.
    #[serde(default = "default_grid_cols")]
    pub grid_cols: u32,
    /// Grid rows.
    #[serde(default = "default_grid_rows")]
    pub grid_rows: u32,
    /// Whether the start menu popup is available.
    #[serde(default = "yes")]
    pub start_menu: bool,
    /// Whether corrupted modifiers are active.
    #[serde(default)]
    pub corrupted: bool,
    /// Whether the battery indicator is shown in the status bar.
    #[serde(default = "yes")]
    pub show_battery: bool,
    /// Whether the clock is shown in the status bar.
    #[serde(default = "yes")]
    pub show_clock: bool,
    /// Whether the version label is shown in the status bar. Defaults to
    /// off — opt in per-skin alongside a branded `version_text`.
    #[serde(default)]
    pub show_version: bool,
    /// Whether top tabs are shown in the status bar.
    #[serde(default)]
    pub show_tabs: bool,
    /// Whether media category tabs (AUDIO/VIDEO/IMAGE/FILE) are shown in the
    /// bottom bar. Defaults to off — opt in per-skin for PSP-style layouts.
    #[serde(default)]
    pub show_media_tabs: bool,
    /// Whether page dots are shown in the bottom bar.
    #[serde(default = "yes")]
    pub show_page_dots: bool,
    /// Render the clock + date in the bottom-right of the taskbar instead of
    /// the top-right status bar (Windows XP style). The top-right clock is
    /// suppressed when this is set. Defaults to `true` for consistency
    /// across skins; opt out with `clock_in_bottombar = false`.
    #[serde(default = "yes")]
    pub clock_in_bottombar: bool,
    /// Custom fade transition duration in frames (default 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_fade_frames: Option<u32>,
    /// Custom slide transition duration in frames (default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_slide_frames: Option<u32>,
    /// Disable animations for accessibility (defaults to false).
    #[serde(default)]
    pub reduced_motion: bool,
    /// Bottom bar style. `""` (default) = classic footer. `"media_dock"` =
    /// decorative PSIX-style media dock (transport pills + progress/volume
    /// tracks alongside the media tab strip). Purely cosmetic — no audio
    /// binding.
    #[serde(default)]
    pub bottombar_style: String,
}

fn yes() -> bool {
    true
}
fn default_pages() -> u32 {
    1
}
fn default_icons_per_page() -> u32 {
    9
}
fn default_icon_layout() -> String {
    "grid".to_string()
}
fn default_grid_cols() -> u32 {
    3
}
fn default_grid_rows() -> u32 {
    3
}

impl Default for SkinFeatures {
    fn default() -> Self {
        Self {
            dashboard: true,
            terminal: true,
            file_browser: true,
            browser: true,
            window_manager: false,
            dashboard_pages: 1,
            icons_per_page: 15,
            icon_layout: "grid".to_string(),
            free_icon_cols: None,
            snap_to_grid: true,
            launch_on_single_click: true,
            software_cursor: false,
            grid_cols: 5,
            grid_rows: 3,
            start_menu: true,
            corrupted: false,
            show_battery: true,
            show_clock: true,
            show_version: false,
            show_tabs: false,
            show_media_tabs: false,
            show_page_dots: true,
            clock_in_bottombar: true,
            transition_fade_frames: None,
            transition_slide_frames: None,
            reduced_motion: false,
            bottombar_style: String::new(),
        }
    }
}

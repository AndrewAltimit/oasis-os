//! Classic skin dashboard -- PSP-style icon grid with cursor navigation.
//!
//! The dashboard manages a paginated icon grid, status bar, and cursor.
//! It creates and updates SDI objects based on its internal state.
//!
//! Layout: [`config`] (skin-derived [`DashboardConfig`]), `navigation`
//! (d-pad / page / hover input), `animation` (page slide, press flash,
//! hover lift, entrance), `layout` (grid vs. free cell origins and hit
//! testing), `render` (the per-frame SDI sync), `icons` / `labels` /
//! `vector_icons` (per-style icon drawing) and `discovery` (finding apps).

mod animation;
mod config;
mod discovery;
mod icons;
mod labels;
mod layout;
mod navigation;
mod render;
mod vector_icons;

pub use config::DashboardConfig;
pub use discovery::{AppEntry, discover_apps, discover_apps_themed};

use std::collections::HashMap;

use crate::active_theme::ActiveTheme;
use animation::PageSlideAnim;

/// Left padding (px) for column-layout icons and their labels, so the
/// left-aligned column hugs the grid's left margin.
const COLUMN_LEFT_PAD: i32 = 6;

/// Pre-computed SDI object names for a single icon slot.
#[derive(Debug, Clone)]
struct IconNames {
    outline: String,
    icon: String,
    stripe: String,
    fold: String,
    gfx: String,
    label: String,
    label2: String,
    shadow: String,
    shadow2: String,
}

impl IconNames {
    fn new(i: usize) -> Self {
        Self {
            outline: format!("icon_outline_{i}"),
            icon: format!("icon_{i}"),
            stripe: format!("icon_stripe_{i}"),
            fold: format!("icon_fold_{i}"),
            gfx: format!("icon_gfx_{i}"),
            label: format!("icon_label_{i}"),
            label2: format!("icon_label2_{i}"),
            shadow: format!("icon_shadow_{i}"),
            shadow2: format!("icon_shadow2_{i}"),
        }
    }

    /// All 9 names as an array of references.
    fn all(&self) -> [&str; 9] {
        [
            &self.outline,
            &self.icon,
            &self.stripe,
            &self.fold,
            &self.gfx,
            &self.label,
            &self.label2,
            &self.shadow,
            &self.shadow2,
        ]
    }
}

/// Icon geometry and cell layout, shared across icon drawing functions.
///
/// Extracts the value-type parameters (positions, sizes) into a struct
/// so that `draw_*_icon` methods take fewer arguments.
#[derive(Debug, Clone, Copy)]
struct IconGeometry {
    ix: i32,
    iy: i32,
    icon_w: u32,
    icon_h: u32,
    cell_x: i32,
    text_pad: i32,
    /// Left-align the label under the icon instead of centering it in the
    /// cell (column layout).
    left_align: bool,
    /// Hover lift (px) already subtracted from `iy`; the label adds it
    /// back so only the icon body rises.
    lift: i32,
}

/// Runtime state for the icon grid dashboard.
#[derive(Debug)]
pub struct DashboardState {
    /// Skin-derived layout and animation parameters.
    pub config: DashboardConfig,
    /// All discovered applications.
    pub apps: Vec<AppEntry>,
    /// Current page index (0-based).
    pub page: usize,
    /// Selected icon index within the current page (0-based).
    pub selected: usize,
    /// Active page-slide animation (None = not animating).
    page_anim: Option<PageSlideAnim>,
    /// Icon press flash countdown (0 = inactive).
    press_flash_frame: u32,
    /// Which icon index is flashing.
    press_flash_index: usize,
    /// Cached icon SDI names (avoids per-frame format! allocations).
    icon_names: Vec<IconNames>,
    /// Elapsed milliseconds since last page change (for entrance animation).
    entrance_elapsed_ms: Option<u32>,
    /// Currently focused icon index within the page (for focus glow).
    selected_index: usize,
    /// Free-layout icon positions: app path -> cell origin (screen coords).
    /// Apps without an entry auto-flow top-to-bottom in columns.
    pub positions: HashMap<String, (i32, i32)>,
    /// Icon index (within the current page) being dragged, if any.
    /// Rendering raises the dragged icon above its siblings.
    pub drag_index: Option<usize>,
    /// Cached scaled vector icon scenes keyed by global icon index.
    /// `RefCell` because the render path borrows the dashboard immutably;
    /// single-threaded like the rest of the SDI pipeline.
    scene_cache: std::cell::RefCell<HashMap<usize, vector_icons::CachedIconScene>>,
    /// Cached word-wrapped label lines per app title (see
    /// [`labels::LabelWrapCache`]). `RefCell` for the same reason as
    /// `scene_cache`: the icon draw methods borrow `self` immutably.
    label_wrap_cache: labels::LabelWrapCache,
    /// Icon index (within the current page) under the pointer, if any.
    /// Set from pointer motion via [`DashboardState::set_hover`].
    pub hover_index: Option<usize>,
    /// Per-slot hover lift progress, 0.0 (resting) ..= 1.0 (lifted),
    /// eased toward the hover target in [`DashboardState::tick_animation`].
    hover_lift: Vec<f32>,
}

impl DashboardState {
    /// Create a new dashboard with the given config and app list.
    pub fn new(config: DashboardConfig, apps: Vec<AppEntry>) -> Self {
        let per_page = config.icons_per_page as usize;
        let icon_names = (0..per_page).map(IconNames::new).collect();
        Self {
            config,
            apps,
            page: 0,
            selected: 0,
            page_anim: None,
            press_flash_frame: 0,
            press_flash_index: 0,
            icon_names,
            entrance_elapsed_ms: Some(1000),
            selected_index: 0,
            positions: HashMap::new(),
            drag_index: None,
            scene_cache: std::cell::RefCell::new(HashMap::new()),
            label_wrap_cache: labels::LabelWrapCache::default(),
            hover_index: None,
            hover_lift: vec![0.0; per_page],
        }
    }

    /// Number of pages needed to show all apps.
    pub fn page_count(&self) -> usize {
        let per_page = self.config.icons_per_page as usize;
        if per_page == 0 || self.apps.is_empty() {
            return 1;
        }
        self.apps.len().div_ceil(per_page)
    }

    /// Apps visible on the current page.
    pub fn current_page_apps(&self) -> &[AppEntry] {
        let per_page = self.config.icons_per_page as usize;
        let start = self.page * per_page;
        let end = (start + per_page).min(self.apps.len());
        if start >= self.apps.len() {
            &[]
        } else {
            &self.apps[start..end]
        }
    }

    /// Get the currently selected app entry, if any.
    pub fn selected_app(&self) -> Option<&AppEntry> {
        self.current_page_apps().get(self.selected)
    }

    /// Horizontal offset (px) of the incoming page while a page slide is
    /// running (0 otherwise): it slides in from off-screen to 0.
    fn slide_offset(&self, at: &ActiveTheme) -> i32 {
        self.page_anim.as_ref().map_or(0, |anim| {
            let t = anim.eased_progress();
            ((1.0 - t) * at.screen_w as f32 * anim.direction as f32) as i32
        })
    }

    /// Top-left of icon `i`'s body given its cell origin (slide offset
    /// already applied): centred in the cell, or hugging the left edge in
    /// the column layout, and raised by the hover lift.
    fn icon_origin(&self, i: usize, (cell_x, cell_y): (i32, i32), at: &ActiveTheme) -> (i32, i32) {
        let (cell_w, cell_h) = self.cell_size();
        let ix = if self.config.left_align_icons {
            cell_x + COLUMN_LEFT_PAD
        } else {
            cell_x + (cell_w as i32 - at.icon_width as i32) / 2
        };
        let iy = cell_y + (cell_h as i32 - at.icon_height as i32) / 4 - self.hover_lift_px(i, at);
        (ix, iy)
    }
}

#[cfg(test)]
pub(crate) mod tests;

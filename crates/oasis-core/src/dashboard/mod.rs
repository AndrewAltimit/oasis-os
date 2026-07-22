//! Classic skin dashboard -- PSP-style icon grid with cursor navigation.
//!
//! The dashboard manages a paginated icon grid, status bar, and cursor.
//! It creates and updates SDI objects based on its internal state.

mod discovery;
mod icons;
mod labels;
mod layout;
mod vector_icons;

pub use discovery::{AppEntry, discover_apps, discover_apps_themed};

use std::collections::HashMap;

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::skin::SkinFeatures;
use crate::ui::flex::GridLayout;
use crate::ui::layout::Padding;

/// Base-layer z for free-layout icons: above the skin's layout chrome
/// (auto z stays in low single digits) and the selection highlight
/// (`FREE_ICON_Z - 10`), below the overlay pass (bars, start menu).
pub const FREE_ICON_Z: i32 = 50;
/// z for the icon being dragged, raised above its siblings.
pub const FREE_ICON_DRAG_Z: i32 = 60;

/// Left padding (px) for column-layout icons and their labels, so the
/// left-aligned column hugs the grid's left margin.
pub(crate) const COLUMN_LEFT_PAD: i32 = 6;

/// Dashboard configuration derived from the skin's feature gates.
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    pub grid_cols: u32,
    pub grid_rows: u32,
    pub icons_per_page: u32,
    pub max_pages: u32,
    /// Grid area origin and cell size (pixels).
    pub grid_x: i32,
    pub grid_y: i32,
    pub cell_w: u32,
    pub cell_h: u32,
    /// Cursor highlight size offset (drawn slightly larger than the icon).
    pub cursor_pad: i32,
    /// Grid layout helper for computing cell positions.
    pub grid_layout: GridLayout,
    /// Total grid area width (for `GridLayout::cell_rect`).
    pub grid_w: u32,
    /// Total grid area height (for `GridLayout::cell_rect`).
    pub grid_h: u32,
    /// Cursor lerp speed (0.0-1.0).
    pub cursor_lerp_speed: f32,
    /// Page slide animation duration in frames.
    pub page_slide_duration: u32,
    /// Press flash duration in frames (0 = disabled).
    pub press_flash_duration: u32,
    /// Free (desktop-style) icon layout: per-icon positions with column
    /// auto-flow instead of uniform grid cells.
    pub free_layout: bool,
    /// Fixed cell size in free mode (grid cells stretch to fill instead).
    pub free_cell_w: u32,
    pub free_cell_h: u32,
    /// Cap free/column auto-flow to this many columns (`None` = as many as
    /// fit the grid width). Set to `Some(1)` by `"column"` layout.
    pub free_cols: Option<u32>,
    /// Left-align icons (and their labels) within the cell instead of
    /// centering. Enabled by the `"column"` layout for a sparse PSIX-style
    /// left column.
    pub left_align_icons: bool,
    /// Snap dragged icons to the virtual grid on drop (free mode).
    pub snap_to_grid: bool,
    /// Page change style: "slide" (default) or "fade" (no icon slide; the
    /// shell overlays a fade transition instead).
    pub page_style: String,
}

impl DashboardConfig {
    /// Create a config from skin features and screen dimensions.
    /// Uses PSIX-style layout: icons on the left side with generous spacing.
    pub fn from_features(features: &SkinFeatures, at: &ActiveTheme) -> Self {
        let cols = features.grid_cols;
        let rows = features.grid_rows;
        let content_top = at.statusbar_height + at.tab_row_height;
        let content_h = at.screen_h - content_top - at.bottombar_height;
        let grid_padding_x = at.grid_padding_x;
        let grid_padding_y = at.grid_padding_y;
        let grid_x = grid_padding_x as i32;
        let grid_y = (content_top + grid_padding_y as u32) as i32;
        let grid_w = at.screen_w - 2 * grid_padding_x as u32;
        let grid_h = content_h - 2 * grid_padding_y as u32;

        // Size cells to fill available space evenly.
        let cell_w = grid_w / cols;
        let cell_h = grid_h / rows;

        let grid_layout = GridLayout::new(cols).with_padding(Padding::ZERO);

        // `"column"` layout: sparse PSIX-style single left column. Same
        // interaction semantics as `"free"`, but auto-flow is capped to
        // `free_icon_cols` columns (default 1) and icons hug the left edge.
        let is_column = features.icon_layout == "column";
        let free_layout = features.icon_layout == "free" || is_column;

        // Free-mode cells are a fixed size derived from the theme's icon
        // dimensions (2x leaves room for a two-line label) instead of
        // stretching to fill the grid area like grid cells do.
        let free_cell_w = (at.icon_width * 2).max(48);
        let free_cell_h = if is_column {
            // Taller pitch so the column reads sparse: icon + two label
            // lines + a comfortable gap. Scales with the theme's icon size
            // so a column skin with small icons still packs several rows.
            (at.icon_height + at.icon_label_pad as u32 + (at.font_small as u32 + 1) * 2 + 8).max(48)
        } else {
            (at.icon_height * 2).max(48)
        };

        // Column mode caps auto-flow to `free_icon_cols` columns, defaulting
        // to a single column. Free mode only honours an explicit cap.
        let free_cols = if is_column {
            Some(features.free_icon_cols.unwrap_or(1))
        } else {
            features.free_icon_cols
        };

        Self {
            grid_cols: cols,
            grid_rows: rows,
            icons_per_page: features.icons_per_page,
            max_pages: features.dashboard_pages,
            grid_x,
            grid_y,
            cell_w,
            cell_h,
            cursor_pad: at.cursor_pad,
            grid_layout,
            grid_w,
            grid_h,
            cursor_lerp_speed: at.cursor_lerp_speed,
            page_slide_duration: at.page_slide_duration,
            press_flash_duration: at.press_flash_duration,
            free_layout,
            free_cell_w,
            free_cell_h,
            free_cols,
            left_align_icons: is_column,
            snap_to_grid: features.snap_to_grid,
            page_style: at.transition_page_style.clone(),
        }
    }
}

/// Active page-slide animation state.
#[derive(Debug)]
struct PageSlideAnim {
    /// Previous page index (for drawing outgoing icons).
    #[allow(dead_code)] // reserved for outgoing page rendering
    from_page: usize,
    /// Current animation frame.
    frame: u32,
    /// Total animation duration in frames (~200ms at 60fps).
    duration: u32,
    /// Slide direction: -1 = left, +1 = right.
    direction: i32,
}

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
}

/// Runtime state for the icon grid dashboard.
#[derive(Debug)]
pub struct DashboardState {
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

    /// Handle a button press for cursor navigation.
    pub fn handle_input(&mut self, button: &Button) {
        let cols = self.config.grid_cols as usize;
        let page_apps = self.current_page_apps().len();
        if page_apps == 0 {
            return;
        }

        match button {
            Button::Right => {
                self.selected = (self.selected + 1) % page_apps;
            },
            Button::Left => {
                if self.selected == 0 {
                    self.selected = page_apps - 1;
                } else {
                    self.selected -= 1;
                }
            },
            Button::Down => {
                let next = self.selected + cols;
                if next < page_apps {
                    self.selected = next;
                }
            },
            Button::Up if self.selected >= cols => {
                self.selected -= cols;
            },
            _ => {},
        }
    }

    /// Trigger an icon press flash on the currently selected icon.
    pub fn trigger_press_flash(&mut self) {
        self.press_flash_frame = self.config.press_flash_duration;
        self.press_flash_index = self.selected;
    }

    /// Switch to the next page (wraps around) with slide animation.
    pub fn next_page(&mut self) {
        let count = self.page_count();
        let from = self.page;
        self.page = (self.page + 1) % count;
        let page_apps = self.current_page_apps().len();
        if self.selected >= page_apps && page_apps > 0 {
            self.selected = page_apps - 1;
        }
        self.page_anim = (self.config.page_style != "fade").then_some(PageSlideAnim {
            from_page: from,
            frame: 0,
            duration: self.config.page_slide_duration,
            direction: -1, // slide left (next)
        });
        self.entrance_elapsed_ms = Some(0);
    }

    /// Switch to the previous page (wraps around) with slide animation.
    pub fn prev_page(&mut self) {
        let count = self.page_count();
        let from = self.page;
        if self.page == 0 {
            self.page = count - 1;
        } else {
            self.page -= 1;
        }
        let page_apps = self.current_page_apps().len();
        if self.selected >= page_apps && page_apps > 0 {
            self.selected = page_apps - 1;
        }
        self.page_anim = (self.config.page_style != "fade").then_some(PageSlideAnim {
            from_page: from,
            frame: 0,
            duration: self.config.page_slide_duration,
            direction: 1, // slide right (prev)
        });
        self.entrance_elapsed_ms = Some(0);
    }

    /// Advance page-slide and cursor-lerp animations by one frame.
    pub fn tick_animation(&mut self) {
        // Page slide.
        if let Some(ref mut anim) = self.page_anim {
            anim.frame += 1;
            if anim.frame >= anim.duration {
                self.page_anim = None;
            }
        }
        // Press flash countdown.
        if self.press_flash_frame > 0 {
            self.press_flash_frame -= 1;
        }
        // Entrance animation.
        if let Some(ref mut elapsed) = self.entrance_elapsed_ms {
            *elapsed = elapsed.saturating_add(16); // ~60fps
        }
        // Track selected index for focus glow.
        self.selected_index = self.selected;
    }

    /// Whether any dashboard animation is (or may be) live this frame.
    ///
    /// Used by the shell's idle frame elision: the vector-icon render
    /// path paints directly to the backend (outside the SDI scene
    /// graph), so its animations need an explicit "keep redrawing"
    /// signal. Deliberately over-approximate — e.g. per-icon
    /// spin/pulse/blink presets are reported page-wide — because a
    /// wasted redraw is cheap and a stale frame is a bug.
    pub fn has_active_animation(&self, at: &ActiveTheme) -> bool {
        // Transient animations: page slide, press flash, icon drag.
        if self.page_anim.is_some() || self.press_flash_frame > 0 || self.drag_index.is_some() {
            return true;
        }
        // Entrance animation. `entrance_elapsed_ms` saturates upward and
        // never returns to `None`, so compare against the full staggered
        // window (last icon's stagger + its duration).
        if at.entrance_style != "none"
            && let Some(elapsed) = self.entrance_elapsed_ms
        {
            let window = at.entrance_duration_ms.saturating_add(
                self.config
                    .icons_per_page
                    .saturating_mul(at.entrance_stagger_ms),
            );
            if elapsed < window {
                return true;
            }
        }
        // Continuous vector-icon idle animations (float bob and the
        // spin/pulse/blink glyph presets).
        if at.icon.style == "vector" {
            let preset_static = matches!(
                at.icon.vector_preset.as_str(),
                "geometric" | "hud" | "outline" | "solid" | "pixel"
            );
            let glyphs_animated = !preset_static
                && (at.icon.spin_enabled || at.icon.pulse_enabled || at.icon.blink_enabled);
            if at.icon.idle_float || glyphs_animated {
                return true;
            }
        }
        false
    }

    /// Get the currently selected app entry, if any.
    pub fn selected_app(&self) -> Option<&AppEntry> {
        self.current_page_apps().get(self.selected)
    }

    /// Synchronize SDI objects to reflect current dashboard state.
    /// Creates/updates icons (style-dependent), text labels, and cursor highlight.
    ///
    /// Accepts an `ActiveTheme` for skin-driven colors. Pass
    /// `&ActiveTheme::default()` for legacy behaviour.
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let page_apps = self.current_page_apps();

        let icon_w = at.icon_width;
        let icon_h = at.icon_height;
        let text_pad = at.icon_label_pad;

        // Compute page-slide x-offset for incoming icons.
        let slide_offset = if let Some(ref anim) = self.page_anim {
            let t = crate::transition::ease_in_out_cubic(anim.frame as f32 / anim.duration as f32);
            // Incoming page slides from off-screen to 0.
            let w = at.screen_w as f32;
            ((1.0 - t) * w * anim.direction as f32) as i32
        } else {
            0
        };

        let origins = self.page_cell_origins();
        let (cell_w, cell_h) = self.cell_size();

        let per_page = self.config.icons_per_page as usize;
        for i in 0..per_page {
            let names = &self.icon_names[i];

            for name in names.all() {
                if !sdi.contains(name) {
                    sdi.create(name);
                }
            }

            if i >= page_apps.len() {
                for name in names.all() {
                    if let Ok(obj) = sdi.get_mut(name) {
                        obj.visible = false;
                    }
                }
                continue;
            }

            let (cell_x, cell_y) = (origins[i].0 + slide_offset, origins[i].1);
            let left_align = self.config.left_align_icons;
            let ix = if left_align {
                cell_x + COLUMN_LEFT_PAD
            } else {
                cell_x + (cell_w as i32 - icon_w as i32) / 2
            };
            let iy = cell_y + (cell_h as i32 - icon_h as i32) / 4;

            let geo = IconGeometry {
                ix,
                iy,
                icon_w,
                icon_h,
                cell_x,
                text_pad,
                left_align,
            };
            match at.icon.style.as_str() {
                "card" => self.draw_card_icon(sdi, at, names, geo, &page_apps[i]),
                "circle" => self.draw_circle_icon(sdi, at, names, geo, &page_apps[i]),
                "vector" => self.draw_vector_icon(sdi, at, names, geo, i, &page_apps[i]),
                _ => self.draw_document_icon(sdi, at, names, geo, i, &page_apps[i]),
            }

            // In free layout, lift icons into an explicit z band above the
            // skin's base-layer chrome (layout objects take single-digit
            // auto z), with the dragged icon raised above its siblings so
            // it stays visible while crossing them.
            if self.config.free_layout {
                let z = if self.drag_index == Some(i) {
                    FREE_ICON_DRAG_Z
                } else {
                    FREE_ICON_Z
                };
                for name in names.all() {
                    if let Ok(obj) = sdi.get_mut(name) {
                        obj.z = z;
                    }
                }
            }
        }

        // Grid mode keeps the historical behaviour: icons respond directly
        // to clicks and no selection box is drawn. Free layout shows a
        // themed highlight behind the selected icon (skin `cursor_style`:
        // "stroke" outlines it, "fill" paints a backdrop, "none" disables).
        let cursor_name = "cursor_highlight";
        if !sdi.contains(cursor_name) {
            sdi.create(cursor_name);
        }
        let sel_rect = (self.config.free_layout
            && at.icon.cursor_style != "none"
            && self.drag_index.is_none()
            && self.selected < page_apps.len())
        .then(|| {
            let (ox, oy) = origins[self.selected];
            let pad = self.config.cursor_pad.max(0);
            let ix = if self.config.left_align_icons {
                ox + slide_offset + COLUMN_LEFT_PAD
            } else {
                ox + slide_offset + (cell_w as i32 - icon_w as i32) / 2
            };
            let iy = oy + (cell_h as i32 - icon_h as i32) / 4;
            (
                ix - pad,
                iy - pad,
                icon_w + 2 * pad as u32,
                icon_h + 2 * pad as u32,
            )
        });
        if let Ok(cursor) = sdi.get_mut(cursor_name) {
            match sel_rect {
                Some((x, y, w, h)) => {
                    cursor.visible = true;
                    cursor.x = x;
                    cursor.y = y;
                    cursor.w = w;
                    cursor.h = h;
                    cursor.z = FREE_ICON_Z - 10;
                    cursor.text = None;
                    cursor.border_radius = Some(at.icon.cursor_border_radius);
                    if at.icon.cursor_style == "fill" {
                        cursor.color = at.icon.cursor_color;
                        cursor.stroke_width = None;
                        cursor.stroke_color = None;
                    } else {
                        cursor.color = Color::rgba(0, 0, 0, 0);
                        cursor.stroke_width = Some(at.icon.cursor_stroke_width);
                        cursor.stroke_color = Some(at.icon.cursor_color);
                    }
                },
                None => cursor.visible = false,
            }
        }
    }

    /// Hide all dashboard SDI objects.
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        for names in &self.icon_names {
            for name in names.all() {
                if let Ok(obj) = sdi.get_mut(name) {
                    obj.visible = false;
                }
            }
        }
        if let Ok(obj) = sdi.get_mut("cursor_highlight") {
            obj.visible = false;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backend::Color;

    pub(crate) fn test_config() -> DashboardConfig {
        DashboardConfig {
            grid_cols: 2,
            grid_rows: 2,
            icons_per_page: 4,
            max_pages: 4,
            grid_x: 16,
            grid_y: 48,
            cell_w: 110,
            cell_h: 95,
            cursor_pad: 4,
            grid_layout: GridLayout::new(2),
            grid_w: 220,
            grid_h: 190,
            cursor_lerp_speed: 0.18,
            page_slide_duration: 12,
            press_flash_duration: 6,
            free_layout: false,
            free_cell_w: 80,
            free_cell_h: 80,
            free_cols: None,
            left_align_icons: false,
            snap_to_grid: true,
            page_style: "slide".to_string(),
        }
    }

    pub(crate) fn test_apps(n: usize) -> Vec<AppEntry> {
        (0..n)
            .map(|i| AppEntry {
                title: format!("App {i}"),
                path: format!("/apps/app{i}"),
                icon_png: Vec::new(),
                color: Color::rgb(100, 100, 100),
            })
            .collect()
    }

    #[test]
    fn page_count_single() {
        let dash = DashboardState::new(test_config(), test_apps(3));
        assert_eq!(dash.page_count(), 1);
    }

    #[test]
    fn page_count_multiple() {
        let dash = DashboardState::new(test_config(), test_apps(6));
        assert_eq!(dash.page_count(), 2);
    }

    #[test]
    fn page_count_exact() {
        let dash = DashboardState::new(test_config(), test_apps(4));
        assert_eq!(dash.page_count(), 1);
    }

    #[test]
    fn page_count_empty() {
        let dash = DashboardState::new(test_config(), vec![]);
        assert_eq!(dash.page_count(), 1);
    }

    #[test]
    fn navigate_right_wraps() {
        let mut dash = DashboardState::new(test_config(), test_apps(3));
        dash.handle_input(&Button::Right);
        assert_eq!(dash.selected, 1);
        dash.handle_input(&Button::Right);
        assert_eq!(dash.selected, 2);
        dash.handle_input(&Button::Right);
        assert_eq!(dash.selected, 0); // Wraps.
    }

    #[test]
    fn navigate_left_wraps() {
        let mut dash = DashboardState::new(test_config(), test_apps(3));
        dash.handle_input(&Button::Left);
        assert_eq!(dash.selected, 2); // Wraps to last.
    }

    #[test]
    fn navigate_down() {
        let mut dash = DashboardState::new(test_config(), test_apps(4));
        dash.handle_input(&Button::Down);
        assert_eq!(dash.selected, 2); // Moved down one row (2 cols).
    }

    #[test]
    fn navigate_up() {
        let mut dash = DashboardState::new(test_config(), test_apps(4));
        dash.selected = 3;
        dash.handle_input(&Button::Up);
        assert_eq!(dash.selected, 1);
    }

    #[test]
    fn page_style_slide_animates_fade_does_not() {
        let mut dash = DashboardState::new(test_config(), test_apps(6));
        dash.next_page();
        assert!(dash.page_anim.is_some(), "slide style animates");

        let mut cfg = test_config();
        cfg.page_style = "fade".to_string();
        let mut dash = DashboardState::new(cfg, test_apps(6));
        dash.next_page();
        assert!(dash.page_anim.is_none(), "fade style suppresses the slide");
        assert_eq!(dash.page, 1);
        dash.prev_page();
        assert!(dash.page_anim.is_none());
        assert_eq!(dash.page, 0);
    }

    #[test]
    fn next_page_wraps() {
        let mut dash = DashboardState::new(test_config(), test_apps(6));
        assert_eq!(dash.page, 0);
        dash.next_page();
        assert_eq!(dash.page, 1);
        dash.next_page();
        assert_eq!(dash.page, 0); // Wraps (2 pages).
    }

    #[test]
    fn prev_page_wraps() {
        let mut dash = DashboardState::new(test_config(), test_apps(6));
        dash.prev_page();
        assert_eq!(dash.page, 1); // Wraps to last.
    }

    #[test]
    fn selected_app() {
        let dash = DashboardState::new(test_config(), test_apps(3));
        let app = dash.selected_app().unwrap();
        assert_eq!(app.title, "App 0");
    }

    #[test]
    fn update_sdi_creates_objects() {
        let mut dash = DashboardState::new(test_config(), test_apps(3));
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        dash.update_sdi(&mut sdi, &at);
        assert!(sdi.contains("icon_0"));
        assert!(sdi.contains("icon_1"));
        assert!(sdi.contains("icon_2"));
        assert!(sdi.contains("icon_label_0"));
        assert!(sdi.contains("icon_label_1"));
        assert!(sdi.contains("cursor_highlight"));
    }

    #[test]
    fn grid_layout_keeps_selection_highlight_hidden() {
        let mut dash = DashboardState::new(test_config(), test_apps(3));
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        dash.update_sdi(&mut sdi, &at);
        assert!(!sdi.get("cursor_highlight").unwrap().visible);
    }

    #[test]
    fn free_layout_shows_selection_highlight() {
        let mut config = test_config();
        config.free_layout = true;
        let mut dash = DashboardState::new(config, test_apps(3));
        dash.selected = 1;
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        dash.update_sdi(&mut sdi, &at);
        let cursor = sdi.get("cursor_highlight").unwrap();
        assert!(cursor.visible);
        // Default cursor_style is "stroke": outline only, transparent fill.
        assert_eq!(cursor.stroke_width, Some(at.icon.cursor_stroke_width));
        // Hidden again while a drag is in flight.
        dash.drag_index = Some(1);
        dash.update_sdi(&mut sdi, &at);
        assert!(!sdi.get("cursor_highlight").unwrap().visible);
    }

    #[test]
    fn selected_clamps_on_page_switch() {
        let mut dash = DashboardState::new(test_config(), test_apps(5));
        // 5 apps, 4 per page: page 0 has 4, page 1 has 1.
        dash.selected = 3; // Last on page 0.
        dash.next_page();
        // Page 1 has only 1 app, so selected should clamp to 0.
        assert_eq!(dash.selected, 0);
    }

    // -----------------------------------------------------------------------
    // PSP 4x3 grid navigation tests
    //
    // The PSP backend uses a 4-column, 3-row grid (12 icons per page).
    // These tests exercise navigation patterns specific to that layout.
    // -----------------------------------------------------------------------

    fn psp_config() -> DashboardConfig {
        DashboardConfig {
            grid_cols: 4,
            grid_rows: 3,
            icons_per_page: 12,
            max_pages: 4,
            grid_x: 8,
            grid_y: 20,
            cell_w: 116,
            cell_h: 72,
            cursor_pad: 2,
            grid_layout: GridLayout::new(4),
            grid_w: 464,
            grid_h: 218,
            cursor_lerp_speed: 0.18,
            page_slide_duration: 12,
            press_flash_duration: 6,
            free_layout: false,
            free_cell_w: 68,
            free_cell_h: 68,
            free_cols: None,
            left_align_icons: false,
            snap_to_grid: true,
            page_style: "slide".to_string(),
        }
    }

    #[test]
    fn psp_grid_navigate_right_wraps_at_row_end() {
        // PSP has 11 apps (one short of filling the 4x3 grid).
        let mut dash = DashboardState::new(psp_config(), test_apps(11));
        // Start at 0, go right to the end of all apps.
        for _ in 0..10 {
            dash.handle_input(&Button::Right);
        }
        assert_eq!(dash.selected, 10);
        // One more right wraps to 0.
        dash.handle_input(&Button::Right);
        assert_eq!(dash.selected, 0);
    }

    #[test]
    fn psp_grid_navigate_down_across_rows() {
        let mut dash = DashboardState::new(psp_config(), test_apps(12));
        // Start at position 0 (row 0, col 0).
        assert_eq!(dash.selected, 0);
        // Down moves to row 1, col 0 = position 4.
        dash.handle_input(&Button::Down);
        assert_eq!(dash.selected, 4);
        // Down again to row 2, col 0 = position 8.
        dash.handle_input(&Button::Down);
        assert_eq!(dash.selected, 8);
        // Down at bottom row: stays put.
        dash.handle_input(&Button::Down);
        assert_eq!(dash.selected, 8);
    }

    #[test]
    fn psp_grid_navigate_up_at_top_stays() {
        let mut dash = DashboardState::new(psp_config(), test_apps(12));
        dash.selected = 2; // Row 0, col 2.
        dash.handle_input(&Button::Up);
        assert_eq!(dash.selected, 2); // Can't go up from row 0.
    }

    #[test]
    fn psp_grid_navigate_left_wraps_to_last() {
        let mut dash = DashboardState::new(psp_config(), test_apps(11));
        assert_eq!(dash.selected, 0);
        dash.handle_input(&Button::Left);
        assert_eq!(dash.selected, 10); // Wraps to last app.
    }

    #[test]
    fn psp_grid_full_page_count() {
        // 11 apps, 12 per page => 1 page.
        let dash = DashboardState::new(psp_config(), test_apps(11));
        assert_eq!(dash.page_count(), 1);
        // 13 apps, 12 per page => 2 pages.
        let dash = DashboardState::new(psp_config(), test_apps(13));
        assert_eq!(dash.page_count(), 2);
        // 24 apps, 12 per page => 2 pages (exact fill).
        let dash = DashboardState::new(psp_config(), test_apps(24));
        assert_eq!(dash.page_count(), 2);
    }

    #[test]
    fn psp_grid_page_switch_clamps_selected() {
        let mut dash = DashboardState::new(psp_config(), test_apps(14));
        // Page 0 has 12 apps, page 1 has 2.
        dash.selected = 11; // Last on page 0.
        dash.next_page();
        // Page 1 has only 2 apps; selected should clamp to 0 or 1.
        assert!(dash.selected < 2);
    }

    #[test]
    fn psp_grid_down_blocked_by_incomplete_row() {
        // 9 apps in a 4-col grid: rows are [0..3], [4..7], [8].
        let mut dash = DashboardState::new(psp_config(), test_apps(9));
        dash.selected = 5; // Row 1, col 1.
        // Down to row 2: only position 8 exists, and 5+4=9 >= 9 apps, stays.
        dash.handle_input(&Button::Down);
        assert_eq!(dash.selected, 5); // Can't go down -- row 2 col 1 doesn't exist.
    }

    /// PSP uses compact grid: 4 cols, 3 rows, 12 icons per page.
    /// `DashboardConfig::from_features` with PSP-specific ActiveTheme
    /// overrides should produce the correct cell geometry.
    #[test]
    fn psp_dashboard_config_from_features() {
        let mut psp_at = crate::active_theme::ActiveTheme::from_base_colors(
            Color::rgb(0x1A, 0x1A, 0x2D),
            Color::rgb(0x32, 0x64, 0xC8),
            Color::rgb(0x50, 0x50, 0x50),
            Color::WHITE,
            Color::rgb(0x80, 0x80, 0x80),
            Color::rgb(0x28, 0x3C, 0x5A),
            Color::rgb(0x00, 0xFF, 0x00),
            Color::rgb(0xCC, 0xCC, 0xCC),
            Color::rgb(0xFF, 0x44, 0x44),
        )
        .with_screen_size(480, 272);

        // Simulate PSP overrides from skins.rs::apply_psp_overrides.
        psp_at.statusbar_height = 18;
        psp_at.tab_row_height = 0;
        psp_at.bottombar_height = 32;
        psp_at.icon_width = 34;
        psp_at.icon_height = 34;
        psp_at.grid_padding_x = 8;
        psp_at.grid_padding_y = 2;
        psp_at.cursor_pad = 2;

        let mut features = crate::skin::SkinFeatures::default();
        features.grid_cols = 4;
        features.grid_rows = 3;
        features.icons_per_page = 12;

        let config = DashboardConfig::from_features(&features, &psp_at);
        assert_eq!(config.grid_cols, 4);
        assert_eq!(config.grid_rows, 3);
        assert_eq!(config.icons_per_page, 12);
        // Content area: 272 - 18 (statusbar) - 0 (tab) - 32 (bottom) = 222
        // Grid height: 222 - 2*2 = 218, cell height: 218 / 3 = 72
        assert_eq!(config.cell_h, 72);
        // Grid width: 480 - 2*8 = 464, cell width: 464 / 4 = 116
        assert_eq!(config.cell_w, 116);
        assert_eq!(config.cursor_pad, 2);
    }
}

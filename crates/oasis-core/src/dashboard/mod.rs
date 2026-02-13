//! Classic skin dashboard -- PSP-style icon grid with cursor navigation.
//!
//! The dashboard manages a paginated icon grid, status bar, and cursor.
//! It creates and updates SDI objects based on its internal state.

mod discovery;

pub use discovery::{AppEntry, discover_apps};

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::skin::SkinFeatures;
use crate::theme;
use crate::ui::flex::GridLayout;
use crate::ui::layout::Padding;

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
}

impl DashboardConfig {
    /// Create a config from skin features and screen dimensions.
    /// Uses PSIX-style layout: icons on the left side with generous spacing.
    pub fn from_features(features: &SkinFeatures, at: &ActiveTheme) -> Self {
        let cols = features.grid_cols;
        let rows = features.grid_rows;
        let content_top = at.statusbar_height + at.tab_row_height;
        let content_h = theme::SCREEN_H - content_top - at.bottombar_height;
        let grid_padding_x = 16u16;
        let grid_padding_y = 6u16;
        let grid_x = grid_padding_x as i32;
        let grid_y = (content_top + grid_padding_y as u32) as i32;
        let grid_w = theme::SCREEN_W - 2 * grid_padding_x as u32;
        let grid_h = content_h - 2 * grid_padding_y as u32;

        // Size cells to fill available space evenly.
        let cell_w = grid_w / cols;
        let cell_h = grid_h / rows;

        let grid_layout = GridLayout::new(cols).with_padding(Padding::ZERO);

        Self {
            grid_cols: cols,
            grid_rows: rows,
            icons_per_page: features.icons_per_page,
            max_pages: features.dashboard_pages,
            grid_x,
            grid_y,
            cell_w,
            cell_h,
            cursor_pad: 3,
            grid_layout,
            grid_w,
            grid_h,
        }
    }
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
}

impl DashboardState {
    /// Create a new dashboard with the given config and app list.
    pub fn new(config: DashboardConfig, apps: Vec<AppEntry>) -> Self {
        Self {
            config,
            apps,
            page: 0,
            selected: 0,
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
            Button::Up => {
                if self.selected >= cols {
                    self.selected -= cols;
                }
            },
            _ => {},
        }
    }

    /// Switch to the next page (wraps around).
    pub fn next_page(&mut self) {
        let count = self.page_count();
        self.page = (self.page + 1) % count;
        let page_apps = self.current_page_apps().len();
        if self.selected >= page_apps && page_apps > 0 {
            self.selected = page_apps - 1;
        }
    }

    /// Switch to the previous page (wraps around).
    pub fn prev_page(&mut self) {
        let count = self.page_count();
        if self.page == 0 {
            self.page = count - 1;
        } else {
            self.page -= 1;
        }
        let page_apps = self.current_page_apps().len();
        if self.selected >= page_apps && page_apps > 0 {
            self.selected = page_apps - 1;
        }
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
    pub fn update_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let cols = self.config.grid_cols as usize;
        let page_apps = self.current_page_apps();

        let icon_w = at.icon_width;
        let icon_h = at.icon_height;
        let text_pad = theme::ICON_LABEL_PAD;

        let per_page = self.config.icons_per_page as usize;
        for i in 0..per_page {
            let outline_name = format!("icon_outline_{i}");
            let icon_name = format!("icon_{i}");
            let stripe_name = format!("icon_stripe_{i}");
            let fold_name = format!("icon_fold_{i}");
            let gfx_name = format!("icon_gfx_{i}");
            let label_name = format!("icon_label_{i}");
            let label2_name = format!("icon_label2_{i}");
            let shadow_name = format!("icon_shadow_{i}");
            let shadow2_name = format!("icon_shadow2_{i}");

            for name in [
                &outline_name,
                &icon_name,
                &stripe_name,
                &fold_name,
                &gfx_name,
                &label_name,
                &label2_name,
                &shadow_name,
                &shadow2_name,
            ] {
                if !sdi.contains(name) {
                    sdi.create(name);
                }
            }

            let cell = self.config.grid_layout.cell_rect(
                i,
                self.config.grid_x,
                self.config.grid_y,
                self.config.grid_w,
                self.config.grid_h,
                per_page,
            );
            let (cell_x, cell_y) = match cell {
                Some(r) => (r.x, r.y),
                None => continue,
            };
            let ix = cell_x + (self.config.cell_w as i32 - icon_w as i32) / 2;
            let iy = cell_y + (self.config.cell_h as i32 - icon_h as i32) / 4;

            if i < page_apps.len() {
                match at.icon_style.as_str() {
                    "card" => self.draw_card_icon(
                        sdi,
                        at,
                        i,
                        ix,
                        iy,
                        icon_w,
                        icon_h,
                        cell_x,
                        &page_apps[i],
                        text_pad,
                    ),
                    "circle" => self.draw_circle_icon(
                        sdi,
                        at,
                        i,
                        ix,
                        iy,
                        icon_w,
                        icon_h,
                        cell_x,
                        &page_apps[i],
                        text_pad,
                    ),
                    _ => self.draw_document_icon(
                        sdi,
                        at,
                        i,
                        ix,
                        iy,
                        icon_w,
                        icon_h,
                        cell_x,
                        &page_apps[i],
                        text_pad,
                    ),
                }
            } else {
                for name in [
                    &outline_name,
                    &icon_name,
                    &stripe_name,
                    &fold_name,
                    &gfx_name,
                    &label_name,
                    &label2_name,
                    &shadow_name,
                    &shadow2_name,
                ] {
                    if let Ok(obj) = sdi.get_mut(name) {
                        obj.visible = false;
                    }
                }
            }
        }

        // Cursor highlight.
        let cursor_name = "cursor_highlight";
        if !sdi.contains(cursor_name) {
            sdi.create(cursor_name);
        }
        if let Ok(cursor) = sdi.get_mut(cursor_name) {
            if !page_apps.is_empty() {
                let sel_col = (self.selected % cols) as i32;
                let sel_row = (self.selected / cols) as i32;
                let pad = self.config.cursor_pad;
                let cell_x = self.config.grid_x + sel_col * self.config.cell_w as i32;
                let cell_y = self.config.grid_y + sel_row * self.config.cell_h as i32;
                let ix = cell_x + (self.config.cell_w as i32 - icon_w as i32) / 2;
                let iy = cell_y + (self.config.cell_h as i32 - icon_h as i32) / 4;

                cursor.visible = true;
                cursor.overlay = true;

                // Include label area (icon + gap + up to 2 lines).
                let glyph_h = at.font_small.max(8) as u32;
                let label_h = text_pad as u32 + glyph_h * 2 + 1;
                let total_h = icon_h + label_h;

                match at.cursor_style.as_str() {
                    "fill" => {
                        cursor.x = ix - pad;
                        cursor.y = iy - pad;
                        cursor.w = icon_w + (pad * 2) as u32;
                        cursor.h = total_h + (pad * 2) as u32;
                        cursor.color = at.cursor_color;
                        cursor.border_radius = Some(at.cursor_border_radius);
                        cursor.stroke_width = None;
                        cursor.stroke_color = None;
                    },
                    "underline" => {
                        cursor.x = cell_x;
                        cursor.y = iy + icon_h as i32 + text_pad + glyph_h as i32 * 2 + 2;
                        cursor.w = self.config.cell_w;
                        cursor.h = 3;
                        cursor.color = at.cursor_color;
                        cursor.border_radius = Some(1);
                        cursor.stroke_width = None;
                        cursor.stroke_color = None;
                    },
                    _ => {
                        // "stroke" (default)
                        cursor.x = ix - pad;
                        cursor.y = iy - pad;
                        cursor.w = icon_w + (pad * 2) as u32;
                        cursor.h = total_h + (pad * 2) as u32;
                        cursor.color = Color::rgba(0, 0, 0, 0);
                        cursor.border_radius = Some(at.cursor_border_radius);
                        cursor.stroke_width = Some(at.cursor_stroke_width);
                        cursor.stroke_color = Some(at.cursor_color);
                    },
                }
            } else {
                cursor.visible = false;
            }
        }
    }

    /// Word-wrap a label into lines that fit within `max_chars` per line.
    fn wrap_label(text: &str, max_chars: usize) -> Vec<String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return vec![];
        }
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in words {
            let test_len = if cur.is_empty() {
                word.len()
            } else {
                cur.len() + 1 + word.len()
            };
            if !cur.is_empty() && test_len > max_chars {
                lines.push(cur);
                cur = word.to_string();
            } else {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(word);
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// Render word-wrapped, centered label lines under an icon.
    #[allow(clippy::too_many_arguments)]
    fn draw_label(
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        i: usize,
        cell_x: i32,
        cell_w: u32,
        label_y: i32,
        title: &str,
    ) {
        let fs = at.font_small;
        let glyph_w = (fs.max(8) / 8) as u32 * 8;
        let max_chars = (cell_w / glyph_w).max(1) as usize;
        let lines = Self::wrap_label(title, max_chars);
        let line_h = glyph_w as i32 + 1; // 1px spacing between lines

        // Label shadows (1px offset).
        if let Some(shadow_color) = at.icon_label_shadow {
            // Shadow for line 1.
            if let Ok(obj) = sdi.get_mut(&format!("icon_shadow_{i}")) {
                if let Some(line) = lines.first() {
                    let tw = line.len() as i32 * glyph_w as i32;
                    obj.x = cell_x + (cell_w as i32 - tw) / 2 + 1;
                    obj.y = label_y + 1;
                    obj.w = 0;
                    obj.h = 0;
                    obj.font_size = fs;
                    obj.text = Some(line.clone());
                    obj.text_color = shadow_color;
                    obj.visible = true;
                    obj.color = Color::rgba(0, 0, 0, 0);
                } else {
                    obj.visible = false;
                }
            }
            // Shadow for line 2.
            if let Ok(obj) = sdi.get_mut(&format!("icon_shadow2_{i}")) {
                if lines.len() > 1 {
                    let tw = lines[1].len() as i32 * glyph_w as i32;
                    obj.x = cell_x + (cell_w as i32 - tw) / 2 + 1;
                    obj.y = label_y + line_h + 1;
                    obj.w = 0;
                    obj.h = 0;
                    obj.font_size = fs;
                    obj.text = Some(lines[1].clone());
                    obj.text_color = shadow_color;
                    obj.visible = true;
                    obj.color = Color::rgba(0, 0, 0, 0);
                } else {
                    obj.visible = false;
                }
            }
        } else {
            if let Ok(obj) = sdi.get_mut(&format!("icon_shadow_{i}")) {
                obj.visible = false;
            }
            if let Ok(obj) = sdi.get_mut(&format!("icon_shadow2_{i}")) {
                obj.visible = false;
            }
        }

        // Line 1.
        if let Ok(obj) = sdi.get_mut(&format!("icon_label_{i}")) {
            if let Some(line) = lines.first() {
                let tw = line.len() as i32 * glyph_w as i32;
                obj.x = cell_x + (cell_w as i32 - tw) / 2;
                obj.y = label_y;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.text = Some(line.clone());
                obj.text_color = at.icon_label_color;
                obj.visible = true;
            } else {
                obj.visible = false;
            }
        }
        // Line 2.
        if let Ok(obj) = sdi.get_mut(&format!("icon_label2_{i}")) {
            if lines.len() > 1 {
                let tw = lines[1].len() as i32 * glyph_w as i32;
                obj.x = cell_x + (cell_w as i32 - tw) / 2;
                obj.y = label_y + line_h;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.text = Some(lines[1].clone());
                obj.text_color = at.icon_label_color;
                obj.visible = true;
            } else {
                obj.visible = false;
            }
        }
    }

    /// Draw a "document" style icon (default PSIX: white page, fold, stripe, gfx).
    #[allow(clippy::too_many_arguments)]
    fn draw_document_icon(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        i: usize,
        ix: i32,
        iy: i32,
        icon_w: u32,
        icon_h: u32,
        cell_x: i32,
        app: &AppEntry,
        text_pad: i32,
    ) {
        let stripe_h = theme::ICON_STRIPE_H;
        let fold_size = theme::ICON_FOLD_SIZE;
        let gfx_pad = theme::ICON_GFX_PAD;
        let gfx_w = icon_w - 2 * gfx_pad;
        let gfx_h = theme::ICON_GFX_H;

        if let Ok(obj) = sdi.get_mut(&format!("icon_outline_{i}")) {
            obj.x = ix - 1;
            obj.y = iy - 1;
            obj.w = icon_w + 2;
            obj.h = icon_h + 2;
            obj.visible = true;
            obj.color = Color::rgba(0, 0, 0, 0);
            obj.text = None;
            obj.border_radius = Some(at.icon_border_radius + 1);
            obj.stroke_width = Some(1);
            obj.stroke_color = Some(at.icon_outline_color);
        }
        if let Ok(obj) = sdi.get_mut(&format!("icon_{i}")) {
            obj.x = ix;
            obj.y = iy;
            obj.w = icon_w;
            obj.h = icon_h;
            obj.visible = true;
            obj.color = at.icon_body_color;
            obj.text = None;
            obj.border_radius = Some(at.icon_border_radius);
            obj.shadow_level = Some(1);
        }
        if let Ok(obj) = sdi.get_mut(&format!("icon_stripe_{i}")) {
            let r = at.icon_border_radius as u32;
            obj.x = ix + r as i32;
            obj.y = iy;
            obj.w = icon_w - fold_size - r;
            obj.h = stripe_h;
            obj.visible = true;
            obj.color = app.color;
            obj.text = None;
        }
        if let Ok(obj) = sdi.get_mut(&format!("icon_fold_{i}")) {
            obj.x = ix + icon_w as i32 - fold_size as i32;
            obj.y = iy;
            obj.w = fold_size;
            obj.h = fold_size;
            obj.visible = true;
            obj.color = at.icon_fold_color;
            obj.text = None;
        }
        if let Ok(obj) = sdi.get_mut(&format!("icon_gfx_{i}")) {
            obj.x = ix + gfx_pad as i32;
            obj.y = iy + stripe_h as i32 + 3;
            obj.w = gfx_w;
            obj.h = gfx_h;
            obj.visible = true;
            let c = app.color;
            obj.color = Color::rgba(
                c.r.saturating_add(30),
                c.g.saturating_add(10),
                c.b.saturating_add(30),
                200,
            );
            obj.text = None;
        }
        Self::draw_label(
            sdi,
            at,
            i,
            cell_x,
            self.config.cell_w,
            iy + icon_h as i32 + text_pad,
            &app.title,
        );
    }

    /// Draw a "card" style icon (flat rounded rect with accent fill, centered label).
    #[allow(clippy::too_many_arguments)]
    fn draw_card_icon(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        i: usize,
        ix: i32,
        iy: i32,
        icon_w: u32,
        icon_h: u32,
        cell_x: i32,
        app: &AppEntry,
        text_pad: i32,
    ) {
        // Hide document-specific sub-objects.
        for prefix in &["icon_outline_", "icon_stripe_", "icon_fold_", "icon_gfx_"] {
            if let Ok(obj) = sdi.get_mut(&format!("{prefix}{i}")) {
                obj.visible = false;
            }
        }
        // Card body: full-bleed accent color.
        if let Ok(obj) = sdi.get_mut(&format!("icon_{i}")) {
            obj.x = ix;
            obj.y = iy;
            obj.w = icon_w;
            obj.h = icon_h;
            obj.visible = true;
            obj.color = app.color;
            obj.text = None;
            obj.border_radius = Some(at.icon_border_radius);
            obj.shadow_level = Some(1);
        }
        // Label below icon.
        Self::draw_label(
            sdi,
            at,
            i,
            cell_x,
            self.config.cell_w,
            iy + icon_h as i32 + text_pad,
            &app.title,
        );
    }

    /// Draw a "circle" style icon (large circle with first letter centered).
    #[allow(clippy::too_many_arguments)]
    fn draw_circle_icon(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        i: usize,
        ix: i32,
        iy: i32,
        icon_w: u32,
        icon_h: u32,
        cell_x: i32,
        app: &AppEntry,
        text_pad: i32,
    ) {
        // Hide document-specific sub-objects.
        for prefix in &["icon_outline_", "icon_stripe_", "icon_fold_", "icon_gfx_"] {
            if let Ok(obj) = sdi.get_mut(&format!("{prefix}{i}")) {
                obj.visible = false;
            }
        }
        // Circle body: use min dimension for a circle.
        let diameter = icon_w.min(icon_h);
        let radius = (diameter / 2) as u16;
        if let Ok(obj) = sdi.get_mut(&format!("icon_{i}")) {
            obj.x = ix + (icon_w as i32 - diameter as i32) / 2;
            obj.y = iy + (icon_h as i32 - diameter as i32) / 2;
            obj.w = diameter;
            obj.h = diameter;
            obj.visible = true;
            obj.color = app.color;
            obj.text = None;
            obj.border_radius = Some(radius);
            obj.shadow_level = Some(1);
        }
        // Label below icon.
        Self::draw_label(
            sdi,
            at,
            i,
            cell_x,
            self.config.cell_w,
            iy + icon_h as i32 + text_pad,
            &app.title,
        );
    }

    /// Hide all dashboard SDI objects.
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        let per_page = self.config.icons_per_page as usize;
        for i in 0..per_page {
            for prefix in &[
                "icon_",
                "icon_label_",
                "icon_label2_",
                "icon_outline_",
                "icon_stripe_",
                "icon_fold_",
                "icon_gfx_",
                "icon_shadow_",
                "icon_shadow2_",
            ] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
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
mod tests {
    use super::*;

    fn test_config() -> DashboardConfig {
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
        }
    }

    fn test_apps(n: usize) -> Vec<AppEntry> {
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
        let dash = DashboardState::new(test_config(), test_apps(3));
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
    fn selected_clamps_on_page_switch() {
        let mut dash = DashboardState::new(test_config(), test_apps(5));
        // 5 apps, 4 per page: page 0 has 4, page 1 has 1.
        dash.selected = 3; // Last on page 0.
        dash.next_page();
        // Page 1 has only 1 app, so selected should clamp to 0.
        assert_eq!(dash.selected, 0);
    }
}

//! SDI rendering for the app runner (update_sdi / hide_sdi).

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::ui::flex;

use super::layout_calc::AppLayout;
use super::runner::AppRunner;
use oasis_app_tv_guide::guide::TvGuideState;

impl AppRunner {
    /// Render the app screen as SDI objects (single-display-interface mode).
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Delegate to extracted app.
        if let Some(ref mut app) = self.delegate {
            app.update_sdi(sdi, at);
            return;
        }

        // TV Guide uses its own custom grid rendering.
        if let Some(ref mut guide) = self.tv_guide {
            guide.update_sdi(sdi, at);
            return;
        }

        // Full-screen background.
        if !sdi.contains("app_bg") {
            sdi.create("app_bg");
        }
        if let Ok(obj) = sdi.get_mut("app_bg") {
            obj.x = 0;
            obj.y = 0;
            obj.w = at.screen_w;
            obj.h = at.screen_h;
            obj.color = at.app.bg;
            obj.visible = true;
            obj.z = 100;
        }

        // Title bar background.
        if !sdi.contains("app_title_bg") {
            sdi.create("app_title_bg");
        }
        if let Ok(obj) = sdi.get_mut("app_title_bg") {
            obj.x = 0;
            obj.y = 0;
            obj.w = at.screen_w;
            obj.h = at.app.title_bar_height;
            obj.color = at.app.title_bar_bg;
            obj.gradient_top = at.app.title_bar_gradient_top;
            obj.gradient_bottom = at.app.title_bar_gradient_bottom;
            obj.shadow_level = Some(1);
            obj.visible = true;
            obj.z = 101;
        }

        // Cache dynamic max-visible for input handling.
        self.cached_max_visible = AppLayout::compute(at, 14).max_visible;

        // Title text.
        if !sdi.contains("app_title_text") {
            sdi.create("app_title_text");
        }

        if let Ok(obj) = sdi.get_mut("app_title_text") {
            let dir_suffix = if let Some(ref file) = self.viewing_file {
                format!("  [{file}]")
            } else {
                self.browse_dir
                    .as_deref()
                    .map(|d| format!("  [{d}]"))
                    .unwrap_or_default()
            };
            obj.text = Some(format!("{}{dir_suffix}", self.title));
            obj.x = 8;
            obj.y = 4;
            obj.font_size = at.font_body;
            obj.text_color = at.app.title_bar_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
            if at.app.title_bar_text_shadow {
                obj.text_shadow_offset = Some((1, 1));
                obj.text_shadow_color = Some(at.app.title_bar_text_shadow_color);
            } else {
                obj.text_shadow_offset = None;
                obj.text_shadow_color = None;
            }
        }

        // Content lines -- responsive to screen resolution.
        let app_layout = AppLayout::compute(at, 14);
        let line_rects = flex::vertical_list(
            app_layout.content_x,
            app_layout.content_y,
            app_layout.content_w,
            app_layout.line_h,
            0,
            app_layout.max_visible,
        );

        // Smooth selection lerp.
        self.visual_selected +=
            (self.cursor as f32 - self.visual_selected) * at.app_selection_lerp_speed;

        // Selection highlight background.
        if !sdi.contains("app_sel_bg") {
            sdi.create("app_sel_bg");
        }
        let sel_y = app_layout.content_y + (self.visual_selected * app_layout.line_h as f32) as i32;
        if let Ok(obj) = sdi.get_mut("app_sel_bg") {
            obj.x = app_layout.content_x;
            obj.y = sel_y;
            obj.w = app_layout.content_w;
            obj.h = at.terminal_line_height;
            obj.color = at.app.selected_bg;
            obj.border_radius = Some(at.app.selection_border_radius);
            obj.visible = !self.lines.is_empty();
            obj.z = 101;
        }
        // Selection accent bar (left edge).
        if !sdi.contains("app_sel_accent") {
            sdi.create("app_sel_accent");
        }
        if let Ok(obj) = sdi.get_mut("app_sel_accent") {
            obj.x = app_layout.content_x;
            obj.y = sel_y;
            obj.w = 3;
            obj.h = at.terminal_line_height;
            obj.color = at.app.selection_accent_color;
            obj.border_radius = Some(at.app.selection_border_radius);
            obj.visible = !self.lines.is_empty();
            obj.z = 102;
        }

        for (i, rect) in line_rects.iter().enumerate() {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                sdi.create(&name);
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                let line_idx = self.scroll + i;
                if line_idx < self.lines.len() {
                    obj.text = Some(self.lines[line_idx].clone());
                    obj.visible = true;
                } else {
                    obj.text = None;
                    obj.visible = false;
                }
                obj.x = rect.x + 6;
                obj.y = rect.y;
                obj.font_size = at.font_body;
                obj.text_color = if i == self.cursor {
                    at.app.selected_text
                } else {
                    at.app.text
                };
                obj.w = 0;
                obj.h = 0;
                obj.z = 102;
            }
        }

        // Scroll indicator.
        if !sdi.contains("app_scroll") {
            sdi.create("app_scroll");
        }
        if let Ok(obj) = sdi.get_mut("app_scroll") {
            if self.lines.len() > app_layout.max_visible {
                obj.text = Some(format!(
                    "[{}/{}]  Cancel=back",
                    self.scroll + 1,
                    self.lines.len().saturating_sub(app_layout.max_visible) + 1,
                ));
            } else {
                obj.text = Some("Cancel=back".to_string());
            }
            obj.x = 8;
            obj.y = at.screen_h as i32 - 14;
            obj.font_size = at.font_hint;
            obj.text_color = at.app.dim_text;
            obj.w = 0;
            obj.h = 0;
            obj.visible = true;
            obj.z = 102;
        }
    }

    /// Hide all app-related SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        let fixed = [
            "app_bg",
            "app_title_bg",
            "app_title_text",
            "app_scroll",
            "app_divider",
            "app_sel_bg",
            "app_sel_accent",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        // Hide up to a generous upper bound (handles all resolutions).
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
        for i in 0..100 {
            let lp = format!("app_lp_line_{i}");
            if !sdi.contains(&lp) {
                break;
            }
            let rp = format!("app_rp_line_{i}");
            if let Ok(obj) = sdi.get_mut(&lp) {
                obj.visible = false;
            }
            if let Ok(obj) = sdi.get_mut(&rp) {
                obj.visible = false;
            }
        }

        // Hide TV Guide objects.
        TvGuideState::hide_sdi(sdi);
    }
}

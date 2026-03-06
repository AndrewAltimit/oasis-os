//! Vector icon rendering for the dashboard.
//!
//! When `icon_style = "vector"`, icons are drawn using `oasis_vector`
//! primitives rather than SDI object manipulation. Each app gets a vector
//! icon from a preset (e.g. "altimit") that is recolored with the app's
//! assigned color and rendered directly to the backend.

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_vector::icons::{self, IconDef};
use oasis_vector::render::render_scene_at;
use oasis_vector::scene::VectorScene;

use crate::active_theme::ActiveTheme;

use super::{AppEntry, DashboardState, IconGeometry, IconNames};

/// Get a vector icon for an app based on the preset and app index.
///
/// The "altimit" preset cycles through the 6 Altimit-inspired icons.
/// Apps are colored with their assigned fallback color.
pub(super) fn icon_for_app(preset: &str, app: &AppEntry, index: usize) -> IconDef {
    match preset {
        "altimit" => altimit_icon(app.color, index),
        _ => altimit_icon(app.color, index),
    }
}

/// Select an Altimit icon by cycling through the 6 available designs.
fn altimit_icon(color: Color, index: usize) -> IconDef {
    match index % 6 {
        0 => icons::icon_the_world(color),
        1 => icons::icon_mailer(color),
        2 => icons::icon_news(color),
        3 => icons::icon_accessory(color, Color::rgba(0, 0, 0, 60)),
        4 => icons::icon_audio(color),
        _ => icons::icon_data(color, Color::rgb(0, 200, 100)),
    }
}

impl DashboardState {
    /// Draw a "vector" style icon — hides SDI sub-objects and prepares a
    /// vector scene for later rendering via [`render_vector_icons`].
    pub(super) fn draw_vector_icon(
        &self,
        sdi: &mut crate::sdi::SdiRegistry,
        at: &ActiveTheme,
        names: &IconNames,
        geo: IconGeometry,
        slot: usize,
        app: &AppEntry,
    ) {
        let IconGeometry {
            ix,
            iy,
            icon_w,
            icon_h,
            cell_x,
            text_pad,
        } = geo;

        // Hide all SDI sub-objects (vector rendering is done directly).
        for name in [
            &names.outline,
            &names.stripe,
            &names.fold,
            &names.gfx,
            &names.icon,
        ] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }

        // Draw label using the shared label renderer (still SDI-based).
        Self::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.config.cell_w,
            iy + icon_h as i32 + text_pad,
            &app.title,
        );

        // Store the vector icon slot for rendering in render_vector_icons().
        // We use the slot index to find the right entry later.
        let _ = (ix, iy, icon_w, icon_h, slot);
    }

    /// Render vector icons directly to the backend.
    ///
    /// Call this between `sdi.draw_base_layer()` and `sdi.draw_overlay_layer()`
    /// to maintain correct z-order (above wallpaper, below cursor/overlays).
    pub fn render_vector_icons(
        &self,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> Result<()> {
        if at.icon.style != "vector" {
            return Ok(());
        }

        let page_apps = self.current_page_apps();
        if page_apps.is_empty() {
            return Ok(());
        }

        let icon_w = at.icon_width;
        let icon_h = at.icon_height;
        let preset = &at.icon.vector_preset;

        // Page-slide offset for animation.
        let slide_offset = if let Some(ref anim) = self.page_anim {
            let t = crate::transition::ease_in_out_cubic(anim.frame as f32 / anim.duration as f32);
            let w = at.screen_w as f32;
            ((1.0 - t) * w * anim.direction as f32) as i32
        } else {
            0
        };

        let per_page = self.config.icons_per_page as usize;
        let page_start = self.page * per_page;

        for (i, app) in page_apps.iter().enumerate() {
            let cell = self.config.grid_layout.cell_rect(
                i,
                self.config.grid_x,
                self.config.grid_y,
                self.config.grid_w,
                self.config.grid_h,
                per_page,
            );
            let (cell_x, cell_y) = match cell {
                Some(r) => (r.x + slide_offset, r.y),
                None => continue,
            };
            let ix = cell_x + (self.config.cell_w as i32 - icon_w as i32) / 2;
            let iy = cell_y + (self.config.cell_h as i32 - icon_h as i32) / 4;

            // Get vector icon for this app.
            let global_index = page_start + i;
            let mut icon = icon_for_app(preset, app, global_index);

            // Apply press flash effect.
            if self.press_flash_frame > 0 && i == self.press_flash_index {
                icon.recolor(oasis_types::color::lighten(
                    app.color,
                    at.press_flash_lighten,
                ));
            }

            // Build a scene from the icon and render it.
            let mut scene = VectorScene::new(icon.width, icon.height);
            for op in icon.ops {
                scene.push(op);
            }

            // Scale the icon to fill the icon cell.
            // Icons are 22x22 base, scale to icon_w x icon_h.
            let scale_x = icon_w as f32 / scene.width.max(1) as f32;
            let scale_y = icon_h as f32 / scene.height.max(1) as f32;
            let scale = scale_x.min(scale_y);

            // Center the scaled icon within the cell.
            let scaled_w = (scene.width as f32 * scale) as i32;
            let scaled_h = (scene.height as f32 * scale) as i32;
            let ox = ix + (icon_w as i32 - scaled_w) / 2;
            let oy = iy + (icon_h as i32 - scaled_h) / 2;

            // For now, render at 1:1 scale centered in the cell.
            // True scaling would require coordinate transform in VectorOp.
            render_scene_at(backend, &scene, ox, oy, 255)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_for_app_cycles_presets() {
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };

        let i0 = icon_for_app("altimit", &app, 0);
        assert_eq!(i0.name, "the_world");

        let i1 = icon_for_app("altimit", &app, 1);
        assert_eq!(i1.name, "mailer");

        let i5 = icon_for_app("altimit", &app, 5);
        assert_eq!(i5.name, "data");

        // Wraps around.
        let i6 = icon_for_app("altimit", &app, 6);
        assert_eq!(i6.name, "the_world");
    }

    #[test]
    fn icon_for_app_uses_app_color() {
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::rgb(255, 0, 0),
        };
        let icon = icon_for_app("altimit", &app, 0);
        // First op should use the app's color.
        match &icon.ops[0] {
            oasis_vector::op::VectorOp::StrokeRect { color, .. } => {
                assert_eq!(*color, Color::rgb(255, 0, 0));
            },
            _ => panic!("expected StrokeRect for the_world icon"),
        }
    }

    #[test]
    fn unknown_preset_defaults_to_altimit() {
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let icon = icon_for_app("nonexistent", &app, 0);
        assert_eq!(icon.name, "the_world");
    }
}

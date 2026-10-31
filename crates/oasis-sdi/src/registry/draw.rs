//! Drawing: z-ordered base / overlay passes and per-object dispatch.

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_types::shadow::Shadow;

use super::SdiRegistry;
use crate::object::SdiObject;

impl SdiRegistry {
    /// Draw all visible objects to the backend, sorted by z-order (ascending).
    /// Uses PSIX-style two-pass rendering: base-layer objects first, then
    /// overlay objects on top. Each pass iterates only its pre-partitioned
    /// list, avoiding per-object overlay checks.
    pub fn draw(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.draw_base_layer(backend)?;
        self.draw_overlay_layer(backend)
    }

    /// Draw only the base layer (non-overlay objects), sorted by z-order.
    ///
    /// Use this with [`Self::draw_overlay_layer`] when you need to insert custom
    /// rendering (e.g., vector scenes) between the base and overlay passes.
    pub fn draw_base_layer(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.ensure_z_sorted();

        for &handle in &self.z_sorted_base {
            let obj = &self.objects[handle];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw only the overlay layer, sorted by z-order.
    ///
    /// Call after [`Self::draw_base_layer`] and any custom mid-layer rendering.
    /// Re-sorts first if objects were created, destroyed or re-ordered, so it
    /// is also safe to call on its own.
    pub fn draw_overlay_layer(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.ensure_z_sorted();
        for &handle in &self.z_sorted_overlay {
            let obj = &self.objects[handle];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw a single named object (if it exists and is visible).
    pub fn draw_named(&self, name: &str, backend: &mut dyn SdiBackend) -> Result<()> {
        if let Some(obj) = self.index.get(name).map(|&h| &self.objects[h])
            && obj.visible
            && obj.alpha > 0
        {
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw all visible objects EXCEPT those whose names start with any of the
    /// given prefixes (e.g., window id prefixes). Useful for drawing
    /// non-window objects separately from window objects.
    pub fn draw_excluding_prefixes(
        &mut self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.ensure_z_sorted();
        let keep = |name: &str| !prefixes.iter().any(|p| name.starts_with(p));
        self.draw_layer_filtered(&self.z_sorted_base, backend, &keep)?;
        self.draw_layer_filtered(&self.z_sorted_overlay, backend, &keep)
    }

    /// Draw only the base layer, excluding objects with given prefixes.
    pub fn draw_base_excluding_prefixes(
        &mut self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.draw_base_filtered(backend, |name| {
            !prefixes.iter().any(|p| name.starts_with(p))
        })
    }

    /// Draw only the overlay layer, excluding objects with given prefixes.
    pub fn draw_overlay_excluding_prefixes(
        &mut self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.draw_overlay_filtered(backend, |name| {
            !prefixes.iter().any(|p| name.starts_with(p))
        })
    }

    /// Draw only the base layer, keeping objects for which `keep` returns true.
    pub fn draw_base_filtered<F>(&mut self, backend: &mut dyn SdiBackend, keep: F) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        self.ensure_z_sorted();
        self.draw_layer_filtered(&self.z_sorted_base, backend, &keep)
    }

    /// Draw only the overlay layer, keeping objects for which `keep` returns true.
    pub fn draw_overlay_filtered<F>(&mut self, backend: &mut dyn SdiBackend, keep: F) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        self.ensure_z_sorted();
        self.draw_layer_filtered(&self.z_sorted_overlay, backend, &keep)
    }

    /// Internal: draw objects from a z-sorted list, applying a filter.
    fn draw_layer_filtered(
        &self,
        layer: &[usize],
        backend: &mut dyn SdiBackend,
        keep: &dyn Fn(&str) -> bool,
    ) -> Result<()> {
        for &handle in layer {
            let obj = &self.objects[handle];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            if !keep(&obj.name) {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Render a single SDI object to the backend.
    ///
    /// Dispatch order for non-textured objects with nonzero area:
    /// 1. Shadow (if `shadow_level > 0`)
    /// 2. Fill: gradient+radius → gradient → rounded → flat (existing)
    /// 3. Stroke (if `stroke_width` set)
    /// 4. Text (if present)
    ///
    /// The object's `alpha` scales every one of these (and a texture's
    /// tint), so a fading object fades as a whole -- including its label.
    fn draw_object(obj: &SdiObject, backend: &mut dyn SdiBackend) -> Result<()> {
        // Textured object -- blit the texture. A non-opaque `alpha` goes
        // through the tinted path so textures can fade/pulse (backends
        // without tint support fall back to a plain blit).
        if let Some(tex) = obj.texture {
            // Nine-patch object: 9-slice blit (corners fixed, edges/center
            // stretched). Alpha fade is not supported on this path.
            if let Some(slices) = obj.nine_patch {
                let patch = oasis_types::nine_patch::NinePatch::from_slices(tex, slices);
                patch.draw(backend, obj.x, obj.y, obj.w, obj.h)?;
                return Ok(());
            }
            if obj.alpha < 255 {
                let tint = Color::rgba(255, 255, 255, obj.alpha);
                backend.blit_tinted(tex, obj.x, obj.y, obj.w, obj.h, tint)?;
            } else {
                backend.blit(tex, obj.x, obj.y, obj.w, obj.h)?;
            }
            return Ok(());
        }

        let has_area = obj.w > 0 && obj.h > 0;
        let radius = obj.border_radius.unwrap_or(0);

        if has_area {
            // 1. Shadow behind the rect.
            if let Some(level) = obj.shadow_level
                && level > 0
            {
                let mut shadow = Shadow::elevation(level);
                if let Some(sc) = obj.shadow_color {
                    shadow = shadow.with_color(sc);
                }
                shadow = shadow.with_opacity(obj.alpha);
                shadow.draw(backend, obj.x, obj.y, obj.w, obj.h, radius)?;
            }

            // 2. Fill: choose the best primitive.
            let color = fade(obj.color, obj.alpha);

            if let (Some(gt), Some(gb)) = (obj.gradient_top, obj.gradient_bottom) {
                let top = fade(gt, obj.alpha);
                let bot = fade(gb, obj.alpha);
                let gradient = oasis_types::backend::GradientStyle::Vertical { top, bottom: bot };
                if radius > 0 {
                    backend.fill_rounded_rect_gradient(
                        obj.x, obj.y, obj.w, obj.h, radius, &gradient,
                    )?;
                } else {
                    backend.fill_rect_gradient(obj.x, obj.y, obj.w, obj.h, &gradient)?;
                }
            } else if radius > 0 {
                backend.fill_rounded_rect(obj.x, obj.y, obj.w, obj.h, radius, color)?;
            } else {
                backend.fill_rect(obj.x, obj.y, obj.w, obj.h, color)?;
            }

            // 3. Stroke outline.
            if let Some(sw) = obj.stroke_width
                && sw > 0
            {
                let sc = fade(
                    obj.stroke_color.unwrap_or(Color::rgba(255, 255, 255, 128)),
                    obj.alpha,
                );
                if radius > 0 {
                    backend.stroke_rounded_rect(obj.x, obj.y, obj.w, obj.h, radius, sw, sc)?;
                } else {
                    backend.stroke_rect(obj.x, obj.y, obj.w, obj.h, sw, sc)?;
                }
            }
        }

        // 4. Text on top (with optional shadow pass).
        if let Some(ref text) = obj.text {
            if let Some((dx, dy)) = obj.text_shadow_offset {
                let shadow_color = fade(
                    obj.text_shadow_color.unwrap_or(Color::rgba(0, 0, 0, 128)),
                    obj.alpha,
                );
                backend.draw_text(text, obj.x + dx, obj.y + dy, obj.font_size, shadow_color)?;
            }
            let text_color = fade(obj.text_color, obj.alpha);
            backend.draw_text(text, obj.x, obj.y, obj.font_size, text_color)?;
        }

        Ok(())
    }
}

/// Scale `c`'s alpha by an object-level `alpha` (255 = unchanged).
fn fade(c: Color, alpha: u8) -> Color {
    c.with_alpha(((c.a as u16) * (alpha as u16) / 255) as u8)
}

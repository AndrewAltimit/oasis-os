//! Icon shape rendering for the dashboard.
//!
//! Provides the "document", "card", and "circle" icon styles that compose
//! SDI objects for each icon slot on the grid.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;

use super::{AppEntry, DashboardState, IconGeometry, IconNames};

impl DashboardState {
    /// Draw a "document" style icon (default PSIX: white page, fold, stripe, gfx).
    pub(super) fn draw_document_icon(
        &self,
        sdi: &mut SdiRegistry,
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
            left_align,
        } = geo;
        let r = at.icon.border_radius as u32;
        // Clamp sub-element sizes to fit within the icon body's rounded rect.
        let stripe_h = at.icon_stripe_h.min(icon_h / 4);
        let fold_size = at.icon_fold_size.min(icon_w / 3).min(icon_h / 4);
        let gfx_pad = at.icon_gfx_pad.max(r);
        let gfx_w = icon_w.saturating_sub(2 * gfx_pad);
        let gfx_gap = 2u32;
        let gfx_h = at
            .icon_gfx_h
            .min(icon_h.saturating_sub(stripe_h + gfx_gap + r));

        if let Ok(obj) = sdi.get_mut(&names.outline) {
            obj.x = ix - 1;
            obj.y = iy - 1;
            obj.w = icon_w + 2;
            obj.h = icon_h + 2;
            obj.visible = true;
            obj.color = Color::rgba(0, 0, 0, 0);
            obj.text = None;
            obj.border_radius = Some(at.icon.border_radius + 1);
            obj.stroke_width = Some(1);
            obj.stroke_color = Some(at.icon.outline_color);
        }
        if let Ok(obj) = sdi.get_mut(&names.icon) {
            obj.x = ix;
            obj.y = iy;
            obj.w = icon_w;
            obj.h = icon_h;
            obj.visible = true;
            // Apply press flash effect: lighten the pressed icon.
            obj.color = if self.press_flash_frame > 0 && slot == self.press_flash_index {
                oasis_types::color::lighten(at.icon.body_color, at.press_flash_lighten)
            } else {
                at.icon.body_color
            };
            obj.text = None;
            obj.border_radius = Some(at.icon.border_radius);
            obj.shadow_level = Some(at.icon.shadow_level);
        }
        // Inset stripe below the top rounded corners.
        if let Ok(obj) = sdi.get_mut(&names.stripe) {
            let inset = r.min(2);
            obj.x = ix + r as i32;
            obj.y = iy + inset as i32;
            obj.w = icon_w.saturating_sub(fold_size + r);
            obj.h = stripe_h;
            obj.visible = stripe_h > 0;
            obj.color = app.color;
            obj.text = None;
        }
        if let Ok(obj) = sdi.get_mut(&names.fold) {
            obj.x = ix + icon_w as i32 - fold_size as i32;
            obj.y = iy;
            obj.w = fold_size;
            obj.h = fold_size;
            obj.visible = fold_size > 0;
            obj.color = at.icon.fold_color;
            obj.text = None;
        }
        if let Ok(obj) = sdi.get_mut(&names.gfx) {
            if at.icon.gfx_anchor == "badge" {
                // PSIX-style emblem badge overlapping the document's
                // bottom-right corner: a third hangs off the right edge,
                // the bottom sits roughly flush with the document bottom.
                let side = gfx_h.max(gfx_w.min(gfx_h + 2));
                obj.x = ix + icon_w as i32 - (side as i32 * 2) / 3;
                obj.y = iy + icon_h as i32 - side as i32 + 2;
                obj.w = side;
                obj.h = side;
                obj.border_radius = Some(2);
                // Solid saturated emblem with a dark edge (reference:
                // opaque crimson badge over the white page; the legacy
                // lighten+alpha tint washes out to salmon here).
                obj.color = app.color;
                obj.stroke_width = Some(1);
                obj.stroke_color = Some(oasis_types::color::darken(app.color, 0.35));
            } else {
                obj.x = ix + gfx_pad as i32;
                obj.y = iy + stripe_h as i32 + gfx_gap as i32;
                obj.w = gfx_w;
                obj.h = gfx_h;
                obj.border_radius = None;
                let c = app.color;
                obj.color =
                    oasis_types::color::with_alpha(oasis_types::color::lighten(c, 0.15), 200);
                obj.stroke_width = None;
                obj.stroke_color = None;
            }
            obj.visible = obj.w > 0 && obj.h > 0;
            obj.text = None;
        }
        super::labels::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.cell_size().0,
            iy + icon_h as i32 + text_pad,
            &app.title,
            left_align.then_some(ix + icon_w as i32 / 2),
            &self.label_wrap_cache,
        );
    }

    /// Draw a "card" style icon (rounded rect with gradient fill and outline).
    pub(super) fn draw_card_icon(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        names: &IconNames,
        geo: IconGeometry,
        app: &AppEntry,
    ) {
        let IconGeometry {
            ix,
            iy,
            icon_w,
            icon_h,
            cell_x,
            text_pad,
            left_align,
        } = geo;
        use oasis_types::color::{darken, lighten};

        // Hide document-specific sub-objects (stripe, fold, gfx).
        for name in [&names.stripe, &names.fold, &names.gfx] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        // Subtle outline for visual depth.
        if let Ok(obj) = sdi.get_mut(&names.outline) {
            obj.x = ix - 1;
            obj.y = iy - 1;
            obj.w = icon_w + 2;
            obj.h = icon_h + 2;
            obj.visible = true;
            obj.color = Color::rgba(0, 0, 0, 0);
            obj.text = None;
            obj.border_radius = Some(at.icon.border_radius + 1);
            obj.stroke_width = Some(1);
            let darker = darken(app.color, 0.25);
            obj.stroke_color = Some(oasis_types::color::with_alpha(darker, 100));
        }
        // Card body: vertical gradient from lightened top to darkened bottom.
        if let Ok(obj) = sdi.get_mut(&names.icon) {
            obj.x = ix;
            obj.y = iy;
            obj.w = icon_w;
            obj.h = icon_h;
            obj.visible = true;
            obj.color = app.color;
            obj.gradient_top = Some(lighten(app.color, 0.3));
            obj.gradient_bottom = Some(darken(app.color, 0.15));
            obj.text = None;
            obj.border_radius = Some(at.icon.border_radius);
            obj.shadow_level = Some(at.icon.shadow_level);
        }
        // Label below icon.
        super::labels::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.cell_size().0,
            iy + icon_h as i32 + text_pad,
            &app.title,
            left_align.then_some(ix + icon_w as i32 / 2),
            &self.label_wrap_cache,
        );
    }

    /// Draw a "circle" style icon (large circle with first letter centered).
    pub(super) fn draw_circle_icon(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        names: &IconNames,
        geo: IconGeometry,
        app: &AppEntry,
    ) {
        let IconGeometry {
            ix,
            iy,
            icon_w,
            icon_h,
            cell_x,
            text_pad,
            left_align,
        } = geo;
        // Hide document-specific sub-objects.
        for name in [&names.outline, &names.stripe, &names.fold, &names.gfx] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        // Circle body: use min dimension for a circle.
        let diameter = icon_w.min(icon_h);
        let radius = (diameter / 2) as u16;
        if let Ok(obj) = sdi.get_mut(&names.icon) {
            obj.x = ix + (icon_w as i32 - diameter as i32) / 2;
            obj.y = iy + (icon_h as i32 - diameter as i32) / 2;
            obj.w = diameter;
            obj.h = diameter;
            obj.visible = true;
            obj.color = app.color;
            obj.text = None;
            obj.border_radius = Some(radius);
            obj.shadow_level = Some(at.icon.shadow_level);
        }
        // Label below icon.
        super::labels::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.cell_size().0,
            iy + icon_h as i32 + text_pad,
            &app.title,
            left_align.then_some(ix + icon_w as i32 / 2),
            &self.label_wrap_cache,
        );
    }
}

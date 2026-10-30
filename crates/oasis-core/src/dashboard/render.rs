//! Per-frame SDI sync: icon objects, z bands, selection/hover highlights.

use super::{DashboardState, IconGeometry};
use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::hide_objects;

/// Base-layer z for dashboard icons: above the skin's layout chrome
/// (auto z stays in low single digits) and the selection highlight
/// (`FREE_ICON_Z - 10`), below the overlay pass (bars, start menu).
const FREE_ICON_Z: i32 = 50;
/// z for the icon being dragged, raised above its siblings.
const FREE_ICON_DRAG_Z: i32 = 60;
/// Hover highlight opacity relative to the selection highlight colour.
const HOVER_HIGHLIGHT_ALPHA_PCT: u16 = 45;
/// Selection and hover highlight objects.
const HIGHLIGHT_NAMES: [&str; 2] = ["cursor_highlight", "hover_highlight"];

impl DashboardState {
    /// Synchronize SDI objects to reflect current dashboard state.
    /// Creates/updates icons (style-dependent), text labels, and cursor highlight.
    ///
    /// Accepts an `ActiveTheme` for skin-driven colors. Pass
    /// `&ActiveTheme::default()` for legacy behaviour.
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let page_apps = self.current_page_apps();
        let slide_offset = self.slide_offset(at);
        let origins = self.page_cell_origins();

        for (i, names) in self.icon_names.iter().enumerate() {
            for name in names.all() {
                if !sdi.contains(name) {
                    sdi.create(name);
                }
            }

            let (Some(app), Some(&(ox, oy))) = (page_apps.get(i), origins.get(i)) else {
                hide_objects(sdi, &names.all());
                continue;
            };

            let cell_x = ox + slide_offset;
            let (ix, iy) = self.icon_origin(i, (cell_x, oy), at);
            let geo = IconGeometry {
                ix,
                iy,
                icon_w: at.icon_width,
                icon_h: at.icon_height,
                cell_x,
                text_pad: at.icon_label_pad,
                left_align: self.config.left_align_icons,
                lift: self.hover_lift_px(i, at),
            };
            match at.icon.style.as_str() {
                "card" => self.draw_card_icon(sdi, at, names, geo, app),
                "circle" => self.draw_circle_icon(sdi, at, names, geo, app),
                "vector" => self.draw_vector_icon(sdi, at, names, geo, i, app),
                _ => self.draw_document_icon(sdi, at, names, geo, i, app),
            }

            // Lift icons into an explicit z band above the skin's
            // base-layer chrome (layout objects take single-digit auto z)
            // and the selection/hover highlights, with a dragged icon
            // raised above its siblings so it stays visible while crossing
            // them.
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

        self.update_highlights(sdi, at, &origins, slide_offset);
    }

    /// Themed highlight behind the selected icon (skin `cursor_style`:
    /// "stroke" outlines it, "fill" paints a backdrop, "none" disables),
    /// in grid and free layouts alike so d-pad users always see the
    /// selection. A fainter copy marks the pointer-hovered icon.
    fn update_highlights(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        origins: &[(i32, i32)],
        slide_offset: i32,
    ) {
        let highlight_rect = |i: usize| -> Option<(i32, i32, u32, u32)> {
            if at.icon.cursor_style == "none" || self.drag_index.is_some() {
                return None;
            }
            let &(ox, oy) = origins.get(i)?;
            let pad = self.config.cursor_pad.max(0);
            let (ix, iy) = self.icon_origin(i, (ox + slide_offset, oy), at);
            Some((
                ix - pad,
                iy - pad,
                at.icon_width + 2 * pad as u32,
                at.icon_height + 2 * pad as u32,
            ))
        };
        let sel_rect = highlight_rect(self.selected);
        let hover_rect = self
            .hover_index
            .filter(|&h| h != self.selected)
            .and_then(highlight_rect);
        let hover_color = {
            let c = at.icon.cursor_color;
            let a = (c.a as u16 * HOVER_HIGHLIGHT_ALPHA_PCT / 100) as u8;
            Color::rgba(c.r, c.g, c.b, a)
        };
        let highlights = [
            (sel_rect, at.icon.cursor_color, FREE_ICON_Z - 10),
            (hover_rect, hover_color, FREE_ICON_Z - 11),
        ];
        for (name, (rect, color, z)) in HIGHLIGHT_NAMES.into_iter().zip(highlights) {
            if !sdi.contains(name) {
                sdi.create(name);
            }
            let Ok(cursor) = sdi.get_mut(name) else {
                continue;
            };
            let Some((x, y, w, h)) = rect else {
                cursor.visible = false;
                continue;
            };
            cursor.visible = true;
            cursor.x = x;
            cursor.y = y;
            cursor.w = w;
            cursor.h = h;
            cursor.z = z;
            cursor.text = None;
            cursor.border_radius = Some(at.icon.cursor_border_radius);
            if at.icon.cursor_style == "fill" {
                cursor.color = color;
                cursor.stroke_width = None;
                cursor.stroke_color = None;
            } else {
                cursor.color = Color::rgba(0, 0, 0, 0);
                cursor.stroke_width = Some(at.icon.cursor_stroke_width);
                cursor.stroke_color = Some(color);
            }
        }
    }

    /// Hide all dashboard SDI objects.
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        for names in &self.icon_names {
            hide_objects(sdi, &names.all());
        }
        hide_objects(sdi, &HIGHLIGHT_NAMES);
    }
}

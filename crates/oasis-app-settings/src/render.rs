//! Windowed rendering for the Settings app: category tab strip, a row list
//! drawn with oasis-ui widgets ([`Slider`], [`Toggle`]) and a hint footer.
//!
//! All geometry comes from [`SettingsLayout`] so drawing and hit-testing
//! agree; colors come from [`SettingsColors`] (theme + `[app_themes.settings]`
//! overrides) and widget styling from the active theme's `ui_theme`.

use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_ui::slider::Slider;
use oasis_ui::toggle::Toggle;
use oasis_ui::{DrawContext, Widget};

use crate::layout::Rect;
use crate::rows::Row;
use crate::{Category, SettingsApp, SettingsColors};

/// Cut `text` to fit `max_w` pixels, ending with `..` when shortened.
fn fit(backend: &dyn SdiBackend, text: &str, font: u16, max_w: u32) -> String {
    if backend.measure_text(text, font) <= max_w {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end = text.floor_char_boundary(end - 1);
        let candidate = format!("{}..", &text[..end]);
        if backend.measure_text(&candidate, font) <= max_w {
            return candidate;
        }
    }
    String::new()
}

/// Vertical offset that centers a `font`-sized line in a box of height `h`.
fn text_dy(h: u32, font: u16) -> i32 {
    (h.saturating_sub(u32::from(font)) / 2) as i32
}

impl SettingsApp {
    /// Draw the whole Settings UI into the content rect.
    pub(crate) fn draw_settings(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = SettingsColors::from_theme(at);
        let l = self.layout(cx, cy, cw, ch);
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;

        // Category tabs.
        let radius = at.ui_theme.border_radius_sm;
        for (cat, r) in Category::ALL.iter().zip(&l.tabs) {
            let active = *cat == self.category;
            let (fill, text_color) = if active {
                (colors.selected_bg, colors.selected_text)
            } else {
                (at.ui_theme.surface, colors.text)
            };
            backend.fill_rounded_rect(r.x, r.y, r.w, r.h, radius, fill)?;
            if active {
                backend.fill_rect(
                    r.x + 2,
                    r.y + r.h as i32 - 2,
                    r.w.saturating_sub(4),
                    2,
                    colors.selection_accent,
                )?;
            }
            let label = fit(backend, cat.label(), at.font_hint, r.w.saturating_sub(6));
            let tw = backend.measure_text(&label, at.font_hint);
            let tx = r.x + (r.w.saturating_sub(tw) / 2) as i32;
            backend.draw_text(
                &label,
                tx,
                r.y + text_dy(r.h, at.font_hint),
                at.font_hint,
                text_color,
            )?;
        }
        let divider_y = l.body.y - 3;
        backend.fill_rect(l.body.x, divider_y, l.body.w, 1, colors.divider)?;

        // Body rows.
        let rows = self.rows();
        let visible = l.visible_rows();
        let scroll = self.body_scroll(&rows, visible);
        let editing = self.appearance.editing_channel.is_some();
        for (vis, row) in rows.iter().skip(scroll).take(visible).enumerate() {
            let r = l.row_rect(vis);
            let selected = row.item() == Some(self.item_cursor)
                && (self.category.has_items() || self.category == Category::Audio);
            self.draw_row(row, r, &l, selected, editing, backend, at, &colors)?;
        }

        // Scroll indicator + footer hint.
        let hint = if rows.len() > visible {
            format!(
                "[{}/{}]  L/R=Category  U/D=Select  Enter=Apply  Esc=Back",
                scroll + 1,
                rows.len() - visible + 1
            )
        } else {
            "L/R=Category  U/D=Select  Enter=Apply  Esc=Back".to_string()
        };
        let hint = fit(backend, &hint, at.font_hint, l.footer.w);
        backend.draw_text(
            &hint,
            l.footer.x,
            l.footer.y + text_dy(l.footer.h, at.font_hint),
            at.font_hint,
            colors.dim_text,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        row: &Row,
        r: Rect,
        l: &crate::layout::SettingsLayout,
        selected: bool,
        editing: bool,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &SettingsColors,
    ) -> oasis_types::error::Result<()> {
        let font = at.font_body;
        let ty = r.y + text_dy(r.h, font);
        let text_color = if selected {
            backend.fill_rounded_rect(
                r.x,
                r.y,
                r.w,
                r.h,
                at.ui_theme.border_radius_sm,
                colors.selected_bg,
            )?;
            backend.fill_rect(
                r.x,
                r.y + 2,
                3,
                r.h.saturating_sub(4),
                colors.selection_accent,
            )?;
            colors.selected_text
        } else {
            colors.text
        };
        match row {
            Row::Blank => {},
            Row::Heading(s) => {
                let s = fit(backend, s, font, r.w);
                backend.draw_text(&s, r.x + 2, ty, font, colors.text)?;
            },
            Row::Text(s) => {
                let s = fit(backend, s, at.font_hint, r.w.saturating_sub(8));
                let y = r.y + text_dy(r.h, at.font_hint);
                backend.draw_text(&s, r.x + 8, y, at.font_hint, colors.dim_text)?;
            },
            Row::Item { label, active, .. } => {
                let marker_w = if *active {
                    backend.measure_text(" *", font) + 4
                } else {
                    0
                };
                let label = fit(backend, label, font, r.w.saturating_sub(12 + marker_w));
                backend.draw_text(&label, r.x + 10, ty, font, text_color)?;
                if *active {
                    let mx = r.x + r.w as i32 - marker_w as i32;
                    backend.draw_text(" *", mx, ty, font, colors.selection_accent)?;
                }
                if selected && editing {
                    // Edit-mode rows carry the channel readout in the label.
                    backend.stroke_rounded_rect(
                        r.x,
                        r.y,
                        r.w,
                        r.h,
                        at.ui_theme.border_radius_sm,
                        1,
                        colors.selection_accent,
                    )?;
                }
            },
            Row::Slider {
                label,
                value,
                min,
                max,
                value_text,
                ..
            } => {
                let c = l.control_rect(r);
                let value_w = backend.measure_text(value_text, at.font_hint) + 6;
                let label_w = (c.x - r.x).max(0) as u32;
                let label = fit(backend, label, font, label_w.saturating_sub(value_w + 14));
                backend.draw_text(&label, r.x + 10, ty, font, text_color)?;
                let vx = c.x - value_w as i32;
                let vy = r.y + text_dy(r.h, at.font_hint);
                backend.draw_text(value_text, vx, vy, at.font_hint, text_color)?;
                let mut slider = Slider::new(*min, *max);
                slider.set_value(*value);
                slider.thumb_size = (c.h as u16).clamp(8, 12);
                slider.focused = selected;
                let mut ctx = DrawContext::new(backend, &at.ui_theme);
                slider.draw(&mut ctx, c.x, c.y, c.w, c.h)?;
            },
            Row::Toggle { label, on, .. } => {
                let c = l.control_rect(r);
                let label = fit(backend, label, font, (c.x - r.x).max(0) as u32);
                backend.draw_text(&label, r.x + 10, ty, font, text_color)?;
                let tw = 28;
                let th = c.h.clamp(10, 16);
                let mut toggle = Toggle::new(*on);
                toggle.focused = selected;
                let tx = c.x + c.w as i32 - tw as i32;
                let tyy = r.y + (r.h.saturating_sub(th) / 2) as i32;
                let mut ctx = DrawContext::new(backend, &at.ui_theme);
                toggle.draw(&mut ctx, tx, tyy, tw, th)?;
            },
        }
        Ok(())
    }
}

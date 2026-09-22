//! Windowed rendering for Paint: menu bar, info strip, scaled canvas with
//! grid + cursor, palette strip and the File > Open picker.
//!
//! All geometry comes from [`PaintLayout`] so drawing and hit-testing
//! agree; all colors come from the active theme via [`PaintColors`].

use oasis_skin::ActiveTheme;
use oasis_types::backend::{BatchRect, Color, SdiBackend};
use oasis_ui::menu_bar::MenuStyle;

use crate::PaintApp;
use crate::layout::{PICKER_ROW_H, PaintLayout, Rect};
use crate::palette::palette;

/// Paint color roles, derived from the active theme. Skins can override
/// any slot via `[app_themes.paint]` in theme.toml.
#[derive(Debug, Clone)]
pub struct PaintColors {
    /// Content background.
    pub bg: Color,
    /// Info strip text.
    pub text: Color,
    /// Secondary info text (file name, status).
    pub dim_text: Color,
    /// Frames around canvas / swatches.
    pub frame: Color,
    /// Backdrop shown through transparent canvas pixels.
    pub canvas_backdrop: Color,
    /// Grid overlay lines.
    pub grid: Color,
    /// Cursor outline and shape-start marker.
    pub cursor: Color,
    /// Highlight around the selected palette swatch.
    pub swatch_selected: Color,
    /// Picker panel background.
    pub picker_bg: Color,
    /// Picker header strip background / text.
    pub picker_header_bg: Color,
    pub picker_header_text: Color,
    /// Picker selected row background / text.
    pub picker_sel_bg: Color,
    pub picker_sel_text: Color,
}

impl PaintColors {
    /// Build colors from the active theme, honouring per-app overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color("paint", key).unwrap_or(default);
        Self {
            bg: c("bg", at.app.bg),
            text: c("text", at.app.text),
            dim_text: c("dim_text", at.app.dim_text),
            frame: c("frame", at.app.divider),
            canvas_backdrop: c("canvas_backdrop", at.app.divider),
            grid: c(
                "grid",
                Color::rgba(at.app.divider.r, at.app.divider.g, at.app.divider.b, 110),
            ),
            cursor: c("cursor", at.app.selection_accent_color),
            swatch_selected: c("swatch_selected", at.app.selected_text),
            picker_bg: c("picker_bg", at.app.bg),
            picker_header_bg: c("picker_header_bg", at.app.title_bar_bg),
            picker_header_text: c("picker_header_text", at.app.title_bar_text),
            picker_sel_bg: c("picker_sel_bg", at.app.selected_bg),
            picker_sel_text: c("picker_sel_text", at.app.selected_text),
        }
    }
}

fn outline(backend: &mut dyn SdiBackend, r: Rect, color: Color) -> oasis_types::error::Result<()> {
    backend.fill_rect(r.x, r.y, r.w, 1, color)?;
    backend.fill_rect(r.x, r.y + r.h as i32 - 1, r.w, 1, color)?;
    backend.fill_rect(r.x, r.y, 1, r.h, color)?;
    backend.fill_rect(r.x + r.w as i32 - 1, r.y, 1, r.h, color)
}

impl PaintApp {
    /// Draw the whole Paint UI into the content rect.
    pub(crate) fn draw_paint(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let colors = PaintColors::from_theme(at);
        let l = PaintLayout::compute(cx, cy, cw, ch, self.canvas.width(), self.canvas.height());
        self.cached_picker_rows.set(l.picker_rows());
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;

        let menu_style = MenuStyle::from_theme(&at.ui_theme);
        self.menu
            .draw_bar(backend, l.menu.x, l.menu.y, l.menu.w, l.menu.h, &menu_style)?;

        self.draw_info(&l, backend, at, &colors)?;
        self.draw_canvas(&l, backend, &colors)?;
        self.draw_palette(&l, backend, &colors)?;
        if self.picker.is_some() {
            self.draw_picker(&l, backend, at, &colors)?;
        }
        // Drop-down floats above everything else.
        if self.menu.is_open() {
            self.menu
                .draw_dropdown(backend, l.menu.x, l.menu.y, l.menu.h, &menu_style)?;
        }
        Ok(())
    }

    fn draw_info(
        &self,
        l: &PaintLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &PaintColors,
    ) -> oasis_types::error::Result<()> {
        let sw = l.info.h.saturating_sub(4);
        backend.fill_rect(l.info.x + 4, l.info.y + 2, sw, sw, self.color)?;
        outline(
            backend,
            Rect {
                x: l.info.x + 4,
                y: l.info.y + 2,
                w: sw,
                h: sw,
            },
            colors.frame,
        )?;
        let from = match self.drag_start {
            Some((sx, sy)) => format!(" from ({sx},{sy})"),
            None => String::new(),
        };
        let layer = self.canvas.active_layer();
        let hidden = if self.canvas.layer_visible(layer) {
            ""
        } else {
            " (hidden)"
        };
        let info = format!(
            "{}  Size {}  L{}/{}{hidden}  ({},{}){from}",
            self.tool.name(),
            self.brush_size,
            layer + 1,
            self.canvas.layer_count(),
            self.cursor_x,
            self.cursor_y,
        );
        let text_x = l.info.x + 8 + sw as i32;
        backend.draw_text(&info, text_x, l.info.y + 2, at.font_hint, colors.text)?;
        let right = self.status.clone().unwrap_or_else(|| self.file_label());
        let right_w = backend.measure_text(&right, at.font_hint) as i32;
        let right_x = (l.info.x + l.info.w as i32 - right_w - 6).max(text_x + 8);
        backend.draw_text(&right, right_x, l.info.y + 2, at.font_hint, colors.dim_text)
    }

    fn draw_canvas(
        &self,
        l: &PaintLayout,
        backend: &mut dyn SdiBackend,
        colors: &PaintColors,
    ) -> oasis_types::error::Result<()> {
        let c = l.canvas;
        outline(
            backend,
            Rect {
                x: c.x - 1,
                y: c.y - 1,
                w: c.w + 2,
                h: c.h + 2,
            },
            colors.frame,
        )?;
        backend.fill_rect(c.x, c.y, c.w, c.h, colors.canvas_backdrop)?;

        // Canvas pixels, one batched rect per non-transparent pixel.
        let w = self.canvas.width();
        let mut rects = Vec::with_capacity(self.canvas.pixels.len());
        for (i, px) in self.canvas.pixels.iter().enumerate() {
            if px.a == 0 {
                continue;
            }
            let r = l.pixel_rect((i as u32 % w) as i32, (i as u32 / w) as i32);
            rects.push(BatchRect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                color: Color::rgb(px.r, px.g, px.b),
            });
        }

        // Grid overlay (only once cells are big enough to see).
        if self.show_grid && l.scale >= 3 {
            for gx in 0..=w {
                rects.push(BatchRect {
                    x: c.x + (gx * l.scale) as i32,
                    y: c.y,
                    w: 1,
                    h: c.h,
                    color: colors.grid,
                });
            }
            for gy in 0..=self.canvas.height() {
                rects.push(BatchRect {
                    x: c.x,
                    y: c.y + (gy * l.scale) as i32,
                    w: c.w,
                    h: 1,
                    color: colors.grid,
                });
            }
        }
        backend.submit_rect_batch(&rects)?;

        // Shape start marker + cursor outline.
        if let Some((sx, sy)) = self.drag_start {
            let r = l.pixel_rect(sx, sy);
            backend.fill_rect(r.x, r.y, r.w, r.h, colors.cursor)?;
        }
        let r = l.pixel_rect(self.cursor_x, self.cursor_y);
        let cursor = Rect {
            x: r.x - 1,
            y: r.y - 1,
            w: r.w + 2,
            h: r.h + 2,
        };
        outline(backend, cursor, colors.cursor)
    }

    fn draw_palette(
        &self,
        l: &PaintLayout,
        backend: &mut dyn SdiBackend,
        colors: &PaintColors,
    ) -> oasis_types::error::Result<()> {
        for (i, color) in palette().iter().enumerate() {
            let r = l.swatch_rect(i);
            backend.fill_rect(r.x, r.y, r.w, r.h, *color)?;
            let frame = if i == self.palette_index {
                colors.swatch_selected
            } else {
                colors.frame
            };
            outline(backend, r, frame)?;
        }
        Ok(())
    }

    fn draw_picker(
        &self,
        l: &PaintLayout,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        colors: &PaintColors,
    ) -> oasis_types::error::Result<()> {
        let Some(picker) = &self.picker else {
            return Ok(());
        };
        let p = l.picker();
        backend.fill_rect(p.x, p.y, p.w, p.h, colors.picker_bg)?;
        outline(backend, p, colors.frame)?;
        backend.fill_rect(p.x, p.y, p.w, PICKER_ROW_H, colors.picker_header_bg)?;
        backend.draw_text(
            &format!("Open picture from {}", crate::io::PICTURES_DIR),
            p.x + 4,
            p.y + 1,
            at.font_hint,
            colors.picker_header_text,
        )?;
        let files = match &picker.files {
            None => {
                return backend.draw_text(
                    "Loading...",
                    p.x + 4,
                    p.y + PICKER_ROW_H as i32 + 1,
                    at.font_hint,
                    colors.dim_text,
                );
            },
            Some(f) if f.is_empty() => {
                return backend.draw_text(
                    "No .bmp files yet - save one first",
                    p.x + 4,
                    p.y + PICKER_ROW_H as i32 + 1,
                    at.font_hint,
                    colors.dim_text,
                );
            },
            Some(f) => f,
        };
        for (row, idx) in (picker.scroll..files.len())
            .take(l.picker_rows())
            .enumerate()
        {
            let y = p.y + ((row as u32 + 1) * PICKER_ROW_H) as i32;
            let name = files[idx].rsplit('/').next().unwrap_or(&files[idx]);
            let text_color = if idx == picker.selected {
                backend.fill_rect(p.x + 1, y, p.w - 2, PICKER_ROW_H, colors.picker_sel_bg)?;
                colors.picker_sel_text
            } else {
                colors.text
            };
            backend.draw_text(name, p.x + 4, y + 1, at.font_hint, text_color)?;
        }
        Ok(())
    }
}

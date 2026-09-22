//! Content-area layout for Paint.
//!
//! [`PaintLayout::compute`] is the single source of truth for where the
//! menu bar, info strip, canvas, palette strip and Open picker sit. Both
//! the renderer and the click hit-tester call it with the same content
//! size, so a click on a drawn canvas pixel always maps back to that pixel.

/// Menu bar height (matches the other apps' `MenuBar` strips).
pub(crate) const MENU_H: u32 = 18;
/// Info strip height (tool / size / layer / cursor / file).
pub(crate) const INFO_H: u32 = 14;
/// Palette strip height at the bottom.
pub(crate) const PALETTE_H: u32 = 14;
/// Padding around the canvas area.
const PAD: u32 = 4;
/// Row height of the Open picker list.
pub(crate) const PICKER_ROW_H: u32 = 12;
/// Number of palette swatches.
const SWATCHES: u32 = 16;

/// Axis-aligned rectangle in content-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub(crate) fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }
}

/// Resolved layout for one content rect + canvas size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaintLayout {
    /// Menu bar strip.
    pub menu: Rect,
    /// Info text strip under the menu.
    pub info: Rect,
    /// On-screen canvas rectangle (canvas pixels * `scale`).
    pub canvas: Rect,
    /// Screen pixels per canvas pixel (>= 1).
    pub scale: u32,
    /// Palette strip at the bottom.
    pub palette: Rect,
    /// Width of one palette swatch.
    pub swatch_w: u32,
}

impl PaintLayout {
    /// Lay out a `cw` x `ch` content rect whose top-left is `(cx, cy)` for
    /// a `canvas_w` x `canvas_h` canvas.
    pub(crate) fn compute(
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        canvas_w: u32,
        canvas_h: u32,
    ) -> Self {
        let menu = Rect {
            x: cx,
            y: cy,
            w: cw,
            h: MENU_H,
        };
        let info = Rect {
            x: cx,
            y: cy + MENU_H as i32,
            w: cw,
            h: INFO_H,
        };
        let palette_y = cy + ch.saturating_sub(PALETTE_H) as i32;
        let swatch_w = (cw.saturating_sub(2 * PAD) / SWATCHES).clamp(1, 20);
        let palette = Rect {
            x: cx + PAD as i32,
            y: palette_y,
            w: swatch_w * SWATCHES,
            h: PALETTE_H,
        };

        let area_top = info.y + INFO_H as i32 + PAD as i32;
        let area_h = (palette_y - PAD as i32 - area_top).max(1) as u32;
        let area_w = cw.saturating_sub(2 * PAD).max(1);
        let scale = (area_w / canvas_w.max(1))
            .min(area_h / canvas_h.max(1))
            .max(1);
        let px_w = canvas_w * scale;
        let px_h = canvas_h * scale;
        let canvas = Rect {
            x: cx + (cw.saturating_sub(px_w) / 2) as i32,
            y: area_top + (area_h.saturating_sub(px_h) / 2) as i32,
            w: px_w,
            h: px_h,
        };
        Self {
            menu,
            info,
            canvas,
            scale,
            palette,
            swatch_w,
        }
    }

    /// Canvas pixel under `(x, y)`, if the point is on the canvas.
    pub(crate) fn canvas_pixel_at(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        if !self.canvas.contains(x, y) {
            return None;
        }
        let s = self.scale as i32;
        Some(((x - self.canvas.x) / s, (y - self.canvas.y) / s))
    }

    /// Screen rectangle of canvas pixel `(px, py)`.
    pub(crate) fn pixel_rect(&self, px: i32, py: i32) -> Rect {
        Rect {
            x: self.canvas.x + px * self.scale as i32,
            y: self.canvas.y + py * self.scale as i32,
            w: self.scale,
            h: self.scale,
        }
    }

    /// Screen rectangle of palette swatch `i`.
    pub(crate) fn swatch_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.palette.x + (i as u32 * self.swatch_w) as i32,
            y: self.palette.y + 1,
            w: self.swatch_w.saturating_sub(1).max(1),
            h: PALETTE_H - 2,
        }
    }

    /// Palette swatch index under `(x, y)`.
    pub(crate) fn swatch_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.palette.contains(x, y) {
            return None;
        }
        let i = ((x - self.palette.x) as u32 / self.swatch_w) as usize;
        (i < SWATCHES as usize).then_some(i)
    }

    /// The Open picker panel (overlays the canvas area).
    pub(crate) fn picker(&self) -> Rect {
        let top = self.info.y + INFO_H as i32 + PAD as i32;
        Rect {
            x: self.menu.x + 2 * PAD as i32,
            y: top,
            w: self.menu.w.saturating_sub(4 * PAD).max(40),
            h: (self.palette.y - PAD as i32 - top).max(PICKER_ROW_H as i32 * 2) as u32,
        }
    }

    /// Number of file rows the picker can show (below its header row).
    pub(crate) fn picker_rows(&self) -> usize {
        (self.picker().h / PICKER_ROW_H).saturating_sub(1).max(1) as usize
    }

    /// Picker list row (0-based, relative to the scroll offset) under
    /// `(x, y)`.
    pub(crate) fn picker_row_at(&self, x: i32, y: i32) -> Option<usize> {
        let p = self.picker();
        if !p.contains(x, y) {
            return None;
        }
        let rel = (y - p.y) as u32 / PICKER_ROW_H;
        (rel >= 1).then(|| (rel - 1) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_fits_between_info_and_palette() {
        let l = PaintLayout::compute(0, 0, 400, 300, 64, 48);
        assert!(l.canvas.y >= l.info.y + INFO_H as i32);
        assert!(l.canvas.y + l.canvas.h as i32 <= l.palette.y);
        assert!(l.canvas.x >= 0 && l.canvas.x + l.canvas.w as i32 <= 400);
        assert_eq!(l.canvas.w, 64 * l.scale);
    }

    #[test]
    fn pixel_rect_round_trips() {
        let l = PaintLayout::compute(10, 20, 480, 272, 64, 48);
        for (px, py) in [(0, 0), (63, 47), (17, 9)] {
            let r = l.pixel_rect(px, py);
            assert_eq!(l.canvas_pixel_at(r.x, r.y), Some((px, py)));
            assert_eq!(
                l.canvas_pixel_at(r.x + r.w as i32 - 1, r.y + r.h as i32 - 1),
                Some((px, py))
            );
        }
        assert_eq!(l.canvas_pixel_at(l.canvas.x - 1, l.canvas.y), None);
    }

    #[test]
    fn swatches_hit_test() {
        let l = PaintLayout::compute(0, 0, 480, 272, 64, 48);
        for i in 0..16 {
            let r = l.swatch_rect(i);
            assert_eq!(l.swatch_at(r.x, r.y), Some(i));
        }
    }

    #[test]
    fn tiny_content_does_not_panic() {
        let l = PaintLayout::compute(0, 0, 10, 10, 256, 256);
        assert_eq!(l.scale, 1);
        let _ = l.picker_rows();
    }
}

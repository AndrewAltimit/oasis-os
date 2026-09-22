//! Content-area layout for the windowed Settings UI.
//!
//! [`SettingsLayout::compute`] is the single source of truth for where the
//! category tab strip, the row list and the hint footer sit. The renderer
//! and the click hit-tester both call it with the same content size and
//! font metrics, so a click always lands on the row that was drawn there.

/// Outer padding.
const PAD: u32 = 4;
/// Gap between the tab strip and the body, and between body and footer.
const GAP: u32 = 4;
/// Minimum width of one category tab before the strip wraps to two rows.
const MIN_TAB_W: u32 = 56;
/// Width reserved at the right of a control row for its widget.
const CONTROL_W: u32 = 132;

/// Axis-aligned rectangle in content coordinates.
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

/// Resolved layout for one content rect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsLayout {
    /// One rect per category tab, in `Category::ALL` order.
    pub tabs: Vec<Rect>,
    /// Row list area.
    pub body: Rect,
    /// Key-hint footer.
    pub footer: Rect,
    /// Height of one body row.
    pub row_h: u32,
}

impl SettingsLayout {
    /// Lay out `tab_count` tabs, the body and the footer inside the
    /// content rect. `font_body` sizes the rows, `font_hint` the tabs and
    /// the footer.
    pub(crate) fn compute(
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        tab_count: usize,
        font_body: u16,
        font_hint: u16,
    ) -> Self {
        let inner_w = cw.saturating_sub(PAD * 2);
        let n = tab_count.max(1) as u32;
        let cols = if inner_w / n >= MIN_TAB_W {
            n
        } else {
            n.div_ceil(2)
        };
        let tab_rows = n.div_ceil(cols);
        let tab_h = u32::from(font_hint) + 8;
        let tab_w = inner_w / cols;
        let tabs = (0..n)
            .map(|i| Rect {
                x: cx + (PAD + (i % cols) * tab_w) as i32,
                y: cy + (PAD + (i / cols) * tab_h) as i32,
                w: tab_w.saturating_sub(2),
                h: tab_h.saturating_sub(2),
            })
            .collect();

        let footer_h = u32::from(font_hint) + 6;
        let body_top = PAD + tab_rows * tab_h + GAP;
        let footer = Rect {
            x: cx + PAD as i32,
            y: cy + ch.saturating_sub(PAD + footer_h) as i32,
            w: inner_w,
            h: footer_h,
        };
        let body_h = ch.saturating_sub(body_top + GAP + footer_h + PAD);
        let body = Rect {
            x: cx + PAD as i32,
            y: cy + body_top as i32,
            w: inner_w,
            h: body_h,
        };
        let row_h = (u32::from(font_body) + 6).max(18);
        Self {
            tabs,
            body,
            footer,
            row_h,
        }
    }

    /// How many body rows fit.
    pub(crate) fn visible_rows(&self) -> usize {
        (self.body.h / self.row_h.max(1)).max(1) as usize
    }

    /// Rect of the `i`-th visible body row.
    pub(crate) fn row_rect(&self, i: usize) -> Rect {
        Rect {
            x: self.body.x,
            y: self.body.y + (i as u32 * self.row_h) as i32,
            w: self.body.w,
            h: self.row_h,
        }
    }

    /// Visible row index under a content-local point, if any.
    pub(crate) fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.body.contains(x, y) {
            return None;
        }
        let i = ((y - self.body.y) as u32 / self.row_h.max(1)) as usize;
        (i < self.visible_rows()).then_some(i)
    }

    /// Rect of the widget (slider / toggle) inside a control row.
    pub(crate) fn control_rect(&self, row: Rect) -> Rect {
        let w = CONTROL_W.min(row.w / 2);
        Rect {
            x: row.x + row.w as i32 - w as i32 - 4,
            y: row.y + 2,
            w,
            h: row.h.saturating_sub(4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_window_single_tab_row() {
        let l = SettingsLayout::compute(0, 0, 640, 300, 8, 12, 10);
        assert_eq!(l.tabs.len(), 8);
        assert!(l.tabs.iter().all(|t| t.y == l.tabs[0].y));
        assert!(l.body.y > l.tabs[0].y + l.tabs[0].h as i32);
        assert!(l.footer.y >= l.body.y + l.body.h as i32);
    }

    #[test]
    fn narrow_window_wraps_tabs() {
        let l = SettingsLayout::compute(0, 0, 300, 300, 8, 12, 10);
        assert_ne!(l.tabs[0].y, l.tabs[7].y);
    }

    #[test]
    fn row_hit_test_roundtrips() {
        let l = SettingsLayout::compute(10, 20, 480, 260, 8, 12, 10);
        for i in 0..l.visible_rows() {
            let r = l.row_rect(i);
            assert_eq!(l.row_at(r.x + 5, r.y + 1), Some(i));
        }
        assert_eq!(l.row_at(l.body.x, l.body.y - 1), None);
    }

    #[test]
    fn control_sits_inside_row() {
        let l = SettingsLayout::compute(0, 0, 480, 260, 8, 12, 10);
        let row = l.row_rect(0);
        let c = l.control_rect(row);
        assert!(c.x > row.x && c.x + c.w as i32 <= row.x + row.w as i32);
    }
}

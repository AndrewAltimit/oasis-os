//! Content-area layout for the Internet Radio window.
//!
//! [`RadioLayout::compute`] is the single source of truth for where the
//! now-playing panel (with its volume bar and Vol-/Vol+/Stop buttons), the
//! station list and the hint footer sit. The renderer and the click
//! hit-tester both call it with the same content size.

/// Outer padding.
const PAD: u32 = 4;
/// Now-playing panel height.
pub(crate) const PANEL_H: u32 = 58;
/// Station list header height.
pub(crate) const LIST_HEADER_H: u32 = 13;
/// Station list row height.
pub(crate) const ROW_H: u32 = 14;
/// Footer (key hints) height.
pub(crate) const FOOTER_H: u32 = 13;
/// Panel button size.
const BTN_W: u32 = 38;
const BTN_H: u32 = 16;
/// Gap between panel buttons.
const BTN_GAP: u32 = 4;
/// Width of the "Vol NN%" label left of the volume bar.
const VOL_LABEL_W: u32 = 52;

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

/// A clickable control in the now-playing panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelButton {
    VolumeDown,
    VolumeUp,
    Stop,
}

impl PanelButton {
    pub(crate) const ALL: [PanelButton; 3] = [
        PanelButton::VolumeDown,
        PanelButton::VolumeUp,
        PanelButton::Stop,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            PanelButton::VolumeDown => "Vol-",
            PanelButton::VolumeUp => "Vol+",
            PanelButton::Stop => "Stop",
        }
    }
}

/// Resolved layout for one content rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RadioLayout {
    /// Now-playing panel.
    pub panel: Rect,
    /// Volume bar inside the panel (click to set the volume).
    pub volume_bar: Rect,
    /// Vol- / Vol+ / Stop buttons, in [`PanelButton::ALL`] order.
    pub buttons: [Rect; 3],
    /// Station list header strip.
    pub list_header: Rect,
    /// Station list rows area.
    pub list: Rect,
    /// Hint footer.
    pub footer: Rect,
}

impl RadioLayout {
    /// Lay out a `cw` x `ch` content rect whose top-left is `(cx, cy)`.
    pub(crate) fn compute(cx: i32, cy: i32, cw: u32, ch: u32) -> Self {
        let inner_w = cw.saturating_sub(2 * PAD).max(1);
        let panel = Rect {
            x: cx + PAD as i32,
            y: cy + PAD as i32,
            w: inner_w,
            h: PANEL_H,
        };
        // Bottom row of the panel: "Vol" [====bar====] [Vol-][Vol+][Stop]
        let row_y = panel.y + (PANEL_H - BTN_H - 6) as i32;
        let buttons_w = 3 * BTN_W + 2 * BTN_GAP;
        let first_btn_x = panel.x + panel.w as i32 - 6 - buttons_w as i32;
        let buttons = [0u32, 1, 2].map(|i| Rect {
            x: first_btn_x + (i * (BTN_W + BTN_GAP)) as i32,
            y: row_y,
            w: BTN_W,
            h: BTN_H,
        });
        let bar_x = panel.x + 6 + VOL_LABEL_W as i32;
        let volume_bar = Rect {
            x: bar_x,
            y: row_y + 4,
            w: (first_btn_x - 8 - bar_x).max(8) as u32,
            h: 8,
        };

        let list_header = Rect {
            x: panel.x,
            y: panel.y + PANEL_H as i32 + PAD as i32,
            w: inner_w,
            h: LIST_HEADER_H,
        };
        let footer = Rect {
            x: panel.x,
            y: cy + ch as i32 - FOOTER_H as i32,
            w: inner_w,
            h: FOOTER_H,
        };
        let list_y = list_header.y + LIST_HEADER_H as i32;
        let list = Rect {
            x: panel.x,
            y: list_y,
            w: inner_w,
            h: (footer.y - list_y).max(ROW_H as i32) as u32,
        };
        Self {
            panel,
            volume_bar,
            buttons,
            list_header,
            list,
            footer,
        }
    }

    /// Number of station rows that fit.
    pub(crate) fn visible_rows(&self) -> usize {
        (self.list.h / ROW_H).max(1) as usize
    }

    /// Screen rectangle of visible list row `row`.
    pub(crate) fn row_rect(&self, row: usize) -> Rect {
        Rect {
            x: self.list.x,
            y: self.list.y + (row as u32 * ROW_H) as i32,
            w: self.list.w,
            h: ROW_H,
        }
    }

    /// Visible list row under `(x, y)`.
    pub(crate) fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.list.contains(x, y) {
            return None;
        }
        let row = ((y - self.list.y) as u32 / ROW_H) as usize;
        (row < self.visible_rows()).then_some(row)
    }

    /// Panel button under `(x, y)`.
    pub(crate) fn button_at(&self, x: i32, y: i32) -> Option<PanelButton> {
        self.buttons
            .iter()
            .position(|r| r.contains(x, y))
            .map(|i| PanelButton::ALL[i])
    }

    /// Volume (0-100) for a click at `x` on the volume bar, if `(x, y)`
    /// hits it (with a few pixels of vertical slack).
    pub(crate) fn volume_at(&self, x: i32, y: i32) -> Option<u8> {
        let bar = self.volume_bar;
        let hit = Rect {
            x: bar.x,
            y: bar.y - 4,
            w: bar.w,
            h: bar.h + 8,
        };
        if !hit.contains(x, y) {
            return None;
        }
        let frac = (x - bar.x) as f32 / (bar.w.max(2) - 1) as f32;
        Some((frac.clamp(0.0, 1.0) * 100.0).round() as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_stack_without_overlap() {
        let l = RadioLayout::compute(0, 0, 400, 240);
        assert!(l.list_header.y >= l.panel.y + l.panel.h as i32);
        assert!(l.list.y >= l.list_header.y + l.list_header.h as i32);
        assert!(l.list.y + l.list.h as i32 <= l.footer.y);
        assert!(l.volume_bar.x + l.volume_bar.w as i32 <= l.buttons[0].x);
        for b in l.buttons {
            assert!(b.x + b.w as i32 <= l.panel.x + l.panel.w as i32);
            assert!(b.y + b.h as i32 <= l.panel.y + l.panel.h as i32);
        }
    }

    #[test]
    fn rows_round_trip() {
        let l = RadioLayout::compute(10, 20, 400, 240);
        for row in 0..l.visible_rows() {
            let r = l.row_rect(row);
            assert_eq!(l.row_at(r.x + 3, r.y), Some(row));
            assert_eq!(l.row_at(r.x + 3, r.y + r.h as i32 - 1), Some(row));
        }
        assert_eq!(l.row_at(l.panel.x + 5, l.panel.y + 5), None);
    }

    #[test]
    fn buttons_and_volume_bar_hit_test() {
        let l = RadioLayout::compute(0, 0, 400, 240);
        for (i, b) in PanelButton::ALL.iter().enumerate() {
            let r = l.buttons[i];
            assert_eq!(l.button_at(r.x + 1, r.y + 1), Some(*b));
        }
        let bar = l.volume_bar;
        assert_eq!(l.volume_at(bar.x, bar.y), Some(0));
        assert_eq!(l.volume_at(bar.x + bar.w as i32 - 1, bar.y), Some(100));
        assert_eq!(l.volume_at(bar.x + bar.w as i32 / 2, bar.y + 2), Some(50));
        assert_eq!(l.volume_at(bar.x - 1, bar.y), None);
    }

    #[test]
    fn tiny_content_does_not_panic() {
        let l = RadioLayout::compute(0, 0, 10, 10);
        assert!(l.visible_rows() >= 1);
        let _ = l.volume_at(0, 0);
        let _ = l.row_at(0, 0);
    }
}

//! Modal prompt overlay (delete confirmation, name entry) for the File
//! Manager.
//!
//! [`dialog_geom`] is the single source of truth for the prompt layout:
//! the windowed renderer and the click hit-test both derive their
//! rectangles from it so they can never drift apart.

use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;

use crate::colors::FileManagerColors;
use crate::model::{Dialog, NamePurpose};
use crate::ops::file_name;
use crate::state::FileManagerApp;
use crate::view::{Box2d, draw_outline};

/// Dialog box height in pixels.
const DLG_H: u32 = 86;
/// Dialog title-strip height.
const DLG_TITLE_H: u32 = 16;
/// Button width / height.
const DLG_BTN_W: u32 = 60;
const DLG_BTN_H: u32 = 16;
/// Rectangles making up the prompt, in the caller's coordinate space.
pub(crate) struct DialogGeom {
    pub frame: Box2d,
    pub title: Box2d,
    pub msg_y: i32,
    pub field: Box2d,
    pub ok: Box2d,
    pub cancel: Box2d,
}

/// Lay the prompt out centred in the content rect `(cx, cy, cw, ch)`.
pub(crate) fn dialog_geom(cx: i32, cy: i32, cw: u32, ch: u32) -> DialogGeom {
    let w = cw.saturating_sub(16).clamp(140, 300);
    let x = cx + (cw as i32 - w as i32) / 2;
    let y = cy + (ch as i32 - DLG_H as i32).max(0) / 2;
    let btn_y = y + DLG_H as i32 - DLG_BTN_H as i32 - 6;
    DialogGeom {
        frame: Box2d { x, y, w, h: DLG_H },
        title: Box2d {
            x: x + 1,
            y: y + 1,
            w: w - 2,
            h: DLG_TITLE_H,
        },
        msg_y: y + DLG_TITLE_H as i32 + 5,
        field: Box2d {
            x: x + 8,
            y: y + DLG_TITLE_H as i32 + 20,
            w: w - 16,
            h: 16,
        },
        ok: Box2d {
            x: x + w as i32 - 2 * (DLG_BTN_W as i32 + 6),
            y: btn_y,
            w: DLG_BTN_W,
            h: DLG_BTN_H,
        },
        cancel: Box2d {
            x: x + w as i32 - (DLG_BTN_W as i32 + 6),
            y: btn_y,
            w: DLG_BTN_W,
            h: DLG_BTN_H,
        },
    }
}

fn contains(b: Box2d, x: i32, y: i32) -> bool {
    x >= b.x && y >= b.y && x < b.x + b.w as i32 && y < b.y + b.h as i32
}

/// Texts shown by a dialog: (title, message, field text, ok, cancel).
struct DialogText {
    title: &'static str,
    msg: String,
    field: Option<String>,
    ok: &'static str,
    cancel: &'static str,
}

fn dialog_text(dialog: &Dialog, status: Option<&str>) -> DialogText {
    match dialog {
        Dialog::ConfirmDelete { path } => DialogText {
            title: "Confirm Delete",
            msg: format!("Delete \"{}\"?  (Y/N)", file_name(path)),
            field: None,
            ok: "Yes",
            cancel: "No",
        },
        Dialog::NameEntry { purpose, text } => DialogText {
            title: match purpose {
                NamePurpose::NewFolder { .. } => "New Folder",
                NamePurpose::Rename { .. } => "Rename",
            },
            msg: status
                .unwrap_or("Type a name, Enter to accept:")
                .to_string(),
            field: Some(format!("{text}_")),
            ok: "OK",
            cancel: "Cancel",
        },
    }
}

/// `s` cut from the front to fit `max_w` pixels (`...tail`): a name being
/// typed keeps its end, where the `_` cursor is, in view.
fn fit_tail(backend: &dyn SdiBackend, s: &str, font: u16, max_w: u32) -> String {
    if backend.measure_text(s, font) <= max_w {
        return s.to_string();
    }
    s.char_indices()
        .map(|(i, _)| format!("...{}", &s[i..]))
        .find(|t| backend.measure_text(t, font) <= max_w)
        .unwrap_or_default()
}

/// Result of clicking while a dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogClick {
    Ok,
    Cancel,
    None,
}

impl FileManagerApp {
    /// Hit-test a click (content-local coords) against the open dialog.
    pub(crate) fn dialog_hit(&self, lx: i32, ly: i32, cw: u32, ch: u32) -> DialogClick {
        let g = dialog_geom(0, 0, cw, ch);
        if contains(g.ok, lx, ly) {
            DialogClick::Ok
        } else if contains(g.cancel, lx, ly) {
            DialogClick::Cancel
        } else {
            DialogClick::None
        }
    }

    /// Draw the open dialog (if any) over the windowed content.
    pub(crate) fn draw_dialog_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        let Some(dialog) = &self.dialog else {
            return Ok(());
        };
        let colors = FileManagerColors::from_theme(at);
        let g = dialog_geom(cx, cy, cw, ch);
        let t = dialog_text(dialog, self.status.as_deref());
        let f = g.frame;
        backend.fill_rect(f.x, f.y, f.w, f.h, colors.bg)?;
        draw_outline(backend, f.x, f.y, f.w, f.h, colors.divider)?;
        let tb = g.title;
        backend.fill_rect(tb.x, tb.y, tb.w, tb.h, colors.status_bg)?;
        backend.draw_text(
            t.title,
            tb.x + 4,
            tb.y + 3,
            at.font_hint,
            colors.status_text,
        )?;
        backend.draw_text_ellipsis(
            &t.msg,
            f.x + 8,
            g.msg_y,
            at.font_hint,
            colors.text,
            f.w.saturating_sub(16),
        )?;
        if let Some(field) = &t.field {
            let fb = g.field;
            backend.fill_rect(fb.x, fb.y, fb.w, fb.h, colors.pane_bg)?;
            draw_outline(backend, fb.x, fb.y, fb.w, fb.h, colors.divider)?;
            backend.draw_text(
                &fit_tail(backend, field, at.font_hint, fb.w.saturating_sub(8)),
                fb.x + 4,
                fb.y + 3,
                at.font_hint,
                colors.pane_text,
            )?;
        }
        for (b, label, primary) in [(g.ok, t.ok, true), (g.cancel, t.cancel, false)] {
            let (bg, fg) = if primary {
                (colors.selected_bg, colors.selected_text)
            } else {
                (colors.status_bg, colors.status_text)
            };
            backend.fill_rect(b.x, b.y, b.w, b.h, bg)?;
            draw_outline(backend, b.x, b.y, b.w, b.h, colors.divider)?;
            backend.draw_text(label, b.x + 6, b.y + 3, at.font_hint, fg)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geom_is_centred_and_buttons_inside() {
        let g = dialog_geom(0, 0, 480, 272);
        assert_eq!(g.frame.x, (480 - g.frame.w as i32) / 2);
        for b in [g.ok, g.cancel, g.field] {
            assert!(b.x >= g.frame.x && b.y >= g.frame.y);
            assert!(b.x + b.w as i32 <= g.frame.x + g.frame.w as i32);
            assert!(b.y + b.h as i32 <= g.frame.y + g.frame.h as i32);
        }
        assert!(g.ok.x + (g.ok.w as i32) <= g.cancel.x);
    }
}

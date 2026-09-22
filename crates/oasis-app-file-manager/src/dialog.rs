//! Modal prompt overlay (delete confirmation, name entry) for the File
//! Manager.
//!
//! [`dialog_geom`] is the single source of truth for the prompt layout:
//! the windowed renderer, the SDI renderer and the click hit-test all
//! derive their rectangles from it so they can never drift apart.

use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;

use crate::colors::FileManagerColors;
use crate::model::{Dialog, NamePurpose};
use crate::ops::file_name;
use crate::state::FileManagerApp;
use crate::view::{Box2d, TextStyle, draw_outline, ensure_rect, ensure_text, outline_rect};

/// Dialog box height in pixels.
const DLG_H: u32 = 86;
/// Dialog title-strip height.
const DLG_TITLE_H: u32 = 16;
/// Button width / height.
const DLG_BTN_W: u32 = 60;
const DLG_BTN_H: u32 = 16;
/// Base z-order for SDI dialog objects (above menus and tiles).
const DLG_Z: i32 = 220;

/// Every SDI object name the dialog may create (hidden when closed).
const DLG_SDI_NAMES: &[&str] = &[
    "app_fm_dlg_bg",
    "app_fm_dlg_o_t",
    "app_fm_dlg_o_b",
    "app_fm_dlg_o_l",
    "app_fm_dlg_o_r",
    "app_fm_dlg_title_bg",
    "app_fm_dlg_title",
    "app_fm_dlg_msg",
    "app_fm_dlg_field",
    "app_fm_dlg_field_o_t",
    "app_fm_dlg_field_o_b",
    "app_fm_dlg_field_o_l",
    "app_fm_dlg_field_o_r",
    "app_fm_dlg_field_text",
    "app_fm_dlg_ok",
    "app_fm_dlg_ok_lbl",
    "app_fm_dlg_cancel",
    "app_fm_dlg_cancel_lbl",
];

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

/// Clip `s` to roughly fit `w` pixels at ~7px per glyph.
fn fit(s: &str, w: u32) -> String {
    crate::model::truncate_label(s, (w as usize / 7).max(4))
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
        backend.draw_text(
            &fit(&t.msg, f.w - 16),
            f.x + 8,
            g.msg_y,
            at.font_hint,
            colors.text,
        )?;
        if let Some(field) = &t.field {
            let fb = g.field;
            backend.fill_rect(fb.x, fb.y, fb.w, fb.h, colors.pane_bg)?;
            draw_outline(backend, fb.x, fb.y, fb.w, fb.h, colors.divider)?;
            backend.draw_text(
                &fit(field, fb.w - 8),
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

    /// Mirror of [`Self::draw_dialog_windowed`] over pooled SDI objects.
    pub(crate) fn update_dialog_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        hide_dialog_sdi(sdi);
        let Some(dialog) = &self.dialog else {
            return;
        };
        let colors = FileManagerColors::from_theme(at);
        let g = dialog_geom(0, 0, at.screen_w, at.screen_h);
        let t = dialog_text(dialog, self.status.as_deref());
        let f = g.frame;
        ensure_rect(sdi, "app_fm_dlg_bg", f, colors.bg, DLG_Z);
        outline_rect(sdi, "app_fm_dlg_o", f, colors.divider, DLG_Z + 1);
        ensure_rect(
            sdi,
            "app_fm_dlg_title_bg",
            g.title,
            colors.status_bg,
            DLG_Z + 1,
        );
        let style = |color, z| TextStyle {
            font_size: at.font_hint,
            color,
            z,
        };
        ensure_text(
            sdi,
            "app_fm_dlg_title",
            t.title,
            g.title.x + 4,
            g.title.y + 3,
            style(colors.status_text, DLG_Z + 2),
        );
        ensure_text(
            sdi,
            "app_fm_dlg_msg",
            &fit(&t.msg, f.w - 16),
            f.x + 8,
            g.msg_y,
            style(colors.text, DLG_Z + 2),
        );
        if let Some(field) = &t.field {
            let fb = g.field;
            ensure_rect(sdi, "app_fm_dlg_field", fb, colors.pane_bg, DLG_Z + 1);
            outline_rect(sdi, "app_fm_dlg_field_o", fb, colors.divider, DLG_Z + 2);
            ensure_text(
                sdi,
                "app_fm_dlg_field_text",
                &fit(field, fb.w - 8),
                fb.x + 4,
                fb.y + 3,
                style(colors.pane_text, DLG_Z + 3),
            );
        }
        for (b, name, label, primary) in [
            (g.ok, "app_fm_dlg_ok", t.ok, true),
            (g.cancel, "app_fm_dlg_cancel", t.cancel, false),
        ] {
            let (bg, fg) = if primary {
                (colors.selected_bg, colors.selected_text)
            } else {
                (colors.status_bg, colors.status_text)
            };
            ensure_rect(sdi, name, b, bg, DLG_Z + 1);
            ensure_text(
                sdi,
                &format!("{name}_lbl"),
                label,
                b.x + 6,
                b.y + 3,
                style(fg, DLG_Z + 2),
            );
        }
    }
}

/// Hide every dialog SDI object.
pub(crate) fn hide_dialog_sdi(sdi: &mut SdiRegistry) {
    for name in DLG_SDI_NAMES {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
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

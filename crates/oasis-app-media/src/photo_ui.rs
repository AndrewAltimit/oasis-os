//! Photo Viewer window rendering: actual image pixels + overlay chrome.
//!
//! Uploads the decoded RGBA buffer to the backend as a texture once per
//! opened image (cached in `BrowsingApp::cached_photo_texture`), then
//! blits it centered with aspect-preserving fit-or-zoom each frame, and
//! draws a small filename/dimensions footer on top. `open_file` parks
//! the previous texture in `stale_photo_texture` so we can destroy it
//! here on the first render of the new image.

use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

use crate::BrowsingApp;

pub fn draw(
    app: &BrowsingApp,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
) -> Result<()> {
    // Dark background — black-matte like most photo viewers.
    backend.fill_rect(cx, cy, cw, ch, Color::rgb(8, 8, 12))?;

    // Reserve a footer strip for filename/dimensions.
    let footer_h: u32 = 18;
    let img_h = ch.saturating_sub(footer_h);

    // Destroy any textures left over from previous images. `open_file`
    // and `inherit_textures_from` stash old handles here because they
    // don't have backend access; a runner that cycles through several
    // photos in a tick can accumulate multiple entries.
    for stale in app.stale_photo_textures.borrow_mut().drain(..) {
        let _ = backend.destroy_texture(stale);
    }

    if let Some(img) = app.decoded_image() {
        let tex = match app.cached_photo_texture.get() {
            Some(t) => t,
            None => match backend.load_texture(img.width, img.height, &img.rgba) {
                Ok(t) => {
                    app.cached_photo_texture.set(Some(t));
                    t
                },
                Err(e) => {
                    log::warn!("photo viewer: load_texture failed: {e}");
                    draw_footer(app, cx, cy, cw, ch, footer_h, backend, at)?;
                    return Ok(());
                },
            },
        };

        // Fit (aspect-preserving) into the available area, multiplied
        // by the user's zoom level. Zoom > 1 may push the image off
        // the edges — simple and expected behaviour for "zoom in".
        let avail_w = cw as f32;
        let avail_h = img_h as f32;
        let scale = (avail_w / img.width as f32)
            .min(avail_h / img.height as f32)
            .min(1.0)
            * app.zoom_level() as f32;
        let draw_w = (img.width as f32 * scale).max(1.0) as u32;
        let draw_h = (img.height as f32 * scale).max(1.0) as u32;
        let draw_x = cx + ((cw as i32 - draw_w as i32) / 2);
        let draw_y = cy + ((img_h as i32 - draw_h as i32) / 2);

        let _ = backend.blit(tex, draw_x, draw_y, draw_w, draw_h);
    } else {
        // Couldn't decode — show a centered placeholder with the
        // metadata we do have.
        backend.fill_rect(
            cx + cw as i32 / 4,
            cy + img_h as i32 / 4,
            cw / 2,
            img_h / 2,
            Color::rgb(30, 30, 40),
        )?;
        let msg = "(preview not available)";
        let x = cx + (cw as i32 / 2) - (msg.len() as i32 * 3);
        backend.draw_text(
            msg,
            x,
            cy + img_h as i32 / 2,
            at.font_body,
            Color::rgb(200, 200, 200),
        )?;
    }

    draw_footer(app, cx, cy, cw, ch, footer_h, backend, at)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_footer(
    app: &BrowsingApp,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    footer_h: u32,
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
) -> Result<()> {
    let fy = cy + ch as i32 - footer_h as i32;
    backend.fill_rect(cx, fy, cw, footer_h, Color::rgba(0, 0, 0, 180))?;

    let path = app.content.viewing_file.as_deref().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or(path);

    let dim_text = app
        .decoded_image()
        .map(|img| format!("  {}x{}", img.width, img.height))
        .unwrap_or_default();
    let zoom_text = if app.zoom_level() > 1 {
        format!("  {}x", app.zoom_level())
    } else {
        String::new()
    };
    let line = format!("{name}{dim_text}{zoom_text}");

    backend.draw_text(
        &line,
        cx + 6,
        fy + 4,
        at.font_hint,
        Color::rgb(220, 220, 220),
    )?;

    // Right-aligned key hint.
    let hint = "\u{25b3} zoom  \u{25a1} rotate  Cancel back";
    let hint_x = cx + cw as i32 - (hint.len() as i32 * 5) - 8;
    backend.draw_text(
        hint,
        hint_x,
        fy + 4,
        at.font_hint,
        Color::rgb(160, 160, 180),
    )?;
    Ok(())
}

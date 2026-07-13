//! Window rendering and clip-rect management.
//!
//! Provides `draw_with_clips` and `draw_with_clips_noalloc` which render
//! windows in z-order with proper content clipping.

use oasis_sdi::SdiRegistry;
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use super::manager::WindowManager;
use super::window::WindowState;

impl WindowManager {
    /// Draw window content with clipping. The caller provides a draw callback
    /// for each window's content. The WM sets up clip rects before each call
    /// and resets them after.
    pub fn draw_with_clips<F>(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
        draw_content: F,
    ) -> Result<()>
    where
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        self.draw_with_clips_overlay(sdi, backend, |_| Ok(()), draw_content)
    }

    /// Like [`Self::draw_with_clips`] but injects an extra rendering step
    /// between the base SDI pass and the per-window pass.
    ///
    /// Callers that render outside of SDI (e.g. vector-icon dashboards that
    /// draw glyphs directly to the backend) need to paint *after* the
    /// wallpaper / dashboard backdrops have been laid down but *before* the
    /// windows, so the glyphs sit underneath any floating app windows. The
    /// standard `draw_with_clips` leaves no seam for that.
    ///
    /// This is the single compositing path for every backend, and it performs
    /// no heap allocation per frame: window-owned SDI objects are excluded with
    /// a byte-comparison filter instead of a `Vec` of `format!`ed prefixes, and
    /// per-window object names are built in a stack buffer.
    pub fn draw_with_clips_overlay<G, F>(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
        mut overlay_after_base: G,
        mut draw_content: F,
    ) -> Result<()>
    where
        G: FnMut(&mut dyn SdiBackend) -> Result<()>,
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        // Stack buffer for building "{id}.{suffix}" names without allocation.
        let mut name_buf = [0u8; 64];

        // Draw non-window base SDI objects (wallpaper, dashboard, bars, etc.);
        // window-owned objects are drawn per-window below instead. With no
        // windows open the filter would reject nothing, so skip it entirely.
        if self.windows.is_empty() {
            sdi.draw_base_layer(backend)?;
        } else {
            sdi.draw_base_filtered(backend, |obj_name| !self.is_window_owned(obj_name))?;
        }

        // Overlay step: e.g. vector icons for dashboards that live behind windows.
        overlay_after_base(backend)?;

        // Draw each window's SDI objects then content in z-order.
        // This ensures the active (topmost) window renders over all others.
        for window in &self.windows {
            if window.state == WindowState::Minimized {
                continue;
            }

            // Draw this window's SDI objects (frame, titlebar, buttons, etc.).
            let id = window.id.as_str();
            for suffix in window.sdi_suffixes() {
                let name = fmt_sdi_name(&mut name_buf, id, suffix);
                sdi.draw_named(name, backend)?;
            }

            // Draw clipped content inside the window.
            let (cx, cy, cw, ch) = window.content_rect(&self.theme);
            if cw > 0 && ch > 0 {
                backend.set_clip_rect(cx, cy, cw, ch)?;
                draw_content(&window.id, cx, cy, cw, ch, backend)?;
                backend.reset_clip_rect()?;
            }
        }

        // Draw non-window overlay SDI objects (cursor, start menu, toasts)
        // AFTER windows so they render on top.
        if self.windows.is_empty() {
            sdi.draw_overlay_layer(backend)?;
        } else {
            sdi.draw_overlay_filtered(backend, |obj_name| !self.is_window_owned(obj_name))?;
        }

        Ok(())
    }

    /// Allocation-free variant of [`Self::draw_with_clips`] for constrained targets.
    ///
    /// Retained as the name PSP-style callers use; [`Self::draw_with_clips`] is
    /// allocation-free too, so this is now a thin alias.
    ///
    /// Callers on constrained platforms should hide expensive base objects
    /// (e.g. dashboard icons) before calling this method.
    pub fn draw_with_clips_noalloc<F>(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
        draw_content: F,
    ) -> Result<()>
    where
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        self.draw_with_clips_overlay(sdi, backend, |_| Ok(()), draw_content)
    }

    /// True if `obj_name` is `"{window_id}.{suffix}"` for any managed window.
    ///
    /// Byte comparison against the live window ids — no prefix strings are
    /// built, and the scan is over the (small) window list, not the SDI scene.
    fn is_window_owned(&self, obj_name: &str) -> bool {
        self.windows.iter().any(|w| {
            let id = w.id.as_str();
            obj_name.len() > id.len()
                && obj_name.as_bytes()[id.len()] == b'.'
                && obj_name.starts_with(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::WindowManager;
    use crate::window::{WindowConfig, WindowType};
    use oasis_types::backend::{Color, SdiCore, TextureId};

    /// Backend that records the red channel of every `fill_rect`, which the
    /// tests use as a per-object tag.
    struct TagRecorder(Vec<u8>);

    impl SdiCore for TagRecorder {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, _color: Color) -> Result<()> {
            Ok(())
        }
        fn blit(&mut self, _t: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, color: Color) -> Result<()> {
            self.0.push(color.r);
            Ok(())
        }
        fn draw_text(&mut self, _t: &str, _x: i32, _y: i32, _fs: u16, _c: Color) -> Result<()> {
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _d: &[u8]) -> Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, _t: &str, _fs: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
            Ok(vec![])
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl oasis_types::backend::SdiShapes for TagRecorder {}
    impl oasis_types::backend::SdiGradients for TagRecorder {}
    impl oasis_types::backend::SdiAlpha for TagRecorder {}
    impl oasis_types::backend::SdiText for TagRecorder {}
    impl oasis_types::backend::SdiTextures for TagRecorder {}
    impl oasis_types::backend::SdiClipTransform for TagRecorder {}
    impl oasis_types::backend::SdiVector for TagRecorder {}
    impl oasis_types::backend::SdiBatch for TagRecorder {}
    impl oasis_types::backend::SdiRenderTarget for TagRecorder {}

    /// Tag an SDI object so the recorder can identify it in the draw stream.
    fn tag(sdi: &mut SdiRegistry, name: &str, tag: u8, overlay: bool) {
        let obj = sdi.create(name);
        obj.w = 10;
        obj.h = 10;
        obj.color = Color::rgb(tag, 0, 0);
        obj.overlay = overlay;
    }

    fn app_config(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(10),
            y: Some(10),
            width: 200,
            height: 150,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        }
    }

    #[test]
    fn base_pass_excludes_window_owned_objects() {
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        tag(&mut sdi, "wallpaper", 1, false);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        // A window-prefixed object the window doesn't list as a suffix: it is
        // excluded from the base pass and never drawn per-window either.
        tag(&mut sdi, "w1.stray", 2, false);
        tag(&mut sdi, "cursor", 3, true);

        let mut backend = TagRecorder(Vec::new());
        wm.draw_with_clips(&mut sdi, &mut backend, |_, _, _, _, _, _| Ok(()))
            .unwrap();

        let tags = &backend.0;
        assert!(tags.contains(&1), "wallpaper must draw in the base pass");
        assert!(!tags.contains(&2), "window-owned objects must be excluded");
        // Cursor is an overlay: it draws after the window's chrome.
        assert_eq!(tags.last(), Some(&3));
    }

    #[test]
    fn window_ids_are_not_prefix_confused() {
        // "w1" must not swallow "w10.frame" or "w1x" — the filter matches on a
        // '.' boundary, not a bare `starts_with`.
        let mut sdi = SdiRegistry::new();
        let mut wm = WindowManager::new(800, 600);
        wm.create_window(&app_config("w1"), &mut sdi).unwrap();
        tag(&mut sdi, "w1x", 7, false);
        tag(&mut sdi, "w10.frame", 8, false);

        let mut backend = TagRecorder(Vec::new());
        wm.draw_with_clips(&mut sdi, &mut backend, |_, _, _, _, _, _| Ok(()))
            .unwrap();

        assert!(backend.0.contains(&7));
        assert!(backend.0.contains(&8));
    }

    #[test]
    fn noalloc_variant_matches_default_path() {
        let build = || {
            let mut sdi = SdiRegistry::new();
            let mut wm = WindowManager::new(800, 600);
            tag(&mut sdi, "wallpaper", 1, false);
            wm.create_window(&app_config("w1"), &mut sdi).unwrap();
            tag(&mut sdi, "cursor", 3, true);
            (sdi, wm)
        };

        let (mut sdi_a, wm_a) = build();
        let mut a = TagRecorder(Vec::new());
        wm_a.draw_with_clips(&mut sdi_a, &mut a, |_, _, _, _, _, _| Ok(()))
            .unwrap();

        let (mut sdi_b, wm_b) = build();
        let mut b = TagRecorder(Vec::new());
        wm_b.draw_with_clips_noalloc(&mut sdi_b, &mut b, |_, _, _, _, _, _| Ok(()))
            .unwrap();

        assert_eq!(a.0, b.0);
    }
}

/// Format `"{id}.{suffix}"` into a stack buffer, returning `&str`.
pub(crate) fn fmt_sdi_name<'a>(buf: &'a mut [u8; 64], id: &str, suffix: &str) -> &'a str {
    let id_bytes = id.as_bytes();
    let suf_bytes = suffix.as_bytes();
    let total = id_bytes.len() + 1 + suf_bytes.len();
    if total > buf.len() {
        return "";
    }
    buf[..id_bytes.len()].copy_from_slice(id_bytes);
    buf[id_bytes.len()] = b'.';
    buf[id_bytes.len() + 1..total].copy_from_slice(suf_bytes);
    // SAFETY: id and suffix are valid UTF-8, '.' is ASCII.
    unsafe { core::str::from_utf8_unchecked(&buf[..total]) }
}

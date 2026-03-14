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
        mut draw_content: F,
    ) -> Result<()>
    where
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        // Collect window id prefixes so we can exclude them from the
        // global SDI draw pass (they'll be drawn per-window instead).
        let prefixes: Vec<String> = self.windows.iter().map(|w| format!("{}.", w.id)).collect();
        let prefix_refs: Vec<&str> = prefixes.iter().map(|s| s.as_str()).collect();

        // Draw non-window base SDI objects (wallpaper, dashboard, bars, etc.).
        sdi.draw_base_excluding_prefixes(backend, &prefix_refs)?;

        // Draw each window's SDI objects then content in z-order.
        // This ensures the active (topmost) window renders over all others.
        for window in &self.windows {
            if window.state == WindowState::Minimized {
                continue;
            }

            // Draw this window's SDI objects (frame, titlebar, buttons, etc.).
            for suffix in window.sdi_suffixes() {
                let name = window.sdi_name(suffix);
                sdi.draw_named(&name, backend)?;
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
        sdi.draw_overlay_excluding_prefixes(backend, &prefix_refs)?;

        Ok(())
    }

    /// Allocation-free variant of [`Self::draw_with_clips`] for constrained targets.
    ///
    /// Uses a fixed stack buffer for SDI name construction instead of
    /// `format!()`/`Vec` allocations. Suitable for PSP where heap
    /// allocations in the render loop cause frame-time spikes.
    pub fn draw_with_clips_noalloc<F>(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
        mut draw_content: F,
    ) -> Result<()>
    where
        F: FnMut(&str, i32, i32, u32, u32, &mut dyn SdiBackend) -> Result<()>,
    {
        // Stack buffer for building "{id}.{suffix}" names without allocation.
        let mut name_buf = [0u8; 64];

        // Draw non-window base SDI objects (skip window-owned ones).
        // Callers on constrained platforms should hide expensive base objects
        // (e.g. dashboard icons) before calling this method.
        sdi.draw_base_filtered(backend, |obj_name| {
            !self.windows.iter().any(|w| {
                let id = w.id.as_str();
                obj_name.len() > id.len()
                    && obj_name.as_bytes()[id.len()] == b'.'
                    && obj_name.starts_with(id)
            })
        })?;

        // Per-window suffixes (compile-time known for AppWindow).
        const APP_SUFFIXES: &[&str] = &[
            "frame",
            "titlebar",
            "title_text",
            "title_shadow",
            "separator",
            "btn_close",
            "btn_close_glyph",
            "btn_minimize",
            "btn_minimize_glyph",
            "btn_maximize",
            "btn_maximize_glyph",
            "content",
            "content_stroke",
        ];

        for window in &self.windows {
            if window.state == WindowState::Minimized {
                continue;
            }

            // Draw this window's SDI objects using stack-formatted names.
            let id = window.id.as_str();
            let suffixes = match window.window_type {
                super::window::WindowType::Fullscreen => &["content"][..],
                super::window::WindowType::Panel => {
                    // frame + titlebar chrome + content, no buttons
                    const PANEL_SUFFIXES: &[&str] = &[
                        "frame",
                        "titlebar",
                        "title_text",
                        "title_shadow",
                        "separator",
                        "content",
                        "content_stroke",
                    ];
                    PANEL_SUFFIXES
                },
                _ => APP_SUFFIXES,
            };
            for suffix in suffixes {
                let name = fmt_sdi_name(&mut name_buf, id, suffix);
                sdi.draw_named(name, backend)?;
            }

            // Draw clipped content.
            let (cx, cy, cw, ch) = window.content_rect(&self.theme);
            if cw > 0 && ch > 0 {
                backend.set_clip_rect(cx, cy, cw, ch)?;
                draw_content(&window.id, cx, cy, cw, ch, backend)?;
                backend.reset_clip_rect()?;
            }
        }

        // Draw non-window overlay SDI objects on top.
        sdi.draw_overlay_filtered(backend, |obj_name| {
            !self.windows.iter().any(|w| {
                let id = w.id.as_str();
                obj_name.len() > id.len()
                    && obj_name.as_bytes()[id.len()] == b'.'
                    && obj_name.starts_with(id)
            })
        })?;

        Ok(())
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

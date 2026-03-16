//! Rendering and paint methods for [`BrowserWidget`].

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_vfs::Vfs;

use crate::paint;
use crate::{BrowserWidget, Focus, LoadingState};

impl BrowserWidget {
    // ---------------------------------------------------------------
    // Painting
    // ---------------------------------------------------------------

    /// Paint the browser to the backend.
    ///
    /// Draws chrome (URL bar, navigation buttons, status bar) and
    /// the page content viewport.
    /// Per-frame update: process pending image loads within a time budget.
    ///
    /// Call this once per frame before `paint()`. Images stream in
    /// progressively so the page is never blocked waiting for all images.
    pub fn tick(&mut self, vfs: &dyn Vfs) {
        self.load_next_image_batch(vfs, 8);

        // Process pending JS navigation actions.
        #[cfg(feature = "javascript")]
        {
            use crate::js_dom;
            let actions = js_dom::drain_nav_actions(&self.js_nav_actions);
            for action in actions {
                match action {
                    js_dom::JsNavAction::Navigate(url) => {
                        self.navigate_to(&url, vfs);
                    },
                    js_dom::JsNavAction::Back => {
                        self.go_back(vfs);
                    },
                    js_dom::JsNavAction::Forward => {
                        self.go_forward(vfs);
                    },
                }
            }
        }
    }

    pub fn paint(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        // Rebuild layout if the viewport was resized since last paint.
        self.relayout_if_dirty();

        // Set clip to our window area.
        backend.set_clip_rect(self.window_x, self.window_y, self.window_w, self.window_h)?;

        // Paint chrome (URL bar + buttons).
        self.paint_chrome(backend)?;

        // Close the window clip before opening a tighter content clip.
        backend.reset_clip_rect()?;

        // Content viewport.
        let content_y = self.window_y + self.config.url_bar_height as i32;
        let content_h = self.config.content_height(self.window_h);

        backend.set_clip_rect(self.window_x, content_y, self.window_w, content_h)?;

        // Paint page background.
        backend.fill_rect(
            self.window_x,
            content_y,
            self.window_w,
            content_h,
            self.config.default_bg_color,
        )?;

        // Upload decoded images as GPU textures and assign to layout
        // nodes (first paint after navigation).
        self.ensure_image_textures(backend);

        // Paint layout tree if available.
        if let Some(layout) = &self.layout_root {
            let result = paint::paint(
                layout,
                backend,
                paint::PaintViewport {
                    scroll_y: self.scroll.scroll_y as f32,
                    scroll_x: self.scroll.scroll_x as f32,
                    x: self.window_x,
                    y: content_y,
                    width: self.window_w as f32,
                    height: content_h as f32,
                },
                &self.href_map,
            )?;
            self.link_map = result.links;
            self.scroll.set_content_height(result.content_height as i32);
        }

        // Paint link highlight if a link is selected.
        if self.selected_link >= 0 {
            let idx = self.selected_link as usize;
            if idx < self.link_map.len() {
                let link = self.link_map[idx].clone();
                paint::paint_link_highlight(&link, backend, Color::rgb(255, 200, 0))?;
            }
        }

        // Paint scrollbar when content overflows viewport.
        if self.scroll.max_scroll() > 0 {
            let sb_w: u32 = 6;
            let sb_x = self.window_x + self.window_w as i32 - sb_w as i32 - 1;
            let track_y = content_y;
            let track_h = content_h;

            // Track.
            backend.fill_rect(sb_x, track_y, sb_w, track_h, Color::rgba(255, 255, 255, 20))?;

            // Thumb: proportional to viewport/content ratio.
            let ratio = self.scroll.viewport_height as f32 / self.scroll.content_height as f32;
            let thumb_h = ((track_h as f32 * ratio) as u32).max(12).min(track_h);
            let scrollable = track_h - thumb_h;
            let frac = self.scroll.scroll_fraction();
            let thumb_y = track_y + (scrollable as f32 * frac) as i32;
            backend.fill_rect(
                sb_x,
                thumb_y,
                sb_w,
                thumb_h,
                Color::rgba(255, 255, 255, 100),
            )?;
        }

        // Paint status bar.
        self.paint_status_bar(backend)?;

        backend.reset_clip_rect()?;
        Ok(())
    }

    /// Paint only the browser chrome (URL bar + status bar), skipping page
    /// content. Used when an external iframe is rendering the page.
    pub fn paint_chrome_only(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        backend.set_clip_rect(self.window_x, self.window_y, self.window_w, self.window_h)?;
        self.paint_chrome(backend)?;
        self.paint_status_bar(backend)?;
        backend.reset_clip_rect()?;
        Ok(())
    }

    /// Paint the URL bar and navigation buttons.
    pub fn paint_chrome(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let h = self.config.url_bar_height;
        let bw = self.config.button_width;
        let themed = self.config.use_themed_chrome;
        let r: u16 = 4; // Chrome element border radius.

        // Chrome background.
        if themed {
            backend.fill_rounded_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                r,
                self.config.chrome_bg,
            )?;
        } else {
            backend.fill_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                self.config.chrome_bg,
            )?;
        }

        // Back button.
        let back_color = if self.nav.can_go_back() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(self.window_x, self.window_y, bw, h, r, back_color)?;
        } else {
            backend.fill_rect(self.window_x, self.window_y, bw, h, back_color)?;
        }
        backend.draw_text(
            "<",
            self.window_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // Forward button.
        let fwd_color = if self.nav.can_go_forward() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(
                self.window_x + bw as i32,
                self.window_y,
                bw,
                h,
                r,
                fwd_color,
            )?;
        } else {
            backend.fill_rect(self.window_x + bw as i32, self.window_y, bw, h, fwd_color)?;
        }
        backend.draw_text(
            ">",
            self.window_x + bw as i32 + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // URL bar.
        let url_x = self.window_x + (bw * 2) as i32;
        let url_w = self.window_w.saturating_sub(bw * 3);

        // Use a highlighted background when the URL bar is focused.
        let bar_bg = if self.focus == Focus::UrlBar {
            Color::rgb(60, 60, 80)
        } else {
            self.config.url_bar_bg
        };
        if themed {
            backend.fill_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                bar_bg,
            )?;
            // Stroke around URL bar for definition.
            backend.stroke_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                1,
                Color::rgba(255, 255, 255, 30),
            )?;
        } else {
            backend.fill_rect(url_x, self.window_y + 2, url_w, h.saturating_sub(4), bar_bg)?;
        }

        // URL text: show the editing buffer when focused, otherwise
        // the current navigation URL.
        let max_chars = (url_w / 8).saturating_sub(1) as usize;
        if self.focus == Focus::UrlBar {
            // Show editing buffer with cursor indicator.
            let display = if self.url_input.len() > max_chars {
                &self.url_input[..self.url_input.floor_char_boundary(max_chars)]
            } else {
                &self.url_input
            };
            backend.draw_text(
                display,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;

            // Draw cursor line.
            let cursor_chars = self.url_input[..self.url_cursor].chars().count();
            let cursor_px = url_x + 4 + cursor_chars as i32 * 8;
            if cursor_px < url_x + url_w as i32 - 4 {
                backend.fill_rect(
                    cursor_px,
                    self.window_y + 3,
                    1,
                    h.saturating_sub(6),
                    self.config.url_bar_text,
                )?;
            }
        } else {
            let url_text = self.nav.current_url().unwrap_or("about:blank");
            let display_url = if url_text.len() > max_chars {
                &url_text[..url_text.floor_char_boundary(max_chars)]
            } else {
                url_text
            };
            backend.draw_text(
                display_url,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;
        }

        // Home button (rightmost).
        let home_x = self.window_x + self.window_w as i32 - bw as i32;
        if themed {
            backend.fill_rounded_rect(
                home_x,
                self.window_y + 2,
                bw,
                h.saturating_sub(4),
                r,
                self.config.chrome_button_bg,
            )?;
        } else {
            backend.fill_rect(home_x, self.window_y, bw, h, self.config.chrome_button_bg)?;
        }
        backend.draw_text(
            "H",
            home_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        Ok(())
    }

    /// Paint the status bar at the bottom.
    pub fn paint_status_bar(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let sh = self.config.status_bar_height;
        let sy = self.window_y + self.window_h as i32 - sh as i32;

        backend.fill_rect(
            self.window_x,
            sy,
            self.window_w,
            sh,
            self.config.status_bar_bg,
        )?;

        // Status text.
        let status = match self.state {
            LoadingState::Idle => {
                if self.reader_mode {
                    "Reader mode"
                } else {
                    "Ready"
                }
            },
            LoadingState::Loading => "Loading...",
            LoadingState::Error => "Error",
        };
        backend.draw_text(
            status,
            self.window_x + 4,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        // Scroll indicator on the right.
        let frac = self.scroll.scroll_fraction();
        let pct = (frac * 100.0) as u32;
        let scroll_text = format!("{}%", pct);
        let text_w = oasis_types::backend::bitmap_measure_text(&scroll_text, 10) as i32;
        backend.draw_text(
            &scroll_text,
            self.window_x + self.window_w as i32 - text_w - 4,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        Ok(())
    }
}

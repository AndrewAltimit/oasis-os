//! In-canvas video player for TV Guide playback.
//!
//! Uses a hidden `<video>` element playing a direct MP4 URL.  Each frame the
//! video content is drawn onto an offscreen `<canvas>` that is registered as a
//! backend texture.  The existing `preview_texture` rendering path in
//! `guide.rs` handles display — no iframe overlay needed.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement};

use oasis_types::backend::{SdiBackend, TextureId};

use crate::renderer::WasmBackend;

/// Manages a hidden `<video>` element whose frames are captured onto an
/// offscreen canvas registered as a backend texture.
pub struct VideoPlayer {
    video: Option<HtmlVideoElement>,
    capture_canvas: Option<HtmlCanvasElement>,
    capture_ctx: Option<CanvasRenderingContext2d>,
    texture_id: Option<TextureId>,
    width: u32,
    height: u32,
    active: bool,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPlayer {
    /// Create in idle (no-video) state.
    pub fn new() -> Self {
        Self {
            video: None,
            capture_canvas: None,
            capture_ctx: None,
            texture_id: None,
            width: 0,
            height: 0,
            active: false,
        }
    }

    /// Start playing a direct MP4 URL.
    ///
    /// Creates a hidden `<video>`, an offscreen capture canvas registered as a
    /// texture, and begins playback.  Returns the `TextureId` the caller
    /// should assign to `guide.preview_texture`.
    pub fn start(
        &mut self,
        url: &str,
        seek_secs: u64,
        w: u32,
        h: u32,
        backend: &mut WasmBackend,
    ) -> Option<TextureId> {
        // Tear down any previous session.
        self.stop(backend);

        let window = web_sys::window()?;
        let document = window.document()?;

        // --- Hidden <video> element ---
        let video: HtmlVideoElement = document.create_element("video").ok()?.dyn_into().ok()?;
        // Do NOT set crossOrigin — archive.org redirects to a CDN that does
        // not return CORS headers, which blocks the video fetch entirely.
        // Without crossOrigin the canvas becomes "tainted" (getImageData
        // blocked) but drawImage for frame capture and blitting works fine.
        video.set_attribute("playsinline", "").ok()?;
        video.set_preload("auto");
        // Start muted — muted autoplay is universally allowed without user
        // gesture, avoiding AbortError that kills the network fetch.
        video.set_muted(true);
        video.set_src(url);
        if seek_secs > 0 {
            video.set_current_time(seek_secs as f64);
        }
        // Hide from viewport.
        video.style().set_property("display", "none").ok()?;
        document.body()?.append_child(&video).ok()?;

        // --- Offscreen capture canvas ---
        let capture: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
        capture.set_width(w);
        capture.set_height(h);

        let ctx: CanvasRenderingContext2d = capture
            .get_context("2d")
            .ok()?
            .and_then(|c| c.dyn_into().ok())?;

        // Register the capture canvas as a texture (zero-copy path).
        let tex_id = backend.register_canvas_as_texture(capture.clone());

        // Start playback. Muted autoplay should succeed immediately.
        // On success, try to unmute for audio.
        let video_clone = video.clone();
        if let Ok(promise) = video.play() {
            let ok_handler = Closure::wrap(Box::new(move |_: JsValue| {
                // Playing muted — try to unmute (may silently fail).
                video_clone.set_muted(false);
            }) as Box<dyn FnMut(JsValue)>);
            let _ = promise.then(&ok_handler);
            ok_handler.forget();
        }

        self.video = Some(video);
        self.capture_canvas = Some(capture);
        self.capture_ctx = Some(ctx);
        self.texture_id = Some(tex_id);
        self.width = w;
        self.height = h;
        self.active = true;

        Some(tex_id)
    }

    /// Capture the current video frame onto the offscreen canvas.
    ///
    /// Should be called once per animation frame while active.  The texture
    /// automatically reflects the new content on the next `blit()`.
    pub fn tick(&self) {
        if let (Some(video), Some(ctx)) = (&self.video, &self.capture_ctx) {
            // HAVE_CURRENT_DATA (readyState >= 2) means at least one frame
            // is available for drawing.
            if video.ready_state() >= 2 {
                let _ = ctx.draw_image_with_html_video_element_and_dw_and_dh(
                    video,
                    0.0,
                    0.0,
                    self.width as f64,
                    self.height as f64,
                );
            }
        }
    }

    /// Stop playback, destroy DOM elements and the backend texture.
    pub fn stop(&mut self, backend: &mut WasmBackend) {
        if let Some(video) = self.video.take() {
            video.pause().ok();
            video.set_src("");
            if let Some(parent) = video.parent_node() {
                let _ = parent.remove_child(&video);
            }
        }
        self.capture_canvas = None;
        self.capture_ctx = None;
        if let Some(tex) = self.texture_id.take() {
            let _ = backend.destroy_texture(tex);
        }
        self.width = 0;
        self.height = 0;
        self.active = false;
    }

    /// Whether the player is currently loading or playing.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        // Best-effort DOM cleanup (no backend reference available here).
        if let Some(video) = self.video.take() {
            video.pause().ok();
            video.set_src("");
            if let Some(parent) = video.parent_node() {
                let _ = parent.remove_child(&video);
            }
        }
    }
}

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

macro_rules! vlog {
    ($($arg:tt)*) => {
        web_sys::console::log_1(&format!("[OASIS Video] {}", format!($($arg)*)).into());
    };
}

macro_rules! verr {
    ($($arg:tt)*) => {
        web_sys::console::error_1(&format!("[OASIS Video] {}", format!($($arg)*)).into());
    };
}

/// Check if the browser can play H.264/MP4 video.
fn can_play_h264() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return false,
    };
    let video: HtmlVideoElement = match document
        .create_element("video")
        .ok()
        .and_then(|e| e.dyn_into().ok())
    {
        Some(v) => v,
        None => return false,
    };
    let result = video.can_play_type("video/mp4; codecs=\"avc1.42E01E\"");
    !result.is_empty()
}

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
    logged_playing: bool,
    logged_error: bool,
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
            logged_playing: false,
            logged_error: false,
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

        vlog!(
            "Starting video: {}...  seek={}s  capture={}x{}",
            &url[..url.len().min(80)],
            seek_secs,
            w,
            h
        );

        // Check H.264 codec support before even trying.
        if !can_play_h264() {
            verr!("Browser cannot play H.264/MP4 video!");
            verr!("Internet Archive videos require H.264 support.");
            verr!(
                "If using Firefox snap on Linux, try installing: \
                 sudo apt install ffmpeg"
            );
            verr!("Or use Chrome/Chromium which has built-in H.264.");
            return None;
        }
        vlog!("H.264 codec: supported");

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

        // --- Error event listener ---
        let url_for_err = url[..url.len().min(80)].to_string();
        let error_handler = Closure::wrap(Box::new(move |_: web_sys::Event| {
            verr!("Video error for: {}", url_for_err);
            verr!("The video failed to load or decode. Check:");
            verr!("  1. Browser H.264/MP4 codec support");
            verr!("  2. Network connectivity to archive.org");
            verr!("  3. Browser console for additional errors");
        }) as Box<dyn FnMut(web_sys::Event)>);
        video
            .add_event_listener_with_callback("error", error_handler.as_ref().unchecked_ref())
            .ok();
        error_handler.forget();

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
                vlog!("Playback started (muted). Unmuting...");
                video_clone.set_muted(false);
            }) as Box<dyn FnMut(JsValue)>);

            let reject_handler = Closure::wrap(Box::new(move |err: JsValue| {
                let msg = js_sys::Object::from(err)
                    .to_string()
                    .as_string()
                    .unwrap_or_default();
                verr!("play() rejected: {}", msg);
            }) as Box<dyn FnMut(JsValue)>);

            let _ = promise.then2(&ok_handler, &reject_handler);
            ok_handler.forget();
            reject_handler.forget();
        }

        self.video = Some(video);
        self.capture_canvas = Some(capture);
        self.capture_ctx = Some(ctx);
        self.texture_id = Some(tex_id);
        self.width = w;
        self.height = h;
        self.active = true;
        self.logged_playing = false;
        self.logged_error = false;

        vlog!("Video player initialized, waiting for data...");

        Some(tex_id)
    }

    /// Capture the current video frame onto the offscreen canvas.
    ///
    /// Should be called once per animation frame while active.  The texture
    /// automatically reflects the new content on the next `blit()`.
    pub fn tick(&mut self) {
        if let Some(ref video) = self.video {
            let ready = video.ready_state();
            let network = video.network_state();

            // Log first frame availability.
            if ready >= 2 && !self.logged_playing {
                self.logged_playing = true;
                vlog!(
                    "First frame ready! {}x{} duration={:.1}s",
                    video.video_width(),
                    video.video_height(),
                    video.duration(),
                );
            }

            // Detect errors (networkState 3 = NETWORK_NO_SOURCE).
            if network == 3 && !self.logged_error {
                self.logged_error = true;
                verr!(
                    "NETWORK_NO_SOURCE — browser cannot load video. \
                     readyState={} networkState={}",
                    ready,
                    network,
                );
            }

            // HAVE_CURRENT_DATA (readyState >= 2) means at least one frame
            // is available for drawing.
            if ready >= 2
                && let Some(ref ctx) = self.capture_ctx
            {
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

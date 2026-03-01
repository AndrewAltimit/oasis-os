//! In-canvas video player for TV Guide playback.
//!
//! Uses a hidden `<video>` element playing a direct MP4 URL.  Each frame the
//! video content is drawn onto an offscreen `<canvas>` that is registered as a
//! backend texture.  The existing `preview_texture` rendering path in
//! `guide.rs` handles display — no iframe overlay needed.
//!
//! When the browser lacks H.264 codec support (e.g. Firefox snap, Playwright
//! Chromium), falls back to software decoding via the `oasis-video` crate:
//! symphonia (MP4 demux + AAC) and openh264 (H.264 → RGBA).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
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

// ---------------------------------------------------------------------------
// Software decode state (shared with async fetch task)
// ---------------------------------------------------------------------------

/// Phases of the software decode pipeline.
enum SoftwareState {
    /// Downloading the MP4 as a byte buffer.
    Fetching,
    /// Decoder is ready; drive frames from `tick()`.
    Decoding(Box<oasis_video::SoftwareVideoDecoder>),
    /// Fetch or init failed.
    Failed(String),
}

/// Shared state between the async fetch and the synchronous `tick()` caller.
struct SoftwareFetchShared {
    state: SoftwareState,
}

// ---------------------------------------------------------------------------
// VideoPlayer
// ---------------------------------------------------------------------------

/// Manages video playback via native `<video>` or software decode fallback.
pub struct VideoPlayer {
    // --- Native path ---
    video: Option<HtmlVideoElement>,
    // --- Software fallback ---
    software: Option<Rc<RefCell<SoftwareFetchShared>>>,
    software_logged_ready: bool,
    // --- Shared ---
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
            software: None,
            software_logged_ready: false,
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
    /// If the browser supports H.264, uses the native `<video>` element path.
    /// Otherwise falls back to software decoding via `oasis-video`.
    ///
    /// Returns the `TextureId` the caller should assign to
    /// `guide.preview_texture`.
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

        if can_play_h264() {
            vlog!("H.264 codec: supported (native path)");
            self.start_native(url, seek_secs, w, h, backend)
        } else {
            vlog!("H.264 codec: NOT supported — using software decode");
            self.start_software(url, seek_secs, w, h, backend)
        }
    }

    /// Native `<video>` element path (zero overhead, browser does all work).
    fn start_native(
        &mut self,
        url: &str,
        seek_secs: u64,
        w: u32,
        h: u32,
        backend: &mut WasmBackend,
    ) -> Option<TextureId> {
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
        let (capture, ctx, tex_id) = self.create_capture_canvas(w, h, backend)?;

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

        vlog!("Native video player initialized, waiting for data...");
        Some(tex_id)
    }

    /// Software decode fallback: fetch MP4 bytes → oasis-video decoder.
    fn start_software(
        &mut self,
        url: &str,
        seek_secs: u64,
        w: u32,
        h: u32,
        backend: &mut WasmBackend,
    ) -> Option<TextureId> {
        let (capture, ctx, tex_id) = self.create_capture_canvas(w, h, backend)?;

        // Shared state for the async fetch task.
        let shared = Rc::new(RefCell::new(SoftwareFetchShared {
            state: SoftwareState::Fetching,
        }));

        // Kick off async MP4 download.
        let shared_clone = Rc::clone(&shared);
        let url_owned = url.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch_mp4_bytes(&url_owned).await {
                Ok(bytes) => {
                    vlog!(
                        "MP4 downloaded: {} bytes. Initializing decoder...",
                        bytes.len()
                    );
                    match oasis_video::SoftwareVideoDecoder::open(bytes) {
                        Ok(mut decoder) => {
                            if seek_secs > 0
                                && let Err(e) = decoder.seek(seek_secs as f64)
                            {
                                verr!("Software seek failed: {e}");
                            }
                            let (vw, vh) = decoder.video_size();
                            let (sr, ch) = decoder.audio_format();
                            vlog!(
                                "Software decoder ready: video={}x{} audio={}Hz/{}ch",
                                vw,
                                vh,
                                sr,
                                ch
                            );
                            shared_clone.borrow_mut().state =
                                SoftwareState::Decoding(Box::new(decoder));
                        },
                        Err(e) => {
                            verr!("Software decoder init failed: {e}");
                            shared_clone.borrow_mut().state = SoftwareState::Failed(format!("{e}"));
                        },
                    }
                },
                Err(e) => {
                    verr!("MP4 fetch failed: {e}");
                    shared_clone.borrow_mut().state = SoftwareState::Failed(e);
                },
            }
        });

        self.software = Some(shared);
        self.software_logged_ready = false;
        self.capture_canvas = Some(capture);
        self.capture_ctx = Some(ctx);
        self.texture_id = Some(tex_id);
        self.width = w;
        self.height = h;
        self.active = true;
        self.logged_playing = false;
        self.logged_error = false;

        vlog!("Software video player initialized, fetching MP4...");
        Some(tex_id)
    }

    /// Create the offscreen capture canvas + 2D context + texture registration.
    fn create_capture_canvas(
        &self,
        w: u32,
        h: u32,
        backend: &mut WasmBackend,
    ) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d, TextureId)> {
        let window = web_sys::window()?;
        let document = window.document()?;

        let capture: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
        capture.set_width(w);
        capture.set_height(h);

        let ctx: CanvasRenderingContext2d = capture
            .get_context("2d")
            .ok()?
            .and_then(|c| c.dyn_into().ok())?;

        let tex_id = backend.register_canvas_as_texture(capture.clone());
        Some((capture, ctx, tex_id))
    }

    /// Capture the current video frame onto the offscreen canvas.
    ///
    /// Should be called once per animation frame while active.  The texture
    /// automatically reflects the new content on the next `blit()`.
    pub fn tick(&mut self) {
        // --- Native path ---
        if let Some(ref video) = self.video {
            let ready = video.ready_state();
            let network = video.network_state();

            if ready >= 2 && !self.logged_playing {
                self.logged_playing = true;
                vlog!(
                    "First frame ready! {}x{} duration={:.1}s",
                    video.video_width(),
                    video.video_height(),
                    video.duration(),
                );
            }

            if network == 3 && !self.logged_error {
                self.logged_error = true;
                verr!(
                    "NETWORK_NO_SOURCE — browser cannot load video. \
                     readyState={} networkState={}",
                    ready,
                    network,
                );
            }

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
            return;
        }

        // --- Software path ---
        let shared = match self.software {
            Some(ref s) => Rc::clone(s),
            None => return,
        };

        let mut borrow = shared.borrow_mut();
        match borrow.state {
            SoftwareState::Fetching => {},
            SoftwareState::Decoding(ref mut decoder) => {
                if !self.software_logged_ready {
                    self.software_logged_ready = true;
                }
                self.tick_software_frame(decoder);
            },
            SoftwareState::Failed(ref msg) => {
                if !self.logged_error {
                    self.logged_error = true;
                    verr!("Software decode failed: {}", msg);
                }
            },
        }
    }

    /// Decode and render one software video frame.
    fn tick_software_frame(&mut self, decoder: &mut oasis_video::SoftwareVideoDecoder) {
        match decoder.next_video_frame() {
            Ok(Some(frame)) => {
                if let Some(ref ctx) = self.capture_ctx {
                    let clamped = wasm_bindgen::Clamped(frame.rgba.as_slice());
                    if let Ok(image_data) = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                        clamped,
                        frame.width,
                        frame.height,
                    ) {
                        let _ = ctx.put_image_data(&image_data, 0.0, 0.0);
                    }
                }
            },
            Ok(None) => {
                // End of stream.
            },
            Err(e) => {
                if !self.logged_error {
                    self.logged_error = true;
                    verr!("Software frame decode error: {e}");
                }
            },
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
        self.software = None;
        self.software_logged_ready = false;
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

// ---------------------------------------------------------------------------
// Async MP4 fetch helper
// ---------------------------------------------------------------------------

/// Download an MP4 URL as a `Vec<u8>` using the Fetch API.
async fn fetch_mp4_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;

    let resp_val = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch error: {e:?}"))?;
    let resp: web_sys::Response = resp_val.dyn_into().map_err(|_| "response cast failed")?;

    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let buf_promise = resp
        .array_buffer()
        .map_err(|_| "arrayBuffer() failed".to_string())?;
    let buf_val = JsFuture::from(buf_promise)
        .await
        .map_err(|e| format!("arrayBuffer await: {e:?}"))?;
    let array = js_sys::Uint8Array::new(&buf_val);
    Ok(array.to_vec())
}

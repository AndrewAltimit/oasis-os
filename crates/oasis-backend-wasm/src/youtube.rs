//! Async YouTube search for the WASM build.
//!
//! Queries a list of public Invidious instances (Invidious is an
//! alternative front-end for YouTube that exposes a CORS-friendly JSON
//! API). The first instance that returns a 2xx response wins. Each result
//! includes a thumbnail URL hosted by the same Invidious instance, which
//! also serves the thumbnail with permissive CORS headers so the browser
//! can paint it onto a `<canvas>` and we can register that canvas as a
//! GPU texture.
//!
//! The fetcher itself only does step 1 (metadata). Step 2 — allocating
//! paintable textures and kicking off the per-thumbnail image loads —
//! happens in the backend `tick()` loop where we have a `&mut WasmBackend`.

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

/// Public Invidious instances to try in order. Each must:
///   * expose `/api/v1/search?q=...&type=video`,
///   * return permissive CORS headers (`Access-Control-Allow-Origin: *`),
///   * proxy thumbnails via `/vi/<id>/<quality>.jpg`.
///
/// The list is short on purpose — most public Invidious instances have
/// either gone offline or restricted CORS to their own UI domain. If
/// every entry here fails, the user sees a clear error rather than a
/// silent hang (each request is bounded by [`PER_INSTANCE_TIMEOUT_MS`]).
const INSTANCES: &[&str] = &[
    "https://invidious.f5.si",
    "https://invidious.darkness.services",
    "https://invidious.nerdvpn.de",
    "https://yewtu.be",
];

/// Hard timeout for each instance attempt (ms). Beyond this we move on
/// to the next instance — otherwise a single dead instance with a slow
/// TCP timeout stalls the whole search.
const PER_INSTANCE_TIMEOUT_MS: i32 = 6_000;

/// Maximum number of search hits to keep / display.
pub const MAX_RESULTS: usize = 18;
/// Thumbnail quality requested from Invidious. `mqdefault` is 320x180,
/// small enough to download quickly but big enough to look crisp.
const THUMB_QUALITY: &str = "mqdefault";
const THUMB_W: u32 = 320;
const THUMB_H: u32 = 180;

/// One hit from the Invidious search API, normalised for our use.
#[derive(Debug, Clone)]
pub struct YoutubeHit {
    pub video_id: String,
    pub title: String,
    pub author: String,
    pub duration: String,
    /// Absolute URL to the JPEG thumbnail (fully qualified, ready to
    /// hand to an `<img>` element).
    pub thumbnail_url: String,
}

#[derive(Deserialize)]
struct InvidiousSearchHit {
    #[serde(default)]
    r#type: String,
    #[serde(default, rename = "videoId")]
    video_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default, rename = "lengthSeconds")]
    length_seconds: i64,
}

struct State {
    hits: Vec<YoutubeHit>,
    error: Option<String>,
    done: bool,
}

/// Asynchronously searches YouTube via Invidious for the given query.
/// Synchronous code polls [`Self::is_ready`] each frame.
pub struct WasmYoutubeSearchFetcher {
    pub query: String,
    shared: Rc<RefCell<State>>,
}

impl WasmYoutubeSearchFetcher {
    /// Spawn an async search task. The task tries each instance in
    /// `INSTANCES` until one succeeds.
    pub fn new(query: &str) -> Self {
        let shared = Rc::new(RefCell::new(State {
            hits: Vec::new(),
            error: None,
            done: false,
        }));
        let shared_clone = Rc::clone(&shared);
        let query_owned = query.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            match search(&query_owned).await {
                Ok(hits) => {
                    let mut s = shared_clone.borrow_mut();
                    s.hits = hits;
                    s.done = true;
                },
                Err(e) => {
                    let mut s = shared_clone.borrow_mut();
                    s.error = Some(e);
                    s.done = true;
                },
            }
        });
        Self {
            query: query.to_string(),
            shared,
        }
    }

    /// Has the underlying fetch finished (success or failure)?
    pub fn is_ready(&self) -> bool {
        self.shared.borrow().done
    }

    /// Drain results once `is_ready()` is true.
    pub fn take_results(&self) -> Result<Vec<YoutubeHit>, String> {
        let mut s = self.shared.borrow_mut();
        if let Some(ref e) = s.error {
            return Err(e.clone());
        }
        Ok(std::mem::take(&mut s.hits))
    }
}

async fn search(query: &str) -> Result<Vec<YoutubeHit>, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let encoded = js_sys::encode_uri_component(query)
        .as_string()
        .unwrap_or_default();

    let mut last_err = String::from("no instances reachable");
    for base in INSTANCES {
        let url = format!("{base}/api/v1/search?q={encoded}&type=video");
        match try_instance(&window, base, &url).await {
            Ok(hits) if !hits.is_empty() => return Ok(hits),
            Ok(_) => {
                last_err = format!("{base}: empty response");
            },
            Err(e) => {
                last_err = format!("{base}: {e}");
            },
        }
    }
    Err(last_err)
}

async fn try_instance(
    window: &web_sys::Window,
    base: &str,
    url: &str,
) -> Result<Vec<YoutubeHit>, String> {
    let fetch_promise = window.fetch_with_str(url);
    let resp_val = race_with_timeout(window, fetch_promise, PER_INSTANCE_TIMEOUT_MS)
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    let resp: web_sys::Response = resp_val
        .dyn_into()
        .map_err(|_| "not a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text_promise = resp.text().map_err(|e| format!("text: {e:?}"))?;
    let body_val = race_with_timeout(window, text_promise, PER_INSTANCE_TIMEOUT_MS)
        .await
        .map_err(|e| format!("text: {e}"))?;
    let body: String = body_val.as_string().unwrap_or_default();
    let parsed: Vec<InvidiousSearchHit> =
        serde_json::from_str(&body).map_err(|e| format!("json: {e}"))?;
    let mut out = Vec::new();
    for hit in parsed {
        if hit.r#type != "video" || hit.video_id.is_empty() {
            continue;
        }
        out.push(YoutubeHit {
            video_id: hit.video_id.clone(),
            title: hit.title,
            author: hit.author,
            duration: format_duration(hit.length_seconds),
            thumbnail_url: format!("{base}/vi/{}/{THUMB_QUALITY}.jpg", hit.video_id),
        });
        if out.len() >= MAX_RESULTS {
            break;
        }
    }
    Ok(out)
}

/// Race a promise against `setTimeout(ms)`. Returns the resolved value on
/// success, or `"timeout"` / the underlying rejection string on failure.
///
/// Uses a unique JS object as a sentinel — the timeout `setTimeout`
/// resolves with that object; we identify it via reference equality.
async fn race_with_timeout(
    window: &web_sys::Window,
    promise: js_sys::Promise,
    ms: i32,
) -> Result<JsValue, String> {
    let sentinel: JsValue = js_sys::Object::new().into();
    let sentinel_for_cb = sentinel.clone();
    let timeout = js_sys::Promise::new(&mut |resolve, _reject| {
        let s = sentinel_for_cb.clone();
        let cb = Closure::once_into_js(move || {
            let _ = resolve.call1(&JsValue::NULL, &s);
        });
        let _ =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), ms);
    });
    let race = js_sys::Promise::race(&js_sys::Array::of2(&promise, &timeout));
    let v = JsFuture::from(race).await.map_err(|e| format!("{e:?}"))?;
    if v == sentinel {
        return Err("timeout".to_string());
    }
    Ok(v)
}

fn format_duration(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Standard width / height of the offscreen canvas we allocate per
/// thumbnail. These match `mqdefault` so the canvas matches the source
/// image and `drawImage` is a 1:1 copy.
pub fn thumb_dimensions() -> (u32, u32) {
    (THUMB_W, THUMB_H)
}

/// Start an async image load that paints the URL onto the given offscreen
/// canvas once the network and decode complete.
///
/// Caller is expected to have already registered the canvas as a texture;
/// blits before the load finishes will simply paint a transparent canvas.
pub fn paint_canvas_from_url(canvas: HtmlCanvasElement, url: &str) -> Result<(), JsValue> {
    let img = HtmlImageElement::new()?;
    img.set_cross_origin(Some("anonymous"));
    img.set_decoding("async");

    let canvas_for_load = canvas.clone();
    let img_for_load = img.clone();
    let onload = Closure::once_into_js(move || {
        let Ok(Some(ctx_obj)) = canvas_for_load.get_context("2d") else {
            return;
        };
        let Ok(ctx) = ctx_obj.dyn_into::<CanvasRenderingContext2d>() else {
            return;
        };
        let cw = canvas_for_load.width() as f64;
        let ch = canvas_for_load.height() as f64;
        ctx.set_image_smoothing_enabled(true);
        let _ =
            ctx.draw_image_with_html_image_element_and_dw_and_dh(&img_for_load, 0.0, 0.0, cw, ch);
    });
    img.set_onload(Some(onload.unchecked_ref()));

    // Onerror: silently ignore — the canvas stays transparent. The grid
    // still shows the title / author so the user can pick a video.
    let onerror = Closure::once_into_js(move || {});
    img.set_onerror(Some(onerror.unchecked_ref()));

    img.set_src(url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_zero_is_blank() {
        assert_eq!(format_duration(0), "");
        assert_eq!(format_duration(-5), "");
    }

    #[test]
    fn duration_minutes_seconds() {
        assert_eq!(format_duration(45), "0:45");
        assert_eq!(format_duration(125), "2:05");
    }

    #[test]
    fn duration_hours() {
        assert_eq!(format_duration(3725), "1:02:05");
    }
}

//! Video overlay for TV Guide playback via `<iframe>` on the IA embed player.
//!
//! Wraps `IframeOverlay` to show Internet Archive video content at either
//! PIP size (small preview in the header) or full-screen (fills content area).

use super::iframe::IframeOverlay;
use web_sys::HtmlCanvasElement;

/// Manages a video iframe overlay for Internet Archive video playback.
pub struct VideoOverlay {
    iframe: IframeOverlay,
}

impl VideoOverlay {
    /// Create a new hidden video overlay.
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self {
            iframe: IframeOverlay::new(canvas)?,
        })
    }

    /// Show video at PIP size in the given canvas-pixel region.
    pub fn show_pip(&mut self, item_id: &str, seek_secs: u64, cx: i32, cy: i32, cw: u32, ch: u32) {
        let url = embed_url(item_id, seek_secs);
        self.iframe.show(&url, cx, cy, cw, ch);
    }

    /// Show video full-screen in the given canvas-pixel region.
    pub fn show_fullscreen(
        &mut self,
        item_id: &str,
        seek_secs: u64,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
    ) {
        let url = embed_url(item_id, seek_secs);
        self.iframe.show(&url, cx, cy, cw, ch);
    }

    /// Hide the video overlay.
    pub fn hide(&mut self) {
        self.iframe.hide();
    }

    /// Whether the video overlay is currently visible.
    pub fn is_visible(&self) -> bool {
        self.iframe.is_visible()
    }

    /// Update position (for window drag/resize).
    pub fn update_position(&self, cx: i32, cy: i32, cw: u32, ch: u32) {
        self.iframe.update_position(cx, cy, cw, ch);
    }
}

/// Build IA embed player URL with optional seek position.
fn embed_url(item_id: &str, seek_secs: u64) -> String {
    if seek_secs > 0 {
        format!("https://archive.org/embed/{item_id}?start={seek_secs}")
    } else {
        format!("https://archive.org/embed/{item_id}")
    }
}

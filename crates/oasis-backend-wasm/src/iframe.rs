//! Iframe overlay for rendering real web pages via a native `<iframe>`.
//!
//! When the OASIS browser navigates to an HTTP/HTTPS URL, this module
//! overlays a real browser iframe on top of the canvas at the exact
//! position of the browser window's content area. VFS pages continue
//! using the custom renderer.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{HtmlCanvasElement, HtmlIFrameElement};

/// Manages a single `<iframe>` element overlaid on the OASIS canvas.
pub struct IframeOverlay {
    iframe: HtmlIFrameElement,
    canvas: HtmlCanvasElement,
    current_src: Option<String>,
    visible: bool,
}

impl IframeOverlay {
    /// Create a hidden iframe and append it to the canvas's parent container.
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .document()
            .ok_or_else(|| JsValue::from_str("no document"))?;

        let iframe = document
            .create_element("iframe")
            .map_err(|e| JsValue::from_str(&format!("create iframe: {e:?}")))?
            .dyn_into::<HtmlIFrameElement>()
            .map_err(|_| JsValue::from_str("element is not an iframe"))?;

        // Security: restrict what the iframe can do.
        iframe.set_attribute(
            "sandbox",
            "allow-scripts allow-same-origin allow-forms allow-popups",
        )?;

        // Accessibility: mark as presentation content.
        iframe.set_attribute("title", "Web page content")?;

        // Initial CSS: hidden, absolutely positioned over the canvas.
        let style = iframe.style();
        style.set_property("position", "absolute")?;
        style.set_property("display", "none")?;
        style.set_property("border", "none")?;
        style.set_property("z-index", "10")?;

        // Append to the canvas's parent so absolute positioning works
        // relative to the #container.
        if let Some(parent) = canvas.parent_element() {
            parent.append_child(&iframe)?;
        } else {
            document
                .body()
                .ok_or_else(|| JsValue::from_str("no body"))?
                .append_child(&iframe)?;
        }

        Ok(Self {
            iframe,
            canvas: canvas.clone(),
            current_src: None,
            visible: false,
        })
    }

    /// Show the iframe at the given canvas-pixel content area, loading `url`.
    ///
    /// `(cx, cy, cw, ch)` are in canvas pixel coordinates (the browser
    /// widget's content area, below the URL bar and above the status bar).
    pub fn show(&mut self, url: &str, cx: i32, cy: i32, cw: u32, ch: u32) {
        // Only update src if the URL actually changed.
        let needs_navigate = self.current_src.as_ref().is_none_or(|s| s != url);

        if needs_navigate {
            self.iframe.set_src(url);
            self.current_src = Some(url.to_string());
        }

        self.apply_position(cx, cy, cw, ch);

        if !self.visible {
            let _ = self.iframe.style().set_property("display", "block");
            self.visible = true;
        }
    }

    /// Hide the iframe and stop loading.
    pub fn hide(&mut self) {
        if self.visible {
            let _ = self.iframe.style().set_property("display", "none");
            self.iframe.set_src("about:blank");
            self.current_src = None;
            self.visible = false;
        }
    }

    /// Update the iframe's CSS position to track window drag/resize.
    pub fn update_position(&self, cx: i32, cy: i32, cw: u32, ch: u32) {
        if self.visible {
            self.apply_position(cx, cy, cw, ch);
        }
    }

    /// Whether the iframe is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Map canvas-pixel coordinates to CSS viewport coordinates and apply.
    fn apply_position(&self, cx: i32, cy: i32, cw: u32, ch: u32) {
        let rect = self.canvas.get_bounding_client_rect();
        let scale_x = rect.width() / self.canvas.width() as f64;
        let scale_y = rect.height() / self.canvas.height() as f64;

        let left = rect.left() + cx as f64 * scale_x;
        let top = rect.top() + cy as f64 * scale_y;
        let width = cw as f64 * scale_x;
        let height = ch as f64 * scale_y;

        let style = self.iframe.style();
        let _ = style.set_property("left", &format!("{left:.1}px"));
        let _ = style.set_property("top", &format!("{top:.1}px"));
        let _ = style.set_property("width", &format!("{width:.1}px"));
        let _ = style.set_property("height", &format!("{height:.1}px"));
    }
}

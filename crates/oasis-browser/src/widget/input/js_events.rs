//! Dispatching DOM events (click, mouse, key) into page scripts.

use super::*;

impl BrowserWidget {
    /// Dispatch a JS click event using the layout tree hit test.
    /// Returns `true` if the JS handler called `event.preventDefault()`
    /// (or an inline `onclick` returned `false`) — the caller should
    /// then skip default behavior like following `<a href>` links.
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_click(&mut self, x: i32, y: i32) -> bool {
        let (lx, ly) = self.screen_to_layout(x, y);
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(lx, ly));
        if let (Some(nid), Some(engine)) = (node_id, &self.js_engine) {
            Self::dispatch_js_event_prevent(engine, nid, "click")
        } else {
            false
        }
    }

    /// Dispatch a named event to JS with bubbling.
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_event(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        event_type: &str,
    ) {
        let _ = Self::dispatch_js_event_prevent(engine, node_id, event_type);
    }

    /// Dispatch and return whether the default action was prevented.
    ///
    /// Resolves the pre-compiled `__oasis_dispatch_click_fast` helper
    /// from globals and calls it with typed args, which is measurably
    /// cheaper than `format!`-ing a JS source string and handing it to
    /// `engine.eval()` (every call re-parsed + re-compiled the snippet).
    /// The helper is installed once by the DOM bootstrap JS; if it's
    /// missing (engine not fully initialised) we fall back to `false`.
    ///
    /// All event dispatch goes through `JsEngine::with_context_guarded`,
    /// so a runaway handler (`onclick="while(1){}"`) is interrupted after
    /// the engine's execution budget instead of freezing the OS.
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_event_prevent(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        event_type: &str,
    ) -> bool {
        use oasis_js::rquickjs;
        engine
            .with_context_guarded(|ctx| -> rquickjs::Result<bool> {
                let globals = ctx.globals();
                let Ok(dispatch): rquickjs::Result<rquickjs::Function> =
                    globals.get("__oasis_dispatch_click_fast")
                else {
                    return Ok(false);
                };
                let ret: bool = dispatch.call((node_id as i32, event_type))?;
                Ok(ret)
            })
            .unwrap_or(false)
    }

    /// Dispatch a mouse event (mousedown, mouseup, etc.) to the node at
    /// the given coordinates, passing `clientX`/`clientY` as detail.
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_mouse_event(&mut self, x: i32, y: i32, event_type: &str) {
        let (lx, ly) = self.screen_to_layout(x, y);
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(lx, ly));
        if let (Some(nid), Some(engine)) = (node_id, &self.js_engine) {
            Self::dispatch_js_mouse_event_to(engine, nid, x, y, event_type);
        }
    }

    /// Dispatch a mouse event to a specific node with coordinates.
    ///
    /// Same pre-compiled-helper pattern as `dispatch_js_event_prevent`
    /// — avoids `format!` + `engine.eval()` per event on the mousemove
    /// hot path (hover-listener-heavy pages).
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_mouse_event_to(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        x: i32,
        y: i32,
        event_type: &str,
    ) {
        use oasis_js::rquickjs;
        let _ = engine.with_context_guarded(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();
            if let Ok(dispatch) =
                globals.get::<_, rquickjs::Function>("__oasis_dispatch_mouse_fast")
            {
                dispatch.call::<_, rquickjs::Value>((node_id as i32, event_type, x, y))?;
            }
            Ok(())
        });
    }

    /// Dispatch a keydown event to JS with key info as detail object.
    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_key_event(engine: &oasis_js::JsEngine, node_id: NodeId, key: char) {
        Self::dispatch_js_key_event_typed(engine, node_id, key, "keydown");
    }

    #[cfg(feature = "javascript")]
    pub(super) fn dispatch_js_key_event_typed(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        key: char,
        event_type: &str,
    ) {
        // Pre-compiled helper call — arguments pass through typed FFI,
        // so we no longer need to escape characters that would break a
        // single-quoted JS string literal (`'`, `\`, newline, CR).
        use oasis_js::rquickjs;
        let key_str = key.to_string();
        let _ = engine.with_context_guarded(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();
            if let Ok(dispatch) = globals.get::<_, rquickjs::Function>("__oasis_dispatch_key_fast")
            {
                dispatch.call::<_, rquickjs::Value>((
                    node_id as i32,
                    event_type,
                    key_str.as_str(),
                    key_str.as_str(),
                ))?;
            }
            Ok(())
        });
    }
}

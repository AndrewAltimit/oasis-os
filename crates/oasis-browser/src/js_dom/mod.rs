//! DOM bindings for JavaScript via rquickjs.
//!
//! This module lives in `oasis-browser` (not `oasis-js`) so that it can
//! directly access `Document` and other DOM types without creating a
//! circular dependency.  It uses `oasis_js::rquickjs` for the JS FFI
//! types.
//!
//! ## Design
//!
//! Rust-side functions (`__oasis_*`) operate on node-id integers and
//! return only primitives (`String`, `i32`, `Vec<i32>`).  This avoids
//! rquickjs lifetime issues with closures returning `Object<'js>`.
//!
//! A JavaScript snippet (installed via `ctx.eval`) defines the
//! `Element` constructor and `document` global, bridging the low-level
//! Rust helpers into the familiar DOM API.
//!
//! ## Layout
//!
//! - [`bindings`] -- DOM node, navigation and computed-style bindings
//! - [`fetch`] -- origin-aware `FetchHandler` behind `oasis-js`'s
//!   Promise-based `fetch()` (URL resolution, TLS, CSP `connect-src`,
//!   same-origin / private-network policy)
//! - [`storage`] -- per-origin, quota'd `localStorage` / `sessionStorage`
//!   and `document.cookie`
//! - [`node_api`] -- node types, text-inclusive child lists, detached-safe
//!   insertion, cloning, fragment parsing, `matches()` / `getElementsBy*`
//! - [`serialize`] -- `innerHTML` serialization and fragment deep-copy
//! - [`compat_shims`] -- site-compat helpers for inline `onclick` code
//! - `canvas` -- `<canvas>` 2D context bindings (feature `canvas`)
//! - `bootstrap.js` / `compat_shims.js` / `canvas.js` -- the JS halves,
//!   embedded with `include_str!`.

mod bindings;
#[cfg(feature = "canvas")]
mod canvas;
mod compat_shims;
mod fetch;
mod node_api;
mod serialize;
mod storage;

#[cfg(test)]
mod tests;

#[cfg(feature = "canvas")]
pub use canvas::install_canvas_bindings;
pub(crate) use compat_shims::install_site_compat_shims;
pub use storage::WebStorage;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use oasis_js::rquickjs::{Ctx, Result as JsResult};
use oasis_net::tls::TlsProvider;

use crate::css::values::ComputedStyle;
#[cfg(test)]
use crate::html::dom::ElementData;
use crate::html::dom::{Document, NodeKind, TagName};

/// Shared, interior-mutable document used during JS execution.
pub type SharedDoc = Rc<RefCell<Document>>;

/// Shared, interior-mutable computed styles for `getComputedStyle()`.
pub type SharedStyles = Rc<RefCell<Vec<Option<ComputedStyle>>>>;

/// Shared, interior-mutable per-origin `localStorage` / `sessionStorage`
/// backing store that persists across page navigations within the same
/// `BrowserWidget` lifetime.
pub type SharedLocalStorage = Rc<RefCell<WebStorage>>;

/// Shared flag set by DOM-mutating JS bindings (setAttribute, classList,
/// style, appendChild, innerHTML, etc.) so the widget can re-run the
/// CSS cascade and layout after an event handler mutates the page.
/// `None` during test contexts that don't care about relayout.
pub type SharedDirty = Rc<Cell<bool>>;

#[inline]
fn mark_dirty(flag: &Option<SharedDirty>) {
    if let Some(d) = flag {
        d.set(true);
    }
}

/// Sentinel returned when a DOM lookup produces no result.
const NO_NODE: i32 = -1;

// ------------------------------------------------------------------
// Navigation action queue (JS -> browser widget)
// ------------------------------------------------------------------

/// A navigation action requested by JavaScript code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsNavAction {
    /// Navigate to a new URL (`location.assign(url)` or
    /// `location.href = url`).
    Navigate(String),
    /// Go back in history (`history.back()`).
    Back,
    /// Go forward in history (`history.forward()`).
    Forward,
}

/// Shared queue of pending navigation actions produced by JS.
pub type SharedNavActions = Rc<RefCell<Vec<JsNavAction>>>;

/// Install the `document` global and `Element` prototype into the JS
/// context, backed by the given shared `Document`.
///
/// `url` is exposed as `window.location.href`. Pass an empty string
/// or the page URL.
///
/// Navigation actions (location.assign, history.back/forward) are
/// silently discarded. Use [`install_document_global_with_nav`] to
/// capture them.
#[cfg(test)]
pub fn install_document_global(ctx: &Ctx<'_>, doc: &SharedDoc) -> JsResult<()> {
    let nav = Rc::new(RefCell::new(Vec::new()));
    install_document_global_full(ctx, doc, "", &nav, None, None, None, None, None)
}

/// Like [`install_document_global`] but accepts an explicit URL for
/// `window.location`.
#[cfg(test)]
pub fn install_document_global_with_url(ctx: &Ctx<'_>, doc: &SharedDoc, url: &str) -> JsResult<()> {
    let nav = Rc::new(RefCell::new(Vec::new()));
    install_document_global_full(ctx, doc, url, &nav, None, None, None, None, None)
}

/// Like [`install_document_global_with_url`] but also accepts a shared
/// navigation action queue that JS `location.assign()` /
/// `history.back()` / `history.forward()` will push to.
#[allow(dead_code)]
pub fn install_document_global_with_nav(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
) -> JsResult<()> {
    install_document_global_full(ctx, doc, url, nav_actions, None, None, None, None, None)
}

/// Like [`install_document_global_with_nav`] but also accepts an
/// optional CSP policy to enforce `connect-src` on `fetch()` calls,
/// the TLS provider `fetch()` uses for `https:` URLs, and an optional
/// persistent per-origin storage backing store.
#[allow(clippy::too_many_arguments)]
pub fn install_document_global_with_csp(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
    styles: &SharedStyles,
    csp: Option<&crate::loader::csp::CspPolicy>,
    tls: Option<Arc<dyn TlsProvider>>,
    persistent_local_storage: Option<&SharedLocalStorage>,
    dom_dirty: Option<&SharedDirty>,
) -> JsResult<()> {
    install_document_global_full(
        ctx,
        doc,
        url,
        nav_actions,
        Some(styles),
        csp,
        tls,
        persistent_local_storage,
        dom_dirty,
    )
}

/// Full installation: document global, location/history, nav actions,
/// computed styles, fetch, localStorage/sessionStorage, document.cookie.
#[allow(clippy::too_many_arguments)]
fn install_document_global_full(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
    styles: Option<&SharedStyles>,
    csp: Option<&crate::loader::csp::CspPolicy>,
    tls: Option<Arc<dyn TlsProvider>>,
    persistent_local_storage: Option<&SharedLocalStorage>,
    dom_dirty: Option<&SharedDirty>,
) -> JsResult<()> {
    // Local clone stored per binding closure so each `move` capture owns
    // its own handle. `Option<Rc<Cell<bool>>>` is cheap to clone.
    let dirty = dom_dirty.map(Rc::clone);

    let freed: node_api::FreedLog = Rc::new(RefCell::new(Vec::new()));
    bindings::install_dom_bindings(ctx, doc, &dirty, &freed)?;
    node_api::install_node_bindings(ctx, doc, &dirty, &freed)?;
    bindings::install_nav_bindings(ctx, nav_actions)?;
    fetch::install_fetch_binding(ctx, url, csp, tls)?;
    bindings::install_computed_style_binding(ctx, styles)?;
    bindings::install_location_bindings(ctx, url)?;
    storage::install_cookie_bindings(ctx)?;
    storage::install_storage_bindings(ctx, url, persistent_local_storage)?;

    // -- JavaScript-side Element class + document global ---------------
    let _: () = ctx.eval(JS_DOM_BOOTSTRAP)?;

    Ok(())
}

/// JavaScript code that defines the `Element` wrapper and `document`
/// global using the `__oasis_*` Rust-backed helper functions.
const JS_DOM_BOOTSTRAP: &str = include_str!("bootstrap.js");

/// Drain and return all pending navigation actions from the queue.
pub fn drain_nav_actions(nav: &SharedNavActions) -> Vec<JsNavAction> {
    std::mem::take(&mut nav.borrow_mut())
}

/// Run the page lifecycle once parser-inserted scripts, site shims and
/// inline handlers are in place: `document.readyState` goes
/// `interactive` -> `complete`, firing `readystatechange`,
/// `DOMContentLoaded` (bubbling to `window`) and then `window` `load`.
pub fn fire_document_lifecycle(engine: &oasis_js::JsEngine) {
    let _ =
        engine.eval("if (typeof __oasis_fire_lifecycle === 'function') __oasis_fire_lifecycle();");
}

/// Inline event handler attribute names and the corresponding DOM
/// event type.
const INLINE_HANDLERS: &[(&str, &str)] = &[
    ("onclick", "click"),
    ("onchange", "change"),
    ("onsubmit", "submit"),
    ("onmouseover", "mouseover"),
    ("onmouseout", "mouseout"),
    ("onkeydown", "keydown"),
    ("oninput", "input"),
    ("onload", "load"),
];

/// Walk the DOM and register inline event handler attributes
/// (e.g. `onclick="..."`) as `addEventListener` calls on the JS side.
///
/// Call this after `engine.eval_all()` in `load_html()` so that inline
/// handlers declared in the HTML source are wired up.
pub fn register_inline_handlers(engine: &oasis_js::JsEngine, doc: &Document) {
    for (id, node) in doc.nodes.iter().enumerate() {
        if let NodeKind::Element(elem) = &node.kind {
            for &(attr_name, event_type) in INLINE_HANDLERS {
                if let Some(handler_body) = elem.get_attribute(attr_name) {
                    // `<body onload>` is a window `load` handler.
                    let target = if elem.tag == TagName::Body && event_type == "load" {
                        "window".to_string()
                    } else {
                        format!("new Element({id})")
                    };
                    // Wrap the handler so `return false` / a falsy
                    // return calls `event.preventDefault()`, matching
                    // the HTML spec. Without this, reddit-style
                    // `onclick="return togglecomment(this)"` would
                    // toggle the class then navigate to `#` anyway.
                    let js = format!(
                        "(function(){{ var el = {target}; \
                         el.addEventListener(\"{event_type}\", \
                         function(event) {{ \
                           var __r = (function(){{ {handler_body} }}).call(el); \
                           if (__r === false && event && event.preventDefault) \
                             event.preventDefault(); \
                           return __r; \
                         }}); }})()"
                    );
                    let _ = engine.eval(&js);
                }
            }
        }
    }
}

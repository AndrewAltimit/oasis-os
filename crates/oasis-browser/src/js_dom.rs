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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use crate::css::values::ComputedStyle;
use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};

/// Shared, interior-mutable document used during JS execution.
pub type SharedDoc = Rc<RefCell<Document>>;

/// Shared, interior-mutable computed styles for `getComputedStyle()`.
pub type SharedStyles = Rc<RefCell<Vec<Option<ComputedStyle>>>>;

/// Shared, interior-mutable localStorage backing store that persists
/// across page navigations within the same `BrowserWidget` lifetime.
pub type SharedLocalStorage = Rc<RefCell<HashMap<String, String>>>;

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
    install_document_global_full(ctx, doc, "", &nav, None, None, None, None)
}

/// Like [`install_document_global`] but accepts an explicit URL for
/// `window.location`.
#[cfg(test)]
pub fn install_document_global_with_url(ctx: &Ctx<'_>, doc: &SharedDoc, url: &str) -> JsResult<()> {
    let nav = Rc::new(RefCell::new(Vec::new()));
    install_document_global_full(ctx, doc, url, &nav, None, None, None, None)
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
    install_document_global_full(ctx, doc, url, nav_actions, None, None, None, None)
}

/// Like [`install_document_global_with_nav`] but also accepts an
/// optional CSP policy to enforce `connect-src` on `fetch()` calls,
/// and an optional persistent localStorage backing store.
#[allow(clippy::too_many_arguments)]
pub fn install_document_global_with_csp(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
    styles: &SharedStyles,
    csp: Option<&crate::loader::csp::CspPolicy>,
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
    persistent_local_storage: Option<&SharedLocalStorage>,
    dom_dirty: Option<&SharedDirty>,
) -> JsResult<()> {
    // Local clone stored per binding closure so each `move` capture owns
    // its own handle. `Option<Rc<Cell<bool>>>` is cheap to clone.
    let dirty = dom_dirty.map(Rc::clone);
    let globals = ctx.globals();

    // -- __oasis_tagname(nid) -> String --------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_tagname",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                match &doc.nodes[id].kind {
                    NodeKind::Element(e) => e.tag.as_str().to_ascii_uppercase(),
                    NodeKind::Text(_) => "#text".into(),
                    _ => String::new(),
                }
            })?,
        )?;
    }

    // -- __oasis_getattr(nid, name) -> String|"" ----------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_getattr",
            Function::new(
                ctx.clone(),
                move |nid: i32, name: String| -> Option<String> {
                    let doc = d.borrow();
                    let id = nid as NodeId;
                    if id >= doc.nodes.len() {
                        return None;
                    }
                    match &doc.nodes[id].kind {
                        NodeKind::Element(e) => e.get_attribute(&name).map(String::from),
                        _ => None,
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_setattr(nid, name, value) ----------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_setattr",
            Function::new(ctx.clone(), move |nid: i32, name: String, value: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    // Skip dirty mark when value is identical — pages
                    // that re-assert the same `aria-expanded="true"`
                    // every frame shouldn't trigger a relayout.
                    let unchanged = e.get_attribute(&name) == Some(value.as_str());
                    // Update the ID index when the `id` attribute changes.
                    if name == "id" {
                        let old_id = e.id().map(String::from);
                        e.set_attribute(&name, &value);
                        doc.update_id_index(id, old_id.as_deref(), Some(&value));
                    } else {
                        e.set_attribute(&name, &value);
                    }
                    if !unchanged {
                        mark_dirty(&dirty);
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_rmattr(nid, name) -> bool ----------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_rmattr",
            Function::new(ctx.clone(), move |nid: i32, name: String| -> bool {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    // Update the ID index when the `id` attribute is removed.
                    if name == "id" {
                        let old_id = e.id().map(String::from);
                        let removed = e.remove_attribute(&name);
                        if removed {
                            doc.update_id_index(id, old_id.as_deref(), None);
                            mark_dirty(&dirty);
                        }
                        return removed;
                    }
                    let removed = e.remove_attribute(&name);
                    if removed {
                        mark_dirty(&dirty);
                    }
                    return removed;
                }
                false
            })?,
        )?;
    }

    // -- __oasis_text(nid) -> String ----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_text",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                doc.text_content(id)
            })?,
        )?;
    }

    // -- __oasis_settext(nid, text) -----------------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_settext",
            Function::new(ctx.clone(), move |nid: i32, text: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len() {
                    doc.set_text_content(id, &text);
                    mark_dirty(&dirty);
                }
            })?,
        )?;
    }

    // -- __oasis_children(nid) -> Vec<i32> ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_children",
            Function::new(ctx.clone(), move |nid: i32| -> Vec<i32> {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return Vec::new();
                }
                doc.nodes[id]
                    .children
                    .iter()
                    .copied()
                    .filter(|&cid| matches!(doc.nodes[cid].kind, NodeKind::Element(_)))
                    .map(|cid| cid as i32)
                    .collect()
            })?,
        )?;
    }

    // -- __oasis_parent(nid) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_parent",
            Function::new(ctx.clone(), move |nid: i32| -> i32 {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return NO_NODE;
                }
                doc.nodes[id]
                    .parent
                    .filter(|&pid| matches!(doc.nodes[pid].kind, NodeKind::Element(_)))
                    .map_or(NO_NODE, |pid| pid as i32)
            })?,
        )?;
    }

    // -- __oasis_getbyid(id) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_getbyid",
            Function::new(ctx.clone(), move |id: String| -> i32 {
                d.borrow()
                    .get_element_by_id(&id)
                    .map_or(NO_NODE, |nid| nid as i32)
            })?,
        )?;
    }

    // -- __oasis_create(tag) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_create",
            Function::new(ctx.clone(), move |tag: String| -> i32 {
                let mut doc = d.borrow_mut();
                let data = ElementData::new(TagName::from_str(&tag));
                doc.add_node(NodeKind::Element(data)) as i32
            })?,
        )?;
    }

    // -- __oasis_createtext(text) -> i32 ------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_createtext",
            Function::new(ctx.clone(), move |text: String| -> i32 {
                let mut doc = d.borrow_mut();
                doc.add_node(NodeKind::Text(text)) as i32
            })?,
        )?;
    }

    // -- __oasis_append(parent_nid, child_nid) ------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_append",
            Function::new(ctx.clone(), {
                let dirty = dirty.clone();
                move |parent_nid: i32, child_nid: i32| {
                    let mut doc = d.borrow_mut();
                    let pid = parent_nid as NodeId;
                    let cid = child_nid as NodeId;
                    if pid < doc.nodes.len() && cid < doc.nodes.len() {
                        doc.remove_child(cid);
                        doc.append_child(pid, cid);
                        mark_dirty(&dirty);
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_remove(child_nid) -> i32 (former parent or -1) --------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_remove",
            Function::new(ctx.clone(), move |child_nid: i32| -> i32 {
                let mut doc = d.borrow_mut();
                let cid = child_nid as NodeId;
                if cid >= doc.nodes.len() {
                    return NO_NODE;
                }
                let res = doc.remove_child(cid);
                if res.is_some() {
                    mark_dirty(&dirty);
                }
                res.map_or(NO_NODE, |pid| pid as i32)
            })?,
        )?;
    }

    // -- __oasis_insertbefore(parent_nid, new_nid, ref_nid) -----------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_insertbefore",
            Function::new(
                ctx.clone(),
                move |parent_nid: i32, new_nid: i32, ref_nid: i32| {
                    let mut doc = d.borrow_mut();
                    let pid = parent_nid as NodeId;
                    let nid = new_nid as NodeId;
                    if pid >= doc.nodes.len() || nid >= doc.nodes.len() {
                        return;
                    }
                    // Remove new_nid from its current parent first.
                    doc.remove_child(nid);
                    // Find the position of ref_nid in parent's children.
                    let pos = if ref_nid >= 0 {
                        let rid = ref_nid as NodeId;
                        doc.nodes[pid].children.iter().position(|&c| c == rid)
                    } else {
                        None
                    };
                    match pos {
                        Some(idx) => {
                            doc.nodes[pid].children.insert(idx, nid);
                            doc.nodes[nid].parent = Some(pid);
                        },
                        None => {
                            // ref_nid not found or -1: append.
                            doc.append_child(pid, nid);
                        },
                    }
                    mark_dirty(&dirty);
                },
            )?,
        )?;
    }

    // -- __oasis_body() -> i32 ----------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_body",
            Function::new(ctx.clone(), move || -> i32 {
                d.borrow().body().map_or(NO_NODE, |nid| nid as i32)
            })?,
        )?;
    }

    // -- __oasis_title() -> String ------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_title",
            Function::new(ctx.clone(), move || -> String {
                d.borrow().title().unwrap_or_default()
            })?,
        )?;
    }

    // -- __oasis_settitle(text) ---------------------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_settitle",
            Function::new(ctx.clone(), move |val: String| {
                let mut doc = d.borrow_mut();
                if let Some(tid) = doc.title_element() {
                    doc.set_text_content(tid, &val);
                    mark_dirty(&dirty);
                }
            })?,
        )?;
    }

    // -- __oasis_inner_html(nid) -> String -----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_inner_html",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                let mut out = String::new();
                for &child in &doc.nodes[id].children {
                    serialize_node(&doc, child, &mut out);
                }
                out
            })?,
        )?;
    }

    // -- __oasis_set_inner_html(nid, html) ----------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_set_inner_html",
            Function::new(ctx.clone(), move |nid: i32, html: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return;
                }
                // Serialize existing children so we can detect no-op writes
                // (e.g. animation loops setting innerHTML to the same string
                // each frame). Mirrors the guard pattern in
                // `classlist_op_mutated` / `__oasis_setattr`.
                let mut old_html = String::new();
                for &child in &doc.nodes[id].children {
                    serialize_node(&doc, child, &mut old_html);
                }
                // Recursively free existing children and all descendants
                // (ID index entries, arena slots).
                let old: Vec<NodeId> = doc.nodes[id].children.clone();
                for child_id in old {
                    doc.free_subtree(child_id);
                }
                doc.nodes[id].children.clear();

                // Parse and transplant unconditionally — the before/after
                // serialize comparison below uses the post-transplant state
                // to detect no-ops. The DOM is always updated to the parsed
                // result; only mark_dirty (cascade + relayout) is skipped
                // when old_html == new_html.
                use crate::html::tokenizer::Tokenizer;
                use crate::html::tree_builder::TreeBuilder;
                let wrapped = format!("<html><body>{html}</body></html>");
                let tokens = Tokenizer::new(&wrapped).tokenize();
                let frag = TreeBuilder::build(tokens);
                // Collect body children from fragment.
                let body_id = frag.body();
                let src_children: Vec<NodeId> = body_id
                    .map(|b| frag.nodes[b].children.clone())
                    .unwrap_or_default();
                // Deep-copy nodes into the live document.
                for &src_child in &src_children {
                    let new_id = deep_copy_node(&frag, &mut doc, src_child);
                    doc.append_child(id, new_id);
                }

                // Re-serialize the freshly-transplanted subtree and compare
                // against the pre-mutation serialization. Only trigger a
                // cascade/relayout when the effective DOM actually changed.
                let mut new_html = String::new();
                for &child in &doc.nodes[id].children {
                    serialize_node(&doc, child, &mut new_html);
                }
                if old_html != new_html {
                    mark_dirty(&dirty);
                }
            })?,
        )?;
    }

    // -- __oasis_query_selector(nid, sel) -> i32 ----------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_query_selector",
            Function::new(ctx.clone(), move |nid: i32, sel: String| -> i32 {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return NO_NODE;
                }
                let Some(parsed) = crate::css::parser::parse_selector_string(&sel) else {
                    return NO_NODE;
                };
                find_matching(&doc, id, &parsed, true)
                    .into_iter()
                    .next()
                    .map_or(NO_NODE, |n| n as i32)
            })?,
        )?;
    }

    // -- __oasis_query_selector_all(nid, sel) -> Vec<i32> -------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_query_selector_all",
            Function::new(ctx.clone(), move |nid: i32, sel: String| -> Vec<i32> {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return Vec::new();
                }
                let Some(parsed) = crate::css::parser::parse_selector_string(&sel) else {
                    return Vec::new();
                };
                find_matching(&doc, id, &parsed, false)
                    .into_iter()
                    .map(|n| n as i32)
                    .collect()
            })?,
        )?;
    }

    // -- __oasis_classlist_op(nid, op, cls) -> bool -------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_classlist_op",
            Function::new(
                ctx.clone(),
                move |nid: i32, op: String, cls: String| -> bool {
                    let mut doc = d.borrow_mut();
                    let id = nid as NodeId;
                    if id >= doc.nodes.len() {
                        return false;
                    }
                    let e = match &mut doc.nodes[id].kind {
                        NodeKind::Element(e) => e,
                        _ => return false,
                    };
                    let (js_ret, mutated) = classlist_op_mutated(e, &op, &cls);
                    if mutated {
                        mark_dirty(&dirty);
                    }
                    js_ret
                },
            )?,
        )?;
    }

    // -- __oasis_style_set(nid, prop, value) --------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_style_set",
            Function::new(ctx.clone(), move |nid: i32, prop: String, value: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return;
                }
                let e = match &mut doc.nodes[id].kind {
                    NodeKind::Element(e) => e,
                    _ => return,
                };
                if set_inline_style(e, &prop, &value) {
                    mark_dirty(&dirty);
                }
            })?,
        )?;
    }

    // -- __oasis_style_get(nid, prop) -> String -----------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_style_get",
            Function::new(ctx.clone(), move |nid: i32, prop: String| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                let e = match &doc.nodes[id].kind {
                    NodeKind::Element(e) => e,
                    _ => return String::new(),
                };
                get_inline_style(e, &prop)
            })?,
        )?;
    }

    // -- __oasis_location_assign(url) ---------------------------------
    {
        let nav = Rc::clone(nav_actions);
        globals.set(
            "__oasis_location_assign",
            Function::new(ctx.clone(), move |url: String| {
                nav.borrow_mut().push(JsNavAction::Navigate(url));
            })?,
        )?;
    }

    // -- __oasis_history_back() ---------------------------------------
    {
        let nav = Rc::clone(nav_actions);
        globals.set(
            "__oasis_history_back",
            Function::new(ctx.clone(), move || {
                nav.borrow_mut().push(JsNavAction::Back);
            })?,
        )?;
    }

    // -- __oasis_history_forward() ------------------------------------
    {
        let nav = Rc::clone(nav_actions);
        globals.set(
            "__oasis_history_forward",
            Function::new(ctx.clone(), move || {
                nav.borrow_mut().push(JsNavAction::Forward);
            })?,
        )?;
    }

    // -- __oasis_fetch(method, url, body) -> String ----------------------
    {
        let fetch_csp = csp.cloned();
        let fetch_page_url = url.to_string();
        globals.set(
            "__oasis_fetch",
            Function::new(
                ctx.clone(),
                move |method: String, url_str: String, body_str: String| -> String {
                    // Enforce CSP connect-src before making the request.
                    if let Some(ref policy) = fetch_csp
                        && policy.is_active()
                        && !policy.allows(
                            &url_str,
                            &fetch_page_url,
                            crate::loader::csp::CspResourceType::Connect,
                        )
                    {
                        return String::new();
                    }
                    let body_bytes = if body_str.is_empty() {
                        None
                    } else {
                        Some(body_str.as_bytes())
                    };
                    match crate::loader::Url::parse(&url_str) {
                        Some(parsed_url) => {
                            match crate::loader::http::http_request(
                                &method,
                                &parsed_url,
                                body_bytes,
                                &[],
                                None,
                            ) {
                                Ok(resp) => String::from_utf8_lossy(&resp.body).into_owned(),
                                Err(_) => String::new(),
                            }
                        },
                        None => String::new(),
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_computed_style(nid, prop) -> String ---------------------
    {
        let styles_ref: SharedStyles = match styles {
            Some(s) => Rc::clone(s),
            None => Rc::new(RefCell::new(Vec::new())),
        };
        globals.set(
            "__oasis_computed_style",
            Function::new(ctx.clone(), move |nid: i32, prop: String| -> String {
                let styles_borrow: std::cell::Ref<'_, Vec<Option<ComputedStyle>>> =
                    styles_ref.borrow();
                let id = nid as NodeId;
                if id < styles_borrow.len()
                    && let Some(ref style) = styles_borrow[id]
                {
                    return style.get_property_value(&prop);
                }
                String::new()
            })?,
        )?;
    }

    // -- __oasis_location_push(url) -- for history.pushState URL updates --
    {
        let pushed_url: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let pu1 = Rc::clone(&pushed_url);
        globals.set(
            "__oasis_location_push",
            Function::new(ctx.clone(), move |url: String| {
                *pu1.borrow_mut() = Some(url);
            })?,
        )?;
        // Also override __oasis_location to return the pushed URL if set.
        let pu2 = Rc::clone(&pushed_url);
        let orig_url = url.to_string();
        globals.set(
            "__oasis_location",
            Function::new(ctx.clone(), move || -> String {
                pu2.borrow().clone().unwrap_or_else(|| orig_url.clone())
            })?,
        )?;
    }

    // -- __oasis_cookie_get() / __oasis_cookie_set(raw) ------------------
    {
        let cookie_map: Rc<RefCell<HashMap<String, String>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let cm1 = Rc::clone(&cookie_map);
        globals.set(
            "__oasis_cookie_get",
            Function::new(ctx.clone(), move || -> String {
                let map = cm1.borrow();
                let mut pairs: Vec<String> = map.iter().map(|(k, v)| format!("{k}={v}")).collect();
                pairs.sort();
                pairs.join("; ")
            })?,
        )?;
        let cm2 = Rc::clone(&cookie_map);
        globals.set(
            "__oasis_cookie_set",
            Function::new(ctx.clone(), move |raw: String| {
                // Parse "name=value; path=/; ..." — only extract the first name=value.
                if let Some(pair) = raw.split(';').next() {
                    let pair = pair.trim();
                    if let Some(eq) = pair.find('=') {
                        let name = pair[..eq].trim().to_string();
                        let value = pair[eq + 1..].trim().to_string();
                        if !name.is_empty() {
                            cm2.borrow_mut().insert(name, value);
                        }
                    }
                }
            })?,
        )?;
    }

    // -- localStorage / sessionStorage -----------------------------------
    // localStorage uses a persistent backing store (shared across page
    // navigations) when provided, otherwise falls back to a fresh map.
    // sessionStorage always uses a fresh map (page-scoped).
    {
        let local_store = match persistent_local_storage {
            Some(store) => Rc::clone(store),
            None => Rc::new(RefCell::new(HashMap::<String, String>::new())),
        };
        let session_store = Rc::new(RefCell::new(HashMap::<String, String>::new()));

        let l1 = Rc::clone(&local_store);
        let ss1 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_get",
            Function::new(ctx.clone(), move |kind: i32, key: String| -> String {
                let store = if kind == 0 { &l1 } else { &ss1 };
                store.borrow().get(&key).cloned().unwrap_or_default()
            })?,
        )?;
        let l2 = Rc::clone(&local_store);
        let ss2 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_set",
            Function::new(ctx.clone(), move |kind: i32, key: String, value: String| {
                let store = if kind == 0 { &l2 } else { &ss2 };
                store.borrow_mut().insert(key, value);
            })?,
        )?;
        let l3 = Rc::clone(&local_store);
        let ss3 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_remove",
            Function::new(ctx.clone(), move |kind: i32, key: String| {
                let store = if kind == 0 { &l3 } else { &ss3 };
                store.borrow_mut().remove(&key);
            })?,
        )?;
        let l4 = Rc::clone(&local_store);
        let ss4 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_clear",
            Function::new(ctx.clone(), move |kind: i32| {
                let store = if kind == 0 { &l4 } else { &ss4 };
                store.borrow_mut().clear();
            })?,
        )?;
        let l5 = Rc::clone(&local_store);
        let ss5 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_length",
            Function::new(ctx.clone(), move |kind: i32| -> i32 {
                let store = if kind == 0 { &l5 } else { &ss5 };
                store.borrow().len() as i32
            })?,
        )?;
    }

    // -- JavaScript-side Element class + document global ---------------
    let _: () = ctx.eval(JS_DOM_BOOTSTRAP)?;

    Ok(())
}

/// Drain and return all pending navigation actions from the queue.
pub fn drain_nav_actions(nav: &SharedNavActions) -> Vec<JsNavAction> {
    std::mem::take(&mut nav.borrow_mut())
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

/// Install compatibility shims for common site-specific helpers that
/// inline `onclick="..."` handlers call. Registered after `engine.eval_all()`
/// so that a page's own `togglecomment`/`hidecomment` definitions (if they
/// parse successfully) take precedence. When the page's script bundle
/// doesn't load — old.reddit.com ships a ~1 MB bundle built around feature
/// detection and jQuery — these shims let the page stay interactive.
///
/// The reddit shims all walk from the clicked element up to the nearest
/// `.comment` ancestor and toggle a `collapsed` class; the fixture's CSS
/// (and the real site's sheet) already hides `.comment.collapsed .child`
/// and friends, so toggling one class is enough to collapse/expand a
/// thread and its replies. Returning `false` from the onclick suppresses
/// the default link navigation.
pub(crate) fn install_site_compat_shims(engine: &oasis_js::JsEngine) {
    // Keep the JS small; each helper is a one-liner wrapped in
    // `if typeof ... === 'undefined'` so a real site script wins.
    let shim = r#"
(function(){
  function climb(el, pred) {
    while (el) {
      if (pred(el)) return el;
      el = el.parentNode;
    }
    return null;
  }
  function nearestComment(el) {
    return climb(el, function(n){
      return n.classList && n.classList.contains('comment');
    });
  }
  if (typeof globalThis.togglecomment === 'undefined') {
    globalThis.togglecomment = function(el) {
      var c = nearestComment(el);
      if (!c) return false;
      if (c.classList.contains('collapsed')) {
        c.classList.remove('collapsed');
        c.classList.add('noncollapsed');
      } else {
        c.classList.add('collapsed');
        c.classList.remove('noncollapsed');
      }
      return false;
    };
  }
  if (typeof globalThis.hidecomment === 'undefined') {
    globalThis.hidecomment = function(el) {
      var c = nearestComment(el);
      if (c) { c.classList.add('collapsed'); c.classList.remove('noncollapsed'); }
      return false;
    };
  }
  if (typeof globalThis.unhidecomment === 'undefined') {
    globalThis.unhidecomment = function(el) {
      var c = nearestComment(el);
      if (c) { c.classList.remove('collapsed'); c.classList.add('noncollapsed'); }
      return false;
    };
  }
  // Reddit's 'load more comments' can't actually fetch without a JSON
  // bridge, but stubbing it prevents reference errors from onclick.
  if (typeof globalThis.morechildren === 'undefined') {
    globalThis.morechildren = function(){ return false; };
  }
  // Vote arrows: toggle an `upmod`/`downmod` class on the .arrow. This
  // matches how reddit's own sheet styles active votes, giving visual
  // feedback even without a backend roundtrip.
  if (typeof globalThis.togglevote === 'undefined') {
    globalThis.togglevote = function(el, dir) {
      if (!el || !el.classList) return false;
      var on = dir > 0 ? 'upmod' : 'downmod';
      var off = dir > 0 ? 'up' : 'down';
      if (el.classList.contains(on)) {
        el.classList.remove(on);
        el.classList.add(off);
      } else {
        el.classList.add(on);
        el.classList.remove(off);
      }
      return false;
    };
  }
})();
"#;
    let _ = engine.eval(shim);
}

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
                    // Wrap the handler so `return false` / a falsy
                    // return calls `event.preventDefault()`, matching
                    // the HTML spec. Without this, reddit-style
                    // `onclick="return togglecomment(this)"` would
                    // toggle the class then navigate to `#` anyway.
                    let js = format!(
                        "(function(){{ var el = new Element({id}); \
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

// ------------------------------------------------------------------
// innerHTML serialization
// ------------------------------------------------------------------

/// Serialize a DOM node (and its subtree) to an HTML string.
fn serialize_node(doc: &Document, id: NodeId, out: &mut String) {
    match &doc.nodes[id].kind {
        NodeKind::Text(s) => {
            escape_html(s, out);
        },
        NodeKind::Element(e) => {
            let tag = e.tag.as_str();
            out.push('<');
            out.push_str(tag);
            for attr in &e.attributes {
                out.push(' ');
                out.push_str(&attr.name);
                out.push_str("=\"");
                escape_html(&attr.value, out);
                out.push('"');
            }
            if e.tag.is_void() {
                out.push_str(" />");
                return;
            }
            out.push('>');
            for &child in &doc.nodes[id].children {
                serialize_node(doc, child, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        },
        NodeKind::Comment(s) => {
            out.push_str("<!--");
            out.push_str(s);
            out.push_str("-->");
        },
        NodeKind::Document => {
            for &child in &doc.nodes[id].children {
                serialize_node(doc, child, out);
            }
        },
    }
}

/// Escape `<`, `>`, `&`, and `"` in text for HTML serialization.
fn escape_html(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// Deep-copy a node (and its subtree) from `src` into `dst`,
/// returning the new node ID in `dst`.
fn deep_copy_node(src: &Document, dst: &mut Document, src_id: NodeId) -> NodeId {
    let new_id = dst.add_node(src.nodes[src_id].kind.clone());
    for &child_src in &src.nodes[src_id].children {
        let child_new = deep_copy_node(src, dst, child_src);
        dst.append_child(new_id, child_new);
    }
    new_id
}

// ------------------------------------------------------------------
// CSS selector matching for querySelector / querySelectorAll
// ------------------------------------------------------------------

/// Walk the subtree rooted at `root` (excluding `root` itself)
/// and collect element node IDs that match any selector in `sel_list`.
/// If `first_only` is true, stop after the first match.
fn find_matching(
    doc: &Document,
    root: NodeId,
    sel_list: &crate::css::parser::SelectorList,
    first_only: bool,
) -> Vec<NodeId> {
    use crate::css::cascade::CascadeContext;
    use crate::css::cascade::matching::matches_selector;

    let ctx = CascadeContext::default();
    let mut results = Vec::new();
    let mut stack: Vec<NodeId> = doc.nodes[root].children.clone();
    // Reverse so we process in document order (left to right).
    stack.reverse();
    while let Some(nid) = stack.pop() {
        if matches!(doc.nodes[nid].kind, NodeKind::Element(_))
            && sel_list
                .selectors
                .iter()
                .any(|sel| matches_selector(doc, nid, sel, &ctx))
        {
            results.push(nid);
            if first_only {
                return results;
            }
        }
        // Push children in reverse order for DFS document order.
        let children = &doc.nodes[nid].children;
        for &child in children.iter().rev() {
            stack.push(child);
        }
    }
    results
}

// ------------------------------------------------------------------
// classList operations
// ------------------------------------------------------------------

/// Perform a classList operation on an element's `class` attribute.
/// Returns a bool (meaningful for "contains" and "toggle").
/// Apply a `classList` op. Returns `(js_return_value, mutated)` where
/// `mutated` is `true` only if the serialized `class` attribute actually
/// changed — `classList.add` of a token the element already has is a
/// DOM no-op and shouldn't force a relayout.
fn classlist_op_mutated(elem: &mut ElementData, op: &str, cls: &str) -> (bool, bool) {
    let current = elem.get_attribute("class").unwrap_or("").to_string();
    let mut parts: Vec<String> = current.split_ascii_whitespace().map(String::from).collect();

    let (js_ret, new_parts): (bool, Option<Vec<String>>) = match op {
        "add" => {
            if parts.iter().any(|c| c == cls) {
                (true, None)
            } else {
                parts.push(cls.to_string());
                (true, Some(parts))
            }
        },
        "remove" => {
            if parts.iter().any(|c| c == cls) {
                parts.retain(|c| c != cls);
                (false, Some(parts))
            } else {
                (false, None)
            }
        },
        "toggle" => {
            let had = parts.iter().any(|c| c == cls);
            if had {
                parts.retain(|c| c != cls);
            } else {
                parts.push(cls.to_string());
            }
            (!had, Some(parts))
        },
        "contains" => (parts.iter().any(|c| c == cls), None),
        _ => (false, None),
    };

    match new_parts {
        Some(tokens) => {
            let serialized = tokens.join(" ");
            if serialized == current {
                (js_ret, false)
            } else {
                elem.set_attribute("class", &serialized);
                (js_ret, true)
            }
        },
        None => (js_ret, false),
    }
}

// ------------------------------------------------------------------
// Inline style helpers
// ------------------------------------------------------------------

/// Set a CSS property in the element's `style` attribute.
///
/// Returns `true` if the rebuilt `style` attribute differs from the
/// previous value — animation loops that re-assign the same value
/// (`element.style.opacity = element.style.opacity`) shouldn't cause
/// a cascade + relayout on every tick.
fn set_inline_style(elem: &mut ElementData, prop: &str, value: &str) -> bool {
    let current = elem.get_attribute("style").unwrap_or("").to_string();
    let mut decls: Vec<(String, String)> = parse_style_attr(&current);
    let prop_lower = prop.to_ascii_lowercase();
    if let Some(existing) = decls.iter_mut().find(|(p, _)| *p == prop_lower) {
        existing.1 = value.to_string();
    } else {
        decls.push((prop_lower, value.to_string()));
    }
    let rebuilt: String = decls
        .iter()
        .map(|(p, v)| format!("{p}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let changed = rebuilt != current;
    if changed {
        elem.set_attribute("style", &rebuilt);
    }
    changed
}

/// Get a CSS property value from the element's `style` attribute.
fn get_inline_style(elem: &ElementData, prop: &str) -> String {
    let current = elem.get_attribute("style").unwrap_or("");
    let prop_lower = prop.to_ascii_lowercase();
    for (p, v) in parse_style_attr(current) {
        if p == prop_lower {
            return v;
        }
    }
    String::new()
}

/// Parse an inline `style` attribute value into property/value pairs.
fn parse_style_attr(style: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for decl in style.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some((prop, val)) = decl.split_once(':') {
            result.push((prop.trim().to_ascii_lowercase(), val.trim().to_string()));
        }
    }
    result
}

/// JavaScript code that defines the `Element` wrapper and `document`
/// global using the `__oasis_*` Rust-backed helper functions.
const JS_DOM_BOOTSTRAP: &str = r#"
(function() {
  "use strict";

  function Element(nid) {
    this.__oasis_node_id = nid;
  }

  // Helper to get-or-create an Element wrapper by nid.
  function __get_el(nid) {
    return nid >= 0 ? new Element(nid) : null;
  }

  Object.defineProperties(Element.prototype, {
    tagName: {
      get: function() {
        return __oasis_tagname(this.__oasis_node_id);
      },
      enumerable: true
    },
    id: {
      get: function() {
        return __oasis_getattr(this.__oasis_node_id, "id") || "";
      },
      set: function(v) {
        if (v) __oasis_setattr(this.__oasis_node_id, "id", v);
        else __oasis_rmattr(this.__oasis_node_id, "id");
      },
      enumerable: true
    },
    textContent: {
      get: function() {
        return __oasis_text(this.__oasis_node_id);
      },
      set: function(v) {
        __oasis_settext(this.__oasis_node_id, String(v));
      },
      enumerable: true
    },
    children: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        var result = [];
        for (var i = 0; i < ids.length; i++)
          result.push(new Element(ids[i]));
        return result;
      },
      enumerable: true
    },
    parentElement: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        return pid >= 0 ? new Element(pid) : null;
      },
      enumerable: true
    },
    parentNode: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        return pid >= 0 ? new Element(pid) : null;
      },
      enumerable: true
    },
    firstChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length > 0 ? new Element(ids[0]) : null;
      },
      enumerable: true
    },
    lastChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length > 0 ? new Element(ids[ids.length - 1]) : null;
      },
      enumerable: true
    },
    childNodes: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        var result = [];
        for (var i = 0; i < ids.length; i++)
          result.push(new Element(ids[i]));
        return result;
      },
      enumerable: true
    },
    nextSibling: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        if (pid < 0) return null;
        var siblings = __oasis_children(pid);
        for (var i = 0; i < siblings.length - 1; i++) {
          if (siblings[i] === this.__oasis_node_id) return new Element(siblings[i + 1]);
        }
        return null;
      },
      enumerable: true
    },
    previousSibling: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        if (pid < 0) return null;
        var siblings = __oasis_children(pid);
        for (var i = 1; i < siblings.length; i++) {
          if (siblings[i] === this.__oasis_node_id) return new Element(siblings[i - 1]);
        }
        return null;
      },
      enumerable: true
    },
    innerHTML: {
      get: function() {
        return __oasis_inner_html(this.__oasis_node_id);
      },
      set: function(v) {
        __oasis_set_inner_html(this.__oasis_node_id, String(v));
      },
      enumerable: true
    },
    classList: {
      get: function() {
        var self = this;
        return {
          add: function(c) {
            __oasis_classlist_op(self.__oasis_node_id, 'add', c);
          },
          remove: function(c) {
            __oasis_classlist_op(
              self.__oasis_node_id, 'remove', c
            );
          },
          toggle: function(c) {
            return __oasis_classlist_op(
              self.__oasis_node_id, 'toggle', c
            );
          },
          contains: function(c) {
            return __oasis_classlist_op(
              self.__oasis_node_id, 'contains', c
            );
          }
        };
      },
      enumerable: true
    },
    style: {
      get: function() {
        var nid = this.__oasis_node_id;
        // Proxy-like object: direct property access (e.g. .color)
        // maps to CSS property names via camelCase-to-kebab conversion.
        return new Proxy({
          setProperty: function(p, v) {
            __oasis_style_set(nid, p, String(v));
          },
          getPropertyValue: function(p) {
            return __oasis_style_get(nid, p);
          }
        }, {
          set: function(target, prop, value) {
            if (typeof prop === 'string') {
              var css_prop = prop.replace(
                /[A-Z]/g,
                function(m) { return '-' + m.toLowerCase(); }
              );
              __oasis_style_set(nid, css_prop, String(value));
            }
            return true;
          },
          get: function(target, prop) {
            if (typeof target[prop] === 'function') return target[prop];
            if (typeof prop === 'string') {
              var css_prop = prop.replace(
                /[A-Z]/g,
                function(m) { return '-' + m.toLowerCase(); }
              );
              return __oasis_style_get(nid, css_prop);
            }
            return undefined;
          }
        });
      },
      enumerable: true
    }
  });

  Element.prototype.getAttribute = function(name) {
    var v = __oasis_getattr(this.__oasis_node_id, name);
    return v === undefined ? null : v;
  };
  Element.prototype.setAttribute = function(name, value) {
    __oasis_setattr(this.__oasis_node_id, name, String(value));
  };
  Element.prototype.removeAttribute = function(name) {
    __oasis_rmattr(this.__oasis_node_id, name);
  };
  Element.prototype.appendChild = function(child) {
    __oasis_append(
      this.__oasis_node_id, child.__oasis_node_id
    );
    return child;
  };
  Element.prototype.removeChild = function(child) {
    __oasis_remove(child.__oasis_node_id);
    return child;
  };
  Element.prototype.insertBefore = function(newNode, refNode) {
    var refId = refNode ? refNode.__oasis_node_id : -1;
    __oasis_insertbefore(
      this.__oasis_node_id,
      newNode.__oasis_node_id,
      refId
    );
    return newNode;
  };
  Element.prototype.querySelector = function(sel) {
    var nid = __oasis_query_selector(
      this.__oasis_node_id, sel
    );
    return __get_el(nid);
  };
  Element.prototype.querySelectorAll = function(sel) {
    var nids = __oasis_query_selector_all(
      this.__oasis_node_id, sel
    );
    return nids.map(function(n) { return new Element(n); });
  };

  // -- Event listener support --
  // Listeners stored as {fn, once, capture, passive} objects.
  var __oasis_listeners = {};

  function __parse_opts(opts) {
    var c = false, o = false, p = false;
    if (opts === true || opts === false) { c = opts; }
    else if (opts && typeof opts === 'object') {
      c = !!opts.capture; o = !!opts.once; p = !!opts.passive;
    }
    return {capture: c, once: o, passive: p};
  }

  Element.prototype.addEventListener = function(type, fn, opts) {
    if (!fn) return;
    var o = __parse_opts(opts);
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    if (!__oasis_listeners[key]) __oasis_listeners[key] = [];
    var arr = __oasis_listeners[key];
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === o.capture) return;
    }
    arr.push({fn: fn, once: o.once, capture: o.capture, passive: o.passive});
  };
  Element.prototype.removeEventListener = function(type, fn, opts) {
    var cap = false;
    if (opts === true || opts === false) cap = opts;
    else if (opts && typeof opts === 'object') cap = !!opts.capture;
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === cap) {
        arr.splice(i, 1); return;
      }
    }
  };
  Element.prototype.dispatchEvent = function(evt) {
    var nid = this.__oasis_node_id;
    var key = nid + ":" + evt.type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    evt.target = this;
    for (var i = 0; i < arr.length; i++) {
      arr[i].fn.call(this, evt);
      if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
    }
  };

  // Helper: invoke matching listeners, handling once removal.
  // phase: 1=capture, 2=target, 3=bubble
  function __fire(key, el, evt, phase) {
    var arr = __oasis_listeners[key];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (evt._stopped) break;
      var e = arr[i];
      if (phase === 2 || (phase === 1 && e.capture) ||
          (phase === 3 && !e.capture)) {
        evt.currentTarget = el;
        e.fn.call(el, evt);
        if (e.once) { arr.splice(i, 1); i--; }
      }
    }
  }

  // Expose dispatch helper for Rust-side event triggering.
  globalThis.__oasis_dispatch_event =
    function(nid, type, detail) {
      var key = nid + ":" + type;
      var arr = __oasis_listeners[key];
      if (!arr || arr.length === 0) return;
      var el = new Element(nid);
      var evt = {
        type: type, target: el, detail: detail || null,
        _stopped: false,
        stopPropagation: function() { this._stopped = true; },
        preventDefault: function() { this._defaultPrevented = true; },
        _defaultPrevented: false
      };
      for (var i = 0; i < arr.length; i++) {
        arr[i].fn.call(el, evt);
        if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
      }
    };

  // Dispatch with capture, target, and bubble phases.
  globalThis.__oasis_dispatch_with_bubbling =
    function(nid, type, detail) {
      var target = new Element(nid);
      var evt = {
        type: type,
        detail: detail || null,
        target: target,
        currentTarget: null,
        eventPhase: 0,
        _stopped: false,
        stopPropagation: function() {
          this._stopped = true;
        },
        preventDefault: function() {
          this._defaultPrevented = true;
        },
        _defaultPrevented: false
      };
      if (detail && typeof detail === 'object') {
        for (var k in detail) {
          if (detail.hasOwnProperty(k)) evt[k] = detail[k];
        }
      }
      // Build ancestor chain (excluding target), root first.
      var ancestors = [];
      var p = __oasis_parent(nid);
      while (p >= 0) { ancestors.push(p); p = __oasis_parent(p); }
      ancestors.reverse();
      // Capture phase: root -> target (ancestors only, capture listeners).
      evt.eventPhase = 1;
      for (var i = 0; i < ancestors.length && !evt._stopped; i++) {
        __fire(ancestors[i] + ":" + type, new Element(ancestors[i]), evt, 1);
      }
      // Target phase: all listeners on target.
      if (!evt._stopped) {
        evt.eventPhase = 2;
        __fire(nid + ":" + type, target, evt, 2);
      }
      // Bubble phase: target -> root (ancestors only, non-capture listeners).
      evt.eventPhase = 3;
      for (var i = ancestors.length - 1; i >= 0 && !evt._stopped; i--) {
        __fire(ancestors[i] + ":" + type, new Element(ancestors[i]), evt, 3);
      }
      // Report default-prevented back to the Rust side so it can skip
      // follow-up behaviors like link navigation when the page says
      // "return false" from an inline onclick handler.
      return evt._defaultPrevented;
    };

  var document = {
    getElementById: function(id) {
      var nid = __oasis_getbyid(id);
      return nid >= 0 ? new Element(nid) : null;
    },
    createElement: function(tag) {
      return new Element(__oasis_create(tag));
    },
    createTextNode: function(text) {
      return new Element(__oasis_createtext(String(text)));
    },
    querySelector: function(sel) {
      var b = __oasis_body();
      if (b < 0) return null;
      var nid = __oasis_query_selector(b, sel);
      return __get_el(nid);
    },
    querySelectorAll: function(sel) {
      var b = __oasis_body();
      if (b < 0) return [];
      var nids = __oasis_query_selector_all(b, sel);
      return nids.map(function(n) {
        return new Element(n);
      });
    }
  };

  Object.defineProperties(document, {
    body: {
      get: function() {
        var nid = __oasis_body();
        return nid >= 0 ? new Element(nid) : null;
      },
      enumerable: true
    },
    title: {
      get: function() { return __oasis_title(); },
      set: function(v) { __oasis_settitle(String(v)); },
      enumerable: true
    }
  });

  // Give document event listener support.
  var __doc_listeners = {};
  document.addEventListener = function(type, fn, opts) {
    if (!fn) return;
    var o = __parse_opts(opts);
    if (!__doc_listeners[type]) __doc_listeners[type] = [];
    var arr = __doc_listeners[type];
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === o.capture) return;
    }
    arr.push({fn: fn, once: o.once, capture: o.capture, passive: o.passive});
  };
  document.removeEventListener = function(type, fn, opts) {
    var cap = false;
    if (opts === true || opts === false) cap = opts;
    else if (opts && typeof opts === 'object') cap = !!opts.capture;
    if (!__doc_listeners[type]) return;
    __doc_listeners[type] = __doc_listeners[type].filter(function(e) {
      return !(e.fn === fn && e.capture === cap);
    });
  };
  document.dispatchEvent = function(evt) {
    var type = evt && evt.type ? evt.type : evt;
    var arr = __doc_listeners[type];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      arr[i].fn(evt);
      if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
    }
  };

  // Minimal Event constructor for DOMContentLoaded etc.
  if (typeof Event === 'undefined') {
    globalThis.Event = function(type) { this.type = type; };
  }

  globalThis.document = document;
  globalThis.Element = Element;
  globalThis.window = globalThis;

  // -- location object with assign() and href setter --
  var __oasis_loc = {
    get href() { return __oasis_location(); },
    set href(v) { __oasis_location_assign(String(v)); },
    assign: function(url) { __oasis_location_assign(String(url)); },
    replace: function(url) { __oasis_location_assign(String(url)); },
    reload: function() { __oasis_location_assign(__oasis_location()); },
    toString: function() { return __oasis_location(); }
  };
  Object.defineProperty(globalThis, 'location', {
    get: function() { return __oasis_loc; },
    set: function(v) { __oasis_location_assign(String(v)); },
    configurable: true
  });

  // -- history object --
  globalThis.history = {
    __state: null,
    back: function() { __oasis_history_back(); },
    forward: function() { __oasis_history_forward(); },
    go: function(delta) {
      if (delta < 0) __oasis_history_back();
      else if (delta > 0) __oasis_history_forward();
      else __oasis_location_assign(__oasis_location());
    },
    pushState: function(state, title, url) {
      this.__state = state;
      if (url) __oasis_location_push(String(url));
    },
    replaceState: function(state, title, url) {
      this.__state = state;
      if (url) __oasis_location_push(String(url));
    },
    get state() { return this.__state; },
    get length() { return 1; }
  };

  // -- fetch API (synchronous under the hood) --
  globalThis.fetch = function(url, options) {
    var method = (options && options.method) || "GET";
    var reqBody = (options && options.body) || "";
    var body = __oasis_fetch(method, String(url), String(reqBody));
    return {
      then: function(fn) {
        var result = fn({
          ok: body.length > 0,
          status: body.length > 0 ? 200 : 0,
          text: function() { return { then: function(f) { return f(body); } }; },
          json: function() { return { then: function(f) { return f(JSON.parse(body)); } }; }
        });
        return { then: function(f) { return f ? f(result) : result; }, catch: function() { return this; } };
      },
      catch: function(fn) { return this; }
    };
  };

  // -- getComputedStyle --
  globalThis.getComputedStyle = function(el) {
    return {
      getPropertyValue: function(prop) {
        return __oasis_computed_style(el.__oasis_node_id, prop);
      }
    };
  };

  // -- localStorage / sessionStorage --
  // kind: 0 = localStorage, 1 = sessionStorage (separate backing stores)
  var __make_storage = function(kind) {
    return {
      getItem: function(k) { var v = __oasis_storage_get(kind, String(k)); return v === "" ? null : v; },
      setItem: function(k, v) { __oasis_storage_set(kind, String(k), String(v)); },
      removeItem: function(k) { __oasis_storage_remove(kind, String(k)); },
      clear: function() { __oasis_storage_clear(kind); },
      get length() { return __oasis_storage_length(kind); }
    };
  };
  globalThis.localStorage = __make_storage(0);
  globalThis.sessionStorage = __make_storage(1);

  // -- document.cookie --
  Object.defineProperty(document, 'cookie', {
    get: function() { return __oasis_cookie_get(); },
    set: function(v) { __oasis_cookie_set(String(v)); },
    configurable: true
  });
})();
"#;

// ------------------------------------------------------------------
// Canvas 2D context bindings
// ------------------------------------------------------------------

/// Install `__oasis_canvas_*` globals for `<canvas>` 2D context support.
///
/// Must be called after [`install_document_global_full`] since the
/// JS bootstrap below extends `Element.prototype` with `getContext()`.
#[cfg(feature = "canvas")]
pub fn install_canvas_bindings(
    ctx: &Ctx<'_>,
    canvas_map: &crate::canvas::SharedCanvasMap,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_canvas_fill_rect(nid, x, y, w, h) -------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.fill_color;
                        s.commands.push(crate::canvas::CanvasCommand::FillRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                            color,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_stroke_rect(nid, x, y, w, h) -----------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_stroke_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.stroke_color;
                        let lw = s.line_width;
                        s.commands.push(crate::canvas::CanvasCommand::StrokeRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                            color,
                            line_width: lw,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_clear_rect(nid, x, y, w, h) ------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_clear_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        s.commands.push(crate::canvas::CanvasCommand::ClearRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_fill_text(nid, text, x, y) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_text",
            Function::new(
                ctx.clone(),
                move |nid: i32, text: String, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.fill_color;
                        let font_size = s.font_size;
                        s.commands.push(crate::canvas::CanvasCommand::FillText {
                            text,
                            x: x as f32,
                            y: y as f32,
                            color,
                            font_size,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_set_fill(nid, color_str) ----------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_fill",
            Function::new(ctx.clone(), move |nid: i32, color: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId))
                    && let Some(c) = crate::svg::parse_svg_color(&color)
                {
                    state.borrow_mut().fill_color = c;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_stroke(nid, color_str) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_stroke",
            Function::new(ctx.clone(), move |nid: i32, color: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId))
                    && let Some(c) = crate::svg::parse_svg_color(&color)
                {
                    state.borrow_mut().stroke_color = c;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_line_width(nid, width) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_line_width",
            Function::new(ctx.clone(), move |nid: i32, width: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().line_width = width as f32;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_font(nid, font_str) -----------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_font",
            Function::new(ctx.clone(), move |nid: i32, font: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    // Extract pixel size from font string, e.g. "12px sans-serif".
                    for part in font.split_whitespace() {
                        if let Some(px) = part.strip_suffix("px")
                            && let Ok(size) = px.parse::<f32>()
                        {
                            state.borrow_mut().font_size = size;
                            break;
                        }
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_line(nid, x1, y1, x2, y2, is_fill) -----------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_line",
            Function::new(
                ctx.clone(),
                move |nid: i32, x1: f64, y1: f64, x2: f64, y2: f64, is_fill: bool| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = if is_fill {
                            s.fill_color
                        } else {
                            s.stroke_color
                        };
                        let lw = s.line_width;
                        s.commands.push(crate::canvas::CanvasCommand::Line {
                            x1: x1 as f32,
                            y1: y1 as f32,
                            x2: x2 as f32,
                            y2: y2 as f32,
                            color,
                            line_width: lw,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_arc(nid, cx, cy, r, fill) ---------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_arc",
            Function::new(
                ctx.clone(),
                move |nid: i32, cx: f64, cy: f64, r: f64, fill: bool| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = if fill { s.fill_color } else { s.stroke_color };
                        s.commands.push(crate::canvas::CanvasCommand::Arc {
                            cx: cx as f32,
                            cy: cy as f32,
                            r: r as f32,
                            color,
                            fill,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_begin_path(nid) ---------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_begin_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_move_to(nid, x, y) ----------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_move_to",
            Function::new(ctx.clone(), move |nid: i32, x: f64, y: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    let pt = (x as f32, y as f32);
                    s.current_path.push(pt);
                    s.path_start = Some(pt);
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_line_to(nid, x, y) ----------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_line_to",
            Function::new(ctx.clone(), move |nid: i32, x: f64, y: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().current_path.push((x as f32, y as f32));
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_bezier_curve_to(nid, cp1x, cp1y, cp2x, cp2y, x, y)
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_bezier_curve_to",
            Function::new(
                ctx.clone(),
                move |nid: i32, cp1x: f64, cp1y: f64, cp2x: f64, cp2y: f64, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let (cx, cy) = s.current_path.last().copied().unwrap_or((0.0, 0.0));
                        crate::svg::flatten_cubic(
                            &mut s.current_path,
                            cx,
                            cy,
                            cp1x as f32,
                            cp1y as f32,
                            cp2x as f32,
                            cp2y as f32,
                            x as f32,
                            y as f32,
                        );
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_quadratic_curve_to(nid, cpx, cpy, x, y) ------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_quadratic_curve_to",
            Function::new(
                ctx.clone(),
                move |nid: i32, cpx: f64, cpy: f64, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let (cx, cy) = s.current_path.last().copied().unwrap_or((0.0, 0.0));
                        crate::svg::flatten_quad(
                            &mut s.current_path,
                            cx,
                            cy,
                            cpx as f32,
                            cpy as f32,
                            x as f32,
                            y as f32,
                        );
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_close_path(nid) -------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_close_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if let Some(start) = s.path_start {
                        s.current_path.push(start);
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_fill_path(nid) --------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if s.current_path.len() >= 3 {
                        let color = s.fill_color;
                        let points = std::mem::take(&mut s.current_path);
                        s.commands
                            .push(crate::canvas::CanvasCommand::FillPath { points, color });
                    }
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_stroke_path(nid) ------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_stroke_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if s.current_path.len() >= 2 {
                        let color = s.stroke_color;
                        let lw = s.line_width;
                        let points = std::mem::take(&mut s.current_path);
                        s.commands.push(crate::canvas::CanvasCommand::StrokePath {
                            points,
                            color,
                            line_width: lw,
                        });
                    }
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_save(nid) / __oasis_canvas_restore(nid) -------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_save",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().save();
                }
            })?,
        )?;
    }
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_restore",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().restore();
                }
            })?,
        )?;
    }

    // -- JavaScript CanvasRenderingContext2D class ---------------------
    #[cfg(feature = "canvas")]
    {
        let _: () = ctx.eval(JS_CANVAS_BOOTSTRAP)?;
    }

    Ok(())
}

/// JavaScript code for the CanvasRenderingContext2D class and
/// `Element.prototype.getContext()`.
#[cfg(feature = "canvas")]
const JS_CANVAS_BOOTSTRAP: &str = r##"
(function() {
  "use strict";

  function CanvasRenderingContext2D(nid) {
    this.__nid = nid;
    this._fillStyle = "#000000";
    this._strokeStyle = "#000000";
    this._lineWidth = 1;
    this._font = "10px sans-serif";
    this._pathX = 0;
    this._pathY = 0;
    this._pathSegments = [];
  }

  Object.defineProperties(CanvasRenderingContext2D.prototype, {
    fillStyle: {
      get: function() { return this._fillStyle; },
      set: function(v) {
        this._fillStyle = v;
        __oasis_canvas_set_fill(this.__nid, String(v));
      },
      enumerable: true
    },
    strokeStyle: {
      get: function() { return this._strokeStyle; },
      set: function(v) {
        this._strokeStyle = v;
        __oasis_canvas_set_stroke(this.__nid, String(v));
      },
      enumerable: true
    },
    lineWidth: {
      get: function() { return this._lineWidth; },
      set: function(v) {
        this._lineWidth = v;
        __oasis_canvas_set_line_width(this.__nid, +v);
      },
      enumerable: true
    },
    font: {
      get: function() { return this._font; },
      set: function(v) {
        this._font = v;
        __oasis_canvas_set_font(this.__nid, String(v));
      },
      enumerable: true
    }
  });

  CanvasRenderingContext2D.prototype.fillRect = function(x, y, w, h) {
    __oasis_canvas_fill_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.strokeRect = function(x, y, w, h) {
    __oasis_canvas_stroke_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.clearRect = function(x, y, w, h) {
    __oasis_canvas_clear_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.fillText = function(text, x, y) {
    __oasis_canvas_fill_text(this.__nid, String(text), +x, +y);
  };
  CanvasRenderingContext2D.prototype.strokeText = function() {};
  CanvasRenderingContext2D.prototype.beginPath = function() {
    __oasis_canvas_begin_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.moveTo = function(x, y) {
    __oasis_canvas_move_to(this.__nid, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.lineTo = function(x, y) {
    __oasis_canvas_line_to(this.__nid, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.bezierCurveTo = function(cp1x, cp1y, cp2x, cp2y, x, y) {
    __oasis_canvas_bezier_curve_to(this.__nid, +cp1x, +cp1y, +cp2x, +cp2y, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.quadraticCurveTo = function(cpx, cpy, x, y) {
    __oasis_canvas_quadratic_curve_to(this.__nid, +cpx, +cpy, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.arc = function(cx, cy, r) {
    // Arc is handled specially: emit as native arc command.
    this._pathSegments.push({
      type: "arc", cx: +cx, cy: +cy, r: +r
    });
  };
  CanvasRenderingContext2D.prototype.closePath = function() {
    __oasis_canvas_close_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.fill = function() {
    // First flush any arc segments (legacy path).
    for (var i = 0; i < this._pathSegments.length; i++) {
      var seg = this._pathSegments[i];
      if (seg.type === "arc") {
        __oasis_canvas_arc(this.__nid, seg.cx, seg.cy, seg.r, true);
      }
    }
    this._pathSegments = [];
    // Then emit the native path fill.
    __oasis_canvas_fill_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.stroke = function() {
    // First flush any arc segments (legacy path).
    for (var i = 0; i < this._pathSegments.length; i++) {
      var seg = this._pathSegments[i];
      if (seg.type === "arc") {
        __oasis_canvas_arc(this.__nid, seg.cx, seg.cy, seg.r, false);
      }
    }
    this._pathSegments = [];
    // Then emit the native path stroke.
    __oasis_canvas_stroke_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.measureText = function(text) {
    return { width: String(text).length * 6 };
  };
  CanvasRenderingContext2D.prototype.save = function() {
    __oasis_canvas_save(this.__nid);
  };
  CanvasRenderingContext2D.prototype.restore = function() {
    __oasis_canvas_restore(this.__nid);
  };

  var __canvas_contexts = {};

  if (typeof Element !== "undefined") {
    Element.prototype.getContext = function(type) {
      if (type !== "2d") return null;
      var nid = this.__oasis_node_id;
      if (!__canvas_contexts[nid]) {
        __canvas_contexts[nid] = new CanvasRenderingContext2D(nid);
      }
      return __canvas_contexts[nid];
    };
  }

  globalThis.CanvasRenderingContext2D = CanvasRenderingContext2D;
})();
"##;

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_js::JsEngine;

    /// Build a tiny DOM:
    ///
    /// ```text
    /// Document(0)
    ///   html(1)
    ///     head(2)
    ///       title(3)
    ///         "Test"(4)
    ///     body(5)
    ///       div#main(6)
    ///         p(7)
    ///           "hello"(8)
    /// ```
    fn sample_doc() -> Document {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        let head = doc.add_node(NodeKind::Element(ElementData::new(TagName::Head)));
        doc.append_child(html, head);
        let title = doc.add_node(NodeKind::Element(ElementData::new(TagName::Title)));
        doc.append_child(head, title);
        let title_text = doc.add_node(NodeKind::Text("Test".into()));
        doc.append_child(title, title_text);

        let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
        doc.append_child(html, body);

        let mut div_data = ElementData::new(TagName::Div);
        div_data.set_attribute("id", "main");
        let div = doc.add_node(NodeKind::Element(div_data));
        doc.append_child(body, div);

        let p = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
        doc.append_child(div, p);
        let text = doc.add_node(NodeKind::Text("hello".into()));
        doc.append_child(p, text);

        doc
    }

    /// Helper: create engine + shared doc, install document global.
    fn setup(doc: Document) -> (JsEngine, SharedDoc) {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let shared: SharedDoc = Rc::new(RefCell::new(doc));
        let s = Rc::clone(&shared);
        engine
            .with_context(|ctx| install_document_global(&ctx, &s))
            .unwrap();
        (engine, shared)
    }

    #[test]
    fn get_element_by_id_returns_proxy() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.getElementById('main').tagName")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
    }

    #[test]
    fn get_element_by_id_returns_null_for_missing() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.getElementById('nope') === null")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::Bool(true));
    }

    #[test]
    fn set_text_content_from_js() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval("document.getElementById('main').textContent = 'new text'")
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        assert_eq!(doc.text_content(main), "new text");
    }

    #[test]
    fn set_attribute_from_js() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval("document.getElementById('main').setAttribute('class', 'foo')")
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        let elem = doc.element(main).unwrap();
        assert_eq!(elem.get_attribute("class"), Some("foo"));
    }

    #[test]
    fn remove_attribute_from_js() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.setAttribute('data-x', '1'); \
                 el.removeAttribute('data-x')",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        let elem = doc.element(main).unwrap();
        assert_eq!(elem.get_attribute("data-x"), None);
    }

    #[test]
    fn create_element_and_append() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var span = document.createElement('span'); \
                 span.textContent = 'added'; \
                 document.getElementById('main').appendChild(span)",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        let text = doc.text_content(main);
        assert!(text.contains("added"));
    }

    #[test]
    fn create_text_node_and_append() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var t = document.createTextNode(' world'); \
                 document.body.appendChild(t)",
            )
            .unwrap();
        let doc = shared.borrow();
        let body = doc.body().unwrap();
        let text = doc.text_content(body);
        assert!(text.contains(" world"));
    }

    #[test]
    fn set_id_from_js() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval("document.getElementById('main').id = 'changed'")
            .unwrap();
        let doc = shared.borrow();
        assert!(doc.get_element_by_id("main").is_none());
        assert!(doc.get_element_by_id("changed").is_some());
    }

    #[test]
    fn document_title_get_and_set() {
        let (engine, shared) = setup(sample_doc());
        let val = engine.eval("document.title").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("Test".into()));

        engine.eval("document.title = 'New Title'").unwrap();
        assert_eq!(shared.borrow().title(), Some("New Title".into()));
    }

    #[test]
    fn append_child_moves_node() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var sec = document.createElement('section'); \
                 document.body.appendChild(sec); \
                 var p = document.getElementById('main').children[0]; \
                 sec.appendChild(p)",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        let main_children: Vec<_> = doc.get(main).children.iter().copied().collect();
        assert!(
            main_children.iter().all(|&c| !matches!(
                doc.get(c).kind,
                NodeKind::Element(ref e) if e.tag == TagName::P
            )),
            "p should no longer be under div#main"
        );
    }

    #[test]
    fn dom_mutations_persist_after_engine_drop() {
        let shared: SharedDoc = Rc::new(RefCell::new(sample_doc()));
        {
            let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
            let s = Rc::clone(&shared);
            engine
                .with_context(|ctx| install_document_global(&ctx, &s))
                .unwrap();
            engine
                .eval("document.getElementById('main').textContent = 'persisted'")
                .unwrap();
        }
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").unwrap();
        assert_eq!(doc.text_content(main), "persisted");
    }

    // ---------------------------------------------------------------
    // Event listener + dispatch tests
    // ---------------------------------------------------------------

    #[test]
    fn add_event_listener_and_dispatch() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var clicked = false; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { clicked = true; }); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("clicked").unwrap();
        assert_eq!(val, oasis_js::JsValue::Bool(true));
    }

    #[test]
    fn multiple_listeners_on_same_element() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }); \
                 el.addEventListener('click', function() { count += 10; }); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(11));
    }

    #[test]
    fn remove_event_listener() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1); \
                 el.removeEventListener('click', fn1); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(0));
    }

    #[test]
    fn dispatch_with_bubbling_child_to_parent() {
        let (engine, _doc) = setup(sample_doc());
        // p(7) is child of div#main(6).
        engine
            .eval(
                "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("order.join(',')").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("p,div".into()));
    }

    #[test]
    fn stop_propagation_prevents_bubbling() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 p.addEventListener('click', function(e) { \
                     order.push('p'); e.stopPropagation(); }); \
                 div.addEventListener('click', function() { order.push('div'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("order.join(',')").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("p".into()));
    }

    #[test]
    fn bubbling_event_target_vs_current_target() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var targetTag = ''; var currentTag = ''; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function(e) { \
                     targetTag = e.target.tagName; \
                     currentTag = e.currentTarget.tagName; \
                 }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let target = engine.eval("targetTag").unwrap();
        let current = engine.eval("currentTag").unwrap();
        assert_eq!(target, oasis_js::JsValue::String("P".into()));
        assert_eq!(current, oasis_js::JsValue::String("DIV".into()));
    }

    #[test]
    fn dispatch_event_receives_detail() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var received = null; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('custom', function(e) { \
                     received = e.detail; \
                 }); \
                 __oasis_dispatch_event(\
                     el.__oasis_node_id, 'custom', 'payload')",
            )
            .unwrap();
        let val = engine.eval("received").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("payload".into()));
    }

    #[test]
    fn retained_engine_fires_events_after_script_exec() {
        // Simulate what widget_pipeline.rs does: create engine, run
        // scripts, then dispatch events later.
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var clicked = false; \
                 document.getElementById('main')\
                     .addEventListener('click', function() { \
                         clicked = true; \
                     })",
            )
            .unwrap();

        // Later, Rust dispatches an event.
        engine
            .eval("__oasis_dispatch_with_bubbling(6, 'click', null)")
            .unwrap();
        let val = engine.eval("clicked").unwrap();
        assert_eq!(val, oasis_js::JsValue::Bool(true));
    }

    // ---------------------------------------------------------------
    // addEventListener options tests
    // ---------------------------------------------------------------

    #[test]
    fn once_option_removes_after_first_call() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }, {once: true}); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(1));
    }

    #[test]
    fn capture_option_fires_in_capture_phase() {
        let (engine, _doc) = setup(sample_doc());
        // p(7) is child of div#main(6).
        // Capture listener on div fires before bubble listener on p.
        engine
            .eval(
                "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function() { order.push('div-cap'); }, true); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div-bub'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("order.join(',')").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("div-cap,p,div-bub".into()));
    }

    #[test]
    fn remove_listener_must_match_capture_flag() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1, true); \
                 el.removeEventListener('click', fn1, false); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        // Listener was added with capture=true, removed with capture=false,
        // so it should NOT be removed.
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(1));
    }

    #[test]
    fn remove_listener_with_matching_capture() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1, true); \
                 el.removeEventListener('click', fn1, true); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(0));
    }

    #[test]
    fn boolean_capture_arg_works() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function() { order.push('cap'); }, true); \
                 div.addEventListener('click', function() { order.push('bub'); }, false); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("order.join(',')").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("cap,bub".into()));
    }

    #[test]
    fn once_with_bubbling() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }, {once: true}); \
                 __oasis_dispatch_with_bubbling(\
                     el.__oasis_node_id, 'click', null); \
                 __oasis_dispatch_with_bubbling(\
                     el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(1));
    }

    #[test]
    fn document_once_listener() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 document.addEventListener('custom', function() { count++; }, {once: true}); \
                 document.dispatchEvent({type: 'custom'}); \
                 document.dispatchEvent({type: 'custom'})",
            )
            .unwrap();
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(1));
    }

    #[test]
    fn duplicate_listener_prevented() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1); \
                 el.addEventListener('click', fn1); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        // Per spec, adding the same fn+capture combo twice is a no-op.
        let val = engine.eval("count").unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(1));
    }

    #[test]
    fn stop_propagation_in_capture_phase() {
        let (engine, _doc) = setup(sample_doc());
        engine
            .eval(
                "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function(e) { \
                     order.push('div-cap'); e.stopPropagation(); \
                 }, true); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div-bub'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
            )
            .unwrap();
        let val = engine.eval("order.join(',')").unwrap();
        // Capture listener stops propagation, so target and bubble never fire.
        assert_eq!(val, oasis_js::JsValue::String("div-cap".into()));
    }

    // ---------------------------------------------------------------
    // innerHTML tests
    // ---------------------------------------------------------------

    #[test]
    fn inner_html_get() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.getElementById('main').innerHTML")
            .unwrap();
        // div#main contains <p>hello</p>
        if let oasis_js::JsValue::String(s) = val {
            assert!(
                s.contains("<p>") && s.contains("hello"),
                "unexpected innerHTML: {s}"
            );
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn inner_html_set() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "document.getElementById('main').innerHTML = \
                 '<span>new</span>'",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").expect("main");
        let text = doc.text_content(main);
        assert_eq!(text, "new");
        // Should have one child: <span>
        let children: Vec<_> = doc.nodes[main]
            .children
            .iter()
            .copied()
            .filter(|&c| matches!(doc.nodes[c].kind, NodeKind::Element(_)))
            .collect();
        assert_eq!(children.len(), 1);
        let child_elem = doc.element(children[0]).expect("elem");
        assert_eq!(child_elem.tag, TagName::Span);
    }

    #[test]
    fn inner_html_set_empty() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval("document.getElementById('main').innerHTML = ''")
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").expect("main");
        assert!(doc.nodes[main].children.is_empty());
    }

    // ---------------------------------------------------------------
    // querySelector / querySelectorAll tests
    // ---------------------------------------------------------------

    #[test]
    fn query_selector_by_tag() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine.eval("document.querySelector('p').tagName").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("P".into()));
    }

    #[test]
    fn query_selector_by_id() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.querySelector('#main').tagName")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
    }

    #[test]
    fn query_selector_by_class() {
        let (engine, _doc) = setup(sample_doc());
        // Add a class first, then query by it.
        engine
            .eval(
                "document.getElementById('main')\
                 .classList.add('highlight')",
            )
            .unwrap();
        let val = engine
            .eval("document.querySelector('.highlight').id")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("main".into()));
    }

    #[test]
    fn query_selector_returns_null_for_no_match() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.querySelector('.nope') === null")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::Bool(true));
    }

    #[test]
    fn query_selector_all_returns_array() {
        let (engine, _doc) = setup(sample_doc());
        // Add another div to body for multiple matches.
        engine
            .eval(
                "var d = document.createElement('div'); \
                 document.body.appendChild(d)",
            )
            .unwrap();
        let val = engine
            .eval("document.querySelectorAll('div').length")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::Int(2));
    }

    #[test]
    fn query_selector_compound() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval("document.querySelector('div#main').tagName")
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
    }

    #[test]
    fn element_query_selector() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval(
                "document.getElementById('main')\
                 .querySelector('p').textContent",
            )
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("hello".into()));
    }

    // ---------------------------------------------------------------
    // classList tests
    // ---------------------------------------------------------------

    #[test]
    fn classlist_add_and_contains() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.classList.add('foo'); \
                 el.classList.add('bar')",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").expect("main");
        let elem = doc.element(main).expect("elem");
        assert!(elem.has_class("foo"));
        assert!(elem.has_class("bar"));
    }

    #[test]
    fn classlist_remove() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.classList.add('foo'); \
                 el.classList.add('bar'); \
                 el.classList.remove('foo')",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").expect("main");
        let elem = doc.element(main).expect("elem");
        assert!(!elem.has_class("foo"));
        assert!(elem.has_class("bar"));
    }

    #[test]
    fn classlist_toggle() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval(
                "var el = document.getElementById('main'); \
                 var r1 = el.classList.toggle('active'); \
                 var r2 = el.classList.toggle('active'); \
                 '' + r1 + ',' + r2",
            )
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("true,false".into()));
    }

    #[test]
    fn classlist_contains() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.classList.add('yes'); \
                 '' + el.classList.contains('yes') + ',' + \
                 el.classList.contains('no')",
            )
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("true,false".into()));
    }

    // ---------------------------------------------------------------
    // style tests
    // ---------------------------------------------------------------

    #[test]
    fn style_set_and_get() {
        let (engine, shared) = setup(sample_doc());
        engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'red'); \
                 el.style.setProperty('font-size', '14px')",
            )
            .unwrap();
        let doc = shared.borrow();
        let main = doc.get_element_by_id("main").expect("main");
        let elem = doc.element(main).expect("elem");
        let style = elem.get_attribute("style").expect("style");
        assert!(style.contains("color: red"));
        assert!(style.contains("font-size: 14px"));
    }

    #[test]
    fn style_get_property_value() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'blue'); \
                 el.style.getPropertyValue('color')",
            )
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("blue".into()));
    }

    #[test]
    fn style_overwrite_property() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine
            .eval(
                "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'red'); \
                 el.style.setProperty('color', 'green'); \
                 el.style.getPropertyValue('color')",
            )
            .unwrap();
        assert_eq!(val, oasis_js::JsValue::String("green".into()));
    }

    // ---------------------------------------------------------------
    // window.location tests
    // ---------------------------------------------------------------

    #[test]
    fn location_href_default() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine.eval("location.href").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("".into()));
    }

    #[test]
    fn location_href_with_url() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let doc = sample_doc();
        let shared: SharedDoc = Rc::new(RefCell::new(doc));
        let s = Rc::clone(&shared);
        engine
            .with_context(|ctx| {
                install_document_global_with_url(&ctx, &s, "https://example.com/page")
            })
            .unwrap();
        let val = engine.eval("location.href").unwrap();
        assert_eq!(
            val,
            oasis_js::JsValue::String("https://example.com/page".into())
        );
    }

    #[test]
    fn window_is_global_this() {
        let (engine, _doc) = setup(sample_doc());
        let val = engine.eval("window === globalThis").unwrap();
        assert_eq!(val, oasis_js::JsValue::Bool(true));
    }

    // ---------------------------------------------------------------
    // Navigation action tests
    // ---------------------------------------------------------------

    /// Helper: create engine + shared doc + nav actions queue.
    fn setup_with_nav(doc: Document, url: &str) -> (JsEngine, SharedDoc, SharedNavActions) {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let shared: SharedDoc = Rc::new(RefCell::new(doc));
        let nav_actions: SharedNavActions = Rc::new(RefCell::new(Vec::new()));
        let s = Rc::clone(&shared);
        let n = Rc::clone(&nav_actions);
        engine
            .with_context(|ctx| install_document_global_with_nav(&ctx, &s, url, &n))
            .unwrap();
        (engine, shared, nav_actions)
    }

    #[test]
    fn location_assign_queues_navigate() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("location.assign('https://other.com')").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            JsNavAction::Navigate("https://other.com".into())
        );
    }

    #[test]
    fn location_href_setter_queues_navigate() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("location.href = 'https://new.com'").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], JsNavAction::Navigate("https://new.com".into()));
    }

    #[test]
    fn location_replace_queues_navigate() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine
            .eval("location.replace('https://replaced.com')")
            .unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            JsNavAction::Navigate("https://replaced.com".into())
        );
    }

    #[test]
    fn history_back_queues_action() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.back()").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], JsNavAction::Back);
    }

    #[test]
    fn history_forward_queues_action() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.forward()").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], JsNavAction::Forward);
    }

    #[test]
    fn history_go_negative_is_back() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.go(-1)").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], JsNavAction::Back);
    }

    #[test]
    fn history_go_positive_is_forward() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.go(1)").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], JsNavAction::Forward);
    }

    #[test]
    fn history_go_zero_reloads() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.go(0)").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            JsNavAction::Navigate("https://example.com".into())
        );
    }

    #[test]
    fn location_href_getter_with_nav() {
        let (engine, _doc, _nav) = setup_with_nav(sample_doc(), "https://example.com/page");
        let val = engine.eval("location.href").unwrap();
        assert_eq!(
            val,
            oasis_js::JsValue::String("https://example.com/page".into())
        );
    }

    #[test]
    fn location_tostring() {
        let (engine, _doc, _nav) = setup_with_nav(sample_doc(), "https://example.com");
        let val = engine.eval("location.toString()").unwrap();
        assert_eq!(val, oasis_js::JsValue::String("https://example.com".into()));
    }

    #[test]
    fn drain_nav_actions_clears_queue() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine.eval("history.back()").unwrap();
        engine.eval("history.forward()").unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 2);
        // Second drain should be empty.
        let actions2 = drain_nav_actions(&nav);
        assert!(actions2.is_empty());
    }

    #[test]
    fn window_location_assign_works() {
        let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
        engine
            .eval("window.location.assign('https://via-window.com')")
            .unwrap();
        let actions = drain_nav_actions(&nav);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            JsNavAction::Navigate("https://via-window.com".into())
        );
    }
}

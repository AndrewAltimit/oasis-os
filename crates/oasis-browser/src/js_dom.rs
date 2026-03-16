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

use std::cell::RefCell;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};

/// Shared, interior-mutable document used during JS execution.
pub type SharedDoc = Rc<RefCell<Document>>;

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
    install_document_global_full(ctx, doc, "", &nav)
}

/// Like [`install_document_global`] but accepts an explicit URL for
/// `window.location`.
#[cfg(test)]
pub fn install_document_global_with_url(ctx: &Ctx<'_>, doc: &SharedDoc, url: &str) -> JsResult<()> {
    let nav = Rc::new(RefCell::new(Vec::new()));
    install_document_global_full(ctx, doc, url, &nav)
}

/// Like [`install_document_global_with_url`] but also accepts a shared
/// navigation action queue that JS `location.assign()` /
/// `history.back()` / `history.forward()` will push to.
pub fn install_document_global_with_nav(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
) -> JsResult<()> {
    install_document_global_full(ctx, doc, url, nav_actions)
}

/// Full installation: document global, location/history, nav actions.
fn install_document_global_full(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    url: &str,
    nav_actions: &SharedNavActions,
) -> JsResult<()> {
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
        globals.set(
            "__oasis_setattr",
            Function::new(ctx.clone(), move |nid: i32, name: String, value: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    e.set_attribute(&name, &value);
                }
            })?,
        )?;
    }

    // -- __oasis_rmattr(nid, name) -> bool ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_rmattr",
            Function::new(ctx.clone(), move |nid: i32, name: String| -> bool {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    return e.remove_attribute(&name);
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
        globals.set(
            "__oasis_settext",
            Function::new(ctx.clone(), move |nid: i32, text: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len() {
                    doc.set_text_content(id, &text);
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
            Function::new(ctx.clone(), move |parent_nid: i32, child_nid: i32| {
                let mut doc = d.borrow_mut();
                let pid = parent_nid as NodeId;
                let cid = child_nid as NodeId;
                if pid < doc.nodes.len() && cid < doc.nodes.len() {
                    doc.remove_child(cid);
                    doc.append_child(pid, cid);
                }
            })?,
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
        globals.set(
            "__oasis_settitle",
            Function::new(ctx.clone(), move |val: String| {
                let mut doc = d.borrow_mut();
                if let Some(tid) = doc.title_element() {
                    doc.set_text_content(tid, &val);
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
        globals.set(
            "__oasis_set_inner_html",
            Function::new(ctx.clone(), move |nid: i32, html: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return;
                }
                // Clear existing children.
                let old: Vec<NodeId> = doc.nodes[id].children.clone();
                for child_id in old {
                    doc.nodes[child_id].parent = None;
                }
                doc.nodes[id].children.clear();

                // Parse the fragment via the tokenizer +
                // tree builder, then transplant body children.
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
                let parsed = parse_simple_selector(&sel);
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
                let parsed = parse_simple_selector(&sel);
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
                    classlist_op(e, &op, &cls)
                },
            )?,
        )?;
    }

    // -- __oasis_style_set(nid, prop, value) --------------------------
    {
        let d = Rc::clone(doc);
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
                set_inline_style(e, &prop, &value);
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

    // -- __oasis_location() -> String ---------------------------------
    {
        let url_owned = url.to_string();
        globals.set(
            "__oasis_location",
            Function::new(ctx.clone(), move || -> String { url_owned.clone() })?,
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

    // -- JavaScript-side Element class + document global ---------------
    let _: () = ctx.eval(JS_DOM_BOOTSTRAP)?;

    Ok(())
}

/// Drain and return all pending navigation actions from the queue.
pub fn drain_nav_actions(nav: &SharedNavActions) -> Vec<JsNavAction> {
    std::mem::take(&mut nav.borrow_mut())
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
// Simple CSS selector matching for querySelector
// ------------------------------------------------------------------

/// A parsed simple selector for querySelector matching.
struct SimpleSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

/// Parse a simple CSS selector string into its components.
///
/// Supports: tag, .class, #id, and combinations like `div.foo#bar`.
fn parse_simple_selector(sel: &str) -> SimpleSelector {
    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();

    let sel = sel.trim();
    if sel.is_empty() {
        return SimpleSelector { tag, id, classes };
    }

    // Split on '#' and '.' boundaries while preserving delimiters.
    let mut tokens: Vec<(char, String)> = Vec::new();
    let mut current = String::new();
    let mut kind = 't'; // 't' = tag, '#' = id, '.' = class
    for ch in sel.chars() {
        if ch == '#' || ch == '.' {
            if !current.is_empty() {
                tokens.push((kind, current.clone()));
                current.clear();
            }
            kind = ch;
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push((kind, current));
    }

    for (k, val) in tokens {
        match k {
            't' => tag = Some(val.to_ascii_lowercase()),
            '#' => id = Some(val),
            '.' => classes.push(val),
            _ => {},
        }
    }

    SimpleSelector { tag, id, classes }
}

/// Test whether an element matches a parsed simple selector.
fn matches_simple_sel(elem: &ElementData, sel: &SimpleSelector) -> bool {
    if let Some(ref t) = sel.tag
        && !elem.tag.as_str().eq_ignore_ascii_case(t)
    {
        return false;
    }
    if let Some(ref sel_id) = sel.id
        && elem.id() != Some(sel_id.as_str())
    {
        return false;
    }
    for cls in &sel.classes {
        if !elem.has_class(cls) {
            return false;
        }
    }
    true
}

/// Walk the subtree rooted at `root` (excluding `root` itself)
/// and collect matching element node IDs. If `first_only` is true,
/// stop after the first match.
fn find_matching(
    doc: &Document,
    root: NodeId,
    sel: &SimpleSelector,
    first_only: bool,
) -> Vec<NodeId> {
    let mut results = Vec::new();
    let mut stack: Vec<NodeId> = doc.nodes[root].children.clone();
    // Reverse so we process in document order (left to right).
    stack.reverse();
    while let Some(nid) = stack.pop() {
        if let NodeKind::Element(ref e) = doc.nodes[nid].kind
            && matches_simple_sel(e, sel)
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
fn classlist_op(elem: &mut ElementData, op: &str, cls: &str) -> bool {
    let current = elem.get_attribute("class").unwrap_or("").to_string();
    let mut parts: Vec<String> = current.split_ascii_whitespace().map(String::from).collect();

    match op {
        "add" => {
            if !parts.iter().any(|c| c == cls) {
                parts.push(cls.to_string());
            }
            elem.set_attribute("class", &parts.join(" "));
            true
        },
        "remove" => {
            parts.retain(|c| c != cls);
            elem.set_attribute("class", &parts.join(" "));
            false
        },
        "toggle" => {
            let had = parts.iter().any(|c| c == cls);
            if had {
                parts.retain(|c| c != cls);
            } else {
                parts.push(cls.to_string());
            }
            elem.set_attribute("class", &parts.join(" "));
            !had
        },
        "contains" => parts.iter().any(|c| c == cls),
        _ => false,
    }
}

// ------------------------------------------------------------------
// Inline style helpers
// ------------------------------------------------------------------

/// Set a CSS property in the element's `style` attribute.
fn set_inline_style(elem: &mut ElementData, prop: &str, value: &str) {
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
    elem.set_attribute("style", &rebuilt);
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
        var self = this;
        return {
          setProperty: function(p, v) {
            __oasis_style_set(
              self.__oasis_node_id, p, String(v)
            );
          },
          getPropertyValue: function(p) {
            return __oasis_style_get(self.__oasis_node_id, p);
          }
        };
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
  var __oasis_listeners = {};

  Element.prototype.addEventListener = function(type, fn) {
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    if (!__oasis_listeners[key]) __oasis_listeners[key] = [];
    __oasis_listeners[key].push(fn);
  };
  Element.prototype.removeEventListener = function(type, fn) {
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (arr[i] === fn) { arr.splice(i, 1); return; }
    }
  };
  Element.prototype.dispatchEvent = function(evt) {
    var nid = this.__oasis_node_id;
    var key = nid + ":" + evt.type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    evt.target = this;
    for (var i = 0; i < arr.length; i++) arr[i].call(this, evt);
  };

  // Expose dispatch helper for Rust-side event triggering.
  globalThis.__oasis_dispatch_event =
    function(nid, type, detail) {
      var key = nid + ":" + type;
      var arr = __oasis_listeners[key];
      if (!arr || arr.length === 0) return;
      var el = new Element(nid);
      var evt = {
        type: type, target: el, detail: detail || null
      };
      for (var i = 0; i < arr.length; i++)
        arr[i].call(el, evt);
    };

  // Dispatch with bubbling: walk from target up to root.
  globalThis.__oasis_dispatch_with_bubbling =
    function(nid, type, detail) {
      var target = new Element(nid);
      var evt = {
        type: type,
        detail: detail || null,
        target: target,
        currentTarget: null,
        _stopped: false,
        stopPropagation: function() {
          this._stopped = true;
        },
        preventDefault: function() {
          this._defaultPrevented = true;
        },
        _defaultPrevented: false
      };
      var current = nid;
      while (current >= 0 && !evt._stopped) {
        var key = current + ":" + type;
        var arr = __oasis_listeners[key];
        if (arr) {
          evt.currentTarget = new Element(current);
          for (var i = 0; i < arr.length; i++) {
            arr[i].call(evt.currentTarget, evt);
          }
        }
        current = __oasis_parent(current);
      }
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
    back: function() { __oasis_history_back(); },
    forward: function() { __oasis_history_forward(); },
    go: function(delta) {
      if (delta < 0) __oasis_history_back();
      else if (delta > 0) __oasis_history_forward();
      else __oasis_location_assign(__oasis_location());
    },
    get length() { return 1; }
  };
})();
"#;

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

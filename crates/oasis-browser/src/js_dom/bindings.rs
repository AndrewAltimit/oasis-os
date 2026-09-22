//! Rust-side DOM bindings: node access/mutation, navigation,
//! `location`, and `getComputedStyle()`.

use std::cell::RefCell;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use super::serialize::{deep_copy_node, serialize_node};
use super::{
    JsNavAction, NO_NODE, SharedDirty, SharedDoc, SharedNavActions, SharedStyles, mark_dirty,
};
use crate::css::values::ComputedStyle;
use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};

/// Install the `__oasis_*` node-level helpers (tag name, attributes,
/// text, tree walking/mutation, innerHTML, selectors, classList, inline
/// style).
pub(super) fn install_dom_bindings(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    dirty: &Option<SharedDirty>,
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

    Ok(())
}

/// Install `location.assign()` / `history.back()` / `history.forward()`
/// helpers that push onto the shared navigation queue.
pub(super) fn install_nav_bindings(ctx: &Ctx<'_>, nav_actions: &SharedNavActions) -> JsResult<()> {
    let globals = ctx.globals();

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

    Ok(())
}

/// Install `__oasis_computed_style` backing `getComputedStyle()`.
pub(super) fn install_computed_style_binding(
    ctx: &Ctx<'_>,
    styles: Option<&SharedStyles>,
) -> JsResult<()> {
    let globals = ctx.globals();

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

    Ok(())
}

/// Install `__oasis_location` / `__oasis_location_push` (the latter
/// used by `history.pushState` URL updates).
pub(super) fn install_location_bindings(ctx: &Ctx<'_>, url: &str) -> JsResult<()> {
    let globals = ctx.globals();

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

    Ok(())
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

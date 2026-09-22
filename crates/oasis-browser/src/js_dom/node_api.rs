//! Node-level DOM helpers behind the fuller DOM API surface: node
//! types, text-inclusive child lists, raw parent links, detached-safe
//! insertion (fragments, cycle checks), cloning, fragment parsing,
//! selector matching and `getElementsBy*` lookups.
//!
//! Arena slots freed by the bindings (`innerHTML` / `textContent`
//! writes) are recorded in a [`FreedLog`] that the JS side drains to
//! evict its per-node-id wrapper cache and listener tables, so a reused
//! slot never resolves to a stale wrapper.

use std::cell::RefCell;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use super::serialize::{deep_copy_node, serialize_node};
use super::{NO_NODE, SharedDirty, SharedDoc, mark_dirty};
use crate::html::dom::{Document, NodeId, NodeKind};

/// Node ids whose arena slots were freed by a binding since the JS side
/// last drained the log (via `__oasis_take_freed`).
pub(super) type FreedLog = Rc<RefCell<Vec<NodeId>>>;

/// `Node.nodeType` values.
const ELEMENT_NODE: i32 = 1;
const TEXT_NODE: i32 = 3;
const COMMENT_NODE: i32 = 8;
const DOCUMENT_NODE: i32 = 9;
const DOCUMENT_FRAGMENT_NODE: i32 = 11;

/// Insertion result codes returned to JS (`0` = success).
pub(super) const INSERT_OK: i32 = 0;
/// The insertion would create a cycle or move the document root.
pub(super) const INSERT_HIERARCHY_ERROR: i32 = -1;
/// A node id was out of range.
pub(super) const INSERT_INVALID: i32 = -2;

/// Convert a JS-supplied node id into a valid arena index.
fn valid(doc: &Document, nid: i32) -> Option<NodeId> {
    let id = usize::try_from(nid).ok()?;
    (id < doc.nodes.len()).then_some(id)
}

/// `nodeType` for a node. Detached `Document`-kind nodes other than the
/// root are document fragments.
pub(super) fn node_type(doc: &Document, id: NodeId) -> i32 {
    match doc.nodes[id].kind {
        NodeKind::Element(_) => ELEMENT_NODE,
        NodeKind::Text(_) => TEXT_NODE,
        NodeKind::Comment(_) => COMMENT_NODE,
        NodeKind::Document if id == doc.root => DOCUMENT_NODE,
        NodeKind::Document => DOCUMENT_FRAGMENT_NODE,
    }
}

/// Whether `id` is reachable from the document root.
pub(super) fn is_connected(doc: &Document, id: NodeId) -> bool {
    let mut cur = id;
    // Bounded walk: a well-formed tree is never deeper than the arena.
    for _ in 0..=doc.nodes.len() {
        if cur == doc.root {
            return true;
        }
        match doc.nodes[cur].parent {
            Some(p) => cur = p,
            None => return false,
        }
    }
    false
}

/// Whether `ancestor` is `node` or one of its ancestors.
fn is_inclusive_ancestor(doc: &Document, ancestor: NodeId, node: NodeId) -> bool {
    let mut cur = Some(node);
    let mut steps = 0;
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        steps += 1;
        if steps > doc.nodes.len() {
            return false;
        }
        cur = doc.nodes[c].parent;
    }
    false
}

/// Insert `node` into `parent` before `reference` (append when
/// `reference` is `None` or not a child of `parent`). The node is
/// detached from its old parent first — without freeing its subtree —
/// and a document fragment contributes its children instead of itself.
pub(super) fn insert_node(
    doc: &mut Document,
    parent: NodeId,
    node: NodeId,
    reference: Option<NodeId>,
) -> i32 {
    if node == doc.root || is_inclusive_ancestor(doc, node, parent) {
        return INSERT_HIERARCHY_ERROR;
    }
    // Inserting a node before itself means "before its next sibling".
    let mut reference = reference;
    if reference == Some(node) {
        reference = next_sibling(doc, node);
    }
    let moving: Vec<NodeId> = if node_type(doc, node) == DOCUMENT_FRAGMENT_NODE {
        doc.nodes[node].children.clone()
    } else {
        vec![node]
    };
    for n in moving {
        doc.detach_node(n);
        let pos = reference.and_then(|r| doc.nodes[parent].children.iter().position(|&c| c == r));
        match pos {
            Some(idx) => {
                doc.nodes[parent].children.insert(idx, n);
                doc.nodes[n].parent = Some(parent);
            },
            None => doc.append_child(parent, n),
        }
    }
    INSERT_OK
}

/// The node following `id` in its parent's child list.
fn next_sibling(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.nodes[id].parent?;
    let kids = &doc.nodes[parent].children;
    let pos = kids.iter().position(|&c| c == id)?;
    kids.get(pos + 1).copied()
}

/// Copy `id` (and its subtree when `deep`) into fresh arena slots of the
/// same document, returning the detached copy.
pub(super) fn clone_node(doc: &mut Document, id: NodeId, deep: bool) -> NodeId {
    let kind = doc.nodes[id].kind.clone();
    let new_id = doc.add_node(kind);
    if deep {
        let kids = doc.nodes[id].children.clone();
        for kid in kids {
            let copy = clone_node(doc, kid, true);
            doc.append_child(new_id, copy);
        }
    }
    new_id
}

/// Free every child subtree of `id`, recording each freed slot in `log`.
pub(super) fn free_children_logged(doc: &mut Document, id: NodeId, log: &FreedLog) {
    let kids = std::mem::take(&mut doc.nodes[id].children);
    let mut log = log.borrow_mut();
    for kid in kids {
        collect_subtree(doc, kid, &mut log);
        doc.free_subtree(kid);
    }
}

/// Push `id` and all of its descendants onto `out`.
fn collect_subtree(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(doc.nodes[n].children.iter().copied());
    }
}

/// Parse `html` as a body fragment and deep-copy its nodes under
/// `parent` (appended in order).
pub(super) fn parse_into(doc: &mut Document, parent: NodeId, html: &str) {
    use crate::html::tokenizer::Tokenizer;
    use crate::html::tree_builder::TreeBuilder;
    let wrapped = format!("<html><body>{html}</body></html>");
    let tokens = Tokenizer::new(&wrapped).tokenize();
    let frag = TreeBuilder::build(tokens);
    let src_children: Vec<NodeId> = frag
        .body()
        .map(|b| frag.nodes[b].children.clone())
        .unwrap_or_default();
    for &src_child in &src_children {
        let new_id = deep_copy_node(&frag, doc, src_child);
        doc.append_child(parent, new_id);
    }
}

/// Descendant elements of `root` (document order, excluding `root`)
/// accepted by `pred`.
fn descendants_where(
    doc: &Document,
    root: NodeId,
    mut pred: impl FnMut(&crate::html::dom::ElementData) -> bool,
) -> Vec<i32> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.nodes[root].children.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        if let NodeKind::Element(e) = &doc.nodes[n].kind
            && pred(e)
        {
            out.push(n as i32);
        }
        stack.extend(doc.nodes[n].children.iter().rev().copied());
    }
    out
}

/// `getElementById` restricted to connected elements: the id index may
/// point at a detached node (removed or a clone), in which case fall
/// back to a document-order search of the live tree.
pub(super) fn connected_element_by_id(doc: &Document, target: &str) -> Option<NodeId> {
    if let Some(id) = doc.get_element_by_id(target)
        && is_connected(doc, id)
        && doc.element(id).and_then(|e| e.id()) == Some(target)
    {
        return Some(id);
    }
    if target.is_empty() {
        return None;
    }
    descendants_where(doc, doc.root, |e| e.id() == Some(target))
        .first()
        .map(|&n| n as NodeId)
}

/// Install the node-level `__oasis_*` helpers described in the module
/// docs.
pub(super) fn install_node_bindings(
    ctx: &Ctx<'_>,
    doc: &SharedDoc,
    dirty: &Option<SharedDirty>,
    freed: &FreedLog,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_root() -> i32 ----------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_root",
            Function::new(ctx.clone(), move || -> i32 { d.borrow().root as i32 })?,
        )?;
    }

    // -- __oasis_head() -> i32 ----------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_head",
            Function::new(ctx.clone(), move || -> i32 {
                d.borrow().head().map_or(NO_NODE, |n| n as i32)
            })?,
        )?;
    }

    // -- __oasis_node_type(nid) -> i32 (0 = invalid) ------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_node_type",
            Function::new(ctx.clone(), move |nid: i32| -> i32 {
                let doc = d.borrow();
                valid(&doc, nid).map_or(0, |id| node_type(&doc, id))
            })?,
        )?;
    }

    // -- __oasis_child_nodes(nid) -> Vec<i32> (all node kinds) --------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_child_nodes",
            Function::new(ctx.clone(), move |nid: i32| -> Vec<i32> {
                let doc = d.borrow();
                valid(&doc, nid).map_or_else(Vec::new, |id| {
                    doc.nodes[id].children.iter().map(|&c| c as i32).collect()
                })
            })?,
        )?;
    }

    // -- __oasis_parent_node(nid) -> i32 (any parent kind) -----------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_parent_node",
            Function::new(ctx.clone(), move |nid: i32| -> i32 {
                let doc = d.borrow();
                valid(&doc, nid)
                    .and_then(|id| doc.nodes[id].parent)
                    .map_or(NO_NODE, |p| p as i32)
            })?,
        )?;
    }

    // -- __oasis_sibling(nid, dir, elements_only) -> i32 --------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_sibling",
            Function::new(
                ctx.clone(),
                move |nid: i32, dir: i32, elements_only: bool| -> i32 {
                    let doc = d.borrow();
                    let Some(id) = valid(&doc, nid) else {
                        return NO_NODE;
                    };
                    let Some(parent) = doc.nodes[id].parent else {
                        return NO_NODE;
                    };
                    let kids = &doc.nodes[parent].children;
                    let Some(pos) = kids.iter().position(|&c| c == id) else {
                        return NO_NODE;
                    };
                    let ok = |c: NodeId| {
                        !elements_only || matches!(doc.nodes[c].kind, NodeKind::Element(_))
                    };
                    let found = if dir < 0 {
                        kids[..pos].iter().rev().copied().find(|&c| ok(c))
                    } else {
                        kids[pos + 1..].iter().copied().find(|&c| ok(c))
                    };
                    found.map_or(NO_NODE, |c| c as i32)
                },
            )?,
        )?;
    }

    // -- __oasis_is_connected(nid) -> bool ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_is_connected",
            Function::new(ctx.clone(), move |nid: i32| -> bool {
                let doc = d.borrow();
                valid(&doc, nid).is_some_and(|id| is_connected(&doc, id))
            })?,
        )?;
    }

    // -- __oasis_node_value(nid) -> String|undefined ------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_node_value",
            Function::new(ctx.clone(), move |nid: i32| -> Option<String> {
                let doc = d.borrow();
                match &doc.nodes[valid(&doc, nid)?].kind {
                    NodeKind::Text(s) | NodeKind::Comment(s) => Some(s.clone()),
                    _ => None,
                }
            })?,
        )?;
    }

    // -- __oasis_set_node_value(nid, text) -----------------------------
    {
        let d = Rc::clone(doc);
        let dirty = dirty.clone();
        globals.set(
            "__oasis_set_node_value",
            Function::new(ctx.clone(), move |nid: i32, text: String| {
                let mut doc = d.borrow_mut();
                let Some(id) = valid(&doc, nid) else {
                    return;
                };
                if let NodeKind::Text(s) | NodeKind::Comment(s) = &mut doc.nodes[id].kind
                    && *s != text
                {
                    *s = text;
                    mark_dirty(&dirty);
                }
            })?,
        )?;
    }

    // -- __oasis_attrs(nid) -> Vec<String> [name0, value0, ...] -------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_attrs",
            Function::new(ctx.clone(), move |nid: i32| -> Vec<String> {
                let doc = d.borrow();
                let Some(e) = valid(&doc, nid).and_then(|id| doc.element(id)) else {
                    return Vec::new();
                };
                e.attributes
                    .iter()
                    .flat_map(|a| [a.name.clone(), a.value.clone()])
                    .collect()
            })?,
        )?;
    }

    // -- __oasis_create_comment(text) / __oasis_create_fragment() -----
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_create_comment",
            Function::new(ctx.clone(), move |text: String| -> i32 {
                d.borrow_mut().add_node(NodeKind::Comment(text)) as i32
            })?,
        )?;
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_create_fragment",
            Function::new(ctx.clone(), move || -> i32 {
                d.borrow_mut().add_node(NodeKind::Document) as i32
            })?,
        )?;
    }

    // -- __oasis_parse_fragment(html) -> i32 (detached fragment) ------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_parse_fragment",
            Function::new(ctx.clone(), move |html: String| -> i32 {
                let mut doc = d.borrow_mut();
                let frag = doc.add_node(NodeKind::Document);
                parse_into(&mut doc, frag, &html);
                frag as i32
            })?,
        )?;
    }

    // -- __oasis_discard_fragment(nid) --------------------------------
    // Free an emptied, detached scratch fragment (insertAdjacentHTML /
    // outerHTML) so repeated calls don't grow the arena.
    {
        let d = Rc::clone(doc);
        let f = Rc::clone(freed);
        globals.set(
            "__oasis_discard_fragment",
            Function::new(ctx.clone(), move |nid: i32| {
                let mut doc = d.borrow_mut();
                if let Some(id) = valid(&doc, nid)
                    && node_type(&doc, id) == DOCUMENT_FRAGMENT_NODE
                    && doc.nodes[id].parent.is_none()
                    && doc.nodes[id].children.is_empty()
                {
                    doc.free_subtree(id);
                    f.borrow_mut().push(id);
                }
            })?,
        )?;
    }

    // -- __oasis_clone(nid, deep) -> i32 ------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_clone",
            Function::new(ctx.clone(), move |nid: i32, deep: bool| -> i32 {
                let mut doc = d.borrow_mut();
                match valid(&doc, nid) {
                    Some(id) if id != doc.root => clone_node(&mut doc, id, deep) as i32,
                    _ => NO_NODE,
                }
            })?,
        )?;
    }

    // -- __oasis_outer_html(nid) -> String ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_outer_html",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let mut out = String::new();
                if let Some(id) = valid(&doc, nid) {
                    serialize_node(&doc, id, &mut out);
                }
                out
            })?,
        )?;
    }

    // -- __oasis_matches(nid, sel) -> i32 (1 / 0 / -1 bad selector) ---
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_matches",
            Function::new(ctx.clone(), move |nid: i32, sel: String| -> i32 {
                use crate::css::cascade::CascadeContext;
                use crate::css::cascade::matching::matches_selector;
                let doc = d.borrow();
                let Some(parsed) = crate::css::parser::parse_selector_string(&sel) else {
                    return -1;
                };
                let Some(id) = valid(&doc, nid) else {
                    return 0;
                };
                if !matches!(doc.nodes[id].kind, NodeKind::Element(_)) {
                    return 0;
                }
                let cctx = CascadeContext::default();
                i32::from(
                    parsed
                        .selectors
                        .iter()
                        .any(|s| matches_selector(&doc, id, s, &cctx)),
                )
            })?,
        )?;
    }

    // -- __oasis_by_tag(nid, tag) -> Vec<i32> -------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_by_tag",
            Function::new(ctx.clone(), move |nid: i32, tag: String| -> Vec<i32> {
                let doc = d.borrow();
                let Some(id) = valid(&doc, nid) else {
                    return Vec::new();
                };
                let all = tag == "*";
                descendants_where(&doc, id, |e| {
                    all || e.tag.as_str().eq_ignore_ascii_case(&tag)
                })
            })?,
        )?;
    }

    // -- __oasis_by_class(nid, names) -> Vec<i32> ---------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_by_class",
            Function::new(ctx.clone(), move |nid: i32, names: String| -> Vec<i32> {
                let doc = d.borrow();
                let Some(id) = valid(&doc, nid) else {
                    return Vec::new();
                };
                let wanted: Vec<&str> = names.split_ascii_whitespace().collect();
                if wanted.is_empty() {
                    return Vec::new();
                }
                descendants_where(&doc, id, |e| wanted.iter().all(|c| e.has_class(c)))
            })?,
        )?;
    }

    // -- __oasis_take_freed() -> Vec<i32> ------------------------------
    {
        let f = Rc::clone(freed);
        globals.set(
            "__oasis_take_freed",
            Function::new(ctx.clone(), move || -> Vec<i32> {
                std::mem::take(&mut *f.borrow_mut())
                    .into_iter()
                    .map(|n| n as i32)
                    .collect()
            })?,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::dom::{ElementData, TagName};

    fn el(doc: &mut Document, tag: TagName) -> NodeId {
        doc.add_node(NodeKind::Element(ElementData::new(tag)))
    }

    #[test]
    fn insert_rejects_cycles_and_moves_without_freeing() {
        let mut doc = Document::new();
        let a = el(&mut doc, TagName::Div);
        doc.append_child(doc.root, a);
        let b = el(&mut doc, TagName::P);
        doc.append_child(a, b);
        let t = doc.add_node(NodeKind::Text("x".into()));
        doc.append_child(b, t);
        assert_eq!(insert_node(&mut doc, b, a, None), INSERT_HIERARCHY_ERROR);
        let c = el(&mut doc, TagName::Span);
        doc.append_child(doc.root, c);
        assert_eq!(insert_node(&mut doc, c, b, None), INSERT_OK);
        assert_eq!(doc.nodes[c].children, vec![b]);
        // The moved subtree keeps its text child.
        assert_eq!(doc.nodes[b].children, vec![t]);
        assert_eq!(doc.text_content(c), "x");
    }

    #[test]
    fn fragment_insertion_moves_children() {
        let mut doc = Document::new();
        let host = el(&mut doc, TagName::Div);
        doc.append_child(doc.root, host);
        let frag = doc.add_node(NodeKind::Document);
        parse_into(&mut doc, frag, "<b>1</b><i>2</i>");
        assert_eq!(node_type(&doc, frag), DOCUMENT_FRAGMENT_NODE);
        assert_eq!(insert_node(&mut doc, host, frag, None), INSERT_OK);
        assert!(doc.nodes[frag].children.is_empty());
        assert_eq!(doc.nodes[host].children.len(), 2);
        assert_eq!(doc.text_content(host), "12");
    }

    #[test]
    fn connected_lookup_skips_detached_clone() {
        let mut doc = Document::new();
        let mut data = ElementData::new(TagName::Div);
        data.set_attribute("id", "x");
        let orig = doc.add_node(NodeKind::Element(data));
        doc.append_child(doc.root, orig);
        let copy = clone_node(&mut doc, orig, true);
        assert_ne!(copy, orig);
        assert_eq!(connected_element_by_id(&doc, "x"), Some(orig));
    }
}

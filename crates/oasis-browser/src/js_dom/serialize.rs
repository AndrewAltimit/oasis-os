//! `innerHTML` serialization and the depth-capped fragment deep copy.

use crate::html::dom::{Document, NodeId, NodeKind};
use crate::html::escape::push_escaped;

/// Serialize a DOM node (and its subtree) to an HTML string.
pub(super) fn serialize_node(doc: &Document, id: NodeId, out: &mut String) {
    match &doc.nodes[id].kind {
        NodeKind::Text(s) => {
            push_escaped(out, s);
        },
        NodeKind::Element(e) => {
            let tag = e.tag.as_str();
            out.push('<');
            out.push_str(tag);
            for attr in &e.attributes {
                out.push(' ');
                out.push_str(&attr.name);
                out.push_str("=\"");
                push_escaped(out, &attr.value);
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

/// Deep-copy `src_id` (and its subtree) from `src` into `dst`, appended
/// as the last child of `dst_parent`, using at most `levels` levels below
/// `dst_parent` (the copy of `src_id` itself is level 1).
///
/// Content deeper than that is flattened, like the parser does at its
/// nesting cap: once one level is left, the remaining descendants are
/// appended (in document order) as leaf siblings of the last copied
/// node instead of nesting further. With `levels == 0` nothing is
/// copied.
pub(super) fn deep_copy_capped(
    src: &Document,
    dst: &mut Document,
    src_id: NodeId,
    dst_parent: NodeId,
    levels: usize,
) {
    if levels == 0 {
        return;
    }
    let new_id = dst.add_node(src.nodes[src_id].kind.clone());
    dst.append_child(dst_parent, new_id);
    if levels > 1 {
        for &child_src in &src.nodes[src_id].children {
            deep_copy_capped(src, dst, child_src, new_id, levels - 1);
        }
        return;
    }
    // Last level: flatten the descendants into leaf siblings.
    let mut stack: Vec<NodeId> = src.nodes[src_id].children.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        let leaf = dst.add_node(src.nodes[n].kind.clone());
        dst.append_child(dst_parent, leaf);
        stack.extend(src.nodes[n].children.iter().rev().copied());
    }
}

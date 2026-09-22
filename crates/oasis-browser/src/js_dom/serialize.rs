//! `innerHTML` serialization and fragment deep-copy helpers.

use crate::html::dom::{Document, NodeId, NodeKind};

/// Serialize a DOM node (and its subtree) to an HTML string.
pub(super) fn serialize_node(doc: &Document, id: NodeId, out: &mut String) {
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
pub(super) fn escape_html(s: &str, out: &mut String) {
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
pub(super) fn deep_copy_node(src: &Document, dst: &mut Document, src_id: NodeId) -> NodeId {
    let new_id = dst.add_node(src.nodes[src_id].kind.clone());
    for &child_src in &src.nodes[src_id].children {
        let child_new = deep_copy_node(src, dst, child_src);
        dst.append_child(new_id, child_new);
    }
    new_id
}

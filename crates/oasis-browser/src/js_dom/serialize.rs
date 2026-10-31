//! `innerHTML` serialization and the depth-capped fragment deep copy.

use crate::html::dom::{Document, NodeId, NodeKind};
use crate::html::escape::push_escaped;

/// Elements whose text children serialize verbatim (HTML fragment
/// serialization): escaping them corrupted `style.innerHTML` round
/// trips (`div > p` came back as `div &gt; p`, which re-parses as is).
fn is_raw_text_element(tag: &str) -> bool {
    matches!(
        tag,
        "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext"
    )
}

/// Whether `name` can be written as a tag or attribute name without
/// changing the markup's structure. Scripts can create names the parser
/// never would (`createElement('img src=x onerror=f()')`); emitting them
/// raw would turn into real attributes when the HTML is parsed back.
fn is_serializable_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '/' | '='))
}

/// Serialize a DOM node (and its subtree) to an HTML string.
pub(super) fn serialize_node(doc: &Document, id: NodeId, out: &mut String) {
    match &doc.nodes[id].kind {
        NodeKind::Text(s) => {
            let raw = doc.nodes[id]
                .parent
                .and_then(|p| doc.element(p))
                .is_some_and(|e| is_raw_text_element(e.tag.as_str()));
            if raw {
                out.push_str(s);
            } else {
                push_escaped(out, s);
            }
        },
        NodeKind::Element(e) => {
            let tag = e.tag.as_str();
            if !is_serializable_name(tag) {
                // Keep the content, drop the unrepresentable element.
                for &child in &doc.nodes[id].children {
                    serialize_node(doc, child, out);
                }
                return;
            }
            out.push('<');
            out.push_str(tag);
            for attr in e
                .attributes
                .iter()
                .filter(|a| is_serializable_name(&a.name))
            {
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

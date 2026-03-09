//! CSS cascade and selector matching.
//!
//! Implements the CSS cascade algorithm: for each element in a DOM tree,
//! collect matching rules from all stylesheets, sort by specificity and
//! source order, then apply declarations to produce computed styles.
//!
//! ## Property resolution order
//!
//! For every DOM element the cascade resolves each CSS property by:
//!
//! 1. **Collect** -- gather all rules whose selector matches the element.
//! 2. **Sort** -- order matches by specificity (a, b, c) then source order.
//! 3. **Apply** -- winning declarations overwrite the `ComputedStyle`.
//! 4. **Inherit** -- unset *inherited* properties (color, font-size, etc.)
//!    fall through to the parent's computed value. Non-inherited properties
//!    revert to their CSS initial value instead.
//!
//! ## Specificity
//!
//! Each selector produces a three-component weight `(a, b, c)`:
//! - **a** = number of ID selectors (`#id`)
//! - **b** = number of class, attribute, and pseudo-class selectors
//! - **c** = number of type (tag) and pseudo-element selectors
//!
//! Higher tuples win; ties are broken by source order (later wins).
//!
//! ## Performance
//!
//! A [`SelectorIndex`] buckets rules by their rightmost (subject) simple
//! selector (ID > class > tag > universal) so that only a small subset of
//! rules is tested against each element.

mod index;
mod matching;
mod var_resolve;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use super::parser::{Declaration, Stylesheet};
use super::values::ComputedStyle;
use crate::html::dom::{Document, NodeId, NodeKind};

pub use index::SelectorIndex;

// -----------------------------------------------------------------------
// Cascade context (stateful pseudo-class state)
// -----------------------------------------------------------------------

/// Context for stateful pseudo-class matching during cascade.
///
/// Carries the current hover target and set of visited URLs so that
/// `:hover`, `:visited`, and `:link` pseudo-classes can be evaluated.
#[derive(Default)]
pub struct CascadeContext<'a> {
    /// The DOM node currently under the cursor (for `:hover`).
    /// `:hover` matches this node and all its ancestors.
    pub hover_node: Option<NodeId>,
    /// Set of visited URLs (for `:visited` / `:link` on `<a>` elements).
    pub visited_urls: Option<&'a HashSet<String>>,
}

// -----------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------

/// Style a DOM tree by applying stylesheets and inline styles.
///
/// Returns a `Vec` indexed by `NodeId`. Elements get `Some(style)`;
/// non-element nodes (text, comments, document root) get `None`.
pub fn style_tree(
    doc: &Document,
    stylesheets: &[&Stylesheet],
    inline_styles: &[(NodeId, Vec<Declaration>)],
    ctx: &CascadeContext<'_>,
) -> Vec<Option<ComputedStyle>> {
    // Build selector index for O(1) bucket lookups instead of O(rules).
    let index = SelectorIndex::build(stylesheets);

    // Build a HashMap for O(1) inline style lookups instead of O(n) per element.
    let inline_map: HashMap<NodeId, &[Declaration]> = inline_styles
        .iter()
        .map(|(nid, decls)| (*nid, decls.as_slice()))
        .collect();
    let mut styles: Vec<Option<ComputedStyle>> = vec![None; doc.nodes.len()];
    style_subtree(
        doc,
        doc.root,
        stylesheets,
        &index,
        &inline_map,
        &mut styles,
        ctx,
    );
    styles
}

/// Recursively compute styles depth-first so that children can inherit
/// from their (already-computed) parent.
fn style_subtree(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &HashMap<NodeId, &[Declaration]>,
    styles: &mut [Option<ComputedStyle>],
    ctx: &CascadeContext<'_>,
) {
    let node = &doc.nodes[node_id];

    // Only elements get computed styles.
    if let NodeKind::Element(_) = &node.kind {
        let parent_style = node.parent.and_then(|pid| styles[pid].as_ref());
        let style = compute_style(
            doc,
            node_id,
            parent_style,
            stylesheets,
            index,
            inline_map,
            ctx,
        );
        styles[node_id] = Some(style);
    }

    // Recurse into children. Iterate by index to avoid cloning the Vec.
    let num_children = doc.nodes[node_id].children.len();
    for i in 0..num_children {
        let child_id = doc.nodes[node_id].children[i];
        style_subtree(doc, child_id, stylesheets, index, inline_map, styles, ctx);
    }
}

/// Compute the final style for a single element.
pub fn compute_style(
    doc: &Document,
    node_id: NodeId,
    parent_style: Option<&ComputedStyle>,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &HashMap<NodeId, &[Declaration]>,
    ctx: &CascadeContext<'_>,
) -> ComputedStyle {
    // Start from inherited values if we have a parent, else defaults.
    let mut style = match parent_style {
        Some(parent) => ComputedStyle::inherit(parent),
        None => ComputedStyle::default(),
    };

    let parent_font_size = parent_style.map_or(super::values::ROOT_FONT_SIZE, |p| p.font_size);

    // Collect all matching declarations with their origin info.
    let mut matched =
        matching::collect_matched_declarations(doc, node_id, stylesheets, index, inline_map, ctx);

    // Sort by cascade order: specificity, then source order.
    // `!important` declarations come after normal ones.
    matched.sort_by(|a, b| {
        a.important
            .cmp(&b.important)
            .then_with(|| a.origin.cmp(&b.origin))
            .then_with(|| a.specificity.cmp(&b.specificity))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });

    // Pass 1: Apply custom property declarations (--*) to build the
    // properties map before resolving any var() references.
    for entry in &matched {
        if entry.property.starts_with("--") {
            style.apply_declaration(&entry.property, &entry.value, parent_font_size);
        }
    }

    // Pass 2: Apply font-size first so that em units in subsequent
    // properties resolve relative to the element's own computed
    // font-size (CSS spec: em in font-size uses parent, em in all
    // other properties uses the element's own font-size).
    for entry in &matched {
        if entry.property == "font-size" {
            let resolved = var_resolve::resolve_css_var(&entry.value, &style.custom_properties);
            style.apply_declaration("font-size", &resolved, parent_font_size);
        }
    }
    let element_font_size = style.font_size;

    // Pass 3: Resolve var() references and apply all other declarations.
    for entry in &matched {
        if entry.property.starts_with("--") || entry.property == "font-size" {
            continue;
        }
        let resolved = var_resolve::resolve_css_var(&entry.value, &style.custom_properties);
        style.apply_declaration(&entry.property, &resolved, element_font_size);
    }

    // Resolve ::before and ::after pseudo-element content and styles.
    let before_ps =
        matching::resolve_pseudo_style(doc, node_id, "before", &style, stylesheets, ctx);
    if let Some(ref ps) = before_ps {
        style.before_content = ps.content.clone();
    }
    style.before_style = before_ps.map(Box::new);

    let after_ps = matching::resolve_pseudo_style(doc, node_id, "after", &style, stylesheets, ctx);
    if let Some(ref ps) = after_ps {
        style.after_content = ps.content.clone();
    }
    style.after_style = after_ps.map(Box::new);

    style
}

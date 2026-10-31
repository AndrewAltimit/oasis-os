//! Ancestor Bloom filter: reject descendant selectors without walking
//! the ancestor chain.
//!
//! For a selector such as `.sidebar ul > li a`, every compound reached
//! from the subject through child / descendant combinators only (`li`,
//! `ul`, `.sidebar`) must match some ancestor of the element. Their
//! type / id / class keys are hashed into a 256-bit [`AncestorMask`] when
//! the selector index is built; an element's ancestors' keys are hashed
//! into the same kind of filter once per element. A selector whose mask
//! has a bit the element's filter lacks cannot match and is skipped
//! before the (backtracking) matcher runs -- most rules on real pages
//! fail exactly this way. The filter is conservative: it never rejects a
//! selector that could match (a Bloom filter has no false negatives).

use std::hash::Hasher;

use oasis_html::dom::{Document, NodeId, NodeKind};
use rustc_hash::FxHasher;

use crate::parser::{Combinator, Selector, SimpleSelector};

/// 256 bits; two per key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AncestorMask([u64; 4]);

impl AncestorMask {
    fn insert(&mut self, hash: u64) {
        for bit in [hash & 0xFF, (hash >> 8) & 0xFF] {
            self.0[(bit >> 6) as usize] |= 1 << (bit & 63);
        }
    }

    fn is_empty(&self) -> bool {
        self.0 == [0; 4]
    }

    /// Whether every bit of `required` is set here.
    pub(super) fn covers(&self, required: &Self) -> bool {
        self.0
            .iter()
            .zip(&required.0)
            .all(|(have, need)| have & need == *need)
    }
}

const KIND_TAG: u8 = 1;
const KIND_ID: u8 = 2;
const KIND_CLASS: u8 = 3;

/// Hash of one key. Tags compare ASCII case-insensitively, so they are
/// hashed lowercased; ids and classes compare exactly.
fn key_hash(kind: u8, s: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write_u8(kind);
    if kind == KIND_TAG {
        for b in s.bytes() {
            h.write_u8(b.to_ascii_lowercase());
        }
    } else {
        h.write(s.as_bytes());
    }
    // Fx's low bits are weak for short keys; fold the high half in.
    let x = h.finish();
    x ^ (x >> 29) ^ (x >> 47)
}

/// The keys `selector`'s ancestor compounds require, or `None` when it
/// requires none (nothing to filter on).
pub(super) fn selector_mask(selector: &Selector) -> Option<AncestorMask> {
    let parts = &selector.parts;
    let mut mask = AncestorMask::default();
    // `parts[i].1` links `parts[i - 1]` to `parts[i]`; walk left from the
    // subject while every link is a child / descendant combinator.
    let mut i = parts.len();
    while i > 1 {
        i -= 1;
        match parts[i].1.as_ref().unwrap_or(&Combinator::Descendant) {
            Combinator::Child | Combinator::Descendant => {},
            Combinator::AdjacentSibling | Combinator::GeneralSibling => break,
        }
        for simple in &parts[i - 1].0.parts {
            match simple {
                SimpleSelector::Type(tag) => mask.insert(key_hash(KIND_TAG, tag)),
                SimpleSelector::Id(id) => mask.insert(key_hash(KIND_ID, id)),
                SimpleSelector::Class(cls) => mask.insert(key_hash(KIND_CLASS, cls)),
                _ => {},
            }
        }
    }
    (!mask.is_empty()).then_some(mask)
}

/// The filter of every element ancestor of `node_id` (not the element
/// itself), the ancestors the matcher's child / descendant walks visit.
pub(super) fn ancestor_filter(doc: &Document, node_id: NodeId) -> AncestorMask {
    let mut mask = AncestorMask::default();
    let mut current = doc.nodes[node_id].parent;
    while let Some(pid) = current {
        if let NodeKind::Element(e) = &doc.nodes[pid].kind {
            mask.insert(key_hash(KIND_TAG, e.tag.as_str()));
            if let Some(id) = e.get_attribute("id") {
                mask.insert(key_hash(KIND_ID, id));
            }
            if let Some(classes) = e.get_attribute("class") {
                for cls in classes.split_ascii_whitespace() {
                    mask.insert(key_hash(KIND_CLASS, cls));
                }
            }
        }
        current = doc.nodes[pid].parent;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::CascadeContext;
    use crate::cascade::matching::matches_selector;
    use crate::parser::Stylesheet;
    use oasis_html::tokenizer::Tokenizer;
    use oasis_html::tree_builder::TreeBuilder;

    /// The filter never rejects a selector that matches (no false
    /// negatives), and does reject ones whose ancestor keys are absent.
    #[test]
    fn filter_is_conservative_and_rejects_missing_ancestors() {
        let html = "<html><body><div id=\"Main\" class=\"a b\"><UL><li>\
                    <p class=\"x\">t</p><span>s</span></li></UL></div></body></html>";
        let doc = TreeBuilder::build(Tokenizer::new(html).tokenize());
        let css = "#Main p {} .a ul > li p {} div UL p {} .b + p {} li span + p {} \
                   body > div > ul p.x {} p {} .missing p {} #main p {} section p {} \
                   .x p {} li ~ span {} div li > :not(.q) {}";
        let sheet = Stylesheet::parse(css);
        let ctx = CascadeContext::default();
        let elements: Vec<NodeId> = (0..doc.nodes.len())
            .filter(|&n| matches!(doc.nodes[n].kind, NodeKind::Element(_)))
            .collect();
        let mut rejected = 0;
        for rule in &sheet.rules {
            for sel in &rule.selectors.selectors {
                let mask = selector_mask(sel);
                for &n in &elements {
                    let passes = mask.is_none_or(|m| ancestor_filter(&doc, n).covers(&m));
                    if matches_selector(&doc, n, sel, &ctx) {
                        assert!(passes, "filter rejected a match: {sel:?}");
                    }
                    rejected += usize::from(!passes);
                }
            }
        }
        // `.missing p`, `#main p` (ids are case-sensitive), `section p`
        // and `.x p` are rejected for every element.
        assert!(rejected >= 4 * elements.len(), "only {rejected} rejections");
    }
}

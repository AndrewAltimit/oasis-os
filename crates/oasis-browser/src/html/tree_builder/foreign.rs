//! Foreign-content (SVG / MathML) insertion — a simplified subset of
//! WHATWG HTML §13.2.6.5.
//!
//! The real algorithm tracks namespaces (HTML / SVG / MathML), applies
//! tag-name case fixups for a curated list of SVG camelCase identifiers,
//! runs integration-point checks for MathML `<annotation-xml>`, and has
//! a large list of HTML "breakout" tags that yank parsing back to HTML.
//! We don't model namespaces and the tokenizer has already lowercased
//! every tag name, so we can't preserve SVG camelCase — but real pages
//! that inline `<svg>` or `<math>` blocks still need to parse without
//! the HTML auto-close rules firing inside. This file implements that:
//! while `foreign_depth > 0` every start tag becomes a literal element
//! and every end tag pops to the matching element without applying
//! `is_block_level` / `close_p_if_in_scope` / adoption-agency rules.
//!
//! HTML breakout — if we see one of the HTML tags listed in
//! [`is_html_breakout_tag`] (the canonical set: `b`, `big`, `body`,
//! `br`, `center`, `code`, `dd`, `div`, `dl`, `dt`, `em`, `embed`,
//! `h1`…`h6`, `head`, `hr`, `i`, `img`, `li`, `listing`, `menu`,
//! `meta`, `nobr`, `ol`, `p`, `pre`, `ruby`, `s`, `small`, `span`,
//! `strong`, `strike`, `sub`, `sup`, `table`, `tt`, `u`, `ul`, `var`),
//! we pop everything in the foreign subtree off the stack back to the
//! svg/math root and reprocess the token via the HTML path.

use super::super::dom::{ElementData, NodeKind, TagName};
use super::super::tokenizer::Token;
use super::{TreeBuilder, is_all_whitespace};

impl TreeBuilder {
    pub(crate) fn handle_foreign_content(&mut self, token: Token) {
        match token {
            Token::Character(ref s) => {
                self.insert_text(s);
                // Per spec, a non-whitespace character token in foreign
                // content flips frameset_ok. We already tore out
                // frameset support, so the bookkeeping just mirrors
                // the flag for completeness.
                if !is_all_whitespace(s) {
                    self.frameset_ok = false;
                }
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text));
                let parent = self.current_node();
                self.doc.append_child(parent, id);
            },
            Token::Doctype(_) => {
                log::trace!("html parse error: doctype in foreign content");
            },
            Token::StartTag(ref tag) => {
                let lower = tag.name.to_ascii_lowercase();
                if is_html_breakout_tag(&lower) {
                    log::trace!("foreign content: breakout on <{lower}>, returning to HTML");
                    self.break_out_of_foreign_content();
                    self.process_token(token);
                    return;
                }
                // Generic foreign-content element insertion. No
                // auto-close of preceding `<p>`, no
                // reconstruct_formatting, no void-element lookup (SVG
                // and MathML self-close via `/>` which the tokenizer
                // surfaces as `self_closing: true`).
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                if !tag.self_closing {
                    self.open_elements.push(id);
                    self.foreign_depth += 1;
                }
            },
            Token::EndTag(ref tag) => {
                let lower = tag.name.to_ascii_lowercase();
                if lower == "script" {
                    // Per spec a `</script>` inside SVG pops the
                    // current SVG script element; we just treat it
                    // like any other close.
                }
                // Pop open elements up to and including the matching
                // foreign element (case-insensitive tag compare),
                // decrementing foreign_depth for each popped
                // foreign-content descendant. If no match is found,
                // the end tag is a parse error and we ignore it.
                let mut found = None;
                for (i, &id) in self.open_elements.iter().enumerate().rev() {
                    if self.tag_of(id).map(TagName::as_str) == Some(lower.as_str()) {
                        found = Some(i);
                        break;
                    }
                    // Spec: if we walk past a non-foreign element,
                    // stop and ignore (we can't cross back into HTML
                    // via an end tag inside foreign content). Our
                    // foreign_depth > 0 invariant means every element
                    // on the stack above the svg/math root is in the
                    // foreign subtree, so this guard is academic here.
                }
                if let Some(pos) = found {
                    let pops = self.open_elements.len() - pos;
                    self.open_elements.truncate(pos);
                    let dec = pops.min(self.foreign_depth as usize);
                    self.foreign_depth -= dec as u32;
                }
            },
            Token::Eof => {
                // Let the normal InBody EOF handling run via the
                // outer process_token path after we clear the
                // foreign subtree.
                self.break_out_of_foreign_content();
            },
        }
    }

    /// Pop every foreign-content element off the open stack and zero
    /// out `foreign_depth`. Used when an HTML breakout token arrives
    /// or when the token stream ends.
    pub(crate) fn break_out_of_foreign_content(&mut self) {
        let depth = self.foreign_depth as usize;
        let keep = self.open_elements.len().saturating_sub(depth);
        self.open_elements.truncate(keep);
        self.foreign_depth = 0;
    }

    /// Entry point used by `handle_start_tag_in_body` when a bare
    /// `<svg>` or `<math>` root is encountered while in normal HTML
    /// parsing. Creates the root element and enters foreign content.
    pub(crate) fn enter_foreign_root(
        &mut self,
        tag: &super::super::tokenizer::StartTagToken,
        as_tag: TagName,
    ) {
        // Don't auto-close a containing <p> — foreign elements are
        // phrasing content and can live inside a paragraph. We also
        // don't reconstruct formatting because the foreign root is
        // outside the HTML formatting list.
        let mut data = ElementData::new(as_tag);
        for attr in &tag.attributes {
            data.attributes.push(super::super::dom::Attribute {
                name: attr.name.clone(),
                value: attr.value.clone(),
            });
        }
        let id = self.doc.add_node(NodeKind::Element(data));
        let parent = self.current_node();
        self.doc.append_child(parent, id);
        if !tag.self_closing {
            self.open_elements.push(id);
            self.foreign_depth += 1;
        }
    }
}

/// HTML tags that force a breakout from foreign content back to HTML
/// parsing per WHATWG §13.2.6.5. This is the canonical list from the
/// spec, verbatim.
fn is_html_breakout_tag(tag: &str) -> bool {
    matches!(
        tag,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

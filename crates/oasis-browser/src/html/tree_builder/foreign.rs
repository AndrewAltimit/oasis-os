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
                //
                // Apply SVG camelCase fixup before creating the element
                // so DOM queries return the spec-correct tag name.
                let id = self.create_foreign_element(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                if !tag.self_closing {
                    self.open_elements.push(id);
                    self.foreign_depth += 1;
                }
            },
            Token::EndTag(ref tag) => {
                let lower = tag.name.to_ascii_lowercase();
                // Per WHATWG §13.2.6.5: search inside the current
                // foreign content subtree only — we can't cross back
                // into HTML via an end tag. Restrict the scan to the
                // topmost `foreign_depth` elements of the open stack
                // (the foreign root + its foreign descendants) so a
                // malformed `</body>` inside `<svg>` can never
                // accidentally close an HTML ancestor even if the
                // `foreign_depth` invariant is later weakened.
                let depth = self.foreign_depth as usize;
                let stack_len = self.open_elements.len();
                debug_assert!(
                    depth <= stack_len,
                    "foreign_depth {depth} exceeds open_elements.len() {stack_len}"
                );
                let subtree_start = stack_len.saturating_sub(depth);
                let mut found = None;
                for (i, &id) in self.open_elements[subtree_start..].iter().enumerate().rev() {
                    if self
                        .tag_of(id)
                        .map(TagName::as_str)
                        .is_some_and(|t| t.eq_ignore_ascii_case(&lower))
                    {
                        found = Some(subtree_start + i);
                        break;
                    }
                }
                if let Some(pos) = found {
                    let pops = stack_len - pos;
                    self.open_elements.truncate(pos);
                    debug_assert!(
                        pops <= self.foreign_depth as usize,
                        "popping {pops} elements but foreign_depth is only {}",
                        self.foreign_depth
                    );
                    self.foreign_depth -= pops as u32;
                }
            },
            Token::Eof => {
                self.break_out_of_foreign_content();
                self.process_token(Token::Eof);
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

    /// Create an element inside foreign content, applying the WHATWG
    /// SVG camelCase fixup table (§13.2.6.5) so tag names are stored
    /// with spec-correct casing (e.g. `foreignObject`, not
    /// `foreignobject`). Attribute names are similarly fixed up.
    fn create_foreign_element(
        &mut self,
        tag: &super::super::tokenizer::StartTagToken,
    ) -> super::super::dom::NodeId {
        let lower = tag.name.to_ascii_lowercase();
        let fixed = svg_tag_case_fixup(&lower);
        let tag_name = TagName::from_str(fixed);
        let mut data = ElementData::new(tag_name);
        for attr in &tag.attributes {
            data.attributes.push(super::super::dom::Attribute {
                name: svg_attr_case_fixup(&attr.name),
                value: attr.value.clone(),
            });
        }
        self.doc.add_node(NodeKind::Element(data))
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

/// SVG tag name case fixup per WHATWG §13.2.6.5. The tokenizer
/// lowercases everything, so we restore the canonical camelCase for
/// SVG elements that use it. Input must already be lowercased.
fn svg_tag_case_fixup(lower: &str) -> &str {
    match lower {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        // Everything else keeps its lowercase form.
        _ => lower,
    }
}

/// SVG attribute name case fixup per WHATWG §13.2.6.5. Only a handful
/// of SVG attributes use camelCase; this list covers the most common.
fn svg_attr_case_fixup(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "attributename" => "attributeName".into(),
        "attributetype" => "attributeType".into(),
        "basefrequency" => "baseFrequency".into(),
        "baseprofile" => "baseProfile".into(),
        "calcmode" => "calcMode".into(),
        "clippathunits" => "clipPathUnits".into(),
        "diffuseconstant" => "diffuseConstant".into(),
        "edgemode" => "edgeMode".into(),
        "filterunits" => "filterUnits".into(),
        "glyphref" => "glyphRef".into(),
        "gradienttransform" => "gradientTransform".into(),
        "gradientunits" => "gradientUnits".into(),
        "kernelmatrix" => "kernelMatrix".into(),
        "kernelunitlength" => "kernelUnitLength".into(),
        "keypoints" => "keyPoints".into(),
        "keysplines" => "keySplines".into(),
        "keytimes" => "keyTimes".into(),
        "lengthadjust" => "lengthAdjust".into(),
        "limitingconeangle" => "limitingConeAngle".into(),
        "markerheight" => "markerHeight".into(),
        "markerunits" => "markerUnits".into(),
        "markerwidth" => "markerWidth".into(),
        "maskcontentunits" => "maskContentUnits".into(),
        "maskunits" => "maskUnits".into(),
        "numoctaves" => "numOctaves".into(),
        "pathlength" => "pathLength".into(),
        "patterncontentunits" => "patternContentUnits".into(),
        "patterntransform" => "patternTransform".into(),
        "patternunits" => "patternUnits".into(),
        "pointsatx" => "pointsAtX".into(),
        "pointsaty" => "pointsAtY".into(),
        "pointsatz" => "pointsAtZ".into(),
        "preservealpha" => "preserveAlpha".into(),
        "preserveaspectratio" => "preserveAspectRatio".into(),
        "primitiveunits" => "primitiveUnits".into(),
        "refx" => "refX".into(),
        "refy" => "refY".into(),
        "repeatcount" => "repeatCount".into(),
        "repeatdur" => "repeatDur".into(),
        "requiredextensions" => "requiredExtensions".into(),
        "requiredfeatures" => "requiredFeatures".into(),
        "specularconstant" => "specularConstant".into(),
        "specularexponent" => "specularExponent".into(),
        "spreadmethod" => "spreadMethod".into(),
        "startoffset" => "startOffset".into(),
        "stddeviation" => "stdDeviation".into(),
        "stitchtiles" => "stitchTiles".into(),
        "surfacescale" => "surfaceScale".into(),
        "systemlanguage" => "systemLanguage".into(),
        "tablevalues" => "tableValues".into(),
        "targetx" => "targetX".into(),
        "targety" => "targetY".into(),
        "textlength" => "textLength".into(),
        "viewbox" => "viewBox".into(),
        "viewtarget" => "viewTarget".into(),
        "xchannelselector" => "xChannelSelector".into(),
        "ychannelselector" => "yChannelSelector".into(),
        "zoomandpan" => "zoomAndPan".into(),
        _ => name.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_tag_fixup_foreignobject() {
        assert_eq!(svg_tag_case_fixup("foreignobject"), "foreignObject");
    }

    #[test]
    fn svg_tag_fixup_textpath() {
        assert_eq!(svg_tag_case_fixup("textpath"), "textPath");
    }

    #[test]
    fn svg_tag_fixup_lineargradient() {
        assert_eq!(svg_tag_case_fixup("lineargradient"), "linearGradient");
    }

    #[test]
    fn svg_tag_fixup_passthrough() {
        assert_eq!(svg_tag_case_fixup("rect"), "rect");
        assert_eq!(svg_tag_case_fixup("circle"), "circle");
    }

    #[test]
    fn svg_attr_fixup_viewbox() {
        assert_eq!(svg_attr_case_fixup("viewbox"), "viewBox");
    }

    #[test]
    fn svg_attr_fixup_preserveaspectratio() {
        assert_eq!(
            svg_attr_case_fixup("preserveaspectratio"),
            "preserveAspectRatio"
        );
    }

    #[test]
    fn svg_attr_fixup_passthrough() {
        assert_eq!(svg_attr_case_fixup("fill"), "fill");
        assert_eq!(svg_attr_case_fixup("d"), "d");
    }
}

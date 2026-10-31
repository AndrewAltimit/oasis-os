//! Foreign content (SVG / MathML) — WHATWG HTML §13.2.6.5.
//!
//! Every element the builder creates gets a namespace ([`Ns`]), kept
//! per node by the builder (the DOM has no namespace field). The tree
//! construction dispatcher (`TreeBuilder::process_token`) sends a token
//! here while the current node is SVG / MathML, except at integration
//! points: inside SVG `foreignObject` / `desc` / `title` and MathML
//! `annotation-xml encoding="text/html"` start tags and text follow the
//! HTML rules, and so do they (bar `mglyph` / `malignmark`) inside the
//! MathML text elements `mi`, `mo`, `mn`, `ms`, `mtext`.
//!
//! Here no HTML auto-close rules apply: start tags become elements in
//! the current node's namespace (SVG names and attributes get their
//! camelCase back), end tags pop to the matching foreign element. An
//! HTML "breakout" tag ([`is_html_breakout_tag`], `<font>` with
//! `color` / `face` / `size`, `</br>`, `</p>`) pops back to the nearest
//! HTML element or integration point and is reprocessed as HTML.

use super::super::dom::{ElementData, NodeKind, TagName};
use super::super::tokenizer::{StartTagToken, Token};
use super::{Ns, TreeBuilder, is_all_whitespace};

impl TreeBuilder {
    pub(crate) fn handle_foreign_content(&mut self, token: Token) {
        match token {
            Token::Character(ref s) => {
                self.insert_text(s);
                if !is_all_whitespace(s) {
                    self.frameset_ok = false;
                }
            },
            Token::Comment(text) => self.insert_comment(text),
            Token::Doctype(_) => {
                log::trace!("html parse error: doctype in foreign content");
            },
            Token::StartTag(ref tag) => {
                let lower = tag.name.to_ascii_lowercase();
                let font_breakout = lower == "font"
                    && tag
                        .attributes
                        .iter()
                        .any(|a| matches!(a.name.as_str(), "color" | "face" | "size"));
                if is_html_breakout_tag(&lower) || font_breakout {
                    log::trace!("foreign content: breakout on <{lower}>, returning to HTML");
                    self.break_out_of_foreign_content();
                    self.process_html(token);
                    return;
                }
                let id = self.create_foreign_element(tag);
                self.insert_leaf(id);
                // Self-closing foreign elements are not pushed; past the
                // open-elements cap nothing is, exactly like
                // `insert_element`.
                if !tag.self_closing {
                    self.push_open(id);
                }
            },
            Token::EndTag(ref tag) => {
                let lower = tag.name.to_ascii_lowercase();
                if lower == "br" || lower == "p" {
                    self.break_out_of_foreign_content();
                    self.process_html(token);
                    return;
                }
                // Walk down from the current node through the foreign
                // elements: a match pops to it; reaching an HTML element
                // hands the token to the HTML insertion mode.
                let mut idx = self.open_elements.len();
                while idx > 1 {
                    idx -= 1;
                    let id = self.open_elements[idx];
                    if !self.ns_of(id).is_foreign() {
                        self.process_html(token);
                        return;
                    }
                    if self
                        .tag_of(id)
                        .is_some_and(|t| t.as_str().eq_ignore_ascii_case(&lower))
                    {
                        self.open_elements.truncate(idx);
                        return;
                    }
                }
            },
            Token::Eof => self.process_html(Token::Eof),
        }
    }

    /// Pop until the current node is an HTML element, an HTML
    /// integration point or a MathML text integration point.
    pub(crate) fn break_out_of_foreign_content(&mut self) {
        while let Some(&cur) = self.open_elements.last() {
            let ns = self.ns_of(cur);
            if !ns.is_foreign() || ns.is_html_integration_point() || ns == Ns::MathTextIp {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Create an element in the current node's (foreign) namespace,
    /// applying the WHATWG SVG camelCase fixups (§13.2.6.5) so names
    /// are stored with spec-correct casing (`foreignObject`, `viewBox`).
    fn create_foreign_element(&mut self, tag: &StartTagToken) -> super::super::dom::NodeId {
        let lower = tag.name.to_ascii_lowercase();
        let svg = self.ns_of(self.current_node()).is_svg();
        let name = if svg {
            svg_tag_case_fixup(&lower)
        } else {
            &lower
        };
        let ns = if svg {
            svg_ns_for(&lower)
        } else {
            mathml_ns_for(&lower, tag)
        };
        let mut data = ElementData::new(TagName::from_str(name));
        for attr in &tag.attributes {
            let name = if svg {
                svg_attr_case_fixup(&attr.name)
            } else if attr.name.eq_ignore_ascii_case("definitionurl") {
                "definitionURL".to_string()
            } else {
                attr.name.clone()
            };
            data.attributes.push(super::super::dom::Attribute {
                name,
                value: attr.value.clone(),
            });
        }
        let id = self.doc.add_node(NodeKind::Element(data));
        self.set_ns(id, ns);
        id
    }

    /// Entry point used by `handle_start_tag_in_body` when a bare
    /// `<svg>` or `<math>` root is encountered while in normal HTML
    /// parsing. Creates the root element and enters foreign content.
    pub(crate) fn enter_foreign_root(&mut self, tag: &StartTagToken, as_tag: TagName) {
        // Don't auto-close a containing <p> — foreign elements are
        // phrasing content and can live inside a paragraph.
        let svg = as_tag == TagName::Svg;
        let mut data = ElementData::new(as_tag);
        for attr in &tag.attributes {
            let name = if svg {
                svg_attr_case_fixup(&attr.name)
            } else {
                attr.name.clone()
            };
            data.attributes.push(super::super::dom::Attribute {
                name,
                value: attr.value.clone(),
            });
        }
        let id = self.doc.add_node(NodeKind::Element(data));
        self.set_ns(id, if svg { Ns::Svg } else { Ns::Math });
        self.insert_leaf(id);
        if !tag.self_closing {
            self.push_open(id);
        }
    }
}

/// Namespace kind of a new SVG element named `lower`.
fn svg_ns_for(lower: &str) -> Ns {
    match lower {
        "foreignobject" | "desc" | "title" => Ns::SvgHtmlIp,
        _ => Ns::Svg,
    }
}

/// Namespace kind of a new MathML element named `lower`.
fn mathml_ns_for(lower: &str, tag: &StartTagToken) -> Ns {
    match lower {
        "mi" | "mo" | "mn" | "ms" | "mtext" => Ns::MathTextIp,
        "annotation-xml" => {
            let html = tag.attributes.iter().any(|a| {
                a.name == "encoding"
                    && (a.value.eq_ignore_ascii_case("text/html")
                        || a.value.eq_ignore_ascii_case("application/xhtml+xml"))
            });
            if html {
                Ns::MathHtmlIp
            } else {
                Ns::MathAnnotationXml
            }
        },
        _ => Ns::Math,
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

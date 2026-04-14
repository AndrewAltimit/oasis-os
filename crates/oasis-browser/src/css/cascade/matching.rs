//! Selector matching and DOM traversal helpers.
//!
//! Implements compound/simple/combinator matching, pseudo-class and
//! pseudo-element matching, attribute matching, and presentational
//! HTML attribute injection.

use super::super::parser::{
    AttrOp, Combinator, CompoundSelector, CssColor, CssValue, LengthUnit, Rule, SimpleSelector,
    Specificity, Stylesheet,
};
use super::super::selectors;
use super::super::values::ComputedStyle;
use super::CascadeContext;
use crate::html::dom::{Document, ElementData, NodeId, NodeKind};

use rustc_hash::FxHashMap;

// -----------------------------------------------------------------------
// Robustness limits
// -----------------------------------------------------------------------

/// Maximum selector nesting depth (combinators). Selectors deeper
/// than this are treated as non-matching.
#[allow(dead_code)]
const MAX_SELECTOR_DEPTH: usize = 64;

/// Maximum number of DOM traversal steps during selector matching.
/// Prevents O(N^2) blowup on pathologically complex selectors.
#[allow(dead_code)]
const MAX_TRAVERSAL_STEPS: usize = 512;

// -----------------------------------------------------------------------
// Matched declaration types
// -----------------------------------------------------------------------

/// The origin of a declaration for cascade ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    /// From an HTML presentational attribute (lowest priority author style).
    Presentational,
    /// From a `<link>` or `<style>` stylesheet.
    Stylesheet,
    /// From the element's `style=""` attribute.
    Inline,
}

/// A single declaration together with its cascade metadata.
#[derive(Debug, Clone)]
pub(super) struct MatchedDeclaration {
    pub(super) property: String,
    pub(super) value: CssValue,
    pub(super) important: bool,
    pub(super) origin: Origin,
    pub(super) specificity: Specificity,
    pub(super) source_order: usize,
    /// Index of the source stylesheet, used to scope cascade-layer
    /// comparison to rules from the same sheet (cross-sheet ordering
    /// still falls through to `source_order`).
    pub(super) sheet_idx: u16,
    /// Cascade-layer index inside the source stylesheet, or `None`
    /// for unlayered rules / non-stylesheet origins.
    pub(super) layer: Option<u16>,
}

/// Compare two declarations by cascade-layer position.
///
/// Spec semantics:
/// - Within the same stylesheet, unlayered author rules beat layered
///   author rules for normal declarations; `!important` reverses that
///   (layered `!important` wins over unlayered `!important`).
/// - Earlier-declared layers lose to later-declared layers for normal
///   declarations; `!important` reverses again (earlier layers win).
/// - Across different stylesheets we fall through to `source_order`
///   since layer names are sheet-local in this v1 implementation.
pub(super) fn compare_layers(a: &MatchedDeclaration, b: &MatchedDeclaration) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    if a.sheet_idx != b.sheet_idx {
        return Ordering::Equal;
    }
    // Only style-origin declarations participate in layering.
    if a.origin != Origin::Stylesheet || b.origin != Origin::Stylesheet {
        return Ordering::Equal;
    }
    // `!important` must be consistent for the comparison to make sense.
    // The outer sort already splits by `important`; this helper runs
    // after that, so we can assume both sides share the same bit.
    let important = a.important;

    match (a.layer, b.layer) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => {
            // Unlayered wins for normal, loses for !important.
            if important {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        },
        (Some(_), None) => {
            if important {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        },
        (Some(ao), Some(bo)) => {
            // Later layers win for normal, earlier wins for !important.
            if important { bo.cmp(&ao) } else { ao.cmp(&bo) }
        },
    }
}

// -----------------------------------------------------------------------
// Pseudo-element resolution
// -----------------------------------------------------------------------

/// Resolve the full computed style for a ::before or ::after pseudo-element.
///
/// Collects ALL declarations from matching pseudo-element rules (not just
/// `content`), sorts by cascade order, inherits from the originating
/// element, and applies declarations. Returns `None` if no matching rule
/// sets `content` to a string value (including empty string for clearfix).
pub(super) fn resolve_pseudo_style(
    doc: &Document,
    node_id: NodeId,
    pseudo: &str,
    element_style: &ComputedStyle,
    stylesheets: &[&Stylesheet],
    ctx: &CascadeContext<'_>,
) -> Option<ComputedStyle> {
    use super::super::values::Display;

    // Collect all matching declarations with cascade metadata.
    let mut matched: Vec<MatchedDeclaration> = Vec::new();
    let mut source_order: usize = 0;

    for (sheet_idx, stylesheet) in stylesheets.iter().enumerate() {
        for rule in &stylesheet.rules {
            let decl_base = source_order;
            source_order += rule.declarations.len();

            for selector in &rule.selectors.selectors {
                if selector_pseudo_element(selector) != Some(pseudo) {
                    continue;
                }
                if !matches_selector_ignoring_pseudo(doc, node_id, selector, ctx) {
                    continue;
                }
                let specificity = selector.specificity();
                for (i, decl) in rule.declarations.iter().enumerate() {
                    matched.push(MatchedDeclaration {
                        property: decl.property.clone(),
                        value: decl.value.clone(),
                        important: decl.important,
                        origin: Origin::Stylesheet,
                        specificity,
                        source_order: decl_base + i,
                        sheet_idx: sheet_idx as u16,
                        layer: rule.layer,
                    });
                }
            }
        }
    }

    if matched.is_empty() {
        return None;
    }

    // Sort by cascade order.
    matched.sort_by(|a, b| {
        a.important
            .cmp(&b.important)
            .then_with(|| a.origin.cmp(&b.origin))
            .then_with(|| compare_layers(a, b))
            .then_with(|| a.specificity.cmp(&b.specificity))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });

    // Find the winning `content` value.
    let content_value = matched
        .iter()
        .rev()
        .find(|d| d.property == "content")
        .and_then(|d| match &d.value {
            CssValue::String(s) => Some(s.clone()),
            CssValue::Keyword(kw) if kw == "none" || kw == "normal" => None,
            _ => None,
        });

    // No content declaration or content:none/normal => no pseudo-element.
    let content_text = content_value?;

    // Pseudo-elements inherit from the originating element.
    let mut style = ComputedStyle::inherit(element_style);
    // Default display for pseudo-elements is inline (CSS spec).
    style.display = Display::Inline;

    let parent_font_size = element_style.font_size;

    // Apply font-size first (for em-unit resolution in other properties).
    for entry in &matched {
        if entry.property == "font-size" {
            style.apply_declaration("font-size", &entry.value, parent_font_size);
        }
    }
    let pseudo_font_size = style.font_size;

    // Apply all other declarations.
    for entry in &matched {
        if entry.property == "font-size" || entry.property == "content" {
            continue;
        }
        style.apply_declaration(&entry.property, &entry.value, pseudo_font_size);
    }

    style.content = Some(content_text);
    Some(style)
}

/// Match a selector against a node, ignoring any pseudo-element part.
///
/// For `p::before`, this checks whether `p` matches the node.
fn matches_selector_ignoring_pseudo(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
) -> bool {
    // Build a temporary selector without the pseudo-element simple selectors.
    let parts = &selector.parts;
    if parts.is_empty() {
        return false;
    }

    // The pseudo-element is in the last compound selector. Strip it.
    let last_idx = parts.len() - 1;
    let last_compound = &parts[last_idx].0;
    let filtered_parts: Vec<SimpleSelector> = last_compound
        .parts
        .iter()
        .filter(|s| !matches!(s, SimpleSelector::PseudoElement(_)))
        .cloned()
        .collect();

    if filtered_parts.is_empty() && last_idx == 0 {
        // Selector was just `::before` with no element part -- matches any element.
        return true;
    }

    let filtered_compound = CompoundSelector {
        parts: filtered_parts,
    };

    // If the filtered compound is empty, the pseudo-element was the
    // only part of that compound.  Replace it with a universal selector
    // so the combinator is preserved (e.g. `div ::before` should match
    // descendants of `div`, not `div` itself).
    if filtered_compound.parts.is_empty() {
        if last_idx == 0 {
            // Selector was just `::before` / `::after` -- matches any element.
            return true;
        }
        let mut new_parts = parts[..last_idx].to_vec();
        new_parts.push((
            CompoundSelector {
                parts: vec![SimpleSelector::Universal],
            },
            parts[last_idx].1.clone(),
        ));
        let adjusted = super::super::parser::Selector { parts: new_parts };
        return matches_selector(doc, node_id, &adjusted, ctx);
    }

    // Replace the last compound with the filtered version.
    let mut new_parts = parts.clone();
    new_parts[last_idx].0 = filtered_compound;
    let temp_selector = super::super::parser::Selector { parts: new_parts };
    matches_selector(doc, node_id, &temp_selector, ctx)
}

// -----------------------------------------------------------------------
// Matched declaration collection
// -----------------------------------------------------------------------

/// Gather every declaration that applies to `node_id` from all
/// stylesheets and inline styles.
///
/// Uses the `SelectorIndex` to test only candidate rules whose subject
/// selector matches the element's tag/id/classes, instead of all rules.
pub(super) fn collect_matched_declarations(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    index: &super::index::SelectorIndex,
    inline_map: &FxHashMap<NodeId, &[super::super::parser::Declaration]>,
    ctx: &CascadeContext<'_>,
    tag_cache: &mut FxHashMap<String, String>,
) -> Vec<MatchedDeclaration> {
    let mut result = Vec::new();

    // Extract element info for index lookup.
    let elem = match &doc.nodes[node_id].kind {
        NodeKind::Element(e) => e,
        _ => return result,
    };
    let tag = elem.tag.as_str();
    let id = elem.get_attribute("id");
    let class_str = elem.get_attribute("class").unwrap_or("");
    let classes: Vec<&str> = class_str.split_whitespace().collect();

    // Use cached lowercased tag name to avoid repeated allocations.
    // Look up by &str first; only allocate on cache misses.
    if !tag_cache.contains_key(tag) {
        tag_cache.insert(tag.to_string(), tag.to_ascii_lowercase());
    }
    let tag_lower = tag_cache.get(tag).expect("just inserted");

    // Get candidate rules from the index.
    let candidates = index.candidates_with_lower(tag, tag_lower, id, &classes);

    for candidate in &candidates {
        let rule = &stylesheets[candidate.sheet_idx].rules[candidate.rule_idx];
        let best_specificity = matching_specificity(doc, node_id, rule, ctx);
        if let Some(specificity) = best_specificity {
            for (decl_idx, decl) in rule.declarations.iter().enumerate() {
                result.push(MatchedDeclaration {
                    property: decl.property.clone(),
                    value: decl.value.clone(),
                    important: decl.important,
                    origin: Origin::Stylesheet,
                    specificity,
                    source_order: candidate.source_order_base + decl_idx,
                    sheet_idx: candidate.sheet_idx as u16,
                    layer: rule.layer,
                });
            }
        }
    }

    // Presentational HTML attributes (bgcolor, width, height, align, etc.)
    // have lowest author-origin priority (overridden by any CSS rule).
    collect_presentational_attrs(elem, &mut result);

    // Inline styles have the highest non-important specificity.
    // O(1) lookup via HashMap instead of linear scan.
    if let Some(decls) = inline_map.get(&node_id) {
        let inline_spec = Specificity {
            inline: 1,
            ids: 0,
            classes: 0,
            types: 0,
        };
        // Inline source_order is after all stylesheet declarations.
        let inline_base = stylesheets
            .iter()
            .flat_map(|s| &s.rules)
            .map(|r| r.declarations.len())
            .sum::<usize>();
        for (i, decl) in decls.iter().enumerate() {
            result.push(MatchedDeclaration {
                property: decl.property.clone(),
                value: decl.value.clone(),
                important: decl.important,
                origin: Origin::Inline,
                specificity: inline_spec,
                source_order: inline_base + i,
                sheet_idx: u16::MAX,
                layer: None,
            });
        }
    }

    result
}

/// Inject synthetic CSS declarations from HTML presentational attributes.
///
/// Per the CSS spec, presentational attributes act as author-origin rules
/// with zero specificity -- overridden by any CSS rule. We use
/// `Origin::Presentational` which sorts before `Origin::Stylesheet`.
fn collect_presentational_attrs(elem: &ElementData, result: &mut Vec<MatchedDeclaration>) {
    use crate::html::dom::TagName;

    let zero_spec = Specificity {
        inline: 0,
        ids: 0,
        classes: 0,
        types: 0,
    };

    let mut push = |property: &str, value: CssValue| {
        result.push(MatchedDeclaration {
            property: property.to_string(),
            value,
            important: false,
            origin: Origin::Presentational,
            specificity: zero_spec,
            source_order: 0,
            sheet_idx: u16::MAX,
            layer: None,
        });
    };

    // bgcolor -> background-color
    if let Some(color_str) = elem.get_attribute("bgcolor")
        && let Some((r, g, b, a)) = parse_html_color(color_str)
    {
        push("background-color", CssValue::Color(CssColor { r, g, b, a }));
    }

    // width attribute -> width (on img, table, td, th, input)
    if matches!(
        elem.tag,
        TagName::Img | TagName::Table | TagName::Td | TagName::Th | TagName::Input
    ) && let Some(val) = elem.get_attribute("width")
        && let Some(css_val) = parse_html_dimension(val)
    {
        push("width", css_val);
    }

    // height attribute -> height (on img, table, td, th, input)
    if matches!(
        elem.tag,
        TagName::Img | TagName::Table | TagName::Td | TagName::Th | TagName::Input
    ) && let Some(val) = elem.get_attribute("height")
        && let Some(css_val) = parse_html_dimension(val)
    {
        push("height", css_val);
    }

    // align attribute -> text-align (on td, th, p, div, center, tr)
    if matches!(
        elem.tag,
        TagName::Td | TagName::Th | TagName::P | TagName::Div | TagName::Center | TagName::Tr
    ) && let Some(val) = elem.get_attribute("align")
    {
        let align = val.to_ascii_lowercase();
        if matches!(align.as_str(), "left" | "center" | "right" | "justify") {
            push("text-align", CssValue::Keyword(align));
        }
    }

    // nowrap attribute -> white-space: nowrap (on td, th)
    if matches!(elem.tag, TagName::Td | TagName::Th) && elem.get_attribute("nowrap").is_some() {
        push("white-space", CssValue::Keyword("nowrap".into()));
    }

    // cellspacing attribute on table -> border-spacing
    if elem.tag == TagName::Table
        && let Some(val) = elem.get_attribute("cellspacing")
        && let Ok(n) = val.parse::<f32>()
    {
        push("border-spacing", CssValue::Length(n, LengthUnit::Px));
    }

    // cellpadding attribute on table -> padding on descendant cells
    // (This is applied on the table and will be propagated separately in
    // the layout engine or by querying the parent table. For now, skip --
    // author CSS can override.)

    // border attribute on table
    if elem.tag == TagName::Table
        && let Some(val) = elem.get_attribute("border")
        && let Ok(n) = val.parse::<f32>()
        && n > 0.0
    {
        push("border-top-width", CssValue::Length(n, LengthUnit::Px));
        push("border-right-width", CssValue::Length(n, LengthUnit::Px));
        push("border-bottom-width", CssValue::Length(n, LengthUnit::Px));
        push("border-left-width", CssValue::Length(n, LengthUnit::Px));
        push("border-top-style", CssValue::Keyword("solid".into()));
        push("border-right-style", CssValue::Keyword("solid".into()));
        push("border-bottom-style", CssValue::Keyword("solid".into()));
        push("border-left-style", CssValue::Keyword("solid".into()));
    }

    // size attribute on input -> width (approximate)
    if elem.tag == TagName::Input
        && let Some(val) = elem.get_attribute("size")
        && let Ok(n) = val.parse::<f32>()
    {
        // Each character ~ 8px in our bitmap font.
        push("width", CssValue::Length(n * 8.0, LengthUnit::Px));
    }
}

/// Parse an HTML color attribute value (e.g., "#fff", "#ffffff", "red").
fn parse_html_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some((r, g, b, 255))
            },
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some((r, g, b, 255))
            },
            _ => None,
        }
    } else {
        // Named color lookup (common ones).
        match s.to_ascii_lowercase().as_str() {
            "white" => Some((255, 255, 255, 255)),
            "black" => Some((0, 0, 0, 255)),
            "red" => Some((255, 0, 0, 255)),
            "green" => Some((0, 128, 0, 255)),
            "blue" => Some((0, 0, 255, 255)),
            "yellow" => Some((255, 255, 0, 255)),
            "gray" | "grey" => Some((128, 128, 128, 255)),
            "silver" => Some((192, 192, 192, 255)),
            _ => None,
        }
    }
}

/// Parse an HTML dimension attribute value (e.g., "25%", "200", "200px").
fn parse_html_dimension(s: &str) -> Option<CssValue> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        pct.parse::<f32>().ok().map(CssValue::Percentage)
    } else if let Some(px) = s.strip_suffix("px") {
        px.parse::<f32>()
            .ok()
            .map(|n| CssValue::Length(n, LengthUnit::Px))
    } else {
        s.parse::<f32>()
            .ok()
            .map(|n| CssValue::Length(n, LengthUnit::Px))
    }
}

/// Return the highest specificity among the rule's selectors that match
/// `node_id`, or `None` if no selector matches.
/// Skips selectors that target pseudo-elements (::before, ::after).
pub(super) fn matching_specificity(
    doc: &Document,
    node_id: NodeId,
    rule: &Rule,
    ctx: &CascadeContext<'_>,
) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for selector in &rule.selectors.selectors {
        if selector_pseudo_element(selector).is_some() {
            continue; // Skip pseudo-element selectors in normal matching.
        }
        if matches_selector(doc, node_id, selector, ctx) {
            let spec = selector.specificity();
            best = Some(match best {
                Some(prev) if prev >= spec => prev,
                _ => spec,
            });
        }
    }
    best
}

/// Check if a selector targets a pseudo-element and return its name.
pub(super) fn selector_pseudo_element(selector: &super::super::parser::Selector) -> Option<&str> {
    let parts = &selector.parts;
    if parts.is_empty() {
        return None;
    }
    let last_compound = &parts[parts.len() - 1].0;
    for simple in &last_compound.parts {
        if let SimpleSelector::PseudoElement(name) = simple {
            return Some(name.as_str());
        }
    }
    None
}

// -----------------------------------------------------------------------
// Selector matching
// -----------------------------------------------------------------------

/// Check if a parsed selector matches a given element in the DOM.
///
/// A `Selector` stores its parts left-to-right: the first compound is
/// the leftmost in the source, and the last compound is the *subject*
/// (the element being tested). Combinators link compounds and are
/// stored as `Option<Combinator>` where `None` marks the first entry.
pub(crate) fn matches_selector(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
) -> bool {
    matches_selector_scoped(doc, node_id, selector, ctx, None, None)
}

/// Like [`matches_selector`] but with an optional `scope` ancestor
/// that bounds combinator walks.
///
/// When `scope` is `Some(E)`, ancestor walks triggered by descendant
/// or child combinators stop *before* reaching `E`, and sibling walks
/// triggered by `+` / `~` are confined to `E`'s subtree. This is how
/// `:has()` evaluates its inner selectors: elements referenced by the
/// relative selector must live strictly inside the subject's subtree,
/// so `article:has(.a .b)` can't match via an `.a` that is an ancestor
/// of `article` itself.
fn matches_selector_scoped(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
    scope: Option<NodeId>,
    scope_combinator: Option<&Combinator>,
) -> bool {
    let parts = &selector.parts;
    if parts.is_empty() {
        return false;
    }

    // The last compound is the subject -- it must match node_id.
    let last_idx = parts.len() - 1;
    if !matches_compound(doc, node_id, &parts[last_idx].0, ctx) {
        return false;
    }

    // Walk remaining parts right-to-left (from subject towards root).
    let mut current = node_id;
    for i in (0..last_idx).rev() {
        let (ref compound, _) = parts[i];
        // The combinator that connects `parts[i]` to `parts[i+1]` is
        // stored in `parts[i+1].1`.
        let combinator = parts[i + 1].1.as_ref();
        match combinator {
            Some(Combinator::Child) => match parent_element(doc, current) {
                Some(pid)
                    if !in_scope_exclusive(pid, scope)
                        || !matches_compound(doc, pid, compound, ctx) =>
                {
                    return false;
                },
                Some(pid) => {
                    current = pid;
                },
                None => return false,
            },
            Some(Combinator::AdjacentSibling) => match previous_sibling_element(doc, current) {
                Some(sid) if matches_compound(doc, sid, compound, ctx) => {
                    current = sid;
                },
                _ => return false,
            },
            Some(Combinator::GeneralSibling) => {
                // Walk previous siblings until one matches.
                let mut found = false;
                let mut sib = previous_sibling_element(doc, current);
                while let Some(sid) = sib {
                    if matches_compound(doc, sid, compound, ctx) {
                        current = sid;
                        found = true;
                        break;
                    }
                    sib = previous_sibling_element(doc, sid);
                }
                if !found {
                    return false;
                }
            },
            Some(Combinator::Descendant) | None => {
                // Walk up ancestors until one matches.
                let mut found = false;
                let mut ancestor = parent_element(doc, current);
                while let Some(anc_id) = ancestor {
                    if !in_scope_exclusive(anc_id, scope) {
                        break;
                    }
                    if matches_compound(doc, anc_id, compound, ctx) {
                        current = anc_id;
                        found = true;
                        break;
                    }
                    ancestor = parent_element(doc, anc_id);
                }
                if !found {
                    return false;
                }
            },
        }
    }

    // Enforce the leading combinator from :has(). After the right-to-left
    // walk, `current` is the node that matched the leftmost compound.
    // scope_combinator specifies the required relationship between that
    // node and the scope element.
    if let (Some(sc), Some(sid)) = (scope_combinator, scope) {
        match sc {
            Combinator::Child => {
                if parent_element(doc, current) != Some(sid) {
                    return false;
                }
            },
            Combinator::AdjacentSibling => {
                if previous_sibling_element(doc, current) != Some(sid) {
                    return false;
                }
            },
            Combinator::GeneralSibling => {
                let mut sib = next_sibling_element(doc, sid);
                let mut found = false;
                while let Some(s) = sib {
                    if s == current {
                        found = true;
                        break;
                    }
                    sib = next_sibling_element(doc, s);
                }
                if !found {
                    return false;
                }
            },
            Combinator::Descendant => {},
        }
    }

    true
}

/// Returns `true` if `node_id` is a strict descendant of `scope`
/// (not equal to it). With `scope = None`, always returns `true`.
#[inline]
fn in_scope_exclusive(node_id: NodeId, scope: Option<NodeId>) -> bool {
    match scope {
        None => true,
        Some(sid) => node_id != sid,
    }
}

/// Check if a compound selector matches a given node.
fn matches_compound(
    doc: &Document,
    node_id: NodeId,
    compound: &CompoundSelector,
    ctx: &CascadeContext<'_>,
) -> bool {
    compound
        .parts
        .iter()
        .all(|simple| matches_simple(doc, node_id, simple, ctx))
}

/// Check if a single simple selector matches a node.
fn matches_simple(
    doc: &Document,
    node_id: NodeId,
    simple: &SimpleSelector,
    ctx: &CascadeContext<'_>,
) -> bool {
    let elem = match &doc.nodes[node_id].kind {
        NodeKind::Element(e) => e,
        _ => return false,
    };

    match simple {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(tag_name) => elem.tag.as_str().eq_ignore_ascii_case(tag_name),
        SimpleSelector::Class(cls) => elem.has_class(cls),
        SimpleSelector::Id(id) => elem.get_attribute("id").is_some_and(|v| v == id),
        SimpleSelector::PseudoClass(pseudo) => match_pseudo_class(doc, node_id, elem, pseudo, ctx),
        SimpleSelector::PseudoClassFn(name, arg) => {
            match_pseudo_class_fn(doc, node_id, elem, name, arg)
        },
        SimpleSelector::Not(inner_list) => !inner_list
            .iter()
            .any(|compound| matches_compound(doc, node_id, compound, ctx)),
        SimpleSelector::Is(inner_list) | SimpleSelector::Where(inner_list) => inner_list
            .iter()
            .any(|compound| matches_compound(doc, node_id, compound, ctx)),
        SimpleSelector::Has(relative_list) => matches_has(doc, node_id, relative_list, ctx),
        SimpleSelector::Attribute { name, op, value } => {
            match_attribute(elem, name, op, value.as_deref())
        },
        SimpleSelector::PseudoElement(_) => {
            // Pseudo-elements don't match real elements directly.
            // They are handled separately by resolve_pseudo_content.
            false
        },
        SimpleSelector::Nest => {
            // Nesting selector is desugared at parse time and should
            // not survive into matching. Treat any residual as non-matching.
            false
        },
    }
}

/// Match pseudo-classes (structural + stateful).
///
/// Structural pseudo-classes (`:first-child`, `:last-child`, etc.)
/// are delegated to the [`selectors`] module. Stateful pseudo-classes
/// (`:hover`, `:visited`, `:link`) remain here because they require
/// the cascade context.
pub(super) fn match_pseudo_class(
    doc: &Document,
    node_id: NodeId,
    elem: &ElementData,
    pseudo: &str,
    ctx: &CascadeContext<'_>,
) -> bool {
    // Stateful pseudo-classes that require CascadeContext.
    match pseudo {
        "hover" => {
            // :hover matches the hovered node and all its ancestors.
            if let Some(hover_nid) = ctx.hover_node {
                let mut current = Some(hover_nid);
                while let Some(nid) = current {
                    if nid == node_id {
                        return true;
                    }
                    current = doc.nodes[nid].parent;
                }
            }
            return false;
        },
        "visited" => {
            // :visited matches <a> elements whose href is in the visited set.
            if elem.tag.as_str().eq_ignore_ascii_case("a")
                && let Some(href) = elem.get_attribute("href")
                && let Some(visited) = ctx.visited_urls
            {
                return visited.contains(href);
            }
            return false;
        },
        "link" => {
            // :link matches <a> elements with href that have NOT been visited.
            if elem.tag.as_str().eq_ignore_ascii_case("a")
                && let Some(href) = elem.get_attribute("href")
            {
                if let Some(visited) = ctx.visited_urls {
                    return !visited.contains(href);
                }
                return true; // No visited info = treat as unvisited.
            }
            return false;
        },
        "focus" | "focus-visible" => {
            // :focus and :focus-visible match the currently focused element.
            if let Some(focused_nid) = ctx.focused_node {
                return focused_nid == node_id;
            }
            return false;
        },
        _ => {},
    }

    // Structural pseudo-classes delegated to the selectors module.
    selectors::matches_pseudo_class(&doc.nodes, node_id, pseudo)
}

/// Match functional pseudo-classes like `:nth-child(An+B)`.
///
/// Delegates to the [`selectors`] module which handles `nth-child`,
/// `nth-last-child`, `nth-of-type`, and `nth-last-of-type`.
fn match_pseudo_class_fn(
    doc: &Document,
    node_id: NodeId,
    _elem: &ElementData,
    name: &str,
    arg: &str,
) -> bool {
    selectors::matches_pseudo_class_fn(&doc.nodes, node_id, name, arg)
}

/// Match an attribute selector against an element.
fn match_attribute(elem: &ElementData, name: &str, op: &AttrOp, value: Option<&str>) -> bool {
    let attr_val = match elem.get_attribute(name) {
        Some(v) => v,
        None => return false,
    };

    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => value.is_some_and(|v| attr_val == v),
        AttrOp::Includes => {
            value.is_some_and(|v| attr_val.split_whitespace().any(|word| word == v))
        },
        AttrOp::DashMatch => {
            value.is_some_and(|v| attr_val == v || attr_val.starts_with(&format!("{v}-")))
        },
        AttrOp::Prefix => value.is_some_and(|v| attr_val.starts_with(v)),
        AttrOp::Suffix => value.is_some_and(|v| attr_val.ends_with(v)),
        AttrOp::Substring => value.is_some_and(|v| attr_val.contains(v)),
    }
}

// -----------------------------------------------------------------------
// DOM traversal helpers
// -----------------------------------------------------------------------

/// Find the nearest ancestor that is an element node.
fn parent_element(doc: &Document, node_id: NodeId) -> Option<NodeId> {
    let mut current = doc.nodes[node_id].parent;
    while let Some(pid) = current {
        if matches!(doc.nodes[pid].kind, NodeKind::Element(_)) {
            return Some(pid);
        }
        current = doc.nodes[pid].parent;
    }
    None
}

/// Match the `:has(relative-selector-list)` relational pseudo-class.
///
/// For each relative selector, collect candidate elements based on the
/// leading combinator (descendants for descendant, direct children for
/// `>`, the next element sibling for `+`, following element siblings
/// for `~`) and succeed if any candidate matches the inner selector
/// with the subject element as the scope boundary. Ancestor walks
/// inside the inner selector stop before reaching the subject, so
/// `article:has(.a .b)` cannot match via an `.a` that lives outside
/// `article`'s subtree.
fn matches_has(
    doc: &Document,
    node_id: NodeId,
    list: &[(Combinator, super::super::parser::Selector)],
    ctx: &CascadeContext<'_>,
) -> bool {
    list.iter().any(|(combinator, sel)| {
        let scope = Some(node_id);
        let sc = Some(combinator);
        let matched = |cand: NodeId| matches_selector_scoped(doc, cand, sel, ctx, scope, sc);
        match combinator {
            Combinator::Descendant | Combinator::Child => {
                // DFS the subject's subtree. For Descendant the scope
                // bounding is sufficient; for Child the scope_combinator
                // check in matches_selector_scoped verifies the leftmost
                // matched compound is a direct child of the subject.
                let mut stack: Vec<NodeId> = doc.nodes[node_id].children.clone();
                while let Some(nid) = stack.pop() {
                    if matches!(doc.nodes[nid].kind, NodeKind::Element(_)) && matched(nid) {
                        return true;
                    }
                    stack.extend(doc.nodes[nid].children.iter().copied());
                }
                false
            },
            Combinator::AdjacentSibling => {
                if let Some(sib) = next_sibling_element(doc, node_id) {
                    if matched(sib) {
                        return true;
                    }
                    if sel.parts.len() > 1 {
                        let mut stack: Vec<NodeId> = doc.nodes[sib].children.clone();
                        while let Some(nid) = stack.pop() {
                            if matches!(doc.nodes[nid].kind, NodeKind::Element(_)) && matched(nid) {
                                return true;
                            }
                            stack.extend(doc.nodes[nid].children.iter().copied());
                        }
                    }
                }
                false
            },
            Combinator::GeneralSibling => {
                let mut sib = next_sibling_element(doc, node_id);
                while let Some(sid) = sib {
                    if matched(sid) {
                        return true;
                    }
                    if sel.parts.len() > 1 {
                        let mut stack: Vec<NodeId> = doc.nodes[sid].children.clone();
                        while let Some(nid) = stack.pop() {
                            if matches!(doc.nodes[nid].kind, NodeKind::Element(_)) && matched(nid) {
                                return true;
                            }
                            stack.extend(doc.nodes[nid].children.iter().copied());
                        }
                    }
                    sib = next_sibling_element(doc, sid);
                }
                false
            },
        }
    })
}

/// Find the next sibling that is an element node.
fn next_sibling_element(doc: &Document, node_id: NodeId) -> Option<NodeId> {
    let parent = doc.nodes[node_id].parent?;
    let siblings = &doc.nodes[parent].children;
    let mut seen = false;
    for &sid in siblings {
        if seen && matches!(doc.nodes[sid].kind, NodeKind::Element(_)) {
            return Some(sid);
        }
        if sid == node_id {
            seen = true;
        }
    }
    None
}

/// Find the previous sibling that is an element node.
fn previous_sibling_element(doc: &Document, node_id: NodeId) -> Option<NodeId> {
    let parent = doc.nodes[node_id].parent?;
    let siblings = &doc.nodes[parent].children;
    let mut prev_elem = None;
    for &sid in siblings {
        if sid == node_id {
            return prev_elem;
        }
        if matches!(doc.nodes[sid].kind, NodeKind::Element(_)) {
            prev_elem = Some(sid);
        }
    }
    None
}

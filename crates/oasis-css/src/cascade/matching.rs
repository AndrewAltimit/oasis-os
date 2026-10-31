//! Selector matching and DOM traversal helpers.
//!
//! Implements compound/simple/combinator matching, pseudo-class and
//! pseudo-element matching, attribute matching, and presentational
//! HTML attribute injection.

use super::super::parser::{
    AttrOp, Combinator, CompoundSelector, ContainerCondition, ContainerFeature, CssColor, CssValue,
    LengthUnit, Rule, ScopeCondition, SimpleSelector, Specificity, Stylesheet,
};
use super::super::selectors;
use super::super::values::ComputedStyle;
use super::CascadeContext;
use oasis_html::dom::{Document, ElementData, NodeId, NodeKind};

use rustc_hash::FxHashMap;
use std::borrow::Cow;

// -----------------------------------------------------------------------
// Robustness limits
// -----------------------------------------------------------------------

/// Maximum number of compounds in one complex selector. Longer
/// selectors are treated as non-matching (this also bounds the matcher's
/// recursion depth).
const MAX_SELECTOR_DEPTH: usize = 64;

/// Maximum number of DOM traversal steps (combinator candidates plus
/// `:has()` subtree nodes) spent matching one selector against one
/// element. Exhaustion is treated as "no match", which bounds hostile
/// selector/DOM combinations; ordinary pages stay far below it.
const MAX_TRAVERSAL_STEPS: usize = 16_384;

/// Sibling nodes structural pseudo-classes may scan while matching one
/// selector against one element (see `Budget::charge_scan`): one check
/// on a list of up to this many children always fits, as do a compound's
/// few checks (`li:nth-child(odd):not(:last-child)`) on a list of a
/// fraction of it. Child-index checks repeated on one parent are
/// memoized (`Budget::sibling_pos`), so they pay a few scans, not one
/// each.
const MAX_STRUCTURAL_SCAN: usize = 128 * 1024;

// -----------------------------------------------------------------------
// Matched declaration types
// -----------------------------------------------------------------------

/// The origin of a declaration for cascade ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    /// From the built-in user-agent sheet
    /// ([`default_stylesheet`](crate::default::default_stylesheet)).
    UserAgent,
    /// From an HTML presentational attribute (lowest priority author style).
    Presentational,
    /// From a `<link>` or `<style>` stylesheet.
    Stylesheet,
    /// From the element's `style=""` attribute.
    Inline,
}

impl Origin {
    /// The origin of rules from `sheet`: the user-agent origin for the
    /// built-in UA sheet (recognised by identity), author otherwise.
    pub(super) fn of_sheet(sheet: &Stylesheet) -> Self {
        if std::ptr::eq(sheet, crate::default::default_stylesheet()) {
            Origin::UserAgent
        } else {
            Origin::Stylesheet
        }
    }
}

impl MatchedDeclaration<'_> {
    /// Origin-and-importance tier (CSS Cascade 5 §6.1), lowest first:
    /// normal UA < presentational hints < normal author < normal inline
    /// < important author < important inline < important UA.
    fn cascade_tier(&self) -> u8 {
        match (self.important, self.origin) {
            (false, Origin::UserAgent) => 0,
            (_, Origin::Presentational) => 1,
            (false, Origin::Stylesheet) => 2,
            (false, Origin::Inline) => 3,
            (true, Origin::Stylesheet) => 4,
            (true, Origin::Inline) => 5,
            (true, Origin::UserAgent) => 6,
        }
    }

    /// True if `self` sorts in a lower cascade layer than `other` of the
    /// same tier (what `revert-layer` rolls back to), or in a lower tier.
    pub(super) fn below_layer_of(&self, other: &Self) -> bool {
        match self.cascade_tier().cmp(&other.cascade_tier()) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => compare_layers(self, other).is_lt(),
        }
    }

    /// True if `self` comes from a lower origin than `other` (what
    /// `revert` rolls back to). Presentational hints and inline styles
    /// count as author, and importance is not an origin: an author
    /// `revert !important` rolls back past every author declaration,
    /// normal ones included, to the user-agent origin.
    pub(super) fn below_origin_of(&self, other: &Self) -> bool {
        let origin_rank = |d: &Self| match d.origin {
            Origin::UserAgent => 0,
            Origin::Presentational | Origin::Stylesheet | Origin::Inline => 1,
        };
        origin_rank(self) < origin_rank(other)
    }
}

/// Cascade order of two declarations (ascending: for one property the
/// last one wins): origin and importance, then layer, specificity and
/// source order.
pub(super) fn cascade_order(a: &MatchedDeclaration, b: &MatchedDeclaration) -> std::cmp::Ordering {
    a.cascade_tier()
        .cmp(&b.cascade_tier())
        .then_with(|| compare_layers(a, b))
        .then_with(|| a.specificity.cmp(&b.specificity))
        .then_with(|| a.source_order.cmp(&b.source_order))
}

/// A single declaration together with its cascade metadata.
///
/// Property and value borrow from the stylesheet (or inline style) they
/// came from: every element matches many declarations, and copying each
/// one's strings and values per element dominated the cascade.
#[derive(Debug, Clone)]
pub(super) struct MatchedDeclaration<'a> {
    pub(super) property: Cow<'a, str>,
    pub(super) value: Cow<'a, CssValue>,
    pub(super) important: bool,
    pub(super) origin: Origin,
    pub(super) specificity: Specificity,
    pub(super) source_order: usize,
    /// Cascade-layer index inside the source stylesheet, or `None`
    /// for unlayered rules / non-stylesheet origins.
    pub(super) layer: Option<u16>,
}

/// Test whether `node_id`'s nearest matching container ancestor
/// satisfies a `@container` condition, given the current layout
/// snapshot in `ctx.containers`.
///
/// Returns `false` when there's no container snapshot (the first cascade
/// pass before layout) or when no ancestor matches the condition's name.
/// An empty feature list is treated as never-matching to stay safe
/// against parser failures producing degenerate conditions.
pub(super) fn container_condition_matches(
    doc: &Document,
    node_id: NodeId,
    cond: &ContainerCondition,
    ctx: &CascadeContext<'_>,
) -> bool {
    let Some(lookup) = ctx.containers else {
        return false;
    };
    if cond.features.is_empty() {
        return false;
    }
    let mut cur = doc.nodes[node_id].parent;
    while let Some(pid) = cur {
        if let Some(entry) = lookup.get(pid) {
            // Skip ancestors whose name doesn't match the requested one.
            let name_ok = match cond.name.as_deref() {
                None => true,
                Some(want) => entry.names.iter().any(|n| n == want),
            };
            if name_ok {
                return cond.features.iter().all(|f| eval_feature(f, entry));
            }
        }
        cur = doc.nodes[pid].parent;
    }
    false
}

/// Test whether `node_id` is inside an `@scope (root) [to (limit)]?`
/// region. Walks up the DOM from the element, failing fast on a limit
/// boundary and succeeding when an ancestor matches the root selector.
/// When `cond.root` is `None` the rule applies anywhere not under a
/// limit boundary.
pub(super) fn scope_condition_matches(
    doc: &Document,
    node_id: NodeId,
    cond: &ScopeCondition,
    ctx: &CascadeContext<'_>,
) -> bool {
    use super::super::parser::parse_selector_string;

    let root_sel = cond.root.as_deref().and_then(parse_selector_string);
    let limit_sel = cond.limit.as_deref().and_then(parse_selector_string);

    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if matches!(doc.nodes[id].kind, NodeKind::Element(_)) {
            if let Some(ref limit) = limit_sel {
                for sel in &limit.selectors {
                    if matches_selector(doc, id, sel, ctx) {
                        return false;
                    }
                }
            }
            if let Some(ref root) = root_sel {
                for sel in &root.selectors {
                    if matches_selector(doc, id, sel, ctx) {
                        return true;
                    }
                }
            }
        }
        cur = doc.nodes[id].parent;
    }
    // Walked all the way to the document root without crossing a
    // limit. Accept iff there's no root constraint.
    cond.root.is_none()
}

fn eval_feature(f: &ContainerFeature, entry: &super::ContainerEntry) -> bool {
    use crate::values::types::ContainerType;

    let is_block_axis = matches!(
        f,
        ContainerFeature::MinHeight(_)
            | ContainerFeature::MaxHeight(_)
            | ContainerFeature::Height(_)
    );
    if is_block_axis && entry.container_type == ContainerType::InlineSize {
        return false;
    }
    match f {
        ContainerFeature::MinWidth(px) => entry.width >= *px,
        ContainerFeature::MaxWidth(px) => entry.width <= *px,
        ContainerFeature::Width(px) => (entry.width - *px).abs() < 0.5,
        ContainerFeature::MinHeight(px) => entry.height >= *px,
        ContainerFeature::MaxHeight(px) => entry.height <= *px,
        ContainerFeature::Height(px) => (entry.height - *px).abs() < 0.5,
        ContainerFeature::Style(prop, val) => {
            // Style queries match when the container's custom properties
            // contain the expected value.
            entry
                .custom_properties
                .get(prop.as_str())
                .is_some_and(|v| v.trim() == val.trim())
        },
    }
}

/// Compare two declarations by cascade-layer position.
///
/// Spec semantics:
/// - Unlayered author rules beat layered author rules for normal
///   declarations; `!important` reverses that (layered `!important`
///   wins over unlayered `!important`).
/// - Earlier-declared layers lose to later-declared layers for normal
///   declarations; `!important` reverses again (earlier layers win).
/// - Layer indices are global across stylesheets when
///   `GlobalLayerMap` is populated.
fn compare_layers(a: &MatchedDeclaration, b: &MatchedDeclaration) -> std::cmp::Ordering {
    use std::cmp::Ordering;

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
#[cfg(test)]
pub(super) fn resolve_pseudo_style(
    doc: &Document,
    node_id: NodeId,
    pseudo: &str,
    element_style: &ComputedStyle,
    stylesheets: &[&Stylesheet],
    ctx: &CascadeContext<'_>,
) -> Option<ComputedStyle> {
    let index = super::SelectorIndex::build(stylesheets);
    let candidates = pseudo_element_candidates(doc, node_id, &index, &mut FxHashMap::default());
    resolve_pseudo_style_indexed(
        doc,
        node_id,
        pseudo,
        element_style,
        stylesheets,
        &candidates,
        ctx,
    )
}

/// The rules with pseudo-element selectors that might apply to
/// `node_id`, from `index`, in source order.
pub(super) fn pseudo_element_candidates(
    doc: &Document,
    node_id: NodeId,
    index: &super::index::SelectorIndex,
    tag_cache: &mut FxHashMap<String, String>,
) -> Vec<super::index::IndexedRule> {
    let NodeKind::Element(elem) = &doc.nodes[node_id].kind else {
        return Vec::new();
    };
    let tag = elem.tag.as_str();
    let classes: Vec<&str> = elem
        .get_attribute("class")
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if !tag_cache.contains_key(tag) {
        tag_cache.insert(tag.to_string(), tag.to_ascii_lowercase());
    }
    let tag_lower = tag_cache.get(tag).expect("just inserted");
    index.pseudo_element_candidates(tag, tag_lower, elem.get_attribute("id"), &classes)
}

/// Resolve the `::before` / `::after` (`pseudo`) style of `node_id`
/// from the pseudo-element rule `candidates` (see
/// [`pseudo_element_candidates`]). Returns `None` when no rule gives it
/// `content`.
pub(super) fn resolve_pseudo_style_indexed(
    doc: &Document,
    node_id: NodeId,
    pseudo: &str,
    element_style: &ComputedStyle,
    stylesheets: &[&Stylesheet],
    candidates: &[super::index::IndexedRule],
    ctx: &CascadeContext<'_>,
) -> Option<ComputedStyle> {
    use super::super::values::Display;

    // Collect all matching declarations with cascade metadata.
    let mut matched: Vec<MatchedDeclaration> = Vec::new();

    for entry in candidates {
        let sheet_idx = entry.sheet_idx;
        let rule = &stylesheets[sheet_idx].rules[entry.rule_idx];
        let decl_base = entry.source_order_base;
        for selector in &rule.selectors.selectors {
            if selector_pseudo_element(selector) != Some(pseudo) {
                continue;
            }
            if let Some(cond) = &rule.container
                && !container_condition_matches(doc, node_id, cond, ctx)
            {
                break;
            }
            if let Some(scope) = &rule.scope
                && !scope_condition_matches(doc, node_id, scope, ctx)
            {
                break;
            }
            if !matches_selector_ignoring_pseudo(doc, node_id, selector, ctx) {
                continue;
            }
            let specificity = selector.specificity();
            let global_layer = rule.layer.and_then(|local| {
                if let Some(glm) = ctx.global_layers {
                    glm.get(&(sheet_idx as u16, local)).copied()
                } else {
                    Some(local)
                }
            });
            for (i, decl) in rule.declarations.iter().enumerate() {
                matched.push(MatchedDeclaration {
                    property: Cow::Borrowed(&decl.property),
                    value: Cow::Borrowed(&decl.value),
                    important: decl.important,
                    origin: Origin::of_sheet(stylesheets[sheet_idx]),
                    specificity,
                    source_order: decl_base + i,
                    layer: global_layer,
                });
            }
        }
    }

    if matched.is_empty() {
        return None;
    }

    // Sort by cascade order.
    matched.sort_by(cascade_order);

    // The winning `content` value; none (or `content: none | normal`)
    // means no pseudo-element box.
    let content_value = matched
        .iter()
        .rev()
        .find(|d| d.property == "content")
        .map(|d| &*d.value)
        .filter(|v| matches!(v, CssValue::String(_) | CssValue::Unresolved(_)))?;

    // Pseudo-elements inherit from the originating element.
    let mut style = ComputedStyle::inherit(element_style);
    // Default display for pseudo-elements is inline (CSS spec).
    style.display = Display::Inline;

    let parent_font_size = element_style.font_size;

    // Custom properties first, as for elements: a `var()` in any other
    // declaration (and in `content`) must see the pseudo-element's own
    // `--*` values, even when the custom property sorts later in
    // cascade order.
    let mut declared_custom: Vec<&str> = Vec::new();
    for (i, entry) in matched.iter().enumerate() {
        if entry.property.starts_with("--") {
            super::apply::apply_matched_custom(&mut style, Some(element_style), &matched, i);
            declared_custom.push(&entry.property);
        }
    }
    if !declared_custom.is_empty() {
        super::var_resolve::resolve_custom_properties(
            std::sync::Arc::make_mut(&mut style.custom_properties),
            &declared_custom,
        );
    }

    let content_text = match content_value {
        CssValue::String(s) => s.clone(),
        // `content: var(--icon)`.
        CssValue::Unresolved(text) => {
            let text = super::var_resolve::substitute_vars(text, &style.custom_properties)?;
            crate::parser::parse_substituted_declaration("content", &text)
                .into_iter()
                .find_map(|d| match d.value {
                    CssValue::String(s) => Some(s),
                    _ => None,
                })?
        },
        _ => return None,
    };

    // Apply font-size first (for em-unit resolution in other properties).
    let parent = Some(element_style);
    for (i, entry) in matched.iter().enumerate() {
        if entry.property == "font-size" {
            super::apply::apply_matched(&mut style, parent, &matched, i, parent_font_size);
        }
    }
    let pseudo_font_size = style.font_size;

    // Apply all other declarations.
    for (i, entry) in matched.iter().enumerate() {
        if entry.property == "font-size"
            || entry.property == "content"
            || entry.property.starts_with("--")
        {
            continue;
        }
        super::apply::apply_matched(&mut style, parent, &matched, i, pseudo_font_size);
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
pub(super) fn collect_matched_declarations<'a>(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&'a Stylesheet],
    index: &super::index::SelectorIndex,
    inline_map: &FxHashMap<NodeId, &'a [super::super::parser::Declaration]>,
    ctx: &CascadeContext<'_>,
    tag_cache: &mut FxHashMap<String, String>,
) -> Vec<MatchedDeclaration<'a>> {
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
    // The element's ancestor filter, built on first use.
    let mut ancestors = None;

    for candidate in &candidates {
        let rule = &stylesheets[candidate.sheet_idx].rules[candidate.rule_idx];
        // `@container`-gated rules only contribute when the nearest
        // matching container satisfies the condition. Skip the whole
        // rule (including ancestor walks) if it can't match.
        if let Some(cond) = &rule.container
            && !container_condition_matches(doc, node_id, cond, ctx)
        {
            continue;
        }
        // `@scope`-gated rules need an in-scope ancestor.
        if let Some(scope) = &rule.scope
            && !scope_condition_matches(doc, node_id, scope, ctx)
        {
            continue;
        }
        let masks = index.selector_masks(candidate.sheet_idx, candidate.rule_idx);
        let best_specificity =
            matching_specificity_filtered(doc, node_id, rule, ctx, masks, &mut ancestors);
        if let Some(specificity) = best_specificity {
            // Convert sheet-local layer index to global if the global
            // map is available (cross-stylesheet @layer merging).
            let global_layer = rule.layer.and_then(|local| {
                if let Some(glm) = ctx.global_layers {
                    glm.get(&(candidate.sheet_idx as u16, local)).copied()
                } else {
                    Some(local)
                }
            });
            let origin = Origin::of_sheet(stylesheets[candidate.sheet_idx]);
            for (decl_idx, decl) in rule.declarations.iter().enumerate() {
                result.push(MatchedDeclaration {
                    property: Cow::Borrowed(&decl.property),
                    value: Cow::Borrowed(&decl.value),
                    important: decl.important,
                    origin,
                    specificity,
                    source_order: candidate.source_order_base + decl_idx,
                    layer: global_layer,
                });
            }
        }
    }

    // Presentational HTML attributes (bgcolor, width, height, align, etc.)
    // have lowest author-origin priority (overridden by any CSS rule).
    collect_presentational_attrs(doc, node_id, elem, &mut result);

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
        let inline_base = index.declaration_count();
        for (i, decl) in decls.iter().enumerate() {
            result.push(MatchedDeclaration {
                property: Cow::Borrowed(&decl.property),
                value: Cow::Borrowed(&decl.value),
                important: decl.important,
                origin: Origin::Inline,
                specificity: inline_spec,
                source_order: inline_base + i,
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
fn collect_presentational_attrs(
    doc: &Document,
    node_id: NodeId,
    elem: &ElementData,
    result: &mut Vec<MatchedDeclaration<'_>>,
) {
    use oasis_html::dom::TagName;

    let zero_spec = Specificity {
        inline: 0,
        ids: 0,
        classes: 0,
        types: 0,
    };

    let mut push = |property: &'static str, value: CssValue| {
        result.push(MatchedDeclaration {
            property: Cow::Borrowed(property),
            value: Cow::Owned(value),
            important: false,
            origin: Origin::Presentational,
            specificity: zero_spec,
            source_order: 0,
            layer: None,
        });
    };

    // bgcolor -> background-color
    if let Some(color_str) = elem.get_attribute("bgcolor")
        && let Some(color) = parse_html_color(color_str)
    {
        push("background-color", CssValue::Color(color));
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

    // valign attribute -> vertical-align (on tr, td, th)
    //
    // Legacy HTML `valign="top|middle|bottom|baseline"` maps directly to
    // the CSS `vertical-align` keyword. Google's search-row is
    // `<tr valign="top">` so the top-aligned cell content sits flush
    // with the top of the row.
    if matches!(elem.tag, TagName::Tr | TagName::Td | TagName::Th)
        && let Some(val) = elem.get_attribute("valign")
    {
        let v = val.to_ascii_lowercase();
        if matches!(v.as_str(), "top" | "middle" | "bottom" | "baseline") {
            push("vertical-align", CssValue::Keyword(v));
        }
    }

    // cellspacing attribute on table -> border-spacing
    if elem.tag == TagName::Table
        && let Some(val) = elem.get_attribute("cellspacing")
        && let Ok(n) = val.parse::<f32>()
    {
        push("border-spacing", CssValue::Length(n, LengthUnit::Px));
    }

    // cellpadding attribute on table propagates to descendant cells as
    // padding. We look up the nearest ancestor `<table>` from each td/th
    // and apply its cellpadding to the cell. This overrides the UA
    // default `td { padding: 2px 4px }` for legacy layouts that rely
    // on `cellpadding="0"` (Google's search-form table is the canonical
    // example).
    if matches!(elem.tag, TagName::Td | TagName::Th)
        && let Some(pad) = ancestor_table_cellpadding(doc, node_id)
    {
        push("padding-top", CssValue::Length(pad, LengthUnit::Px));
        push("padding-right", CssValue::Length(pad, LengthUnit::Px));
        push("padding-bottom", CssValue::Length(pad, LengthUnit::Px));
        push("padding-left", CssValue::Length(pad, LengthUnit::Px));
    }

    // <br clear="left|right|all"> -> clear: left|right|both.
    // A legacy way to break out of a float context.
    if elem.tag == TagName::Br
        && let Some(val) = elem.get_attribute("clear")
    {
        let v = val.to_ascii_lowercase();
        let mapped = match v.as_str() {
            "left" | "right" => Some(v),
            "all" => Some("both".to_string()),
            _ => None,
        };
        if let Some(k) = mapped {
            push("clear", CssValue::Keyword(k));
        }
    }

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

/// Walk up the DOM from `start` looking for the nearest `<table>`
/// ancestor; if found and it has a `cellpadding="N"` attribute with
/// a non-negative integer value, return that padding in pixels.
///
/// Used to propagate the table's cellpadding to descendant cells so
/// `cellpadding="0"` actually collapses the UA default cell padding.
fn ancestor_table_cellpadding(doc: &Document, start: NodeId) -> Option<f32> {
    use oasis_html::dom::TagName;
    let mut cur = doc.nodes[start].parent;
    while let Some(pid) = cur {
        if let NodeKind::Element(ref e) = doc.nodes[pid].kind
            && e.tag == TagName::Table
        {
            return e
                .get_attribute("cellpadding")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| *v >= 0.0);
        }
        cur = doc.nodes[pid].parent;
    }
    None
}

/// Parse an HTML colour attribute value (`"#fff"`, `"#ffffff"`, `"red"`):
/// hex digits or one of the 148 CSS colour names. Byte-safe on
/// non-ASCII input (a hand-rolled slicer here used to panic on
/// `bgcolor="#\u{e9}1"`).
fn parse_html_color(s: &str) -> Option<CssColor> {
    let s = s.trim();
    match s.strip_prefix('#') {
        Some(hex) => crate::color::parse_hex_color(hex),
        None => crate::color::named_color(s),
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
/// `node_id`, or `None` if no selector matches. Skips selectors that
/// target pseudo-elements (::before, ::after), and every selector whose
/// ancestor `masks` (the rule's [`SelectorIndex::selector_masks`]) the
/// element's ancestor filter (built into `ancestors` on first use) does
/// not cover: such a selector cannot match, so the result is the same.
///
/// [`SelectorIndex::selector_masks`]: super::index::SelectorIndex::selector_masks
fn matching_specificity_filtered(
    doc: &Document,
    node_id: NodeId,
    rule: &Rule,
    ctx: &CascadeContext<'_>,
    masks: &[Option<super::ancestor_filter::AncestorMask>],
    ancestors: &mut Option<super::ancestor_filter::AncestorMask>,
) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for (i, selector) in rule.selectors.selectors.iter().enumerate() {
        if selector_pseudo_element(selector).is_some() {
            continue; // Skip pseudo-element selectors in normal matching.
        }
        if let Some(Some(mask)) = masks.get(i)
            && !ancestors
                .get_or_insert_with(|| super::ancestor_filter::ancestor_filter(doc, node_id))
                .covers(mask)
        {
            continue;
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
pub fn matches_selector(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
) -> bool {
    matches_selector_scoped(doc, node_id, selector, ctx, None, None, &Budget::new())
}

/// [`matches_selector`] plus the budget (steps and scan units) it spent.
#[cfg(test)]
pub(super) fn matches_selector_cost(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
) -> (bool, usize) {
    let budget = Budget::new();
    let matched = matches_selector_scoped(doc, node_id, selector, ctx, None, None, &budget);
    (matched, budget.spent())
}

/// Outcome of matching a selector suffix against one candidate element.
///
/// The three failure kinds let the right-to-left walk prune retries
/// that cannot succeed (the scheme Servo / Firefox use), which keeps
/// backtracking linear in practice instead of exponential:
///
/// * `RestartFromClosestLaterSibling` -- this candidate failed; any
///   enclosing combinator may try its next candidate.
/// * `RestartFromClosestDescendant` -- a `+` / `~` chain ran out of
///   siblings; only an enclosing descendant (` `) combinator may still
///   retry with a higher ancestor.
/// * `Globally` -- an ancestor walk ran out of ancestors (or the
///   traversal budget ran out); no retry anywhere can succeed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MatchResult {
    Matched,
    RestartFromClosestLaterSibling,
    RestartFromClosestDescendant,
    Globally,
}

/// Per-match traversal budget shared by combinator walks, sibling-list
/// scans and `:has()` subtree scans. Exhaustion is treated as "no
/// match" for the whole selector (including under `:not()`).
struct Budget {
    left: std::cell::Cell<usize>,
    /// Separate allowance for sibling-index pseudo-class scans
    /// ([`Budget::charge_scan`]).
    scan_left: std::cell::Cell<usize>,
    exhausted: std::cell::Cell<bool>,
    /// Memoized sibling positions ([`Budget::sibling_pos`]).
    siblings: std::cell::RefCell<SiblingMemo>,
}

/// Where an element sits among its parent's element children: all a
/// sibling-index pseudo-class (`:nth-child()`, `:last-of-type`, ...)
/// needs. 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SiblingPos {
    index: usize,
    count: usize,
}

/// Per-match memo of one parent's sibling positions, so a check repeated
/// for many siblings (`:has(> li:last-child)`, `:has(~ :nth-last-child(..))`,
/// `li:nth-child(odd) ~ li`) costs O(1) per check instead of a scan of the
/// whole list each time. Only the first check on a parent is a plain scan
/// (one `li:nth-child(odd)` per element stays as cheap as before); the
/// second indexes the parent with one more scan and no hashing. Only one
/// parent is kept: a match alternating between parents (nested lists
/// inside `:has()`) re-indexes on each switch, paying the scan again, so
/// it stays bounded by the allowance, no worse than without the memo.
/// `:first-child` / `:last-child` / `:only-child` do not use it: they walk
/// in from one end of the list ([`Budget::is_edge_child`]).
enum SiblingMemo {
    Empty,
    /// One plain scan of this parent was done.
    Scanned(NodeId),
    Indexed {
        parent: NodeId,
        /// 1-based element index per child slot (0 for non-elements).
        index: Vec<u32>,
        count: usize,
        /// Child slot of the last lookup: candidates are usually visited
        /// in order, so the next one is at, right after or right before it.
        hint: usize,
    },
}

impl Budget {
    fn new() -> Self {
        Self {
            left: std::cell::Cell::new(MAX_TRAVERSAL_STEPS),
            scan_left: std::cell::Cell::new(MAX_STRUCTURAL_SCAN),
            exhausted: std::cell::Cell::new(false),
            siblings: std::cell::RefCell::new(SiblingMemo::Empty),
        }
    }

    /// Whether `node` is the first (`from_start`) or last element child of
    /// its parent. Walks in from that end to the nearest element, charging
    /// only the nodes it passes, so `li:first-child` on a long list is O(1)
    /// per element rather than a scan of the whole list.
    fn is_edge_child(&self, doc: &Document, node: NodeId, from_start: bool) -> bool {
        let Some(parent) = doc.nodes[node].parent else {
            return false;
        };
        if !matches!(doc.nodes[node].kind, NodeKind::Element(_)) {
            return false;
        }
        let kids = doc.nodes[parent].children.as_slice();
        let is_elem = |c: &NodeId| matches!(doc.nodes[*c].kind, NodeKind::Element(_));
        let (passed, edge) = if from_start {
            let p = kids.iter().position(is_elem);
            (p.map_or(kids.len(), |p| p + 1), p.map(|p| kids[p]))
        } else {
            let p = kids.iter().rposition(is_elem);
            (p.map_or(kids.len(), |p| kids.len() - p), p.map(|p| kids[p]))
        };
        self.charge_scan(passed) && edge == Some(node)
    }

    /// `node`'s position among its parent's element children for
    /// `:nth-child()` / `:nth-last-child()` (the `*-of-type` ones scan on
    /// their own), or `None` for an orphan / non-element or once the scan
    /// allowance is spent.
    fn sibling_pos(&self, doc: &Document, node: NodeId) -> Option<SiblingPos> {
        let parent = doc.nodes[node].parent?;
        let kids = doc.nodes[parent].children.as_slice();
        let mut memo = self.siblings.borrow_mut();
        match &mut *memo {
            SiblingMemo::Indexed {
                parent: p,
                index,
                count,
                hint,
            } if *p == parent => {
                // `index` was built from this list; a lookup off the hint
                // is a scan, charged like one.
                let at = |i: usize| kids.get(i) == Some(&node);
                let back = hint.checked_sub(1);
                let slot = if at(*hint) {
                    *hint
                } else if at(*hint + 1) {
                    *hint + 1
                } else if let Some(b) = back.filter(|&b| at(b)) {
                    // Walks over earlier siblings (`li:nth-child(odd) ~ li`).
                    b
                } else {
                    if !self.charge_scan(kids.len()) {
                        return None;
                    }
                    kids.iter().position(|&c| c == node)?
                };
                *hint = slot;
                let i = *index.get(slot)? as usize;
                return (i > 0).then_some(SiblingPos {
                    index: i,
                    count: *count,
                });
            },
            SiblingMemo::Scanned(p) if *p == parent => {
                if !self.charge_scan(kids.len()) {
                    return None;
                }
                let mut index = Vec::with_capacity(kids.len());
                let mut count = 0u32;
                for &c in kids {
                    let is_elem = matches!(doc.nodes[c].kind, NodeKind::Element(_));
                    count += u32::from(is_elem);
                    index.push(if is_elem { count } else { 0 });
                }
                *memo = SiblingMemo::Indexed {
                    parent,
                    index,
                    count: count as usize,
                    hint: 0,
                };
                drop(memo);
                return self.sibling_pos(doc, node);
            },
            _ => {},
        }
        if !self.charge_scan(kids.len()) {
            return None;
        }
        *memo = SiblingMemo::Scanned(parent);
        let (mut index, mut count) = (0, 0);
        for &c in kids {
            if matches!(doc.nodes[c].kind, NodeKind::Element(_)) {
                count += 1;
                if c == node {
                    index = count;
                }
            }
        }
        (index > 0 && matches!(doc.nodes[node].kind, NodeKind::Element(_)))
            .then_some(SiblingPos { index, count })
    }

    /// Consume `n` nodes of sibling-list scanning by a structural
    /// pseudo-class (`:nth-child()`, `:last-of-type`, ...); `false` once
    /// that allowance is spent. Kept apart from the step budget so one
    /// `li:nth-child(odd)` on a huge list still matches, while the same
    /// check repeated per candidate (`:has(~ :nth-last-child(..))`,
    /// `.a ~ :nth-child(..)`) can no longer go cubic.
    #[inline]
    fn charge_scan(&self, n: usize) -> bool {
        let left = self.scan_left.get();
        if n > left {
            self.scan_left.set(0);
            self.exhausted.set(true);
            return false;
        }
        self.scan_left.set(left - n);
        true
    }

    /// Consume one step; `false` once the budget is spent.
    #[inline]
    fn tick(&self) -> bool {
        self.charge(1)
    }

    /// Consume `n` steps (the nodes a scan visited); `false` once the
    /// budget is spent.
    #[inline]
    fn charge(&self, n: usize) -> bool {
        let left = self.left.get();
        if n > left {
            self.left.set(0);
            self.exhausted.set(true);
            return false;
        }
        self.left.set(left - n);
        true
    }

    /// Whether some step was refused: any result computed since is
    /// unreliable and must not count as a match.
    #[inline]
    fn exhausted(&self) -> bool {
        self.exhausted.get()
    }

    /// Steps plus scan units spent so far (tests pin the cost of
    /// pathological selectors with it).
    #[cfg(test)]
    fn spent(&self) -> usize {
        (MAX_TRAVERSAL_STEPS - self.left.get()) + (MAX_STRUCTURAL_SCAN - self.scan_left.get())
    }
}

/// Where a node sits among its siblings.
enum SiblingSlot<'d> {
    /// `(parent's children, index of the node in them)`.
    At(&'d [NodeId], usize),
    /// The node has no parent.
    Orphan,
    /// Finding the index would overrun the budget.
    OutOfBudget,
}

/// Locate `node` in its parent's child list, charging the scan that
/// found it to `budget` (sibling lists can be huge, and the index is
/// re-derived at every sibling-combinator step).
fn sibling_slot<'d>(doc: &'d Document, node: NodeId, budget: &Budget) -> SiblingSlot<'d> {
    let Some(parent) = doc.nodes[node].parent else {
        return SiblingSlot::Orphan;
    };
    let kids = doc.nodes[parent].children.as_slice();
    // Never scan further than the budget could pay for.
    let affordable = budget.left.get();
    match kids.iter().take(affordable).position(|&c| c == node) {
        Some(pos) if budget.charge(pos + 1) => SiblingSlot::At(kids, pos),
        Some(_) => SiblingSlot::OutOfBudget,
        None if kids.len() > affordable => {
            budget.charge(affordable + 1);
            SiblingSlot::OutOfBudget
        },
        None => SiblingSlot::Orphan,
    }
}

/// Element siblings before index `pos`, nearest first.
fn elements_before<'d>(
    doc: &'d Document,
    kids: &'d [NodeId],
    pos: usize,
) -> impl Iterator<Item = NodeId> + 'd {
    kids[..pos]
        .iter()
        .rev()
        .copied()
        .filter(move |&id| matches!(doc.nodes[id].kind, NodeKind::Element(_)))
}

/// Element siblings after index `pos`, nearest first.
fn elements_after<'d>(
    doc: &'d Document,
    kids: &'d [NodeId],
    pos: usize,
) -> impl Iterator<Item = NodeId> + 'd {
    kids[pos + 1..]
        .iter()
        .copied()
        .filter(move |&id| matches!(doc.nodes[id].kind, NodeKind::Element(_)))
}

/// Shared state of one complex-selector match.
struct Matcher<'a, 'c> {
    doc: &'a Document,
    parts: &'a [(CompoundSelector, Option<Combinator>)],
    ctx: &'a CascadeContext<'c>,
    scope: Option<NodeId>,
    scope_combinator: Option<&'a Combinator>,
    budget: &'a Budget,
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
///
/// Matching walks right-to-left with backtracking: a descendant or
/// general-sibling combinator tries every candidate until the rest of
/// the selector matches, so `A > B C` matches `<A><B><X><B><C>` via the
/// outer `B`.
fn matches_selector_scoped(
    doc: &Document,
    node_id: NodeId,
    selector: &super::super::parser::Selector,
    ctx: &CascadeContext<'_>,
    scope: Option<NodeId>,
    scope_combinator: Option<&Combinator>,
    budget: &Budget,
) -> bool {
    let parts = &selector.parts;
    if parts.is_empty() || parts.len() > MAX_SELECTOR_DEPTH {
        return false;
    }
    let m = Matcher {
        doc,
        parts,
        ctx,
        scope,
        scope_combinator,
        budget,
    };
    m.match_from(parts.len() - 1, node_id) == MatchResult::Matched && !budget.exhausted()
}

impl Matcher<'_, '_> {
    /// Match `parts[..=idx]` with `parts[idx]` anchored at `node`.
    fn match_from(&self, idx: usize, node: NodeId) -> MatchResult {
        if !matches_compound(self.doc, node, &self.parts[idx].0, self.ctx, self.budget) {
            return MatchResult::RestartFromClosestLaterSibling;
        }
        if idx == 0 {
            return if self.leading_scope_ok(node) {
                MatchResult::Matched
            } else {
                MatchResult::RestartFromClosestLaterSibling
            };
        }
        // The combinator linking `parts[idx - 1]` to `parts[idx]` is
        // stored on `parts[idx]`.
        let combinator = self.parts[idx]
            .1
            .as_ref()
            .unwrap_or(&Combinator::Descendant);
        let doc = self.doc;
        match combinator {
            Combinator::Child => {
                let Some(pid) = parent_element(doc, node) else {
                    return MatchResult::Globally;
                };
                if !in_scope_exclusive(pid, self.scope) || !self.budget.tick() {
                    return MatchResult::Globally;
                }
                match self.match_from(idx - 1, pid) {
                    r @ (MatchResult::Matched | MatchResult::Globally) => r,
                    _ => MatchResult::RestartFromClosestDescendant,
                }
            },
            Combinator::Descendant => {
                let mut ancestor = parent_element(doc, node);
                while let Some(anc) = ancestor {
                    if !in_scope_exclusive(anc, self.scope) || !self.budget.tick() {
                        break;
                    }
                    let r = self.match_from(idx - 1, anc);
                    if matches!(r, MatchResult::Matched | MatchResult::Globally) {
                        return r;
                    }
                    ancestor = parent_element(doc, anc);
                }
                MatchResult::Globally
            },
            Combinator::AdjacentSibling => {
                let (kids, pos) = match sibling_slot(doc, node, self.budget) {
                    SiblingSlot::At(kids, pos) => (kids, pos),
                    SiblingSlot::Orphan => return MatchResult::RestartFromClosestDescendant,
                    SiblingSlot::OutOfBudget => return MatchResult::Globally,
                };
                let Some(sid) = elements_before(doc, kids, pos).next() else {
                    return MatchResult::RestartFromClosestDescendant;
                };
                if !self.budget.tick() {
                    return MatchResult::Globally;
                }
                self.match_from(idx - 1, sid)
            },
            Combinator::GeneralSibling => {
                let (kids, pos) = match sibling_slot(doc, node, self.budget) {
                    SiblingSlot::At(kids, pos) => (kids, pos),
                    SiblingSlot::Orphan => return MatchResult::RestartFromClosestDescendant,
                    SiblingSlot::OutOfBudget => return MatchResult::Globally,
                };
                for sid in elements_before(doc, kids, pos) {
                    if !self.budget.tick() {
                        return MatchResult::Globally;
                    }
                    let r = self.match_from(idx - 1, sid);
                    if r != MatchResult::RestartFromClosestLaterSibling {
                        return r;
                    }
                }
                MatchResult::RestartFromClosestDescendant
            },
        }
    }

    /// Enforce the leading combinator from `:has()`: the node matching
    /// the leftmost compound must stand in that relationship to the
    /// scope element.
    fn leading_scope_ok(&self, leftmost: NodeId) -> bool {
        let (Some(sc), Some(sid)) = (self.scope_combinator, self.scope) else {
            return true;
        };
        let doc = self.doc;
        match sc {
            Combinator::Child => parent_element(doc, leftmost) == Some(sid),
            Combinator::AdjacentSibling => match sibling_slot(doc, leftmost, self.budget) {
                SiblingSlot::At(kids, pos) => elements_before(doc, kids, pos).next() == Some(sid),
                SiblingSlot::Orphan | SiblingSlot::OutOfBudget => false,
            },
            // `leftmost` must be a later sibling of the scope element.
            Combinator::GeneralSibling => {
                doc.nodes[leftmost].parent.is_some()
                    && doc.nodes[leftmost].parent == doc.nodes[sid].parent
                    && match (
                        sibling_slot(doc, sid, self.budget),
                        sibling_slot(doc, leftmost, self.budget),
                    ) {
                        (SiblingSlot::At(_, a), SiblingSlot::At(_, b)) => a < b,
                        _ => false,
                    }
            },
            Combinator::Descendant => true,
        }
    }
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
    budget: &Budget,
) -> bool {
    compound
        .parts
        .iter()
        .all(|simple| matches_simple(doc, node_id, simple, ctx, budget))
}

/// Check if a single simple selector matches a node.
fn matches_simple(
    doc: &Document,
    node_id: NodeId,
    simple: &SimpleSelector,
    ctx: &CascadeContext<'_>,
    budget: &Budget,
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
        // Sibling-index pseudo-classes scan the parent's child list: that
        // work is charged like a sibling combinator walk (the edge ones walk
        // in from one end; the `nth` ones are memoized per parent).
        SimpleSelector::PseudoClass(pseudo) if pseudo == "first-child" => {
            budget.is_edge_child(doc, node_id, true)
        },
        SimpleSelector::PseudoClass(pseudo) if pseudo == "last-child" => {
            budget.is_edge_child(doc, node_id, false)
        },
        SimpleSelector::PseudoClass(pseudo) if pseudo == "only-child" => {
            budget.is_edge_child(doc, node_id, true) && budget.is_edge_child(doc, node_id, false)
        },
        SimpleSelector::PseudoClassFn(name, arg)
            if matches!(name.as_str(), "nth-child" | "nth-last-child") =>
        {
            let Some(anb) = selectors::AnB::parse(arg) else {
                return false;
            };
            budget.sibling_pos(doc, node_id).is_some_and(|p| {
                let i = if name == "nth-child" {
                    p.index
                } else {
                    p.count - p.index + 1
                };
                i32::try_from(i).is_ok_and(|i| anb.matches(i))
            })
        },
        SimpleSelector::PseudoClass(pseudo) => {
            budget.charge_scan(selectors::structural_scan_cost(&doc.nodes, node_id, pseudo))
                && match_pseudo_class(doc, node_id, elem, pseudo, ctx)
        },
        SimpleSelector::PseudoClassFn(name, arg) => {
            budget.charge_scan(selectors::structural_scan_cost(&doc.nodes, node_id, name))
                && match_pseudo_class_fn(doc, node_id, elem, name, arg)
        },
        // An inner result cut short by the budget is "unknown", not
        // "no match": it must not turn into a `:not()` match.
        // The argument selectors are complex and unscoped: in
        // `:is(.a .b)` the `.a` may be any ancestor of the element.
        SimpleSelector::Not(inner_list) => {
            !inner_list
                .iter()
                .any(|sel| matches_selector_scoped(doc, node_id, sel, ctx, None, None, budget))
                && !budget.exhausted()
        },
        SimpleSelector::Is(inner_list) | SimpleSelector::Where(inner_list) => inner_list
            .iter()
            .any(|sel| matches_selector_scoped(doc, node_id, sel, ctx, None, None, budget)),
        SimpleSelector::Has(relative_list) => matches_has(doc, node_id, relative_list, ctx, budget),
        SimpleSelector::Attribute {
            name,
            op,
            value,
            case_insensitive,
        } => match_attribute(elem, name, op, value.as_deref(), *case_insensitive),
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
                    current = doc.nodes.get(nid).and_then(|n| n.parent);
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
///
/// With `case_insensitive` the selector `value` is already ASCII-lowercase
/// (the parser folds it), so only the attribute value needs folding.
/// `^=`, `$=`, `*=` with an empty value and `~=` with an empty or
/// whitespace-containing value match nothing (Selectors 4 §6.1-6.2).
fn match_attribute(
    elem: &ElementData,
    name: &str,
    op: &AttrOp,
    value: Option<&str>,
    case_insensitive: bool,
) -> bool {
    let Some(raw) = elem.get_attribute(name) else {
        return false;
    };
    let Some(v) = value else {
        return *op == AttrOp::Exists;
    };
    let folded;
    let attr_val = if case_insensitive && raw.bytes().any(|b| b.is_ascii_uppercase()) {
        folded = raw.to_ascii_lowercase();
        folded.as_str()
    } else {
        raw
    };

    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => attr_val == v,
        AttrOp::Includes => {
            !v.is_empty()
                && !v.contains(|c: char| c.is_ascii_whitespace())
                && attr_val.split_ascii_whitespace().any(|word| word == v)
        },
        AttrOp::DashMatch => {
            attr_val == v
                || attr_val
                    .strip_prefix(v)
                    .is_some_and(|rest| rest.starts_with('-'))
        },
        AttrOp::Prefix => !v.is_empty() && attr_val.starts_with(v),
        AttrOp::Suffix => !v.is_empty() && attr_val.ends_with(v),
        AttrOp::Substring => !v.is_empty() && attr_val.contains(v),
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
    budget: &Budget,
) -> bool {
    // Depth-first scan of `root`'s subtree (excluding `root`), charging
    // every visited node to the shared budget.
    let any_in_subtree = |root: NodeId, matched: &dyn Fn(NodeId) -> bool| -> bool {
        let mut stack: Vec<NodeId> = doc.nodes[root].children.clone();
        while let Some(nid) = stack.pop() {
            if !budget.tick() {
                return false;
            }
            if matches!(doc.nodes[nid].kind, NodeKind::Element(_)) && matched(nid) {
                return true;
            }
            stack.extend(doc.nodes[nid].children.iter().copied());
        }
        false
    };
    list.iter().any(|(combinator, sel)| {
        let scope = Some(node_id);
        let sc = Some(combinator);
        let matched =
            |cand: NodeId| matches_selector_scoped(doc, cand, sel, ctx, scope, sc, budget);
        match combinator {
            // For Descendant the scope bounding is sufficient; for Child
            // the leading-combinator check verifies the leftmost matched
            // compound is a direct child of the subject.
            // `:has(> x)` with a one-compound selector can only match a
            // direct child: scanning the whole subtree (last child
            // first) could spend the budget before reaching one.
            Combinator::Child if sel.parts.len() == 1 => doc.nodes[node_id]
                .children
                .iter()
                .take_while(|_| budget.tick())
                .any(|&cid| matches!(doc.nodes[cid].kind, NodeKind::Element(_)) && matched(cid)),
            Combinator::Descendant | Combinator::Child => any_in_subtree(node_id, &matched),
            Combinator::AdjacentSibling => match sibling_slot(doc, node_id, budget) {
                SiblingSlot::At(kids, pos) => {
                    elements_after(doc, kids, pos).next().is_some_and(|sib| {
                        matched(sib) || (sel.parts.len() > 1 && any_in_subtree(sib, &matched))
                    })
                },
                SiblingSlot::Orphan | SiblingSlot::OutOfBudget => false,
            },
            Combinator::GeneralSibling => {
                let SiblingSlot::At(kids, pos) = sibling_slot(doc, node_id, budget) else {
                    return false;
                };
                for sid in elements_after(doc, kids, pos) {
                    if !budget.tick() {
                        return false;
                    }
                    if matched(sid) || (sel.parts.len() > 1 && any_in_subtree(sid, &matched)) {
                        return true;
                    }
                }
                false
            },
        }
    })
}

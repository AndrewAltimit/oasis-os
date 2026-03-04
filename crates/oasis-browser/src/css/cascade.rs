//! CSS cascade and selector matching.
//!
//! Implements the CSS cascade algorithm: for each element in a DOM tree,
//! collect matching rules from all stylesheets, sort by specificity and
//! source order, then apply declarations to produce computed styles.

use std::collections::{HashMap, HashSet};

use super::parser::{
    AttrOp, Combinator, CompoundSelector, CssColor, CssValue, Declaration, LengthUnit, Rule,
    SimpleSelector, Specificity, Stylesheet, parse_value_list,
};
use super::selectors;
use super::tokenizer::CssTokenizer;
use super::values::ComputedStyle;
use crate::html::dom::{Document, ElementData, NodeId, NodeKind};

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
// Selector index
// -----------------------------------------------------------------------

/// An indexed reference to a specific rule in a specific stylesheet.
#[derive(Debug, Clone, Copy)]
struct IndexedRule {
    /// Index into the `stylesheets` slice.
    sheet_idx: usize,
    /// Index into the stylesheet's `rules` Vec.
    rule_idx: usize,
    /// Global source order counter for cascade ordering.
    source_order_base: usize,
}

/// Pre-built index that buckets rules by the rightmost (subject)
/// selector's most specific part. This avoids testing every rule
/// against every element — only rules whose subject could possibly
/// match are considered.
pub struct SelectorIndex {
    by_id: HashMap<String, Vec<IndexedRule>>,
    by_class: HashMap<String, Vec<IndexedRule>>,
    by_tag: HashMap<String, Vec<IndexedRule>>,
    universal: Vec<IndexedRule>,
}

impl SelectorIndex {
    /// Build a selector index from a list of stylesheets.
    ///
    /// For each rule, inspects the *rightmost* compound selector (the
    /// subject) and files the rule under the most specific bucket
    /// found: ID > class > tag > universal.
    pub fn build(stylesheets: &[&Stylesheet]) -> Self {
        let mut index = SelectorIndex {
            by_id: HashMap::new(),
            by_class: HashMap::new(),
            by_tag: HashMap::new(),
            universal: Vec::new(),
        };

        let mut source_order: usize = 0;
        for (sheet_idx, sheet) in stylesheets.iter().enumerate() {
            for (rule_idx, rule) in sheet.rules.iter().enumerate() {
                let entry = IndexedRule {
                    sheet_idx,
                    rule_idx,
                    source_order_base: source_order,
                };
                // Count declarations for source_order advancement.
                source_order += rule.declarations.len();

                // Examine each selector individually. If a selector's
                // subject has an id/class/tag key, file the rule in
                // that bucket. If any selector has NO such key (e.g.
                // `*:hover`), the rule must also go into `universal`
                // so non-keyed elements can still match it.
                let mut needs_universal = false;
                for selector in &rule.selectors.selectors {
                    let mut selector_filed = false;
                    if let Some(subject) = selector.parts.last() {
                        for simple in &subject.0.parts {
                            match simple {
                                SimpleSelector::Id(id) => {
                                    index
                                        .by_id
                                        .entry(id.to_ascii_lowercase())
                                        .or_default()
                                        .push(entry);
                                    selector_filed = true;
                                },
                                SimpleSelector::Class(cls) => {
                                    index.by_class.entry(cls.clone()).or_default().push(entry);
                                    selector_filed = true;
                                },
                                SimpleSelector::Type(tag) => {
                                    index
                                        .by_tag
                                        .entry(tag.to_ascii_lowercase())
                                        .or_default()
                                        .push(entry);
                                    selector_filed = true;
                                },
                                _ => {},
                            }
                        }
                    }
                    if !selector_filed {
                        needs_universal = true;
                    }
                }
                // If any selector had no id/class/tag key (e.g. `*`,
                // `:hover`, `:not(...)` only), put it in universal.
                if needs_universal {
                    index.universal.push(entry);
                }
            }
        }

        index
    }

    /// Collect candidate rules that might match an element with the
    /// given tag, id, and classes.
    fn candidates(&self, tag: &str, id: Option<&str>, classes: &[&str]) -> Vec<IndexedRule> {
        let mut result = Vec::new();

        // Always include universal rules.
        result.extend_from_slice(&self.universal);

        // Tag bucket.
        let tag_lower = tag.to_ascii_lowercase();
        if let Some(rules) = self.by_tag.get(&tag_lower) {
            result.extend_from_slice(rules);
        }

        // ID bucket.
        if let Some(id) = id {
            let id_lower = id.to_ascii_lowercase();
            if let Some(rules) = self.by_id.get(&id_lower) {
                result.extend_from_slice(rules);
            }
        }

        // Class buckets.
        for cls in classes {
            if let Some(rules) = self.by_class.get(*cls) {
                result.extend_from_slice(rules);
            }
        }

        // Deduplicate by (sheet_idx, rule_idx) since a rule can appear
        // in multiple buckets if its selector has both class and tag.
        result.sort_by_key(|r| (r.sheet_idx, r.rule_idx, r.source_order_base));
        result.dedup_by_key(|r| (r.sheet_idx, r.rule_idx));

        result
    }
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
        collect_matched_declarations(doc, node_id, stylesheets, index, inline_map, ctx);

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
            let resolved = resolve_css_var(&entry.value, &style.custom_properties);
            style.apply_declaration("font-size", &resolved, parent_font_size);
        }
    }
    let element_font_size = style.font_size;

    // Pass 3: Resolve var() references and apply all other declarations.
    for entry in &matched {
        if entry.property.starts_with("--") || entry.property == "font-size" {
            continue;
        }
        let resolved = resolve_css_var(&entry.value, &style.custom_properties);
        style.apply_declaration(&entry.property, &resolved, element_font_size);
    }

    // Resolve ::before and ::after pseudo-element content.
    style.before_content = resolve_pseudo_content(doc, node_id, "before", stylesheets, ctx);
    style.after_content = resolve_pseudo_content(doc, node_id, "after", stylesheets, ctx);

    style
}

/// Find the `content` property value from matching ::before or ::after
/// rules for a given element. Respects cascade ordering (important,
/// specificity, source order) so that higher-specificity rules win
/// regardless of source order.
fn resolve_pseudo_content(
    doc: &Document,
    node_id: NodeId,
    pseudo: &str,
    stylesheets: &[&Stylesheet],
    ctx: &CascadeContext<'_>,
) -> Option<String> {
    // Collect all matching content declarations with cascade metadata.
    let mut matched: Vec<(bool, Specificity, usize, CssValue)> = Vec::new();
    let mut source_order: usize = 0;

    for stylesheet in stylesheets {
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
                    if decl.property == "content" {
                        matched.push((
                            decl.important,
                            specificity,
                            decl_base + i,
                            decl.value.clone(),
                        ));
                    }
                }
            }
        }
    }

    // Sort by cascade order: !important, then specificity, then source order.
    matched.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    // The last entry after sorting wins.
    matched.last().and_then(|(_, _, _, value)| match value {
        CssValue::String(s) => Some(s.clone()),
        CssValue::Keyword(kw) if kw == "none" || kw == "normal" => None,
        _ => None,
    })
}

/// Match a selector against a node, ignoring any pseudo-element part.
///
/// For `p::before`, this checks whether `p` matches the node.
fn matches_selector_ignoring_pseudo(
    doc: &Document,
    node_id: NodeId,
    selector: &super::parser::Selector,
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
        let adjusted = super::parser::Selector { parts: new_parts };
        return matches_selector(doc, node_id, &adjusted, ctx);
    }

    // Replace the last compound with the filtered version.
    let mut new_parts = parts.clone();
    new_parts[last_idx].0 = filtered_compound;
    let temp_selector = super::parser::Selector { parts: new_parts };
    matches_selector(doc, node_id, &temp_selector, ctx)
}

// -----------------------------------------------------------------------
// Matched declaration collection
// -----------------------------------------------------------------------

/// The origin of a declaration for cascade ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    /// From an HTML presentational attribute (lowest priority author style).
    Presentational,
    /// From a `<link>` or `<style>` stylesheet.
    Stylesheet,
    /// From the element's `style=""` attribute.
    Inline,
}

/// A single declaration together with its cascade metadata.
#[derive(Debug, Clone)]
struct MatchedDeclaration {
    property: String,
    value: CssValue,
    important: bool,
    origin: Origin,
    specificity: Specificity,
    source_order: usize,
}

/// Gather every declaration that applies to `node_id` from all
/// stylesheets and inline styles.
///
/// Uses the `SelectorIndex` to test only candidate rules whose subject
/// selector matches the element's tag/id/classes, instead of all rules.
fn collect_matched_declarations(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &HashMap<NodeId, &[Declaration]>,
    ctx: &CascadeContext<'_>,
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

    // Get candidate rules from the index.
    let candidates = index.candidates(tag, id, &classes);

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
            });
        }
    }

    result
}

/// Inject synthetic CSS declarations from HTML presentational attributes.
///
/// Per the CSS spec, presentational attributes act as author-origin rules
/// with zero specificity — overridden by any CSS rule. We use
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
        });
    };

    // bgcolor → background-color
    if let Some(color_str) = elem.get_attribute("bgcolor")
        && let Some((r, g, b, a)) = parse_html_color(color_str)
    {
        push("background-color", CssValue::Color(CssColor { r, g, b, a }));
    }

    // width attribute → width (on img, table, td, th, input)
    if matches!(
        elem.tag,
        TagName::Img | TagName::Table | TagName::Td | TagName::Th | TagName::Input
    ) && let Some(val) = elem.get_attribute("width")
        && let Some(css_val) = parse_html_dimension(val)
    {
        push("width", css_val);
    }

    // height attribute → height (on img, table, td, th, input)
    if matches!(
        elem.tag,
        TagName::Img | TagName::Table | TagName::Td | TagName::Th | TagName::Input
    ) && let Some(val) = elem.get_attribute("height")
        && let Some(css_val) = parse_html_dimension(val)
    {
        push("height", css_val);
    }

    // align attribute → text-align (on td, th, p, div, center, tr)
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

    // nowrap attribute → white-space: nowrap (on td, th)
    if matches!(elem.tag, TagName::Td | TagName::Th) && elem.get_attribute("nowrap").is_some() {
        push("white-space", CssValue::Keyword("nowrap".into()));
    }

    // cellspacing attribute on table → border-spacing
    if elem.tag == TagName::Table
        && let Some(val) = elem.get_attribute("cellspacing")
        && let Ok(n) = val.parse::<f32>()
    {
        push("border-spacing", CssValue::Length(n, LengthUnit::Px));
    }

    // cellpadding attribute on table → padding on descendant cells
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

    // size attribute on input → width (approximate)
    if elem.tag == TagName::Input
        && let Some(val) = elem.get_attribute("size")
        && let Ok(n) = val.parse::<f32>()
    {
        // Each character ≈ 8px in our bitmap font.
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
fn matching_specificity(
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
fn selector_pseudo_element(selector: &super::parser::Selector) -> Option<&str> {
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
fn matches_selector(
    doc: &Document,
    node_id: NodeId,
    selector: &super::parser::Selector,
    ctx: &CascadeContext<'_>,
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
                Some(pid) if matches_compound(doc, pid, compound, ctx) => {
                    current = pid;
                },
                _ => return false,
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

    true
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
        SimpleSelector::Not(inner) => !matches_compound(doc, node_id, inner, ctx),
        SimpleSelector::Attribute { name, op, value } => {
            match_attribute(elem, name, op, value.as_deref())
        },
        SimpleSelector::PseudoElement(_) => {
            // Pseudo-elements don't match real elements directly.
            // They are handled separately by resolve_pseudo_content.
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
fn match_pseudo_class(
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

// -----------------------------------------------------------------------
// CSS custom property (var()) resolution
// -----------------------------------------------------------------------

/// Recursively resolve `CssValue::Var` references using the element's
/// custom property map.
///
/// If the custom property exists, its raw CSS text is re-tokenized and
/// re-parsed. If not, the fallback value is used. If neither exists,
/// an empty keyword is returned (property will be silently ignored).
fn resolve_css_var(value: &CssValue, props: &HashMap<String, String>) -> CssValue {
    resolve_css_var_depth(value, props, 0)
}

/// Maximum recursion depth for `var()` resolution. Prevents stack overflow
/// on cyclic custom properties like `--a: var(--a)`.
const MAX_VAR_DEPTH: u32 = 16;

fn resolve_css_var_depth(
    value: &CssValue,
    props: &HashMap<String, String>,
    depth: u32,
) -> CssValue {
    if depth >= MAX_VAR_DEPTH {
        return CssValue::Keyword(String::new());
    }
    match value {
        CssValue::Var(name, fallback) => {
            let raw = props
                .get(name.as_str())
                .map(|s| s.as_str())
                .or(fallback.as_deref());
            if let Some(css_text) = raw {
                let tokens = CssTokenizer::new(css_text).tokenize();
                let parsed = parse_value_list(&tokens);
                match parsed.len() {
                    0 => CssValue::Keyword(String::new()),
                    1 => {
                        let v = parsed.into_iter().next().expect("len checked");
                        // Handle chained var() references.
                        resolve_css_var_depth(&v, props, depth + 1)
                    },
                    _ => {
                        let resolved: Vec<CssValue> = parsed
                            .into_iter()
                            .map(|v| resolve_css_var_depth(&v, props, depth + 1))
                            .collect();
                        CssValue::Multiple(resolved)
                    },
                }
            } else {
                CssValue::Keyword(String::new())
            }
        },
        CssValue::Multiple(parts) => {
            let resolved: Vec<CssValue> = parts
                .iter()
                .map(|p| resolve_css_var_depth(p, props, depth + 1))
                .collect();
            CssValue::Multiple(resolved)
        },
        other => other.clone(),
    }
}

// -----------------------------------------------------------------------
// Default (user-agent) stylesheet
// -----------------------------------------------------------------------

/// Return the built-in user-agent stylesheet.
///
/// This is the CSS2.1 default stylesheet for HTML elements. It
/// participates in the normal cascade so author/skin stylesheets can
/// override any rule using standard specificity rules.
pub fn default_stylesheet() -> Stylesheet {
    Stylesheet::parse(UA_CSS)
}

/// User-agent stylesheet following CSS 2.1 defaults with visual styling
/// for semantic elements.
const UA_CSS: &str = r#"
/* -- Block-level elements ------------------------------------------- */
html, body, div, main, section, article, nav, aside,
header, footer, figure, figcaption, address, fieldset, form,
hgroup, search, dialog {
    display: block;
}

body {
    margin: 8px;
}

/* -- Paragraphs ----------------------------------------------------- */
p {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}

/* -- Headings ------------------------------------------------------- */
h1 {
    display: block;
    font-size: 2em;
    font-weight: bold;
    margin-top: 0.67em;
    margin-bottom: 0.67em;
}
h2 {
    display: block;
    font-size: 1.5em;
    font-weight: bold;
    margin-top: 0.83em;
    margin-bottom: 0.83em;
}
h3 {
    display: block;
    font-size: 1.17em;
    font-weight: bold;
    margin-top: 1em;
    margin-bottom: 1em;
}
h4 {
    display: block;
    font-size: 1em;
    font-weight: bold;
    margin-top: 1.33em;
    margin-bottom: 1.33em;
}
h5 {
    display: block;
    font-size: 0.83em;
    font-weight: bold;
    margin-top: 1.67em;
    margin-bottom: 1.67em;
}
h6 {
    display: block;
    font-size: 0.67em;
    font-weight: bold;
    margin-top: 2.33em;
    margin-bottom: 2.33em;
}

/* -- Lists ---------------------------------------------------------- */
ul, ol {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    padding-left: 40px;
}
ul li {
    display: list-item;
    list-style-type: disc;
}
ol li {
    display: list-item;
    list-style-type: decimal;
}
li {
    display: list-item;
}
dl {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}
dt { display: block; font-weight: bold; }
dd { display: block; margin-left: 40px; }

/* -- Blockquote ----------------------------------------------------- */
blockquote {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    margin-left: 10px;
    padding-left: 10px;
    border-left-width: 3px;
    border-left-style: solid;
    border-left-color: #808080;
}

/* -- Preformatted & Code -------------------------------------------- */
pre {
    display: block;
    white-space: pre;
    font-family: monospace;
    margin-top: 1em;
    margin-bottom: 1em;
    padding-top: 6px;
    padding-bottom: 6px;
    padding-left: 8px;
    padding-right: 8px;
    background-color: rgba(128, 128, 128, 25);
    border-width: 1px;
    border-style: solid;
    border-color: rgba(128, 128, 128, 50);
}
code, kbd, samp, var {
    font-family: monospace;
    background-color: rgba(128, 128, 128, 25);
}

/* -- Inline text semantics ------------------------------------------ */
mark {
    background-color: rgba(255, 255, 0, 128);
    color: #000000;
}
small { font-size: 0.83em; }
sub { font-size: 0.83em; }
sup { font-size: 0.83em; }
abbr { text-decoration: underline; }
dfn { font-style: italic; }

/* -- Details/Summary ------------------------------------------------ */
details {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}
summary {
    display: block;
    font-weight: bold;
}

/* -- Horizontal rule ------------------------------------------------ */
hr {
    display: block;
    margin-top: 8px;
    margin-bottom: 8px;
    border-top-width: 1px;
    border-top-style: solid;
    border-top-color: #808080;
}

/* -- Text formatting ------------------------------------------------ */
b, strong { font-weight: bold; }
i, em, cite { font-style: italic; }
u, ins { text-decoration: underline; }
s, del { text-decoration: line-through; }

/* -- Links ---------------------------------------------------------- */
a {
    color: #0066cc;
    text-decoration: underline;
}

/* -- Tables --------------------------------------------------------- */
table {
    display: table;
    border-collapse: collapse;
}
caption {
    display: block;
    text-align: center;
    font-weight: bold;
    padding-top: 4px;
    padding-bottom: 4px;
}
thead { display: block; }
tbody { display: block; }
tfoot { display: block; }
colgroup { display: none; }
col { display: none; }
tr { display: table-row; }
td, th {
    display: table-cell;
    padding-top: 2px;
    padding-bottom: 2px;
    padding-left: 4px;
    padding-right: 4px;
    border-top-width: 1px;
    border-right-width: 1px;
    border-bottom-width: 1px;
    border-left-width: 1px;
    border-top-style: solid;
    border-right-style: solid;
    border-bottom-style: solid;
    border-left-style: solid;
    border-top-color: #ccc;
    border-right-color: #ccc;
    border-bottom-color: #ccc;
    border-left-color: #ccc;
}
th {
    font-weight: bold;
    text-align: center;
}

/* -- Form elements -------------------------------------------------- */
br, img, input, button, select, textarea {
    display: inline;
}
option {
    display: none;
}
fieldset {
    display: block;
    margin-top: 0;
    margin-bottom: 0;
    padding-top: 4px;
    padding-bottom: 4px;
    padding-left: 8px;
    padding-right: 8px;
    border-width: 1px;
    border-style: solid;
    border-color: #808080;
}
legend {
    display: block;
    font-weight: bold;
    padding-left: 4px;
    padding-right: 4px;
}
label { display: inline; }

/* -- Deprecated elements -------------------------------------------- */
center {
    display: block;
    text-align: center;
}

/* -- Hidden elements ------------------------------------------------ */
head, script, style, link, meta, title, noscript, template {
    display: none;
}

input[type="hidden"] {
    display: none;
}
"#;

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::parser::{
        AttrOp, Combinator, CompoundSelector, CssValue, Declaration, Rule, Selector, SelectorList,
        SimpleSelector, Stylesheet,
    };
    use super::super::values::{Display, FontWeight};
    use super::*;
    use crate::html::dom::{Attribute, Document, ElementData, Node, NodeKind, TagName};
    use oasis_types::backend::Color;

    /// Default cascade context for tests (no hover, no visited URLs).
    fn ctx() -> CascadeContext<'static> {
        CascadeContext::default()
    }

    // -- Test DOM helpers -----------------------------------------------

    /// Build a minimal document: <html><body>...</body></html>.
    fn make_doc(body_children: Vec<(TagName, Vec<Attribute>)>) -> Document {
        let mut nodes = Vec::new();

        // 0: Document root
        nodes.push(Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        });

        // 1: <html>
        nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        });

        // 2: <body>
        let body_child_ids: Vec<NodeId> = (3..3 + body_children.len()).collect();
        nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: body_child_ids,
        });

        // Body children.
        for (tag, attrs) in body_children {
            nodes.push(Node {
                kind: NodeKind::Element(ElementData {
                    tag,
                    attributes: attrs,
                }),
                parent: Some(2),
                children: vec![],
            });
        }

        Document { nodes, root: 0 }
    }

    fn make_rule(selectors: Vec<Selector>, declarations: Vec<Declaration>) -> Rule {
        Rule {
            selectors: SelectorList { selectors },
            declarations,
        }
    }

    /// Create a type selector: `tag`.
    fn simple_type_selector(tag: &str) -> Selector {
        Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Type(tag.to_string())],
                },
                None,
            )],
        }
    }

    /// Create a class selector: `.cls`.
    fn simple_class_selector(cls: &str) -> Selector {
        Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Class(cls.to_string())],
                },
                None,
            )],
        }
    }

    /// Create an ID selector: `#id`.
    fn simple_id_selector(id: &str) -> Selector {
        Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Id(id.to_string())],
                },
                None,
            )],
        }
    }

    /// Create a descendant selector: `ancestor descendant`.
    fn descendant_selector(ancestor_tag: &str, descendant_tag: &str) -> Selector {
        // Parts stored left-to-right: ancestor first, descendant last.
        Selector {
            parts: vec![
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type(ancestor_tag.to_string())],
                    },
                    None,
                ),
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type(descendant_tag.to_string())],
                    },
                    Some(Combinator::Descendant),
                ),
            ],
        }
    }

    fn decl(property: &str, value: CssValue, important: bool) -> Declaration {
        Declaration {
            property: property.to_string(),
            value,
            important,
        }
    }

    // -- Tests ----------------------------------------------------------

    #[test]
    fn type_selector_matching() {
        let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
        let sel = simple_type_selector("p");
        // Node 3 is <p>, node 4 is <div>.
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
        assert!(!matches_selector(&doc, 4, &sel, &ctx()));
    }

    #[test]
    fn class_selector_matching() {
        let doc = make_doc(vec![
            (
                TagName::P,
                vec![Attribute {
                    name: "class".to_string(),
                    value: "highlight important".to_string(),
                }],
            ),
            (TagName::P, vec![]),
        ]);
        let sel = simple_class_selector("highlight");
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
        assert!(!matches_selector(&doc, 4, &sel, &ctx()));
    }

    #[test]
    fn id_selector_matching() {
        let doc = make_doc(vec![(
            TagName::Div,
            vec![Attribute {
                name: "id".to_string(),
                value: "main".to_string(),
            }],
        )]);
        let sel = simple_id_selector("main");
        assert!(matches_selector(&doc, 3, &sel, &ctx()));

        let wrong = simple_id_selector("other");
        assert!(!matches_selector(&doc, 3, &wrong, &ctx()));
    }

    #[test]
    fn descendant_selector_matching() {
        // <body> > <div> > <p>
        let mut doc = make_doc(vec![(TagName::Div, vec![])]);
        // Add <p> as child of <div> (node 3).
        let p_id = doc.nodes.len();
        doc.nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        });
        doc.nodes[3].children.push(p_id);

        let sel = descendant_selector("div", "p");
        assert!(
            matches_selector(&doc, p_id, &sel, &ctx()),
            "p inside div should match `div p`"
        );

        // <p> directly in <body> should NOT match `div p`.
        let doc2 = make_doc(vec![(TagName::P, vec![])]);
        assert!(
            !matches_selector(&doc2, 3, &sel, &ctx()),
            "p in body should not match `div p`"
        );
    }

    #[test]
    fn specificity_ordering() {
        // An ID selector (#main) should beat a class (.cls).
        let doc = make_doc(vec![(
            TagName::Div,
            vec![
                Attribute {
                    name: "id".to_string(),
                    value: "main".to_string(),
                },
                Attribute {
                    name: "class".to_string(),
                    value: "cls".to_string(),
                },
            ],
        )]);

        let rule_class = make_rule(
            vec![simple_class_selector("cls")],
            vec![decl("color", CssValue::Keyword("red".to_string()), false)],
        );
        let rule_id = make_rule(
            vec![simple_id_selector("main")],
            vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
        );

        // Class rule comes first, ID rule second.
        let sheet = Stylesheet {
            rules: vec![rule_class, rule_id],
        };
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let style = styles[3].as_ref().expect("div should have style");
        // Blue wins because #main has higher specificity.
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn inheritance_of_color_and_font() {
        // Parent <div> sets color: red, font-weight: bold (as Number).
        // Child <p> should inherit those.
        let mut doc = make_doc(vec![(TagName::Div, vec![])]);
        let p_id = doc.nodes.len();
        doc.nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        });
        doc.nodes[3].children.push(p_id);

        let rule = make_rule(
            vec![simple_type_selector("div")],
            vec![
                decl("color", CssValue::Keyword("red".to_string()), false),
                decl("font-weight", CssValue::Number(700.0), false),
            ],
        );
        let sheet = Stylesheet { rules: vec![rule] };
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[p_id].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        assert_eq!(p_style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn important_overrides_specificity() {
        let doc = make_doc(vec![(
            TagName::Div,
            vec![Attribute {
                name: "id".to_string(),
                value: "main".to_string(),
            }],
        )]);

        // Normal ID rule: color blue.
        let rule_id = make_rule(
            vec![simple_id_selector("main")],
            vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
        );
        // Type rule with !important: color green.
        let rule_type = make_rule(
            vec![simple_type_selector("div")],
            vec![decl("color", CssValue::Keyword("green".to_string()), true)],
        );

        let sheet = Stylesheet {
            rules: vec![rule_id, rule_type],
        };
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());
        let style = styles[3].as_ref().expect("div should have style");
        // Green wins because !important beats higher specificity.
        assert_eq!(style.color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn multiple_stylesheets_merged() {
        let doc = make_doc(vec![(TagName::P, vec![])]);

        let sheet1 = Stylesheet {
            rules: vec![make_rule(
                vec![simple_type_selector("p")],
                vec![decl("color", CssValue::Keyword("red".to_string()), false)],
            )],
        };
        let sheet2 = Stylesheet {
            rules: vec![make_rule(
                vec![simple_type_selector("p")],
                vec![decl("font-weight", CssValue::Number(700.0), false)],
            )],
        };

        let styles = style_tree(&doc, &[&sheet1, &sheet2], &[], &ctx());
        let style = styles[3].as_ref().expect("p should have style");
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn inline_style_override() {
        let doc = make_doc(vec![(TagName::P, vec![])]);

        // Stylesheet says color: red.
        let sheet = Stylesheet {
            rules: vec![make_rule(
                vec![simple_type_selector("p")],
                vec![decl("color", CssValue::Keyword("red".to_string()), false)],
            )],
        };

        // Inline style says color: blue.
        let inline = vec![(
            3_usize,
            vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
        )];

        let styles = style_tree(&doc, &[&sheet], &inline, &ctx());
        let style = styles[3].as_ref().expect("p should have style");
        // Inline wins over stylesheet.
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn element_defaults_applied() {
        let doc = make_doc(vec![
            (TagName::P, vec![]),
            (TagName::H1, vec![]),
            (TagName::A, vec![]),
        ]);
        let ua = default_stylesheet();
        let styles = style_tree(&doc, &[&ua], &[], &ctx());

        let p_style = styles[3].as_ref().unwrap();
        assert_eq!(p_style.display, Display::Block);

        let h1_style = styles[4].as_ref().unwrap();
        assert_eq!(h1_style.display, Display::Block);
        assert_eq!(h1_style.font_weight, FontWeight::Bold);
        assert_eq!(h1_style.font_style, crate::css::values::FontStyle::Normal);
        // h1 = 2em * ROOT_FONT_SIZE
        assert!(
            (h1_style.font_size - crate::css::values::ROOT_FONT_SIZE * 2.0).abs() < f32::EPSILON
        );

        let a_style = styles[5].as_ref().unwrap();
        assert_eq!(a_style.color, Color::rgb(0, 0x66, 0xcc));
    }

    #[test]
    fn non_element_nodes_get_no_style() {
        let mut nodes = Vec::new();
        // 0: Document root
        nodes.push(Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        });
        // 1: <html>
        nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        });
        // 2: Text node
        nodes.push(Node {
            kind: NodeKind::Text("hello".to_string()),
            parent: Some(1),
            children: vec![],
        });

        let doc = Document { nodes, root: 0 };
        let sheet = Stylesheet { rules: vec![] };
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        assert!(styles[0].is_none(), "Document node");
        assert!(styles[1].is_some(), "html element");
        assert!(styles[2].is_none(), "Text node");
    }

    // -- New selector tests (Phase 4.3) ---------------------------------

    #[test]
    fn attribute_exists_selector() {
        let doc = make_doc(vec![
            (
                TagName::Div,
                vec![Attribute {
                    name: "data-x".to_string(),
                    value: "1".to_string(),
                }],
            ),
            (TagName::Div, vec![]),
        ]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "data-x".to_string(),
                        op: AttrOp::Exists,
                        value: None,
                    }],
                },
                None,
            )],
        };
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
        assert!(!matches_selector(&doc, 4, &sel, &ctx()));
    }

    #[test]
    fn attribute_equals_selector() {
        let doc = make_doc(vec![(
            TagName::Div,
            vec![Attribute {
                name: "lang".to_string(),
                value: "en".to_string(),
            }],
        )]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "lang".to_string(),
                        op: AttrOp::Equals,
                        value: Some("en".to_string()),
                    }],
                },
                None,
            )],
        };
        assert!(matches_selector(&doc, 3, &sel, &ctx()));

        let wrong = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "lang".to_string(),
                        op: AttrOp::Equals,
                        value: Some("fr".to_string()),
                    }],
                },
                None,
            )],
        };
        assert!(!matches_selector(&doc, 3, &wrong, &ctx()));
    }

    #[test]
    fn attribute_prefix_selector() {
        let doc = make_doc(vec![(
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "https://example.com".to_string(),
            }],
        )]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "href".to_string(),
                        op: AttrOp::Prefix,
                        value: Some("https".to_string()),
                    }],
                },
                None,
            )],
        };
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
    }

    #[test]
    fn attribute_substring_selector() {
        let doc = make_doc(vec![(
            TagName::Div,
            vec![Attribute {
                name: "class".to_string(),
                value: "my-widget-box".to_string(),
            }],
        )]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "class".to_string(),
                        op: AttrOp::Substring,
                        value: Some("widget".to_string()),
                    }],
                },
                None,
            )],
        };
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
    }

    #[test]
    fn not_selector() {
        let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Not(Box::new(CompoundSelector {
                        parts: vec![SimpleSelector::Type("div".to_string())],
                    }))],
                },
                None,
            )],
        };
        // <p> is not <div>, so it matches :not(div).
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
        // <div> is <div>, so it does NOT match :not(div).
        assert!(!matches_selector(&doc, 4, &sel, &ctx()));
    }

    #[test]
    fn adjacent_sibling_selector() {
        // <body> has three children: <h1>, <p>, <div>
        let doc = make_doc(vec![
            (TagName::H1, vec![]),
            (TagName::P, vec![]),
            (TagName::Div, vec![]),
        ]);
        // h1 + p should match the <p> (node 4).
        let sel = Selector {
            parts: vec![
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type("h1".to_string())],
                    },
                    None,
                ),
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type("p".to_string())],
                    },
                    Some(Combinator::AdjacentSibling),
                ),
            ],
        };
        assert!(matches_selector(&doc, 4, &sel, &ctx()));
        // <div> (node 5) is not immediately after <h1>.
        assert!(!matches_selector(&doc, 5, &sel, &ctx()));
    }

    #[test]
    fn general_sibling_selector() {
        let doc = make_doc(vec![
            (TagName::H1, vec![]),
            (TagName::P, vec![]),
            (TagName::Div, vec![]),
        ]);
        // h1 ~ div should match <div> (node 5) because <h1> precedes it.
        let sel = Selector {
            parts: vec![
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type("h1".to_string())],
                    },
                    None,
                ),
                (
                    CompoundSelector {
                        parts: vec![SimpleSelector::Type("div".to_string())],
                    },
                    Some(Combinator::GeneralSibling),
                ),
            ],
        };
        assert!(matches_selector(&doc, 5, &sel, &ctx()));
    }

    #[test]
    fn nth_child_matching() {
        use super::selectors::AnB;
        assert!(AnB { a: 2, b: 1 }.matches(1)); // odd: 1
        assert!(!AnB { a: 2, b: 1 }.matches(2)); // odd: 2 is even
        assert!(AnB { a: 2, b: 1 }.matches(3)); // odd: 3
        assert!(AnB { a: 2, b: 0 }.matches(2)); // even: 2
        assert!(!AnB { a: 2, b: 0 }.matches(1)); // even: 1 is odd
        assert!(AnB { a: 0, b: 3 }.matches(3)); // exactly 3
        assert!(!AnB { a: 0, b: 3 }.matches(4)); // not 3
        assert!(AnB { a: 3, b: 0 }.matches(6)); // 3n: 6
    }

    #[test]
    fn parse_an_plus_b_cases() {
        use super::selectors::AnB;
        assert_eq!(AnB::parse("odd"), Some(AnB { a: 2, b: 1 }));
        assert_eq!(AnB::parse("even"), Some(AnB { a: 2, b: 0 }));
        assert_eq!(AnB::parse("3"), Some(AnB { a: 0, b: 3 }));
        assert_eq!(AnB::parse("2n+1"), Some(AnB { a: 2, b: 1 }));
        assert_eq!(AnB::parse("2n"), Some(AnB { a: 2, b: 0 }));
        assert_eq!(AnB::parse("n+3"), Some(AnB { a: 1, b: 3 }));
        assert_eq!(AnB::parse("-n+3"), Some(AnB { a: -1, b: 3 }));
    }

    #[test]
    fn only_child_pseudo_class() {
        // Single child.
        let doc = make_doc(vec![(TagName::P, vec![])]);
        assert!(match_pseudo_class(
            &doc,
            3,
            match &doc.nodes[3].kind {
                NodeKind::Element(e) => e,
                _ => panic!(),
            },
            "only-child",
            &ctx(),
        ));

        // Multiple children.
        let doc2 = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
        assert!(!match_pseudo_class(
            &doc2,
            3,
            match &doc2.nodes[3].kind {
                NodeKind::Element(e) => e,
                _ => panic!(),
            },
            "only-child",
            &ctx(),
        ));
    }

    #[test]
    fn selector_parsing_attribute() {
        let sheet = Stylesheet::parse("[type=text] { color: red; }");
        assert!(!sheet.rules.is_empty());
        let rule = &sheet.rules[0];
        let sel = &rule.selectors.selectors[0];
        let compound = &sel.parts[0].0;
        assert!(matches!(
            &compound.parts[0],
            SimpleSelector::Attribute {
                name,
                op: AttrOp::Equals,
                value: Some(val),
            } if name == "type" && val == "text"
        ));
    }

    #[test]
    fn selector_parsing_not() {
        let sheet = Stylesheet::parse(":not(.hidden) { display: block; }");
        assert!(!sheet.rules.is_empty());
        let sel = &sheet.rules[0].selectors.selectors[0];
        let compound = &sel.parts[0].0;
        assert!(matches!(&compound.parts[0], SimpleSelector::Not(_)));
    }

    #[test]
    fn selector_parsing_adjacent_sibling() {
        let sheet = Stylesheet::parse("h1 + p { color: red; }");
        assert!(!sheet.rules.is_empty());
        let sel = &sheet.rules[0].selectors.selectors[0];
        assert_eq!(sel.parts.len(), 2);
        assert_eq!(sel.parts[1].1, Some(Combinator::AdjacentSibling));
    }

    #[test]
    fn selector_parsing_general_sibling() {
        let sheet = Stylesheet::parse("h1 ~ p { color: red; }");
        assert!(!sheet.rules.is_empty());
        let sel = &sheet.rules[0].selectors.selectors[0];
        assert_eq!(sel.parts.len(), 2);
        assert_eq!(sel.parts[1].1, Some(Combinator::GeneralSibling));
    }

    #[test]
    fn attribute_includes_selector() {
        let doc = make_doc(vec![(
            TagName::Div,
            vec![Attribute {
                name: "class".to_string(),
                value: "foo bar baz".to_string(),
            }],
        )]);
        let sel = Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![SimpleSelector::Attribute {
                        name: "class".to_string(),
                        op: AttrOp::Includes,
                        value: Some("bar".to_string()),
                    }],
                },
                None,
            )],
        };
        assert!(matches_selector(&doc, 3, &sel, &ctx()));
    }

    // -- Stateful pseudo-class tests (Phase 10) ---------------------------

    #[test]
    fn hover_matches_hovered_node() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let hctx = CascadeContext {
            hover_node: Some(3),
            visited_urls: None,
        };
        let elem = match &doc.nodes[3].kind {
            NodeKind::Element(e) => e,
            _ => panic!(),
        };
        assert!(match_pseudo_class(&doc, 3, elem, "hover", &hctx));
        assert!(!match_pseudo_class(&doc, 3, elem, "hover", &ctx()));
    }

    #[test]
    fn hover_matches_ancestor_of_hovered_node() {
        // <body> (2) > <div> (3) > <p> (4)
        let mut doc = make_doc(vec![(TagName::Div, vec![])]);
        let p_id = doc.nodes.len();
        doc.nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        });
        doc.nodes[3].children.push(p_id);

        // Hover is on the <p> (inner element).
        let hctx = CascadeContext {
            hover_node: Some(p_id),
            visited_urls: None,
        };
        // <div> (ancestor) should also match :hover.
        let div_elem = match &doc.nodes[3].kind {
            NodeKind::Element(e) => e,
            _ => panic!(),
        };
        assert!(match_pseudo_class(&doc, 3, div_elem, "hover", &hctx));
    }

    #[test]
    fn visited_matches_with_visited_url() {
        let mut visited = std::collections::HashSet::new();
        visited.insert("/page1".to_string());

        let doc = make_doc(vec![(
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/page1".to_string(),
            }],
        )]);
        let vctx = CascadeContext {
            hover_node: None,
            visited_urls: Some(&visited),
        };
        let elem = match &doc.nodes[3].kind {
            NodeKind::Element(e) => e,
            _ => panic!(),
        };
        assert!(match_pseudo_class(&doc, 3, elem, "visited", &vctx));
        assert!(!match_pseudo_class(&doc, 3, elem, "link", &vctx));
    }

    #[test]
    fn link_matches_unvisited_anchor() {
        let visited = std::collections::HashSet::new();

        let doc = make_doc(vec![(
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/page2".to_string(),
            }],
        )]);
        let vctx = CascadeContext {
            hover_node: None,
            visited_urls: Some(&visited),
        };
        let elem = match &doc.nodes[3].kind {
            NodeKind::Element(e) => e,
            _ => panic!(),
        };
        assert!(match_pseudo_class(&doc, 3, elem, "link", &vctx));
        assert!(!match_pseudo_class(&doc, 3, elem, "visited", &vctx));
    }

    #[test]
    fn hover_style_applied_via_cascade() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let sheet = Stylesheet::parse("p:hover { color: red; }");
        let hctx = CascadeContext {
            hover_node: Some(3),
            visited_urls: None,
        };
        let styles = style_tree(&doc, &[&sheet], &[], &hctx);
        let style = styles[3].as_ref().expect("p should have style");
        assert_eq!(style.color, Color::rgb(255, 0, 0));

        // Without hover, color should be inherited default (white).
        let styles_no_hover = style_tree(&doc, &[&sheet], &[], &ctx());
        let style_no = styles_no_hover[3].as_ref().unwrap();
        assert_ne!(style_no.color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn visited_style_applied_via_cascade() {
        let mut visited = std::collections::HashSet::new();
        visited.insert("/page1".to_string());

        let doc = make_doc(vec![(
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/page1".to_string(),
            }],
        )]);
        let sheet = Stylesheet::parse("a:visited { color: purple; }");
        let vctx = CascadeContext {
            hover_node: None,
            visited_urls: Some(&visited),
        };
        let styles = style_tree(&doc, &[&sheet], &[], &vctx);
        let style = styles[3].as_ref().expect("a should have style");
        assert_eq!(style.color, Color::rgb(128, 0, 128));
    }

    // -- CSS custom properties / var() tests (var support) ----------------

    #[test]
    fn root_pseudo_class_matches_html() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let html_elem = match &doc.nodes[1].kind {
            NodeKind::Element(e) => e,
            _ => panic!("node 1 should be <html>"),
        };
        // <html> (node 1) has parent Document (node 0) → matches :root.
        assert!(match_pseudo_class(&doc, 1, html_elem, "root", &ctx()));

        // <body> (node 2) has parent <html> (element) → does NOT match :root.
        let body_elem = match &doc.nodes[2].kind {
            NodeKind::Element(e) => e,
            _ => panic!("node 2 should be <body>"),
        };
        assert!(!match_pseudo_class(&doc, 2, body_elem, "root", &ctx()));
    }

    #[test]
    fn custom_property_stored_and_inherited() {
        // :root { --color: red } p { color: var(--color) }
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --color: red; } p { color: var(--color); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        // var(--color) should resolve to "red" → Color(255,0,0).
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn var_with_fallback() {
        // No --missing defined, fallback value should be used.
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = "p { color: var(--missing, blue); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn var_with_hex_fallback() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = "p { color: var(--missing, #202122); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(0x20, 0x21, 0x22));
    }

    #[test]
    fn chained_variables() {
        // --a references --b, which holds a concrete value.
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --b: green; --a: var(--b); } p { color: var(--a); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn var_in_border_shorthand() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --bc: red; } p { border: 1px solid var(--bc); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert_eq!(p_style.border_top_color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn var_in_background() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --bg: #ff0000; } p { background: var(--bg); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert_eq!(p_style.background_color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn var_margin_property() {
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --sp: 10px; } p { margin-top: var(--sp); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[3].as_ref().expect("p should have style");
        assert!((p_style.margin_top - 10.0).abs() < 0.01);
    }

    #[test]
    fn custom_props_inherit_to_descendants() {
        // <body> > <div> (node 3) > <p> (node 4)
        let mut doc = make_doc(vec![(TagName::Div, vec![])]);
        let p_id = doc.nodes.len();
        doc.nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        });
        doc.nodes[3].children.push(p_id);

        let css = ":root { --text-color: purple; } p { color: var(--text-color); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        let p_style = styles[p_id].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(128, 0, 128));
    }

    #[test]
    fn var_background_color_from_root() {
        // Wikipedia-style pattern: :root { --bg: #fff } body { background-color: var(--bg) }
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --background-color-base: #fff; } \
                   body { background-color: var(--background-color-base); color: var(--background-color-base); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());

        // Node 1 = <html>, node 2 = <body>
        let body_style = styles[2].as_ref().expect("body should have style");
        assert_eq!(
            body_style.background_color,
            Color::rgb(255, 255, 255),
            "body bg should be white from var(--background-color-base), got {:?}",
            body_style.background_color
        );
        assert_eq!(
            body_style.color,
            Color::rgb(255, 255, 255),
            "body color should be white from var(--background-color-base), got {:?}",
            body_style.color
        );
    }

    #[test]
    fn prefers_color_scheme_dark_rejected() {
        // @media (prefers-color-scheme: dark) should NOT match.
        let doc = make_doc(vec![(TagName::P, vec![])]);
        let css = ":root { --bg: #ffffff; } \
                   @media (prefers-color-scheme: dark) { :root { --bg: #000000; } } \
                   body { background-color: var(--bg); }";
        let sheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());
        let body_style = styles[2].as_ref().expect("body should have style");
        assert_eq!(
            body_style.background_color,
            Color::rgb(255, 255, 255),
            "dark mode should not apply; expected white, got {:?}",
            body_style.background_color
        );
    }

    #[test]
    fn cyclic_var_does_not_stack_overflow() {
        // `--a` references itself — should resolve to empty (not crash).
        let mut props = HashMap::new();
        props.insert("--a".to_string(), "var(--a)".to_string());
        let val = CssValue::Var("--a".to_string(), None);
        let resolved = resolve_css_var(&val, &props);
        assert_eq!(resolved, CssValue::Keyword(String::new()));
    }

    #[test]
    fn indirect_cyclic_var_does_not_stack_overflow() {
        // `--a` -> `var(--b)`, `--b` -> `var(--a)` — indirect cycle.
        let mut props = HashMap::new();
        props.insert("--a".to_string(), "var(--b)".to_string());
        props.insert("--b".to_string(), "var(--a)".to_string());
        let val = CssValue::Var("--a".to_string(), None);
        let resolved = resolve_css_var(&val, &props);
        assert_eq!(resolved, CssValue::Keyword(String::new()));
    }

    // -- Selector index tests (Phase 2) ----------------------------------

    #[test]
    fn selector_index_reduces_comparisons() {
        // Build a stylesheet with 3 rules: .foo, .bar, p
        let sheet = Stylesheet::parse(
            ".foo { color: red; } .bar { color: blue; } p { font-weight: bold; }",
        );
        let index = SelectorIndex::build(&[&sheet]);

        // An element with class "foo" and tag "p" should only get
        // candidates from .foo and p buckets (not .bar).
        let candidates = index.candidates("p", None, &["foo"]);
        assert!(
            candidates.len() == 2,
            "should get 2 candidates (.foo and p), got {}",
            candidates.len()
        );

        // An element with class "bar" tag "div" should only get .bar.
        let candidates = index.candidates("div", None, &["bar"]);
        assert_eq!(candidates.len(), 1, "should get 1 candidate (.bar)");
    }

    #[test]
    fn selector_index_universal_rules() {
        let sheet = Stylesheet::parse("* { margin: 0; } .cls { color: red; }");
        let index = SelectorIndex::build(&[&sheet]);

        // Any element should get the universal rule.
        let candidates = index.candidates("div", None, &[]);
        assert_eq!(candidates.len(), 1, "universal rule");

        // Element with class "cls" gets universal + .cls.
        let candidates = index.candidates("div", None, &["cls"]);
        assert_eq!(candidates.len(), 2, "universal + class");
    }

    #[test]
    fn selector_index_mixed_selector_list() {
        // A rule with both a keyed selector (.foo) and a non-keyed
        // selector (*) must appear in universal so that non-.foo
        // elements still match via the `*` selector.
        let sheet = Stylesheet::parse("*, .foo { color: red; }");
        let index = SelectorIndex::build(&[&sheet]);

        // Element without class "foo" should still get the rule via universal.
        let candidates = index.candidates("div", None, &[]);
        assert_eq!(
            candidates.len(),
            1,
            "non-.foo element should match via universal bucket"
        );

        // Element with class "foo" gets the rule via both .foo bucket
        // and universal, but dedup should give exactly 1.
        let candidates = index.candidates("div", None, &["foo"]);
        assert_eq!(
            candidates.len(),
            1,
            "dedup should collapse .foo + universal into 1"
        );
    }

    #[test]
    fn pseudo_content_respects_specificity() {
        // Higher-specificity rule (.special::before) should win even
        // when it appears before a lower-specificity rule (p::before).
        let sheet =
            Stylesheet::parse(".special::before { content: \"B\"; } p::before { content: \"A\"; }");
        // Build a <p class="special"> element (node 3 in make_doc).
        let doc = make_doc(vec![(
            TagName::P,
            vec![Attribute {
                name: "class".into(),
                value: "special".into(),
            }],
        )]);
        let ctx = ctx();
        let p_id = 3; // first body child in make_doc
        let result = resolve_pseudo_content(&doc, p_id, "before", &[&sheet], &ctx);
        assert_eq!(
            result,
            Some("B".to_string()),
            ".special::before (higher specificity) should beat p::before",
        );
    }

    #[test]
    fn test_body_has_default_margin() {
        let ua = default_stylesheet();
        let doc = make_doc(vec![]);
        let body_id = 2; // body is node 2 in make_doc
        let ctx = ctx();
        let index = SelectorIndex::build(&[&ua]);
        let inline_map = std::collections::HashMap::new();
        let style = compute_style(&doc, body_id, None, &[&ua], &index, &inline_map, &ctx);
        assert!(
            (style.margin_top - 8.0).abs() < 0.01,
            "body should have 8px top margin, got {}",
            style.margin_top,
        );
        assert!(
            (style.margin_left - 8.0).abs() < 0.01,
            "body should have 8px left margin, got {}",
            style.margin_left,
        );
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// "odd" parses to AnB(2,1) and "even" parses to AnB(2,0).
            #[test]
            fn an_plus_b_odd_even(
                input in proptest::sample::select(vec![
                    "odd".to_string(), "ODD".to_string(), "Odd".to_string(),
                    "even".to_string(), "EVEN".to_string(), "Even".to_string(),
                ]),
            ) {
                use super::super::selectors::AnB;
                let anb = AnB::parse(&input).unwrap();
                let lower = input.to_ascii_lowercase();
                if lower == "odd" {
                    prop_assert_eq!((anb.a, anb.b), (2, 1));
                } else {
                    prop_assert_eq!((anb.a, anb.b), (2, 0));
                }
            }

            /// A plain positive integer parses as AnB(0, n).
            #[test]
            fn an_plus_b_plain_number(n in 1i32..100) {
                use super::super::selectors::AnB;
                let anb = AnB::parse(&n.to_string()).unwrap();
                prop_assert_eq!(anb.a, 0);
                prop_assert_eq!(anb.b, n);
            }

            /// "An" form parses as AnB(A, 0).
            #[test]
            fn an_plus_b_an_form(coeff in 1i32..20) {
                use super::super::selectors::AnB;
                let input = format!("{coeff}n");
                let anb = AnB::parse(&input).unwrap();
                prop_assert_eq!(anb.a, coeff);
                prop_assert_eq!(anb.b, 0);
            }

            /// "An+B" form parses correctly.
            #[test]
            fn an_plus_b_full_form(
                coeff in 1i32..20,
                offset in 0i32..20,
            ) {
                use super::super::selectors::AnB;
                let input = format!("{coeff}n+{offset}");
                let anb = AnB::parse(&input).unwrap();
                prop_assert_eq!(anb.a, coeff);
                prop_assert_eq!(anb.b, offset);
            }

            /// AnB::matches: if a==0, only index==b matches.
            #[test]
            fn anb_matches_a_zero(b in 1i32..50, index in 1i32..50) {
                use super::super::selectors::AnB;
                let result = AnB { a: 0, b }.matches(index);
                prop_assert_eq!(result, index == b);
            }

            /// AnB::matches: index == a*1 + b always matches.
            #[test]
            fn anb_matches_first_match(a in 1i32..20, b in 0i32..10) {
                use super::super::selectors::AnB;
                let index = a + b;
                if index > 0 {
                    prop_assert!(
                        AnB { a, b }.matches(index),
                        "{a}n+{b} should match index {index}",
                    );
                }
            }

            /// AnB::parse never panics on arbitrary ASCII.
            #[test]
            fn anb_parse_never_panics(input in "[ -~]{0,30}") {
                use super::super::selectors::AnB;
                let _ = AnB::parse(&input);
            }
        }
    }
}

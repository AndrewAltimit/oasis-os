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
pub(crate) mod matching;
mod var_resolve;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

#[cfg(feature = "parallel-style")]
use std::cell::UnsafeCell;

use rustc_hash::FxHashMap;

#[cfg(feature = "parallel-style")]
use rayon::prelude::*;

use super::parser::{Declaration, Stylesheet};
use super::values::ComputedStyle;
use crate::html::dom::{Document, NodeId, NodeKind};

pub use index::SelectorIndex;

// -----------------------------------------------------------------------
// Parallel style computation support
// -----------------------------------------------------------------------

/// Wrapper around the styles vector that allows disjoint mutable access
/// from parallel threads.  Safety invariant: each `NodeId` is unique in
/// the DOM tree, so two threads never write to the same index.
///
/// Each element is wrapped in its own `UnsafeCell` so that concurrent
/// `&self` access to the outer `Vec` (via `Index`, not `IndexMut`) is
/// sound — only the per-element cell requires unsafe interior mutation.
#[cfg(feature = "parallel-style")]
struct ParallelStyles {
    inner: Vec<UnsafeCell<Option<ComputedStyle>>>,
}

#[cfg(feature = "parallel-style")]
// SAFETY: Each thread writes to a disjoint index (unique NodeId).
// Reads only access the parent's index which is already written
// before any child processing begins.  The `UnsafeCell` is per-element,
// so no `&mut Vec` is ever created — only individual cells are mutated.
unsafe impl Sync for ParallelStyles {}

#[cfg(feature = "parallel-style")]
impl ParallelStyles {
    fn new(styles: Vec<Option<ComputedStyle>>) -> Self {
        Self {
            inner: styles.into_iter().map(UnsafeCell::new).collect(),
        }
    }

    fn set(&self, idx: usize, style: ComputedStyle) {
        // SAFETY: Only one thread ever writes to a given `idx` because
        // node IDs are unique in the tree and subtrees are disjoint.
        // We access the Vec immutably (Index, not IndexMut) and only
        // mutate the individual UnsafeCell.
        unsafe {
            *self.inner[idx].get() = Some(style);
        }
    }

    fn get(&self, idx: usize) -> Option<&ComputedStyle> {
        // SAFETY: The parent node's style is fully written before any
        // child subtree begins processing, so this read is race-free.
        unsafe { (*self.inner[idx].get()).as_ref() }
    }

    fn into_inner(self) -> Vec<Option<ComputedStyle>> {
        self.inner.into_iter().map(UnsafeCell::into_inner).collect()
    }
}

// -----------------------------------------------------------------------
// Cascade context (stateful pseudo-class state)
// -----------------------------------------------------------------------

/// Context for stateful pseudo-class matching during cascade.
///
/// Carries the current hover target, focused element, and set of visited
/// URLs so that `:hover`, `:visited`, `:link`, `:focus`, and
/// `:focus-visible` pseudo-classes can be evaluated.
#[derive(Default)]
pub struct CascadeContext<'a> {
    /// The DOM node currently under the cursor (for `:hover`).
    /// `:hover` matches this node and all its ancestors.
    pub hover_node: Option<NodeId>,
    /// Set of visited URLs (for `:visited` / `:link` on `<a>` elements).
    pub visited_urls: Option<&'a HashSet<String>>,
    /// The DOM node that currently has keyboard focus (for `:focus` and
    /// `:focus-visible`).
    pub focused_node: Option<NodeId>,
    /// Snapshot of every query container's box size and name list,
    /// keyed by `NodeId`. Used to evaluate `@container` rule
    /// conditions during cascade. `None` on the first cascade pass
    /// before layout has run; populated for the second pass.
    pub containers: Option<&'a ContainerLookup>,
    /// Global layer name → index map for cross-stylesheet `@layer`
    /// ordering. Built from all stylesheets before cascade.
    /// `None` falls back to sheet-local ordering.
    pub global_layers: Option<&'a GlobalLayerMap>,
}

/// Global layer ordering map. Maps `(sheet_idx, local_layer_idx)` to
/// a global layer index so that the same `@layer` name in different
/// stylesheets gets the same cascade position.
pub type GlobalLayerMap = FxHashMap<(u16, u16), u16>;

/// Build a global layer ordering table from all stylesheets.
///
/// Layer names are assigned global indices in the order they are first
/// encountered across all stylesheets (which follows source order).
/// Subsequent sheets that declare the same layer name reuse the index.
pub fn build_global_layer_map(stylesheets: &[&Stylesheet]) -> GlobalLayerMap {
    let mut name_to_global: FxHashMap<&str, u16> = FxHashMap::default();
    let mut next_global: u16 = 0;
    let mut map = GlobalLayerMap::default();

    for (sheet_idx, sheet) in stylesheets.iter().enumerate() {
        for (local_idx, name) in sheet.layers.iter().enumerate() {
            let global = *name_to_global.entry(name.as_str()).or_insert_with(|| {
                let idx = next_global;
                next_global = next_global.saturating_add(1);
                idx
            });
            map.insert((sheet_idx as u16, local_idx as u16), global);
        }
    }
    map
}

/// Per-element container metadata feeding `@container` rule evaluation.
#[derive(Debug, Clone)]
pub struct ContainerEntry {
    /// `container-name` identifiers declared on the element. May be empty.
    pub names: Vec<String>,
    /// Border-box inline-axis size in CSS pixels.
    pub width: f32,
    /// Border-box block-axis size in CSS pixels.
    pub height: f32,
    /// The container type — `InlineSize` rejects block-axis queries.
    pub container_type: crate::css::values::types::ContainerType,
    /// Custom properties on the container element, for `style()` queries.
    pub custom_properties: FxHashMap<String, String>,
}

/// Map of container `NodeId` → metadata. Built post-layout from a walk
/// over the layout tree by [`build_container_lookup`].
#[derive(Debug, Default, Clone)]
pub struct ContainerLookup {
    pub map: rustc_hash::FxHashMap<NodeId, ContainerEntry>,
}

impl ContainerLookup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: NodeId) -> Option<&ContainerEntry> {
        self.map.get(&id)
    }

    pub fn insert(&mut self, id: NodeId, entry: ContainerEntry) {
        self.map.insert(id, entry);
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Walk a laid-out `LayoutBox` tree and snapshot every query container
/// (any element whose computed `container-type` is not `Normal`), keyed
/// by its DOM node id. `@container` rule conditions are evaluated
/// against the container's content-box size.
pub fn build_container_lookup(root: &crate::layout::box_model::LayoutBox) -> ContainerLookup {
    use crate::css::values::types::ContainerType;

    fn walk(b: &crate::layout::box_model::LayoutBox, out: &mut ContainerLookup) {
        if let Some(nid) = b.node
            && b.style.container_type != ContainerType::Normal
        {
            out.insert(
                nid,
                ContainerEntry {
                    names: b.style.container_name.clone(),
                    width: b.dimensions.content.width,
                    height: b.dimensions.content.height,
                    container_type: b.style.container_type,
                    custom_properties: b.style.custom_properties.clone(),
                },
            );
        }
        for c in &b.children {
            walk(c, out);
        }
    }

    let mut lookup = ContainerLookup::new();
    walk(root, &mut lookup);
    lookup
}

/// Return true if any rule in any of the given stylesheets is gated
/// behind a `@container` condition. Used to skip the second cascade
/// pass on pages that don't use container queries.
pub fn stylesheets_use_container_queries(sheets: &[&super::parser::Stylesheet]) -> bool {
    sheets
        .iter()
        .any(|s| s.rules.iter().any(|r| r.container.is_some()))
}

// -----------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Diagnostic progress + yield hooks
// ---------------------------------------------------------------------------
//
// PSP needs cooperative yields out of the cascade walk so the
// `cmd_server` thread (and the WLAN driver) get CPU between batches
// of compute_style calls. Without this, the synchronous `style_tree`
// hogs the main thread for tens of seconds on a Wikipedia-sized DOM
// (2430 elements) and the firmware kills the WLAN driver.

type CascadeProgressFn = fn(u64, u64);
type CascadeYieldFn = fn();
static CASCADE_PROGRESS_HOOK: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static CASCADE_YIELD_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Install a cascade progress hook fired every 256 element styled.
/// Args are `(elements_styled, total_elements)`.
pub fn set_cascade_progress_hook(hook: CascadeProgressFn) {
    CASCADE_PROGRESS_HOOK.store(hook as usize, std::sync::atomic::Ordering::Release);
}

/// Install a cooperative yield hook fired every 64 elements styled.
pub fn set_cascade_yield_hook(hook: CascadeYieldFn) {
    CASCADE_YIELD_HOOK.store(hook as usize, std::sync::atomic::Ordering::Release);
}

fn cascade_progress(idx: u64, total: u64) {
    let raw = CASCADE_PROGRESS_HOOK.load(std::sync::atomic::Ordering::Acquire);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 (handled above) or a `CascadeProgressFn`
    // we previously stored.
    let hook: CascadeProgressFn = unsafe { std::mem::transmute::<usize, CascadeProgressFn>(raw) };
    hook(idx, total);
}

fn cascade_yield_fn() {
    let raw = CASCADE_YIELD_HOOK.load(std::sync::atomic::Ordering::Acquire);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 or a `CascadeYieldFn` we stored.
    let hook: CascadeYieldFn = unsafe { std::mem::transmute::<usize, CascadeYieldFn>(raw) };
    hook();
}

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
    cascade_progress(100, stylesheets.len() as u64); // marker: enter style_tree
    // Reset the root font size before this cascade pass. The `<html>`
    // element update below will publish the real value; this clears any
    // value lingering from a previous page in the same browser process.
    super::values::types::set_root_font_size(super::values::ROOT_FONT_SIZE);
    // Build selector index for O(1) bucket lookups instead of O(rules).
    let index = SelectorIndex::build(stylesheets);
    cascade_progress(101, 0); // marker: SelectorIndex built

    // Build global layer ordering for cross-stylesheet @layer merging.
    let glm = build_global_layer_map(stylesheets);
    let ctx = &CascadeContext {
        hover_node: ctx.hover_node,
        visited_urls: ctx.visited_urls,
        focused_node: ctx.focused_node,
        containers: ctx.containers,
        global_layers: Some(&glm),
    };

    // Build a HashMap for O(1) inline style lookups instead of O(n) per element.
    let inline_map: FxHashMap<NodeId, &[Declaration]> = inline_styles
        .iter()
        .map(|(nid, decls)| (*nid, decls.as_slice()))
        .collect();
    cascade_progress(102, 0); // marker: inline_map built
    // --- Parallel path (feature = "parallel-style") ---
    #[cfg(feature = "parallel-style")]
    {
        let styles_vec: Vec<Option<ComputedStyle>> = vec![None; doc.nodes.len()];
        let par_styles = ParallelStyles::new(styles_vec);

        style_subtree_parallel(
            doc,
            doc.root,
            stylesheets,
            &index,
            &inline_map,
            &par_styles,
            ctx,
        );
        return par_styles.into_inner();
    }

    // --- Sequential path (default) ---
    #[cfg(not(feature = "parallel-style"))]
    {
        cascade_progress(0, 0); // marker: about to allocate styles vec
        let mut styles: Vec<Option<ComputedStyle>> = vec![None; doc.nodes.len()];
        cascade_progress(1, 0); // marker: styles vec allocated

        // Cache for lowercased tag names to avoid repeated allocations
        // during selector index lookups.
        let mut tag_cache = FxHashMap::<String, String>::default();
        cascade_progress(2, 0); // marker: tag_cache ready

        // Count of element nodes for the progress hook denominator.
        let element_count = doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Element(_)))
            .count() as u64;
        cascade_progress(3, element_count); // marker: counted elements
        let mut elements_styled: u64 = 0;
        style_subtree(
            doc,
            doc.root,
            stylesheets,
            &index,
            &inline_map,
            &mut styles,
            ctx,
            &mut tag_cache,
            &mut elements_styled,
            element_count,
        );
        cascade_progress(elements_styled, element_count);
        styles
    }
}

/// Recursively compute styles depth-first so that children can inherit
/// from their (already-computed) parent.
#[cfg(not(feature = "parallel-style"))]
#[allow(clippy::too_many_arguments)]
fn style_subtree(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &FxHashMap<NodeId, &[Declaration]>,
    styles: &mut [Option<ComputedStyle>],
    ctx: &CascadeContext<'_>,
    tag_cache: &mut FxHashMap<String, String>,
    elements_styled: &mut u64,
    total_elements: u64,
) {
    let node = &doc.nodes[node_id];

    // Only elements get computed styles.
    if let NodeKind::Element(elem) = &node.kind {
        let parent_style = node.parent.and_then(|pid| styles[pid].as_ref());
        let style = compute_style(
            doc,
            node_id,
            parent_style,
            stylesheets,
            index,
            inline_map,
            ctx,
            tag_cache,
        );
        // The `<html>` element's resolved font-size is the reference
        // for all subsequent `rem` unit resolution. Publish it to the
        // `values::types` atomic so `calc(1.4rem)` later in the tree
        // picks up, e.g. Wikipedia's `html { font-size: 62.5% }` =
        // 10px rather than the 8px engine default.
        if matches!(elem.tag, crate::html::dom::TagName::Html) {
            crate::css::values::types::set_root_font_size(style.font_size);
        }
        styles[node_id] = Some(style);
        *elements_styled += 1;
        // Cooperative yield + sparse progress logging so PSP's
        // cmd_server thread keeps getting CPU during the cascade.
        // Yield aggressively at the start (every 16 elements for the
        // first 256) so the cmd_server stays alive while the heap
        // allocator is most contended, then back off to every 64.
        let yield_interval = if *elements_styled < 256 { 16 } else { 64 };
        if elements_styled.is_multiple_of(yield_interval) {
            cascade_yield_fn();
        }
        // Progress every 64 elements for the first 512, then every
        // 256 — so we can see early progress in the diag log even if
        // cascade dies early.
        let log_interval = if *elements_styled < 512 { 64 } else { 256 };
        if elements_styled.is_multiple_of(log_interval) {
            cascade_progress(*elements_styled, total_elements);
        }
    }

    // Recurse into children. Iterate by index to avoid cloning the Vec.
    let num_children = doc.nodes[node_id].children.len();
    for i in 0..num_children {
        let child_id = doc.nodes[node_id].children[i];
        style_subtree(
            doc,
            child_id,
            stylesheets,
            index,
            inline_map,
            styles,
            ctx,
            tag_cache,
            elements_styled,
            total_elements,
        );
    }
}

/// Recursively compute styles in parallel using rayon.
///
/// Sibling subtrees are independent: each child inherits from the
/// already-computed parent style and writes only to its own `NodeId`
/// slots.  We use [`ParallelStyles`] to provide disjoint mutable
/// access without runtime locking.
///
/// Each thread maintains its own `tag_cache` (cheap `FxHashMap` of
/// lowercased tag names) to avoid contention on a shared cache.
#[cfg(feature = "parallel-style")]
#[allow(clippy::too_many_arguments)]
fn style_subtree_parallel(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &FxHashMap<NodeId, &[Declaration]>,
    styles: &ParallelStyles,
    ctx: &CascadeContext<'_>,
) {
    let node = &doc.nodes[node_id];

    // Only elements get computed styles.
    if let NodeKind::Element(_) = &node.kind {
        let parent_style = node.parent.and_then(|pid| styles.get(pid));
        let mut tag_cache = FxHashMap::<String, String>::default();
        let style = compute_style(
            doc,
            node_id,
            parent_style,
            stylesheets,
            index,
            inline_map,
            ctx,
            &mut tag_cache,
        );
        styles.set(node_id, style);
    }

    let children = &doc.nodes[node_id].children;

    // Use rayon parallel iteration for children with enough siblings
    // to amortise the scheduling overhead.
    const PAR_THRESHOLD: usize = 4;
    if children.len() >= PAR_THRESHOLD {
        children.par_iter().for_each(|&child_id| {
            style_subtree_parallel(doc, child_id, stylesheets, index, inline_map, styles, ctx);
        });
    } else {
        for &child_id in children {
            style_subtree_parallel(doc, child_id, stylesheets, index, inline_map, styles, ctx);
        }
    }
}

/// The CSS "medium" keyword size in real browsers. Used only as the
/// implicit parent-font-size for the `<html>` element so percentage
/// font-size declarations there (the common `62.5%` pattern) produce
/// the pixel values authors designed against.
const CSS_MEDIUM_FONT_SIZE: f32 = 16.0;

/// Compute the final style for a single element.
#[allow(clippy::too_many_arguments)]
pub fn compute_style(
    doc: &Document,
    node_id: NodeId,
    parent_style: Option<&ComputedStyle>,
    stylesheets: &[&Stylesheet],
    index: &SelectorIndex,
    inline_map: &FxHashMap<NodeId, &[Declaration]>,
    ctx: &CascadeContext<'_>,
    tag_cache: &mut FxHashMap<String, String>,
) -> ComputedStyle {
    // Start from inherited values if we have a parent, else defaults.
    let mut style = match parent_style {
        Some(parent) => ComputedStyle::inherit(parent),
        None => ComputedStyle::default(),
    };

    // The `<html>` element has no parent in CSS; percentage font-sizes
    // on it resolve against the initial ("medium") browser font size.
    // Real browsers use 16px here, and authors write `html { font-size:
    // 62.5% }` expecting 10px. Our engine defaults `ROOT_FONT_SIZE` to
    // 8px for PSP readability — if we used that here, `62.5%` would
    // collapse to 5px and every rem-based page would render absurdly
    // small. Use 16 for the html-element baseline specifically so the
    // Wikipedia-style 62.5% trick produces its intended 10px.
    let is_html = matches!(
        doc.nodes[node_id].kind,
        NodeKind::Element(ref e) if matches!(e.tag, crate::html::dom::TagName::Html)
    );
    let parent_font_size = if is_html && parent_style.is_none() {
        CSS_MEDIUM_FONT_SIZE
    } else {
        parent_style.map_or(super::values::ROOT_FONT_SIZE, |p| p.font_size)
    };

    // Collect all matching declarations with their origin info.
    let mut matched = matching::collect_matched_declarations(
        doc,
        node_id,
        stylesheets,
        index,
        inline_map,
        ctx,
        tag_cache,
    );

    // Sort by cascade order: origin, layer, specificity, source order.
    // `!important` declarations come after normal ones.
    matched.sort_by(|a, b| {
        a.important
            .cmp(&b.important)
            .then_with(|| a.origin.cmp(&b.origin))
            .then_with(|| matching::compare_layers(a, b))
            .then_with(|| a.specificity.cmp(&b.specificity))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });

    // Pass 0: `@property` registrations.
    //   * Strip non-inheriting registered properties that we picked up
    //     via `ComputedStyle::inherit(parent)` (default for unregistered
    //     properties is "inherits", which is what `inherit()` already
    //     does — only `inherits: false` registrations need stripping).
    //   * Seed initial-values for any registration that doesn't yet
    //     have a value on this element. Subsequent declarations win.
    {
        let mut last_inherits: rustc_hash::FxHashMap<&str, bool> = rustc_hash::FxHashMap::default();
        let mut last_initial: rustc_hash::FxHashMap<&str, &str> = rustc_hash::FxHashMap::default();
        for sheet in stylesheets {
            for prop in &sheet.properties {
                last_inherits.insert(&prop.name, prop.inherits);
                if let Some(ref initial) = prop.initial_value {
                    last_initial.insert(&prop.name, initial);
                }
            }
        }
        for (name, &inherits) in &last_inherits {
            if !inherits {
                style.custom_properties.remove(*name);
            }
        }
        for (name, initial) in last_initial {
            if !style.custom_properties.contains_key(name) {
                style
                    .custom_properties
                    .insert(name.to_string(), initial.to_string());
            }
        }
    }

    // Pass 1: Apply custom property declarations (--*) to build the
    // properties map before resolving any var() references.
    for entry in &matched {
        if entry.property.starts_with("--") {
            style.apply_declaration(&entry.property, &entry.value, parent_font_size);
        }
    }

    // Pass 2a: Apply `direction` before any other properties so that
    // inline-axis logical properties (margin-inline-start, etc.)
    // resolve to the correct physical side in RTL contexts.
    // `matched` is sorted ascending by cascade order (origin, layer,
    // specificity, source order), so the last `direction` entry is
    // the cascade winner — apply only that one.
    if let Some(entry) = matched.iter().rfind(|e| e.property == "direction") {
        let resolved = var_resolve::resolve_css_var(&entry.value, &style.custom_properties);
        style.apply_declaration("direction", &resolved, parent_font_size);
    }

    // Pass 2b: Apply font-size first so that em units in subsequent
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
    let mut has_explicit_line_height = false;
    for entry in &matched {
        if entry.property.starts_with("--")
            || entry.property == "font-size"
            || entry.property == "direction"
        {
            continue;
        }
        if entry.property == "line-height" {
            has_explicit_line_height = true;
        }
        let resolved = var_resolve::resolve_css_var(&entry.value, &style.custom_properties);
        style.apply_declaration(&entry.property, &resolved, element_font_size);
    }

    // CSS 2.1 §17.21: unitless line-height inherits the *factor*, not
    // the computed value.  If no explicit line-height was declared and
    // the inherited factor differs from the element's font-size, recompute.
    if !has_explicit_line_height && let Some(factor) = style.line_height_factor {
        style.line_height = factor * element_font_size;
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

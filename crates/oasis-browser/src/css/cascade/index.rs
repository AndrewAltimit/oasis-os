//! Selector index for efficient rule lookup.
//!
//! Buckets rules by the rightmost (subject) simple selector
//! (ID > class > tag > universal) so that only a small subset of
//! rules is tested against each element.

use rustc_hash::FxHashMap;

use super::super::parser::{SimpleSelector, Stylesheet};

/// An indexed reference to a specific rule in a specific stylesheet.
#[derive(Debug, Clone, Copy)]
pub(super) struct IndexedRule {
    /// Index into the `stylesheets` slice.
    pub(super) sheet_idx: usize,
    /// Index into the stylesheet's `rules` Vec.
    pub(super) rule_idx: usize,
    /// Global source order counter for cascade ordering.
    pub(super) source_order_base: usize,
}

/// Pre-built index that buckets rules by the rightmost (subject)
/// selector's most specific part. This avoids testing every rule
/// against every element -- only rules whose subject could possibly
/// match are considered.
pub struct SelectorIndex {
    by_id: FxHashMap<String, Vec<IndexedRule>>,
    by_class: FxHashMap<String, Vec<IndexedRule>>,
    by_tag: FxHashMap<String, Vec<IndexedRule>>,
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
            by_id: FxHashMap::default(),
            by_class: FxHashMap::default(),
            by_tag: FxHashMap::default(),
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
    ///
    /// Lowercases the tag name internally. For hot paths with many
    /// elements, prefer [`candidates_with_lower`] to avoid repeated
    /// lowercasing allocations.
    #[cfg(test)]
    pub(super) fn candidates(
        &self,
        tag: &str,
        id: Option<&str>,
        classes: &[&str],
    ) -> Vec<IndexedRule> {
        let tag_lower = tag.to_ascii_lowercase();
        self.candidates_with_lower(tag, &tag_lower, id, classes)
    }

    /// Collect candidate rules using a pre-lowercased tag name.
    ///
    /// Avoids repeated `to_ascii_lowercase()` allocations when the
    /// caller caches lowercased tag names across elements.
    pub(super) fn candidates_with_lower(
        &self,
        _tag: &str,
        tag_lower: &str,
        id: Option<&str>,
        classes: &[&str],
    ) -> Vec<IndexedRule> {
        let mut result = Vec::new();

        // Always include universal rules.
        result.extend_from_slice(&self.universal);

        // Tag bucket.
        if let Some(rules) = self.by_tag.get(tag_lower) {
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

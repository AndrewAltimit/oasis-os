//! CSS Nesting: parsed rule tree and flattening.
//!
//! [`ParsedRule`] is the parser's intermediate representation — a rule
//! that may carry nested child rules. [`flatten_nested_rule`] desugars
//! the `&` nesting selector and produces a flat list of [`Rule`]s for
//! the cascade, applying CSS Nesting semantics from CSS Cascade Level 6.

use super::types::{
    self, Combinator, CompoundSelector, ContainerCondition, Rule, ScopeCondition, Selector,
    SelectorList,
};

/// A rule as it comes out of the parser, possibly carrying nested child
/// rules from CSS Nesting. Flattened into concrete [`Rule`]s by
/// [`flatten_nested_rule`] before the stylesheet is finalised.
pub(super) struct ParsedRule {
    pub(super) selectors: SelectorList,
    pub(super) declarations: Vec<types::Declaration>,
    pub(super) nested: Vec<ParsedRule>,
    /// Cascade layer the rule belongs to, or `None` if unlayered.
    pub(super) layer: Option<u16>,
    /// `@container` condition the rule was nested inside, if any.
    pub(super) container: Option<ContainerCondition>,
    /// `@scope` condition the rule was nested inside, if any.
    pub(super) scope: Option<ScopeCondition>,
}

/// Flatten a (possibly-nested) parsed rule into concrete [`Rule`]s,
/// desugaring the `&` nesting selector using the CSS Nesting semantics.
///
/// Parent and child selector lists are combined via a Cartesian product:
/// for each (parent, child) pair, we substitute `&` with the parent's
/// compound chain. Child rules without any `&` are treated as
/// `& <child>` (descendant combinator), matching the spec.
pub(super) fn flatten_nested_rule(
    parent: Option<&SelectorList>,
    rule: ParsedRule,
    out: &mut Vec<Rule>,
) {
    let ParsedRule {
        selectors,
        declarations,
        nested,
        layer,
        container,
        scope,
    } = rule;

    let effective = match parent {
        None => selectors,
        Some(parent_list) => combine_selector_lists(parent_list, &selectors),
    };

    // Skip emitting an empty-body parent rule when it exists purely as a
    // container for nested children. A bare `p {}` with no nested
    // children is still emitted so that existing behaviour is preserved.
    if !declarations.is_empty() || nested.is_empty() {
        out.push(Rule {
            selectors: effective.clone(),
            declarations,
            layer,
            container: container.clone(),
            scope: scope.clone(),
        });
    }

    for child in nested {
        flatten_nested_rule(Some(&effective), child, out);
    }
}

/// Combine a parent selector list with a child selector list using the
/// CSS Nesting desugaring rules. Produces the Cartesian product of
/// (parent × child) with `&` substituted for each parent selector.
fn combine_selector_lists(parent: &SelectorList, child: &SelectorList) -> SelectorList {
    let mut out = Vec::with_capacity(parent.selectors.len() * child.selectors.len());
    for child_sel in &child.selectors {
        for parent_sel in &parent.selectors {
            out.push(combine_parent_child(parent_sel, child_sel));
        }
    }
    SelectorList { selectors: out }
}

/// Combine one parent selector with one child selector. Replaces each
/// occurrence of the nesting marker `&` in `child` with `parent`'s
/// compound chain. If `child` does not contain `&`, the result is
/// `parent <descendant> child`.
fn combine_parent_child(parent: &Selector, child: &Selector) -> Selector {
    let child_has_nest = child.parts.iter().any(|(c, _)| {
        c.parts
            .iter()
            .any(|s| matches!(s, types::SimpleSelector::Nest))
    });

    if !child_has_nest {
        // Implicit `& descendant`: prepend parent chain before child,
        // with a descendant combinator linking them.
        let mut parts = parent.parts.clone();
        for (i, (compound, comb)) in child.parts.iter().enumerate() {
            let effective_comb = if i == 0 {
                Some(Combinator::Descendant)
            } else {
                comb.clone()
            };
            parts.push((compound.clone(), effective_comb));
        }
        return Selector { parts };
    }

    // Child contains `&` — splice parent into each occurrence.
    let mut out: Vec<(CompoundSelector, Option<Combinator>)> = Vec::new();
    for (compound, combinator) in &child.parts {
        let has_nest_here = compound
            .parts
            .iter()
            .any(|s| matches!(s, types::SimpleSelector::Nest));
        if !has_nest_here {
            let effective_comb = if out.is_empty() {
                None
            } else {
                combinator.clone()
            };
            out.push((compound.clone(), effective_comb));
            continue;
        }

        // Extract the non-`&` parts of this compound.
        let extras: Vec<types::SimpleSelector> = compound
            .parts
            .iter()
            .filter(|s| !matches!(s, types::SimpleSelector::Nest))
            .cloned()
            .collect();

        if parent.parts.is_empty() {
            // Shouldn't happen — a parent selector always has parts.
            continue;
        }
        let parent_last_idx = parent.parts.len() - 1;
        for (pi, (pcomp, pcomb)) in parent.parts.iter().enumerate() {
            let effective_comb = if pi == 0 {
                if out.is_empty() {
                    None
                } else {
                    combinator.clone()
                }
            } else {
                pcomb.clone()
            };
            if pi == parent_last_idx && !extras.is_empty() {
                let mut merged = pcomp.parts.clone();
                merged.extend(extras.iter().cloned());
                out.push((CompoundSelector { parts: merged }, effective_comb));
            } else {
                out.push((pcomp.clone(), effective_comb));
            }
        }
    }

    Selector { parts: out }
}

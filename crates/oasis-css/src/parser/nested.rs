//! CSS Nesting: parsed rule tree and flattening.
//!
//! [`ParsedRule`] is the parser's intermediate representation — a rule
//! that may carry nested child rules. [`flatten_nested_rule`] desugars
//! the `&` nesting selector and produces a flat list of [`Rule`]s for
//! the cascade, applying CSS Nesting semantics from CSS Cascade Level 6.

use super::types::{
    self, Combinator, CompoundSelector, ContainerCondition, Rule, ScopeCondition, Selector,
    SelectorList, SimpleSelector,
};

/// Most selectors one flattened rule may carry. Nesting combines parent
/// and child lists as a Cartesian product, so `a,b{a,b{a,b{...` doubles
/// per level (2^32 at the nesting cap); past this the rule and its
/// children are dropped.
const MAX_FLATTENED_SELECTORS: usize = 1024;

/// Most compound selectors in one flattened selector: every `&` splices
/// in the whole parent chain, so `& & &{& & &{...` grows geometrically.
/// Longer selectors are dropped from their list.
const MAX_SELECTOR_COMPOUNDS: usize = 64;

/// Most compounds (including those nested in `:is()` & co.) a selector
/// may reach once `&`s inside functional pseudo-classes are replaced by
/// `:is(parent)`.
const MAX_SUBSTITUTED_COMPOUNDS: usize = 1024;

/// The selector list `&`: the parent selector. Bare declarations in a
/// nested at-rule (`.a { @media (..) { color: red } }`) become a nested
/// rule with it.
pub(super) fn nesting_selector_list() -> SelectorList {
    SelectorList {
        selectors: vec![Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![types::SimpleSelector::Nest],
                },
                None,
            )],
        }],
    }
}

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
        Some(parent_list) => match combine_selector_lists(parent_list, &selectors) {
            Some(list) => list,
            None => {
                log::warn!("CSS nesting: flattened selector list too large, rule dropped");
                return;
            },
        },
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
/// (parent × child) with `&` substituted for each parent selector, or
/// `None` when that would exceed [`MAX_FLATTENED_SELECTORS`] (or leave no
/// selector within [`MAX_SELECTOR_COMPOUNDS`]).
fn combine_selector_lists(parent: &SelectorList, child: &SelectorList) -> Option<SelectorList> {
    let count = parent.selectors.len().saturating_mul(child.selectors.len());
    if count > MAX_FLATTENED_SELECTORS {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for child_sel in &child.selectors {
        for parent_sel in &parent.selectors {
            if let Some(sel) = combine_parent_child(parent_sel, child_sel) {
                out.push(sel);
            }
        }
    }
    (!out.is_empty()).then_some(SelectorList { selectors: out })
}

/// Combine one parent selector with one child selector. Replaces each
/// occurrence of the nesting marker `&` in `child` with `parent`'s
/// compound chain. If `child` does not contain `&`, the result is
/// `parent <descendant> child`. `None` when the result would have more
/// than [`MAX_SELECTOR_COMPOUNDS`] compounds.
fn combine_parent_child(parent: &Selector, child: &Selector) -> Option<Selector> {
    // `&` inside a functional pseudo-class (`:where(& > p)`,
    // `:not(&.x)`) stands for the parent too: substitute `:is(parent)`.
    // Such a child is not prefixed with an implicit `& `.
    let mut inner_nest = false;
    let substituted;
    let child = if selector_has_inner_nest(child) {
        inner_nest = true;
        substituted = substitute_inner_nest(child, parent);
        // `:is(&, &)` per level would otherwise grow exponentially.
        if compound_count(&substituted) > MAX_SUBSTITUTED_COMPOUNDS {
            return None;
        }
        &substituted
    } else {
        child
    };
    let nests = child
        .parts
        .iter()
        .filter(|(c, _)| {
            c.parts
                .iter()
                .any(|s| matches!(s, types::SimpleSelector::Nest))
        })
        .count();
    if child.parts.len() + parent.parts.len() * nests.max(1) > MAX_SELECTOR_COMPOUNDS {
        return None;
    }

    if nests == 0 && inner_nest {
        return Some(child.clone());
    }
    if nests == 0 {
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
        return Some(Selector { parts });
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

    Some(Selector { parts: out })
}

/// The selectors nested inside a functional pseudo-class.
fn inner_selectors(simple: &SimpleSelector) -> Box<dyn Iterator<Item = &Selector> + '_> {
    match simple {
        SimpleSelector::Not(l) | SimpleSelector::Is(l) | SimpleSelector::Where(l) => {
            Box::new(l.iter())
        },
        SimpleSelector::Has(l) => Box::new(l.iter().map(|(_, s)| s)),
        _ => Box::new(std::iter::empty()),
    }
}

/// Compounds in `sel`, counting those inside functional pseudo-classes.
fn compound_count(sel: &Selector) -> usize {
    sel.parts
        .iter()
        .map(|(c, _)| {
            1 + c
                .parts
                .iter()
                .flat_map(inner_selectors)
                .map(compound_count)
                .sum::<usize>()
        })
        .sum()
}

/// True if `sel` has a `&` anywhere inside a functional pseudo-class.
fn selector_has_inner_nest(sel: &Selector) -> bool {
    fn contains_nest(sel: &Selector) -> bool {
        sel.parts
            .iter()
            .flat_map(|(c, _)| &c.parts)
            .any(|s| matches!(s, SimpleSelector::Nest) || inner_selectors(s).any(contains_nest))
    }
    sel.parts
        .iter()
        .flat_map(|(c, _)| &c.parts)
        .any(|s| inner_selectors(s).any(contains_nest))
}

/// `sel` with every `&` inside a functional pseudo-class replaced by
/// `:is(parent)` (top-level `&`s are left for the splicing above).
fn substitute_inner_nest(sel: &Selector, parent: &Selector) -> Selector {
    fn in_selector(sel: &Selector, parent: &Selector, top: bool) -> Selector {
        let parts = sel
            .parts
            .iter()
            .map(|(compound, comb)| {
                let parts = compound
                    .parts
                    .iter()
                    .map(|s| in_simple(s, parent, top))
                    .collect();
                (CompoundSelector { parts }, comb.clone())
            })
            .collect();
        Selector { parts }
    }
    fn in_simple(simple: &SimpleSelector, parent: &Selector, top: bool) -> SimpleSelector {
        let list = |l: &[Selector]| -> Vec<Selector> {
            l.iter().map(|s| in_selector(s, parent, false)).collect()
        };
        match simple {
            SimpleSelector::Nest if !top => SimpleSelector::Is(vec![parent.clone()]),
            SimpleSelector::Not(l) => SimpleSelector::Not(list(l)),
            SimpleSelector::Is(l) => SimpleSelector::Is(list(l)),
            SimpleSelector::Where(l) => SimpleSelector::Where(list(l)),
            SimpleSelector::Has(l) => SimpleSelector::Has(
                l.iter()
                    .map(|(c, s)| (c.clone(), in_selector(s, parent, false)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    in_selector(sel, parent, true)
}

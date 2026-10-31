//! Applying sorted cascade entries to a [`ComputedStyle`]: `var()`
//! substitution, the CSS-wide keywords (`inherit`, `unset`, `revert`,
//! `revert-layer`) and their custom-property forms.

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::matching::MatchedDeclaration;
use super::var_resolve;
use crate::parser::{CssValue, Declaration, parse_substituted_declaration};
use crate::values::ComputedStyle;

/// `revert` or `revert-layer`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rollback {
    /// `revert`: roll back to the previous cascade origin.
    Origin,
    /// `revert-layer`: roll back to the previous cascade layer.
    Layer,
}

impl Rollback {
    fn of_keyword(kw: &str) -> Option<Self> {
        if kw.eq_ignore_ascii_case("revert") {
            Some(Rollback::Origin)
        } else if kw.eq_ignore_ascii_case("revert-layer") {
            Some(Rollback::Layer)
        } else {
            None
        }
    }

    /// The entry of `matched[..i]` for `matched[i]`'s property that this
    /// rollback falls back to: the cascade winner among the entries from
    /// an earlier origin (`revert`) or layer (`revert-layer`). That entry
    /// may be a shorthand still holding `var()` (`background: var(--bg)`
    /// is expanded only at computed-value time) that sets the property.
    fn fallback(self, matched: &[MatchedDeclaration], i: usize) -> Option<usize> {
        let this = &matched[i];
        (0..i).rev().find(|&j| {
            let cand = &matched[j];
            let below = match self {
                Rollback::Origin => cand.below_origin_of(this),
                Rollback::Layer => cand.below_layer_of(this),
            };
            below
                && (cand.property == this.property
                    || (matches!(&*cand.value, CssValue::Unresolved(_))
                        && shorthand_sets(&cand.property, &this.property)))
        })
    }
}

/// Whether the shorthand `shorthand` expands to (among others) the
/// longhand `longhand`.
fn shorthand_sets(shorthand: &str, longhand: &str) -> bool {
    !shorthand.starts_with("--")
        && parse_substituted_cached(shorthand, "unset")
            .iter()
            .any(|d| d.property == longhand)
}

/// Apply only the `longhand` part of the `var()`-holding `shorthand`
/// declaration `value` (a `revert` / `revert-layer` fallback): the other
/// longhands it sets keep their cascaded values.
fn apply_longhand_of(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    shorthand: &str,
    value: &CssValue,
    longhand: &str,
    font_size: f32,
) {
    let CssValue::Unresolved(text) = value else {
        return;
    };
    // Invalid at computed-value time: the longhand stays `unset`, which
    // the caller already applied.
    let Some(substituted) = var_resolve::substitute_vars(text, &style.custom_properties) else {
        return;
    };
    for decl in parse_substituted_cached(shorthand, &substituted).iter() {
        if decl.property == longhand {
            apply_value(style, parent, &decl.property, &decl.value, font_size);
        }
    }
}

impl Rollback {}

/// Apply `matched[i]` (sorted in cascade order) to `style`.
///
/// `revert` / `revert-layer` reset the property as if it had no
/// declaration (`unset`) and then re-apply the winning declaration from
/// the previous origin / layer, if any. Later (higher-priority) entries
/// are applied afterwards by the caller and override it as usual.
pub(super) fn apply_matched(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    matched: &[MatchedDeclaration],
    i: usize,
    font_size: f32,
) {
    let entry = &matched[i];
    let rollback = match &*entry.value {
        CssValue::Keyword(kw) => Rollback::of_keyword(kw),
        _ => None,
    };
    let Some(rollback) = rollback else {
        apply_cascaded(style, parent, &entry.property, &entry.value, font_size);
        return;
    };
    apply_unset(style, parent, &entry.property);
    if let Some(j) = rollback.fallback(matched, i) {
        let cand = &matched[j];
        if cand.property == entry.property {
            apply_matched(style, parent, matched, j, font_size);
        } else {
            let (shorthand, value) = (&cand.property, &cand.value);
            apply_longhand_of(style, parent, shorthand, value, &entry.property, font_size);
        }
    }
}

/// Apply the custom-property entry `matched[i]` (`--name: value`).
///
/// Handles the CSS-wide keywords, which a custom property's raw value
/// can be: `initial` gives the guaranteed-invalid value (so `var()`
/// fallbacks apply), `inherit` / `unset` take the parent's value, and
/// `revert` / `revert-layer` roll back as for other properties.
pub(super) fn apply_matched_custom(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    matched: &[MatchedDeclaration],
    i: usize,
) {
    let entry = &matched[i];
    let CssValue::String(raw) = &*entry.value else {
        return;
    };
    let kw = raw.trim();
    let rollback = Rollback::of_keyword(kw);
    let inherit = rollback.is_some()
        || kw.eq_ignore_ascii_case("inherit")
        || kw.eq_ignore_ascii_case("unset");
    if !inherit && !kw.eq_ignore_ascii_case("initial") {
        style.apply_declaration(&entry.property, &entry.value, 0.0);
        return;
    }
    let inherited = if inherit {
        parent.and_then(|p| p.custom_properties.get(&*entry.property).cloned())
    } else {
        None
    };
    let props = std::sync::Arc::make_mut(&mut style.custom_properties);
    match inherited {
        Some(v) => {
            props.insert(entry.property.to_string(), v);
        },
        None => {
            props.remove(&*entry.property);
        },
    }
    if let Some(j) = rollback.and_then(|r| r.fallback(matched, i)) {
        apply_matched_custom(style, parent, matched, j);
    }
}

/// Apply one cascaded declaration to `style`.
///
/// [`CssValue::Unresolved`] values have their `var()` references
/// substituted from `style`'s custom properties and are re-parsed with
/// the property's normal parser (which may expand a shorthand into
/// several longhands). A declaration that is invalid at computed-value
/// time behaves as `unset` (CSS Variables §3.1). The CSS-wide `unset`
/// and `inherit` keywords take their value from `parent`.
pub(super) fn apply_cascaded(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    property: &str,
    value: &CssValue,
    font_size: f32,
) {
    let CssValue::Unresolved(text) = value else {
        apply_value(style, parent, property, value, font_size);
        return;
    };
    match var_resolve::substitute_vars(text, &style.custom_properties) {
        Some(substituted) => {
            for decl in parse_substituted_cached(property, &substituted).iter() {
                apply_value(style, parent, &decl.property, &decl.value, font_size);
            }
        },
        None => {
            for decl in parse_substituted_cached(property, "unset").iter() {
                apply_unset(style, parent, &decl.property);
            }
        },
    }
}

/// Bound on [`SUBSTITUTED_CACHE`] entries before it is cleared.
const SUBSTITUTED_CACHE_MAX: usize = 4096;

/// Longest substituted value text [`SUBSTITUTED_CACHE`] keeps.
const SUBSTITUTED_CACHE_MAX_KEY: usize = 1024;

/// Parsed declarations keyed by property, then by substituted value text.
#[derive(Default)]
struct SubstitutedCache {
    entries: usize,
    map: FxHashMap<String, FxHashMap<String, Rc<[Declaration]>>>,
}

thread_local! {
    /// Parsed (and shorthand-expanded) substituted declarations. Design-
    /// token CSS resolves the same handful of values for thousands of
    /// elements, and parsing is pure, so each distinct value is tokenized
    /// and parsed once per thread.
    static SUBSTITUTED_CACHE: RefCell<SubstitutedCache> = RefCell::default();
}

fn parse_substituted_cached(property: &str, text: &str) -> Rc<[Declaration]> {
    SUBSTITUTED_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.map.get(property).and_then(|m| m.get(text)) {
            return Rc::clone(hit);
        }
        let parsed: Rc<[Declaration]> = parse_substituted_declaration(property, text).into();
        // Long values are not worth keeping: design tokens are short, and
        // 4096 hostile 64 KiB keys would pin 256 MiB per thread.
        if text.len() > SUBSTITUTED_CACHE_MAX_KEY {
            return parsed;
        }
        if cache.entries >= SUBSTITUTED_CACHE_MAX {
            cache.map.clear();
            cache.entries = 0;
        }
        cache
            .map
            .entry(property.to_string())
            .or_default()
            .insert(text.to_string(), Rc::clone(&parsed));
        cache.entries += 1;
        parsed
    })
}

fn apply_value(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    property: &str,
    value: &CssValue,
    font_size: f32,
) {
    if let CssValue::Keyword(kw) = value {
        // `revert` reached through `var()` substitution (no cascade
        // context here) behaves as `unset`, like `revert` in the UA origin.
        if kw.eq_ignore_ascii_case("unset") || Rollback::of_keyword(kw).is_some() {
            apply_unset(style, parent, property);
            return;
        }
        // `inherit` takes the parent's computed value for every property,
        // inherited or not (`border: inherit`, `background-color: inherit`).
        if kw.eq_ignore_ascii_case("inherit")
            && let Some(p) = parent
            && (style.inherit_property_from(property, p)
                || style.inherit_non_inherited_from(property, p))
        {
            return;
        }
    }
    style.apply_declaration(property, value, font_size);
}

/// `unset`: inherit for inherited properties, `initial` otherwise.
fn apply_unset(style: &mut ComputedStyle, parent: Option<&ComputedStyle>, property: &str) {
    let inherited = match parent {
        Some(p) => style.inherit_property_from(property, p),
        None => style.inherit_property_from(property, &ComputedStyle::default()),
    };
    if !inherited {
        style.apply_declaration(property, &CssValue::Keyword("initial".into()), 0.0);
    }
}

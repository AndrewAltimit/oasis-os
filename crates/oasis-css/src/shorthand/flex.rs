//! Flex shorthand expansion (CSS Flexbox 1 §7.1.1).
//!
//! `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`.
//! Omitted grow/shrink default to `1`; an omitted basis is `0` (not the
//! longhand's `auto` initial value). `auto` is `1 1 auto`, `none` is
//! `0 0 auto`, `initial` is `0 1 auto`.

use crate::parser::{CssValue, Declaration, LengthUnit, PropertyId};

fn longhands(grow: CssValue, shrink: CssValue, basis: CssValue, imp: bool) -> Vec<Declaration> {
    [
        ("flex-grow", grow),
        ("flex-shrink", shrink),
        ("flex-basis", basis),
    ]
    .into_iter()
    .map(|(property, value)| Declaration {
        property: property.into(),
        value,
        important: imp,
        property_id: PropertyId::from_name(property),
    })
    .collect()
}

/// A value usable as `flex-basis`: a length, percentage, `calc()`, or
/// one of the sizing keywords.
fn is_basis(v: &CssValue) -> bool {
    match v {
        CssValue::Length(..) | CssValue::Percentage(_) | CssValue::Calc(_) => true,
        CssValue::Keyword(kw) => matches!(
            kw.to_ascii_lowercase().as_str(),
            "auto" | "content" | "min-content" | "max-content" | "fit-content"
        ),
        _ => false,
    }
}

pub(super) fn expand_flex(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };
    let num = CssValue::Number;
    let zero_basis = || CssValue::Length(0.0, LengthUnit::Px);
    let auto = || CssValue::Keyword("auto".into());

    // Keyword-only forms.
    if let [CssValue::Keyword(kw)] = values.as_slice() {
        match kw.to_ascii_lowercase().as_str() {
            "none" => return longhands(num(0.0), num(0.0), auto(), important),
            "auto" => return longhands(num(1.0), num(1.0), auto(), important),
            "initial" => return longhands(num(0.0), num(1.0), auto(), important),
            k @ ("inherit" | "unset" | "revert" | "revert-layer") => {
                let kw = || CssValue::Keyword(k.to_string());
                return longhands(kw(), kw(), kw(), important);
            },
            _ => {},
        }
    }

    // `<grow> <shrink>?` and `<basis>` in either order.
    let mut grow: Option<f32> = None;
    let mut shrink: Option<f32> = None;
    let mut basis: Option<CssValue> = None;
    let mut iter = values.into_iter().peekable();
    while let Some(v) = iter.next() {
        match v {
            // A bare number is grow (then shrink); a third number after
            // both, or a `0` once grow/shrink are taken, is a zero basis.
            CssValue::Number(n) if grow.is_none() => {
                grow = Some(n);
                if let Some(CssValue::Number(s)) = iter.peek() {
                    shrink = Some(*s);
                    iter.next();
                }
            },
            CssValue::Number(n) if n == 0.0 && basis.is_none() => basis = Some(zero_basis()),
            v if basis.is_none() && is_basis(&v) => basis = Some(v),
            // Anything else makes the declaration invalid: drop it.
            _ => return Vec::new(),
        }
    }
    if grow.is_none() && basis.is_none() {
        return Vec::new();
    }
    longhands(
        num(grow.unwrap_or(1.0)),
        num(shrink.unwrap_or(1.0)),
        basis.unwrap_or_else(zero_basis),
        important,
    )
}

//! `@container` query condition parsing.
//!
//! Translates the raw text between `@container` and the opening brace
//! into a [`ContainerCondition`] of [`ContainerFeature`] predicates.
//! The cascade evaluates these against the nearest container ancestor
//! at match time.

use super::types::{ContainerCondition, ContainerFeature};

/// Case-insensitive split on the `and` combinator from CSS conditional
/// rules. Matches the `and` keyword wherever its left and right sides
/// are token boundaries — whitespace, `)` on the left, `(` on the
/// right, or the start/end of the input. This correctly handles the
/// zero-whitespace case `(a)and(b)` that the spec allows but which a
/// naive `" and "` substring search misses.
///
/// Identifiers that happen to contain the letters `and` (e.g.
/// `expand`, `andante`) are not split because either side would be an
/// alphanumeric / `-` / `_` continuation, not a token boundary.
pub(super) fn split_css_and(s: &str) -> Vec<&str> {
    fn is_left_boundary(b: u8) -> bool {
        b.is_ascii_whitespace() || b == b')'
    }
    fn is_right_boundary(b: u8) -> bool {
        b.is_ascii_whitespace() || b == b'('
    }
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + 3 <= len {
        let is_and = matches!(bytes[i], b'a' | b'A')
            && matches!(bytes[i + 1], b'n' | b'N')
            && matches!(bytes[i + 2], b'd' | b'D');
        if is_and {
            let left_ok = i == 0 || is_left_boundary(bytes[i - 1]);
            let right_ok = i + 3 == len || is_right_boundary(bytes[i + 3]);
            if left_ok && right_ok {
                result.push(s[start..i].trim());
                start = i + 3;
                i = start;
                continue;
            }
        }
        i += 1;
    }
    result.push(s[start..].trim());
    result
}

/// Parse a `@container` condition string into a [`ContainerCondition`].
///
/// Recognises `(min-width: Npx)`, `(max-width: Npx)`, `(width: Npx)`,
/// the `height` variants, and the `inline-size` / `block-size` aliases
/// (treated as their physical equivalents under our LTR-only horizontal
/// writing-mode assumption). Multiple features are joined with ` and `.
/// Any feature we can't parse causes that predicate to be dropped — the
/// remaining predicates still apply, and an empty feature list never
/// matches (always evaluates false).
pub(super) fn parse_container_condition(name: Option<String>, raw: &str) -> ContainerCondition {
    let mut features = Vec::new();
    let raw = raw.trim();
    if !raw.is_empty() {
        for part in split_css_and(raw) {
            let trimmed = part.trim();
            // Check for `style(...)` query in this part.
            if let Some(inner) = trimmed
                .strip_prefix("style(")
                .and_then(|s| s.strip_suffix(')'))
            {
                if let Some(f) = parse_style_query(inner) {
                    features.push(f);
                }
            } else {
                let inner = trimmed.trim_start_matches('(').trim_end_matches(')').trim();
                if let Some(f) = parse_container_feature(inner) {
                    features.push(f);
                }
            }
        }
    }
    ContainerCondition { name, features }
}

/// Parse a `style(--prop: value)` query into a `ContainerFeature::Style`.
fn parse_style_query(inner: &str) -> Option<ContainerFeature> {
    let (prop, val) = inner.split_once(':')?;
    let prop = prop.trim().to_string();
    let val = val.trim().to_string();
    if prop.is_empty() || val.is_empty() {
        return None;
    }
    Some(ContainerFeature::Style(prop, val))
}

fn parse_container_feature(inner: &str) -> Option<ContainerFeature> {
    let (key, value) = inner.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    let px = parse_px_in_condition(value.trim())?;
    Some(match key.as_str() {
        "min-width" | "min-inline-size" => ContainerFeature::MinWidth(px),
        "max-width" | "max-inline-size" => ContainerFeature::MaxWidth(px),
        "width" | "inline-size" => ContainerFeature::Width(px),
        "min-height" | "min-block-size" => ContainerFeature::MinHeight(px),
        "max-height" | "max-block-size" => ContainerFeature::MaxHeight(px),
        "height" | "block-size" => ContainerFeature::Height(px),
        _ => return None,
    })
}

fn parse_px_in_condition(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    s.trim().parse::<f32>().ok()
}

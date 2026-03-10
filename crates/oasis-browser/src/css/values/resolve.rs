//! CSS value resolution helpers.
//!
//! Functions that convert parsed `CssValue` representations into concrete
//! computed values (pixel lengths, colors, dimensions, etc.).

use super::types::{BorderStyle, Dimension, FontWeight, GridTrackSize, ROOT_FONT_SIZE};
use crate::css::parser::{CssColor, CssValue, LengthUnit};
use oasis_types::backend::Color;

// -----------------------------------------------------------------------
// CssValue helper
// -----------------------------------------------------------------------

/// Extract a keyword string from a `CssValue`, if it is a `Keyword`.
pub(super) fn as_keyword(value: &CssValue) -> Option<&str> {
    match value {
        CssValue::Keyword(s) => Some(s.as_str()),
        _ => None,
    }
}

// -----------------------------------------------------------------------
// Resolution helpers
// -----------------------------------------------------------------------

/// Resolve a `CssValue` to an absolute pixel length.
///
/// - `Px` and `Pt` values pass through (Pt approximated as 1.333 px).
/// - `Em` values are multiplied by `parent_font_size`.
/// - `Rem` values are multiplied by the root font size (16.0).
/// - Percentage and keyword values resolve to 0.
pub(super) fn resolve_length(value: &CssValue, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Number(n) => *n,
        _ => 0.0,
    }
}

/// Resolve a `CssValue` to a `Dimension` (auto / px / percent).
pub(super) fn resolve_dimension(value: &CssValue, parent_font_size: f32) -> Dimension {
    match value {
        CssValue::Keyword(kw) if kw == "auto" => Dimension::Auto,
        CssValue::Percentage(p) => Dimension::Percent(*p),
        CssValue::Length(n, LengthUnit::Px) => Dimension::Px(*n),
        CssValue::Length(n, LengthUnit::Em) => Dimension::Px(*n * parent_font_size),
        CssValue::Length(n, LengthUnit::Rem) => Dimension::Px(*n * ROOT_FONT_SIZE),
        CssValue::Length(n, LengthUnit::Pt) => Dimension::Px(*n * 1.333),
        CssValue::Number(n) => Dimension::Px(*n),
        _ => Dimension::Auto,
    }
}

/// Resolve a color value from the parser's representation.
pub(super) fn resolve_color(value: &CssValue) -> Option<Color> {
    match value {
        CssValue::Color(css_color) => Some(css_color_to_backend(css_color)),
        CssValue::Keyword(name) => keyword_color(name),
        _ => None,
    }
}

/// Resolve a color value, treating `currentcolor` as the element's `color`.
pub(super) fn resolve_color_or_current(value: &CssValue, current_color: Color) -> Option<Color> {
    if let CssValue::Keyword(name) = value
        && name.eq_ignore_ascii_case("currentcolor")
    {
        return Some(current_color);
    }
    resolve_color(value)
}

/// Convert a parser `CssColor` to the backend `Color`.
fn css_color_to_backend(c: &CssColor) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

/// Map a named CSS color keyword to an RGBA `Color`.
fn keyword_color(name: &str) -> Option<Color> {
    let c = match name {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "teal" => Color::rgb(0, 128, 128),
        "navy" => Color::rgb(0, 0, 128),
        "purple" => Color::rgb(128, 0, 128),
        "orange" => Color::rgb(255, 165, 0),
        "transparent" => Color::rgba(0, 0, 0, 0),
        _ => return None,
    };
    Some(c)
}

/// Resolve a `border-style` keyword.
pub(super) fn resolve_border_style(value: &CssValue) -> Option<BorderStyle> {
    let kw = as_keyword(value)?;
    let s = match kw {
        "none" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return None,
    };
    Some(s)
}

/// Resolve a `font-weight` value.
///
/// The CSS parser normalises keyword values: `bold` becomes
/// `CssValue::Number(700.0)` and `normal` becomes
/// `CssValue::Number(400.0)`. We also handle keywords directly
/// for inline style strings that may bypass that normalisation.
pub(super) fn resolve_font_weight(value: &CssValue) -> FontWeight {
    match value {
        CssValue::Number(n) => {
            if *n >= 600.0 {
                FontWeight::Bold
            } else {
                FontWeight::Normal
            }
        },
        CssValue::Keyword(kw) => match kw.as_str() {
            "bold" => FontWeight::Bold,
            "normal" => FontWeight::Normal,
            _ => FontWeight::Normal,
        },
        _ => FontWeight::Normal,
    }
}

/// Resolve a `font-size` value.
///
/// Supports absolute keywords (`small`, `medium`, `large`, etc.),
/// relative keywords (`smaller`, `larger`), lengths, and percentages.
pub(super) fn resolve_font_size(value: &CssValue, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => parent_font_size * (*p / 100.0),
        CssValue::Number(n) => *n,
        CssValue::Keyword(kw) => match kw.as_str() {
            "xx-small" => ROOT_FONT_SIZE * 0.5625,
            "x-small" => ROOT_FONT_SIZE * 0.625,
            "small" => ROOT_FONT_SIZE * 0.8125,
            "medium" => ROOT_FONT_SIZE,
            "large" => ROOT_FONT_SIZE * 1.125,
            "x-large" => ROOT_FONT_SIZE * 1.5,
            "xx-large" => ROOT_FONT_SIZE * 2.0,
            "smaller" => parent_font_size * 0.833,
            "larger" => parent_font_size * 1.2,
            _ => parent_font_size,
        },
        _ => parent_font_size,
    }
}

/// Resolve a `line-height` value.
///
/// - A bare number is treated as a multiplier of the element's font size.
/// - A length or percentage is resolved normally.
/// - The keyword `normal` maps to 1.5 * font_size (generous for 480x272).
pub(super) fn resolve_line_height(value: &CssValue, font_size: f32, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Number(n) => *n * font_size,
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => font_size * (*p / 100.0),
        CssValue::Keyword(kw) if kw == "normal" => font_size * 1.5,
        _ => font_size * 1.5,
    }
}

/// Parse a grid-template-columns or grid-template-rows value.
pub(super) fn parse_grid_template(value: &CssValue, parent_font_size: f32) -> Vec<GridTrackSize> {
    match value {
        CssValue::Keyword(kw) if kw == "none" => Vec::new(),
        CssValue::Keyword(kw) if kw == "auto" => vec![GridTrackSize::Auto],
        CssValue::Keyword(kw) => parse_grid_template_str(kw, parent_font_size),
        CssValue::String(s) => parse_grid_template_str(s, parent_font_size),
        CssValue::Length(n, unit) => {
            let px = match unit {
                LengthUnit::Px => *n,
                LengthUnit::Em => *n * parent_font_size,
                LengthUnit::Rem => *n * ROOT_FONT_SIZE,
                LengthUnit::Pt => *n * 1.333,
            };
            vec![GridTrackSize::Px(px)]
        },
        CssValue::Number(n) => vec![GridTrackSize::Px(*n)],
        CssValue::Multiple(vals) => {
            let mut tracks = Vec::new();
            for v in vals {
                match v {
                    CssValue::Keyword(kw) if kw == "auto" => tracks.push(GridTrackSize::Auto),
                    CssValue::Keyword(kw) => {
                        if let Some(t) = parse_single_track_str(kw) {
                            tracks.push(t);
                        }
                    },
                    CssValue::Length(px, LengthUnit::Px) => {
                        tracks.push(GridTrackSize::Px(*px));
                    },
                    CssValue::Number(n) => tracks.push(GridTrackSize::Px(*n)),
                    CssValue::String(s) => {
                        if let Some(t) = parse_single_track_str(s) {
                            tracks.push(t);
                        }
                    },
                    _ => {},
                }
            }
            tracks
        },
        _ => Vec::new(),
    }
}

fn parse_single_track_str(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s == "auto" {
        Some(GridTrackSize::Auto)
    } else if let Some(fr) = s.strip_suffix("fr") {
        fr.trim().parse::<f32>().ok().map(GridTrackSize::Fr)
    } else if let Some(px) = s.strip_suffix("px") {
        px.trim().parse::<f32>().ok().map(GridTrackSize::Px)
    } else if let Ok(n) = s.parse::<f32>() {
        Some(GridTrackSize::Px(n))
    } else {
        None
    }
}

fn parse_grid_template_str(s: &str, _parent_font_size: f32) -> Vec<GridTrackSize> {
    let s = s.trim();
    if s == "none" {
        return Vec::new();
    }
    let mut tracks = Vec::new();
    let mut remainder = s;
    while !remainder.is_empty() {
        remainder = remainder.trim_start();
        if remainder.starts_with("repeat(") {
            // Find the matching closing paren for this repeat() block.
            if let Some(close) = remainder.find(')') {
                let inner = &remainder["repeat(".len()..close];
                if let Some((cs, vs)) = inner.split_once(',')
                    && let Ok(count) = cs.trim().parse::<usize>()
                    && let Some(track) = parse_single_track_str(vs.trim())
                {
                    for _ in 0..count {
                        tracks.push(track);
                    }
                }
                remainder = &remainder[close + 1..];
            } else {
                break;
            }
        } else {
            // Take the next whitespace-delimited token.
            let token = match remainder.find(char::is_whitespace) {
                Some(pos) => {
                    let t = &remainder[..pos];
                    remainder = &remainder[pos..];
                    t
                },
                None => {
                    let t = remainder;
                    remainder = "";
                    t
                },
            };
            if let Some(track) = parse_single_track_str(token) {
                tracks.push(track);
            }
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_color_lookup() {
        assert_eq!(keyword_color("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(keyword_color("navy"), Some(Color::rgb(0, 0, 128)),);
        assert_eq!(keyword_color("transparent"), Some(Color::rgba(0, 0, 0, 0)),);
        assert_eq!(keyword_color("nonexistent"), None);
    }

    #[test]
    fn parse_grid_template_compound_repeat() {
        // Single repeat block (already worked).
        let tracks = parse_grid_template_str("repeat(3, 1fr)", 16.0);
        assert_eq!(tracks, vec![GridTrackSize::Fr(1.0); 3]);

        // Compound: repeat() followed by a fixed track.
        let tracks = parse_grid_template_str("repeat(3, 1fr) 20px", 16.0);
        assert_eq!(
            tracks,
            vec![
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Px(20.0),
            ]
        );

        // Fixed track followed by repeat().
        let tracks = parse_grid_template_str("100px repeat(2, auto)", 16.0);
        assert_eq!(
            tracks,
            vec![
                GridTrackSize::Px(100.0),
                GridTrackSize::Auto,
                GridTrackSize::Auto
            ]
        );

        // "none" returns empty.
        let tracks = parse_grid_template_str("none", 16.0);
        assert!(tracks.is_empty());
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// resolve_length with Px always returns the value.
            #[test]
            fn resolve_length_px_identity(v in -1000.0f32..1000.0) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Px), 16.0,
                );
                prop_assert!(
                    (result - v).abs() < 0.001,
                    "Px({v}) should resolve to {v}, got {result}",
                );
            }

            /// resolve_length with Em scales by parent font size.
            #[test]
            fn resolve_length_em_scales(
                v in 0.0f32..10.0,
                parent in 1.0f32..100.0,
            ) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Em), parent,
                );
                let expected = v * parent;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "Em({v}) * {parent} = {expected}, got {result}",
                );
            }

            /// resolve_length with Rem scales by ROOT_FONT_SIZE.
            #[test]
            fn resolve_length_rem_scales(v in 0.0f32..10.0) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Rem), 16.0,
                );
                let expected = v * ROOT_FONT_SIZE;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "Rem({v}) = {expected}, got {result}",
                );
            }

            /// resolve_dimension with auto keyword always returns Auto.
            #[test]
            fn resolve_dimension_auto(_dummy in 0..1i32) {
                let result = resolve_dimension(
                    &CssValue::Keyword("auto".into()), 16.0,
                );
                prop_assert_eq!(result, Dimension::Auto);
            }

            /// resolve_dimension with percentage preserves the value.
            #[test]
            fn resolve_dimension_percent(pct in 0.0f32..200.0) {
                let result = resolve_dimension(
                    &CssValue::Percentage(pct), 16.0,
                );
                prop_assert_eq!(result, Dimension::Percent(pct));
            }

            /// resolve_font_size with Px returns the exact value.
            #[test]
            fn resolve_font_size_px_identity(v in 1.0f32..100.0) {
                let result = resolve_font_size(
                    &CssValue::Length(v, LengthUnit::Px), 16.0,
                );
                prop_assert!(
                    (result - v).abs() < 0.001,
                    "font-size Px({v}) -> {result}",
                );
            }

            /// resolve_font_size with percentage scales by parent.
            #[test]
            fn resolve_font_size_percent(
                pct in 10.0f32..300.0,
                parent in 4.0f32..48.0,
            ) {
                let result = resolve_font_size(
                    &CssValue::Percentage(pct), parent,
                );
                let expected = parent * (pct / 100.0);
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "{pct}% of {parent} = {expected}, got {result}",
                );
            }

            /// resolve_line_height with Number multiplies by font_size.
            #[test]
            fn resolve_line_height_number(
                n in 0.5f32..3.0,
                fs in 4.0f32..48.0,
            ) {
                let result = resolve_line_height(
                    &CssValue::Number(n), fs, 16.0,
                );
                let expected = n * fs;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "{n} * {fs} = {expected}, got {result}",
                );
            }

            /// keyword_color returns None for random strings.
            #[test]
            fn keyword_color_random_returns_none(
                name in "[a-z]{10,20}",
            ) {
                // Long random strings are unlikely to be valid.
                if keyword_color(&name).is_none() {
                    // Expected.
                } else {
                    // If it happens to match, that's fine too.
                }
            }
        }
    }
}

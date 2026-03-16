//! Gradient parsing: linear, repeating-linear, and radial.

use crate::css::helpers::try_parse_color;
use crate::css::parser::CssColor;
use crate::css::tokenizer::CssToken;

/// Parse the inner tokens of a `linear-gradient(...)` function call.
///
/// Supports:
/// - Direction keywords: `to top`, `to right`, `to bottom`, `to left`
/// - Angle values: `180deg`, `0.5turn`
/// - Color stops with optional positions: `red 0%`, `#fff 50%`, `blue`
pub fn parse_linear_gradient(
    inner_tokens: &[CssToken],
) -> Option<crate::css::values::LinearGradient> {
    use crate::css::values::{GradientStop, LinearGradient};

    // Skip the Function token and trailing CloseParen.
    let args = match inner_tokens.first() {
        Some(CssToken::Function(_)) => &inner_tokens[1..],
        _ => inner_tokens,
    };
    let args = match args.last() {
        Some(CssToken::CloseParen) => &args[..args.len() - 1],
        _ => args,
    };

    // Split by commas into argument groups.
    let mut groups: Vec<Vec<&CssToken>> = Vec::new();
    let mut current: Vec<&CssToken> = Vec::new();
    for tok in args {
        if matches!(tok, CssToken::Comma) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else if !matches!(tok, CssToken::Whitespace) {
            current.push(tok);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }

    if groups.len() < 2 {
        return None;
    }

    // Try to parse direction from first group.
    let (direction, color_start) = parse_gradient_direction(&groups[0]);
    let color_groups = &groups[color_start..];

    if color_groups.len() < 2 {
        return None;
    }

    // Parse color stops.
    let mut stops = Vec::new();
    for (i, group) in color_groups.iter().enumerate() {
        let (color, position) = parse_gradient_stop(group)?;
        let pos = position.unwrap_or_else(|| {
            if color_groups.len() <= 1 {
                0.0
            } else {
                i as f32 / (color_groups.len() - 1) as f32
            }
        });
        stops.push(GradientStop {
            color: oasis_types::backend::Color::rgba(color.r, color.g, color.b, color.a),
            position: pos,
        });
    }

    Some(LinearGradient {
        direction,
        stops,
        repeating: false,
    })
}

fn parse_gradient_direction(group: &[&CssToken]) -> (crate::css::values::GradientDirection, usize) {
    use crate::css::values::GradientDirection;

    // Check for "to <side>" keyword.
    if group.len() >= 2
        && let CssToken::Ident(first) = group[0]
        && first.eq_ignore_ascii_case("to")
        && let CssToken::Ident(side) = group[1]
    {
        let dir = match side.to_ascii_lowercase().as_str() {
            "top" => GradientDirection::ToTop,
            "right" => GradientDirection::ToRight,
            "bottom" => GradientDirection::ToBottom,
            "left" => GradientDirection::ToLeft,
            _ => return (GradientDirection::ToBottom, 0),
        };
        return (dir, 1); // skip first group (direction)
    }

    // Check for angle value (e.g. "180deg", "0.5turn").
    if group.len() == 1
        && let CssToken::Dimension(val, unit) = group[0]
    {
        let angle = match unit.to_ascii_lowercase().as_str() {
            "deg" => *val,
            "rad" => val * 180.0 / std::f32::consts::PI,
            "turn" => val * 360.0,
            "grad" => val * 0.9,
            _ => return (GradientDirection::ToBottom, 0),
        };
        return (GradientDirection::Angle(angle), 1);
    }

    // Default direction: to bottom.
    (GradientDirection::ToBottom, 0)
}

/// Parse `repeating-linear-gradient(...)` -- identical to linear but with
/// the `repeating` flag set.
pub fn parse_repeating_linear_gradient(
    inner_tokens: &[CssToken],
) -> Option<crate::css::values::LinearGradient> {
    let mut grad = parse_linear_gradient(inner_tokens)?;
    grad.repeating = true;
    Some(grad)
}

/// Parse the inner tokens of a `radial-gradient(...)` function call.
///
/// Supports:
/// - Shape keyword: `circle` or `ellipse` (default)
/// - Color stops with optional positions (reuses linear stop parsing)
///
/// The center is always 50% 50% (center of the element).
pub fn parse_radial_gradient(
    inner_tokens: &[CssToken],
) -> Option<crate::css::values::RadialGradient> {
    use crate::css::values::{GradientStop, RadialGradient};

    // Strip wrapping Function / CloseParen tokens.
    let args = match inner_tokens.first() {
        Some(CssToken::Function(_)) => &inner_tokens[1..],
        _ => inner_tokens,
    };
    let args = match args.last() {
        Some(CssToken::CloseParen) => &args[..args.len() - 1],
        _ => args,
    };

    // Split by commas into argument groups.
    let mut groups: Vec<Vec<&CssToken>> = Vec::new();
    let mut current: Vec<&CssToken> = Vec::new();
    for tok in args {
        if matches!(tok, CssToken::Comma) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else if !matches!(tok, CssToken::Whitespace) {
            current.push(tok);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }

    if groups.len() < 2 {
        return None;
    }

    // Check if the first group is a shape keyword.
    let (shape_circle, color_start) = parse_radial_shape(&groups[0]);
    let color_groups = &groups[color_start..];

    if color_groups.len() < 2 {
        return None;
    }

    // Parse color stops.
    let mut stops = Vec::new();
    for (i, group) in color_groups.iter().enumerate() {
        let (color, position) = parse_gradient_stop(group)?;
        let pos = position.unwrap_or_else(|| {
            if color_groups.len() <= 1 {
                0.0
            } else {
                i as f32 / (color_groups.len() - 1) as f32
            }
        });
        stops.push(GradientStop {
            color: oasis_types::backend::Color::rgba(color.r, color.g, color.b, color.a),
            position: pos,
        });
    }

    Some(RadialGradient {
        shape_circle,
        stops,
    })
}

/// Try to parse a radial gradient shape keyword from the first group.
/// Returns `(is_circle, groups_to_skip)`.
fn parse_radial_shape(group: &[&CssToken]) -> (bool, usize) {
    // Look for "circle" or "ellipse" keyword (possibly alongside
    // other keywords like "at center" which we ignore).
    for tok in group {
        if let CssToken::Ident(id) = tok {
            let lower = id.to_ascii_lowercase();
            if lower == "circle" {
                return (true, 1);
            }
            if lower == "ellipse" {
                return (false, 1);
            }
        }
    }
    // If the first group doesn't look like a color, it might be a
    // shape descriptor we don't fully parse (e.g. "closest-side").
    // Try treating it as a color; if it fails, skip it.
    let color_tokens: Vec<CssToken> = group.iter().map(|t| (*t).clone()).collect();
    if try_parse_color(&color_tokens).is_some() {
        return (false, 0); // default ellipse, first group is a color
    }
    (false, 1) // skip unrecognised shape descriptor
}

fn parse_gradient_stop(group: &[&CssToken]) -> Option<(CssColor, Option<f32>)> {
    // Try to parse the color from the tokens.
    let color_tokens: Vec<CssToken> = group.iter().map(|t| (*t).clone()).collect();

    // Try to get color from first token(s).
    let color = try_parse_color(&color_tokens)?;

    // Check for position (percentage or length) in remaining tokens.
    let position = group.iter().find_map(|tok| match tok {
        CssToken::Percentage(p) => Some(*p / 100.0),
        CssToken::Dimension(v, unit) if unit.eq_ignore_ascii_case("px") => {
            // Position in px -- can't resolve without knowing box size,
            // treat as percentage of a typical box (approximate).
            Some((*v / 100.0).clamp(0.0, 1.0))
        },
        _ => None,
    });

    Some((color, position))
}

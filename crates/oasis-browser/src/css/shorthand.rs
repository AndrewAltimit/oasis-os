//! CSS shorthand expansion and gradient parsing.
//!
//! Extracts shorthand CSS properties (margin, padding, border, background,
//! list-style, flex, font) into their longhand equivalents, and parses
//! `linear-gradient(...)` function values.

use super::helpers::{named_color, try_parse_color};
use super::parser::{CssColor, CssValue, Declaration, LengthUnit};
use super::tokenizer::CssToken;

pub(crate) fn expand_shorthands(decls: Vec<Declaration>) -> Vec<Declaration> {
    let mut out = Vec::new();
    for decl in decls {
        match decl.property.as_str() {
            "margin" => {
                out.extend(expand_box_shorthand("margin", &decl.value, decl.important));
            },
            "padding" => {
                out.extend(expand_box_shorthand("padding", &decl.value, decl.important));
            },
            "border" => {
                out.extend(expand_border(&decl.value, decl.important));
            },
            "border-top" | "border-right" | "border-bottom" | "border-left" => {
                out.extend(expand_border_side(
                    &decl.property,
                    &decl.value,
                    decl.important,
                ));
            },
            "background" => {
                out.extend(expand_background(&decl.value, decl.important));
            },
            "list-style" => {
                out.extend(expand_list_style(&decl.value, decl.important));
            },
            "border-width" => {
                let vals = match &decl.value {
                    CssValue::Multiple(vs) => vs.clone(),
                    other => vec![other.clone()],
                };
                let (t, r, b, l) = match vals.len() {
                    1 => (
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                    ),
                    2 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[0].clone(),
                        vals[1].clone(),
                    ),
                    3 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[2].clone(),
                        vals[1].clone(),
                    ),
                    _ => (
                        vals[0].clone(),
                        vals.get(1).cloned().unwrap_or(vals[0].clone()),
                        vals.get(2).cloned().unwrap_or(vals[0].clone()),
                        vals.get(3).cloned().unwrap_or(vals[0].clone()),
                    ),
                };
                for (prop, val) in [
                    ("border-top-width", t),
                    ("border-right-width", r),
                    ("border-bottom-width", b),
                    ("border-left-width", l),
                ] {
                    out.push(Declaration {
                        property: prop.into(),
                        value: val,
                        important: decl.important,
                    });
                }
            },
            "border-style" => {
                let vals = match &decl.value {
                    CssValue::Multiple(vs) => vs.clone(),
                    other => vec![other.clone()],
                };
                let (t, r, b, l) = match vals.len() {
                    1 => (
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                    ),
                    2 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[0].clone(),
                        vals[1].clone(),
                    ),
                    3 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[2].clone(),
                        vals[1].clone(),
                    ),
                    _ => (
                        vals[0].clone(),
                        vals.get(1).cloned().unwrap_or(vals[0].clone()),
                        vals.get(2).cloned().unwrap_or(vals[0].clone()),
                        vals.get(3).cloned().unwrap_or(vals[0].clone()),
                    ),
                };
                for (prop, val) in [
                    ("border-top-style", t),
                    ("border-right-style", r),
                    ("border-bottom-style", b),
                    ("border-left-style", l),
                ] {
                    out.push(Declaration {
                        property: prop.into(),
                        value: val,
                        important: decl.important,
                    });
                }
            },
            "border-color" => {
                let vals = match &decl.value {
                    CssValue::Multiple(vs) => vs.clone(),
                    other => vec![other.clone()],
                };
                let (t, r, b, l) = match vals.len() {
                    1 => (
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                        vals[0].clone(),
                    ),
                    2 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[0].clone(),
                        vals[1].clone(),
                    ),
                    3 => (
                        vals[0].clone(),
                        vals[1].clone(),
                        vals[2].clone(),
                        vals[1].clone(),
                    ),
                    _ => (
                        vals[0].clone(),
                        vals.get(1).cloned().unwrap_or(vals[0].clone()),
                        vals.get(2).cloned().unwrap_or(vals[0].clone()),
                        vals.get(3).cloned().unwrap_or(vals[0].clone()),
                    ),
                };
                for (prop, val) in [
                    ("border-top-color", t),
                    ("border-right-color", r),
                    ("border-bottom-color", b),
                    ("border-left-color", l),
                ] {
                    out.push(Declaration {
                        property: prop.into(),
                        value: val,
                        important: decl.important,
                    });
                }
            },
            "flex" => {
                out.extend(expand_flex(&decl.value, decl.important));
            },
            "font" => {
                out.extend(expand_font(&decl.value, decl.important));
            },
            "overflow" => {
                // `overflow` shorthand sets both overflow-x and overflow-y.
                // We only support a single overflow property, so just pass through.
                out.push(decl);
            },
            _ => out.push(decl),
        }
    }
    out
}

fn expand_box_shorthand(prefix: &str, value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let (top, right, bottom, left) = match values.len() {
        1 => {
            let v = &values[0];
            (v.clone(), v.clone(), v.clone(), v.clone())
        },
        2 => {
            let tb = &values[0];
            let lr = &values[1];
            (tb.clone(), lr.clone(), tb.clone(), lr.clone())
        },
        3 => (
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[1].clone(),
        ),
        _ => (
            values[0].clone(),
            values.get(1).cloned().unwrap_or_else(|| values[0].clone()),
            values.get(2).cloned().unwrap_or_else(|| values[0].clone()),
            values.get(3).cloned().unwrap_or_else(|| values[0].clone()),
        ),
    };

    vec![
        Declaration {
            property: format!("{}-top", prefix),
            value: top,
            important,
        },
        Declaration {
            property: format!("{}-right", prefix),
            value: right,
            important,
        },
        Declaration {
            property: format!("{}-bottom", prefix),
            value: bottom,
            important,
        },
        Declaration {
            property: format!("{}-left", prefix),
            value: left,
            important,
        },
    ]
}

fn expand_border(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let mut width = CssValue::Keyword("medium".into());
    let mut style = CssValue::Keyword("none".into());
    let mut color = CssValue::Keyword("currentcolor".into());

    for v in &values {
        match v {
            CssValue::Length(..) | CssValue::Number(_) => {
                width = v.clone();
            },
            CssValue::Color(_) => {
                color = v.clone();
            },
            CssValue::Keyword(kw) => {
                let lower = kw.to_ascii_lowercase();
                if is_border_style(&lower) {
                    style = v.clone();
                } else if let Some(c) = named_color(&lower) {
                    color = CssValue::Color(c);
                } else {
                    // Fallback: treat as style.
                    style = v.clone();
                }
            },
            CssValue::Var(..) => {
                // Unresolved var() -- most likely a color reference.
                color = v.clone();
            },
            _ => {},
        }
    }

    vec![
        Declaration {
            property: "border-width".into(),
            value: width,
            important,
        },
        Declaration {
            property: "border-style".into(),
            value: style,
            important,
        },
        Declaration {
            property: "border-color".into(),
            value: color,
            important,
        },
    ]
}

/// Expand `border-top`, `border-right`, `border-bottom`, `border-left`
/// shorthands into their `*-width`, `*-style`, `*-color` longhands.
fn expand_border_side(property: &str, value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let mut width = CssValue::Keyword("medium".into());
    let mut style = CssValue::Keyword("none".into());
    let mut color = CssValue::Keyword("currentcolor".into());

    for v in &values {
        match v {
            CssValue::Length(..) | CssValue::Number(_) => width = v.clone(),
            CssValue::Color(_) => color = v.clone(),
            CssValue::Keyword(kw) => {
                let lower = kw.to_ascii_lowercase();
                if is_border_style(&lower) {
                    style = v.clone();
                } else if let Some(c) = named_color(&lower) {
                    color = CssValue::Color(c);
                } else {
                    color = v.clone();
                }
            },
            CssValue::Var(..) => color = v.clone(),
            _ => {},
        }
    }

    // side = "border-top" -> prefix for longhands: "border-top-width", etc.
    vec![
        Declaration {
            property: format!("{property}-width"),
            value: width,
            important,
        },
        Declaration {
            property: format!("{property}-style"),
            value: style,
            important,
        },
        Declaration {
            property: format!("{property}-color"),
            value: color,
            important,
        },
    ]
}

fn is_border_style(s: &str) -> bool {
    matches!(
        s,
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

fn expand_background(value: &CssValue, important: bool) -> Vec<Declaration> {
    // Simple heuristic: if the value is a color, set background-color.
    // If the value is a url(), set background-image.
    match value {
        CssValue::Url(_) | CssValue::Gradient(_) => {
            vec![Declaration {
                property: "background-image".into(),
                value: value.clone(),
                important,
            }]
        },
        CssValue::Color(_) | CssValue::Var(..) => {
            vec![Declaration {
                property: "background-color".into(),
                value: value.clone(),
                important,
            }]
        },
        CssValue::Multiple(vs) => {
            let mut decls = Vec::new();
            for v in vs {
                if matches!(v, CssValue::Url(_) | CssValue::Gradient(_)) {
                    decls.push(Declaration {
                        property: "background-image".into(),
                        value: v.clone(),
                        important,
                    });
                } else if matches!(v, CssValue::Color(_) | CssValue::Var(..)) {
                    decls.push(Declaration {
                        property: "background-color".into(),
                        value: v.clone(),
                        important,
                    });
                }
            }
            if decls.is_empty() {
                decls.push(Declaration {
                    property: "background".into(),
                    value: value.clone(),
                    important,
                });
            }
            decls
        },
        CssValue::Keyword(name) => {
            if name.eq_ignore_ascii_case("transparent") || name.eq_ignore_ascii_case("none") {
                vec![Declaration {
                    property: "background-color".into(),
                    value: CssValue::Color(CssColor::new(0, 0, 0, 0)),
                    important,
                }]
            } else if let Some(c) = named_color(name) {
                vec![Declaration {
                    property: "background-color".into(),
                    value: CssValue::Color(c),
                    important,
                }]
            } else {
                vec![Declaration {
                    property: "background".into(),
                    value: value.clone(),
                    important,
                }]
            }
        },
        _ => {
            vec![Declaration {
                property: "background".into(),
                value: value.clone(),
                important,
            }]
        },
    }
}

fn expand_list_style(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let mut result = Vec::new();
    for v in &values {
        if let CssValue::Keyword(kw) = v {
            let kw_lower = kw.to_ascii_lowercase();
            match kw_lower.as_str() {
                "none"
                | "disc"
                | "circle"
                | "square"
                | "decimal"
                | "decimal-leading-zero"
                | "lower-roman"
                | "upper-roman"
                | "lower-alpha"
                | "upper-alpha"
                | "lower-latin"
                | "upper-latin" => {
                    result.push(Declaration {
                        property: "list-style-type".into(),
                        value: CssValue::Keyword(kw_lower),
                        important,
                    });
                },
                "inside" | "outside" => {
                    result.push(Declaration {
                        property: "list-style-position".into(),
                        value: CssValue::Keyword(kw_lower),
                        important,
                    });
                },
                _ => {},
            }
        }
    }

    // If "none" was the only value, also reset list-style-type.
    if result.is_empty() {
        result.push(Declaration {
            property: "list-style-type".into(),
            value: CssValue::Keyword("none".into()),
            important,
        });
    }

    result
}

fn expand_flex(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    match values.len() {
        1 => match &values[0] {
            CssValue::Keyword(kw) if kw == "none" => {
                vec![
                    Declaration {
                        property: "flex-grow".into(),
                        value: CssValue::Number(0.0),
                        important,
                    },
                    Declaration {
                        property: "flex-shrink".into(),
                        value: CssValue::Number(0.0),
                        important,
                    },
                    Declaration {
                        property: "flex-basis".into(),
                        value: CssValue::Keyword("auto".into()),
                        important,
                    },
                ]
            },
            CssValue::Number(n) => {
                vec![
                    Declaration {
                        property: "flex-grow".into(),
                        value: CssValue::Number(*n),
                        important,
                    },
                    Declaration {
                        property: "flex-shrink".into(),
                        value: CssValue::Number(1.0),
                        important,
                    },
                    Declaration {
                        property: "flex-basis".into(),
                        value: CssValue::Length(0.0, LengthUnit::Px),
                        important,
                    },
                ]
            },
            _ => vec![Declaration {
                property: "flex".into(),
                value: value.clone(),
                important,
            }],
        },
        _ => vec![Declaration {
            property: "flex".into(),
            value: value.clone(),
            important,
        }],
    }
}

fn expand_font(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let mut result = Vec::new();

    for v in &values {
        match v {
            CssValue::Length(_, _) | CssValue::Percentage(_) => {
                // This is likely the font-size.
                result.push(Declaration {
                    property: "font-size".into(),
                    value: v.clone(),
                    important,
                });
            },
            CssValue::Number(n) if *n > 0.0 => {
                // Could be line-height if we already have font-size, or font-weight.
                if result.iter().any(|d| d.property == "font-size") {
                    result.push(Declaration {
                        property: "line-height".into(),
                        value: v.clone(),
                        important,
                    });
                } else if *n >= 100.0 {
                    result.push(Declaration {
                        property: "font-weight".into(),
                        value: v.clone(),
                        important,
                    });
                }
            },
            CssValue::Keyword(kw) => {
                let kw_lower = kw.to_ascii_lowercase();
                match kw_lower.as_str() {
                    "bold" => {
                        result.push(Declaration {
                            property: "font-weight".into(),
                            value: CssValue::Keyword("bold".into()),
                            important,
                        });
                    },
                    "normal" => {
                        // Could be font-weight or font-style -- skip ambiguous.
                    },
                    "italic" | "oblique" => {
                        result.push(Declaration {
                            property: "font-style".into(),
                            value: CssValue::Keyword(kw_lower),
                            important,
                        });
                    },
                    "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" => {
                        result.push(Declaration {
                            property: "font-family".into(),
                            value: CssValue::Keyword(kw_lower),
                            important,
                        });
                    },
                    _ => {},
                }
            },
            _ => {},
        }
    }

    if result.is_empty() {
        // Pass through as-is if we couldn't extract anything.
        result.push(Declaration {
            property: "font".into(),
            value: value.clone(),
            important,
        });
    }

    result
}

// -------------------------------------------------------------------
// Linear gradient parser
// -------------------------------------------------------------------

/// Parse the inner tokens of a `linear-gradient(...)` function call.
///
/// Supports:
/// - Direction keywords: `to top`, `to right`, `to bottom`, `to left`
/// - Angle values: `180deg`, `0.5turn`
/// - Color stops with optional positions: `red 0%`, `#fff 50%`, `blue`
pub(crate) fn parse_linear_gradient(
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

    Some(LinearGradient { direction, stops })
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

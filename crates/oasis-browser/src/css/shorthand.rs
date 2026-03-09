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

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers ---------------------------------------------------

    fn decl(property: &str, value: CssValue) -> Declaration {
        Declaration {
            property: property.into(),
            value,
            important: false,
        }
    }

    fn decl_important(property: &str, value: CssValue) -> Declaration {
        Declaration {
            property: property.into(),
            value,
            important: true,
        }
    }

    fn px(v: f32) -> CssValue {
        CssValue::Length(v, LengthUnit::Px)
    }

    fn kw(s: &str) -> CssValue {
        CssValue::Keyword(s.into())
    }

    fn num(n: f32) -> CssValue {
        CssValue::Number(n)
    }

    fn pct(p: f32) -> CssValue {
        CssValue::Percentage(p)
    }

    fn expand(property: &str, value: CssValue) -> Vec<Declaration> {
        expand_shorthands(vec![decl(property, value)])
    }

    fn expand_imp(property: &str, value: CssValue) -> Vec<Declaration> {
        expand_shorthands(vec![decl_important(property, value)])
    }

    // -- margin shorthand -----------------------------------------

    #[test]
    fn margin_one_value() {
        let result = expand("margin", px(10.0));
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("margin-top", px(10.0)));
        assert_eq!(result[1], decl("margin-right", px(10.0)));
        assert_eq!(result[2], decl("margin-bottom", px(10.0)));
        assert_eq!(result[3], decl("margin-left", px(10.0)));
    }

    #[test]
    fn margin_two_values() {
        let val = CssValue::Multiple(vec![px(10.0), px(20.0)]);
        let result = expand("margin", val);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("margin-top", px(10.0)));
        assert_eq!(result[1], decl("margin-right", px(20.0)));
        assert_eq!(result[2], decl("margin-bottom", px(10.0)));
        assert_eq!(result[3], decl("margin-left", px(20.0)));
    }

    #[test]
    fn margin_three_values() {
        let val = CssValue::Multiple(vec![px(10.0), px(20.0), px(30.0)]);
        let result = expand("margin", val);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("margin-top", px(10.0)));
        assert_eq!(result[1], decl("margin-right", px(20.0)));
        assert_eq!(result[2], decl("margin-bottom", px(30.0)));
        assert_eq!(result[3], decl("margin-left", px(20.0)));
    }

    #[test]
    fn margin_four_values() {
        let val = CssValue::Multiple(vec![px(10.0), px(20.0), px(30.0), px(40.0)]);
        let result = expand("margin", val);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("margin-top", px(10.0)));
        assert_eq!(result[1], decl("margin-right", px(20.0)));
        assert_eq!(result[2], decl("margin-bottom", px(30.0)));
        assert_eq!(result[3], decl("margin-left", px(40.0)));
    }

    #[test]
    fn margin_auto_keyword() {
        let val = CssValue::Multiple(vec![px(0.0), kw("auto")]);
        let result = expand("margin", val);
        assert_eq!(result[0], decl("margin-top", px(0.0)));
        assert_eq!(result[1], decl("margin-right", kw("auto")));
        assert_eq!(result[2], decl("margin-bottom", px(0.0)));
        assert_eq!(result[3], decl("margin-left", kw("auto")));
    }

    #[test]
    fn margin_preserves_important() {
        let result = expand_imp("margin", px(5.0));
        assert_eq!(result.len(), 4);
        for d in &result {
            assert!(d.important);
        }
    }

    // -- padding shorthand ----------------------------------------

    #[test]
    fn padding_one_value() {
        let result = expand("padding", px(8.0));
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("padding-top", px(8.0)));
        assert_eq!(result[1], decl("padding-right", px(8.0)));
        assert_eq!(result[2], decl("padding-bottom", px(8.0)));
        assert_eq!(result[3], decl("padding-left", px(8.0)));
    }

    #[test]
    fn padding_two_values() {
        let val = CssValue::Multiple(vec![px(4.0), px(8.0)]);
        let result = expand("padding", val);
        assert_eq!(result[0], decl("padding-top", px(4.0)));
        assert_eq!(result[1], decl("padding-right", px(8.0)));
        assert_eq!(result[2], decl("padding-bottom", px(4.0)));
        assert_eq!(result[3], decl("padding-left", px(8.0)));
    }

    #[test]
    fn padding_three_values() {
        let val = CssValue::Multiple(vec![px(1.0), px(2.0), px(3.0)]);
        let result = expand("padding", val);
        assert_eq!(result[0], decl("padding-top", px(1.0)));
        assert_eq!(result[1], decl("padding-right", px(2.0)));
        assert_eq!(result[2], decl("padding-bottom", px(3.0)));
        assert_eq!(result[3], decl("padding-left", px(2.0)));
    }

    #[test]
    fn padding_four_values() {
        let val = CssValue::Multiple(vec![px(1.0), px(2.0), px(3.0), px(4.0)]);
        let result = expand("padding", val);
        assert_eq!(result[0], decl("padding-top", px(1.0)));
        assert_eq!(result[1], decl("padding-right", px(2.0)));
        assert_eq!(result[2], decl("padding-bottom", px(3.0)));
        assert_eq!(result[3], decl("padding-left", px(4.0)));
    }

    #[test]
    fn padding_percentage_values() {
        let result = expand("padding", pct(50.0));
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("padding-top", pct(50.0)));
    }

    // -- border shorthand -----------------------------------------

    #[test]
    fn border_width_style_color() {
        let val = CssValue::Multiple(vec![
            px(1.0),
            kw("solid"),
            CssValue::Color(CssColor::new(255, 0, 0, 255)),
        ]);
        let result = expand("border", val);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("border-width", px(1.0)));
        assert_eq!(result[1], decl("border-style", kw("solid")));
        assert_eq!(
            result[2],
            decl(
                "border-color",
                CssValue::Color(CssColor::new(255, 0, 0, 255))
            )
        );
    }

    #[test]
    fn border_style_only() {
        let result = expand("border", kw("dashed"));
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("border-width", kw("medium")));
        assert_eq!(result[1], decl("border-style", kw("dashed")));
        assert_eq!(result[2], decl("border-color", kw("currentcolor")));
    }

    #[test]
    fn border_named_color() {
        let val = CssValue::Multiple(vec![px(2.0), kw("solid"), kw("red")]);
        let result = expand("border", val);
        assert_eq!(result[0], decl("border-width", px(2.0)));
        assert_eq!(result[1], decl("border-style", kw("solid")));
        // "red" should be resolved to CssValue::Color.
        assert!(
            matches!(result[2].value, CssValue::Color(_)),
            "expected Color, got {:?}",
            result[2].value
        );
    }

    #[test]
    fn border_defaults_when_empty_values() {
        // Single keyword that is neither a style nor a named color.
        let result = expand("border", kw("unknownstyle"));
        // Falls through to style (fallback behavior).
        assert_eq!(result[0].property, "border-width");
        assert_eq!(result[1].property, "border-style");
        assert_eq!(result[2].property, "border-color");
    }

    #[test]
    fn border_with_number_width() {
        let val = CssValue::Multiple(vec![num(1.0), kw("solid")]);
        let result = expand("border", val);
        assert_eq!(result[0], decl("border-width", num(1.0)));
        assert_eq!(result[1], decl("border-style", kw("solid")));
    }

    #[test]
    fn border_with_var() {
        let val = CssValue::Multiple(vec![
            px(1.0),
            kw("solid"),
            CssValue::Var("--border-clr".into(), None),
        ]);
        let result = expand("border", val);
        assert_eq!(
            result[2],
            decl("border-color", CssValue::Var("--border-clr".into(), None))
        );
    }

    // -- border-side shorthands -----------------------------------

    #[test]
    fn border_top_shorthand() {
        let val = CssValue::Multiple(vec![px(3.0), kw("dotted"), kw("blue")]);
        let result = expand("border-top", val);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].property, "border-top-width");
        assert_eq!(result[0].value, px(3.0));
        assert_eq!(result[1].property, "border-top-style");
        assert_eq!(result[1].value, kw("dotted"));
        assert_eq!(result[2].property, "border-top-color");
        assert!(matches!(result[2].value, CssValue::Color(_)));
    }

    #[test]
    fn border_left_shorthand_style_only() {
        let result = expand("border-left", kw("none"));
        assert_eq!(result[0].property, "border-left-width");
        assert_eq!(result[1].property, "border-left-style");
        assert_eq!(result[1].value, kw("none"));
        assert_eq!(result[2].property, "border-left-color");
    }

    // -- border-width shorthand -----------------------------------

    #[test]
    fn border_width_one_value() {
        let result = expand("border-width", px(2.0));
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("border-top-width", px(2.0)));
        assert_eq!(result[1], decl("border-right-width", px(2.0)));
        assert_eq!(result[2], decl("border-bottom-width", px(2.0)));
        assert_eq!(result[3], decl("border-left-width", px(2.0)));
    }

    #[test]
    fn border_width_two_values() {
        let val = CssValue::Multiple(vec![px(1.0), px(2.0)]);
        let result = expand("border-width", val);
        assert_eq!(result[0].value, px(1.0)); // top
        assert_eq!(result[1].value, px(2.0)); // right
        assert_eq!(result[2].value, px(1.0)); // bottom
        assert_eq!(result[3].value, px(2.0)); // left
    }

    #[test]
    fn border_width_three_values() {
        let val = CssValue::Multiple(vec![px(1.0), px(2.0), px(3.0)]);
        let result = expand("border-width", val);
        assert_eq!(result[0].value, px(1.0)); // top
        assert_eq!(result[1].value, px(2.0)); // right
        assert_eq!(result[2].value, px(3.0)); // bottom
        assert_eq!(result[3].value, px(2.0)); // left
    }

    #[test]
    fn border_width_four_values() {
        let val = CssValue::Multiple(vec![px(1.0), px(2.0), px(3.0), px(4.0)]);
        let result = expand("border-width", val);
        assert_eq!(result[0].value, px(1.0));
        assert_eq!(result[1].value, px(2.0));
        assert_eq!(result[2].value, px(3.0));
        assert_eq!(result[3].value, px(4.0));
    }

    // -- border-style shorthand -----------------------------------

    #[test]
    fn border_style_one_value() {
        let result = expand("border-style", kw("solid"));
        assert_eq!(result.len(), 4);
        for d in &result {
            assert_eq!(d.value, kw("solid"));
        }
        assert_eq!(result[0].property, "border-top-style");
        assert_eq!(result[3].property, "border-left-style");
    }

    #[test]
    fn border_style_two_values() {
        let val = CssValue::Multiple(vec![kw("solid"), kw("dashed")]);
        let result = expand("border-style", val);
        assert_eq!(result[0].value, kw("solid"));
        assert_eq!(result[1].value, kw("dashed"));
        assert_eq!(result[2].value, kw("solid"));
        assert_eq!(result[3].value, kw("dashed"));
    }

    // -- border-color shorthand -----------------------------------

    #[test]
    fn border_color_one_value() {
        let c = CssValue::Color(CssColor::new(0, 0, 0, 255));
        let result = expand("border-color", c.clone());
        assert_eq!(result.len(), 4);
        for d in &result {
            assert_eq!(d.value, c);
        }
        assert_eq!(result[0].property, "border-top-color");
        assert_eq!(result[3].property, "border-left-color");
    }

    #[test]
    fn border_color_two_values() {
        let a = CssValue::Color(CssColor::new(255, 0, 0, 255));
        let b = CssValue::Color(CssColor::new(0, 0, 255, 255));
        let val = CssValue::Multiple(vec![a.clone(), b.clone()]);
        let result = expand("border-color", val);
        assert_eq!(result[0].value, a); // top
        assert_eq!(result[1].value, b); // right
        assert_eq!(result[2].value, a); // bottom
        assert_eq!(result[3].value, b); // left
    }

    // -- background shorthand -------------------------------------

    #[test]
    fn background_color() {
        let c = CssValue::Color(CssColor::new(255, 255, 0, 255));
        let result = expand("background", c.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("background-color", c));
    }

    #[test]
    fn background_url() {
        let url = CssValue::Url("img.png".into());
        let result = expand("background", url.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("background-image", url));
    }

    #[test]
    fn background_named_color_keyword() {
        let result = expand("background", kw("red"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].property, "background-color");
        assert!(matches!(result[0].value, CssValue::Color(_)));
    }

    #[test]
    fn background_transparent() {
        let result = expand("background", kw("transparent"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].property, "background-color");
        assert_eq!(result[0].value, CssValue::Color(CssColor::new(0, 0, 0, 0)));
    }

    #[test]
    fn background_none() {
        let result = expand("background", kw("none"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].property, "background-color");
        assert_eq!(result[0].value, CssValue::Color(CssColor::new(0, 0, 0, 0)));
    }

    #[test]
    fn background_unknown_keyword_passthrough() {
        let result = expand("background", kw("fancy"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].property, "background");
        assert_eq!(result[0].value, kw("fancy"));
    }

    #[test]
    fn background_var() {
        let v = CssValue::Var("--bg".into(), None);
        let result = expand("background", v.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("background-color", v));
    }

    #[test]
    fn background_multiple_with_color_and_url() {
        let val = CssValue::Multiple(vec![
            CssValue::Color(CssColor::new(0, 0, 0, 255)),
            CssValue::Url("bg.png".into()),
        ]);
        let result = expand("background", val);
        assert_eq!(result.len(), 2);
        // Iteration order: color first, then url.
        assert_eq!(result[0].property, "background-color");
        assert_eq!(result[1].property, "background-image");
    }

    #[test]
    fn background_multiple_no_recognizable() {
        let val = CssValue::Multiple(vec![kw("no-repeat"), kw("center")]);
        let result = expand("background", val.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("background", val));
    }

    #[test]
    fn background_gradient() {
        use crate::css::values::LinearGradient;
        let grad = CssValue::Gradient(LinearGradient {
            direction: crate::css::values::GradientDirection::ToBottom,
            stops: vec![],
        });
        let result = expand("background", grad.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("background-image", grad));
    }

    // -- list-style shorthand -------------------------------------

    #[test]
    fn list_style_type_disc() {
        let result = expand("list-style", kw("disc"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-type", kw("disc")));
    }

    #[test]
    fn list_style_position_inside() {
        let result = expand("list-style", kw("inside"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-position", kw("inside")));
    }

    #[test]
    fn list_style_type_and_position() {
        let val = CssValue::Multiple(vec![kw("square"), kw("outside")]);
        let result = expand("list-style", val);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], decl("list-style-type", kw("square")));
        assert_eq!(result[1], decl("list-style-position", kw("outside")));
    }

    #[test]
    fn list_style_none() {
        let result = expand("list-style", kw("none"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-type", kw("none")));
    }

    #[test]
    fn list_style_unrecognized_falls_back() {
        // A keyword not in the recognized set produces the
        // fallback "none" for list-style-type.
        let result = expand("list-style", kw("unknown"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-type", kw("none")));
    }

    #[test]
    fn list_style_decimal() {
        let result = expand("list-style", kw("decimal"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-type", kw("decimal")));
    }

    #[test]
    fn list_style_upper_case_normalised() {
        let result = expand("list-style", kw("DISC"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("list-style-type", kw("disc")));
    }

    // -- flex shorthand -------------------------------------------

    #[test]
    fn flex_none() {
        let result = expand("flex", kw("none"));
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("flex-grow", num(0.0)));
        assert_eq!(result[1], decl("flex-shrink", num(0.0)));
        assert_eq!(result[2], decl("flex-basis", kw("auto")));
    }

    #[test]
    fn flex_single_number() {
        let result = expand("flex", num(2.0));
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("flex-grow", num(2.0)));
        assert_eq!(result[1], decl("flex-shrink", num(1.0)));
        assert_eq!(result[2], decl("flex-basis", px(0.0)));
    }

    #[test]
    fn flex_keyword_passthrough() {
        let result = expand("flex", kw("auto"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("flex", kw("auto")));
    }

    #[test]
    fn flex_multiple_passthrough() {
        let val = CssValue::Multiple(vec![num(1.0), num(0.0)]);
        let result = expand("flex", val.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("flex", val));
    }

    // -- font shorthand -------------------------------------------

    #[test]
    fn font_size_and_family() {
        let val = CssValue::Multiple(vec![px(16.0), kw("sans-serif")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], decl("font-size", px(16.0)));
        assert_eq!(result[1], decl("font-family", kw("sans-serif")));
    }

    #[test]
    fn font_bold_italic_size_family() {
        let val = CssValue::Multiple(vec![kw("bold"), kw("italic"), px(14.0), kw("monospace")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], decl("font-weight", kw("bold")));
        assert_eq!(result[1], decl("font-style", kw("italic")));
        assert_eq!(result[2], decl("font-size", px(14.0)));
        assert_eq!(result[3], decl("font-family", kw("monospace")));
    }

    #[test]
    fn font_weight_numeric() {
        let val = CssValue::Multiple(vec![num(700.0), px(12.0), kw("serif")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("font-weight", num(700.0)));
        assert_eq!(result[1], decl("font-size", px(12.0)));
        assert_eq!(result[2], decl("font-family", kw("serif")));
    }

    #[test]
    fn font_line_height_after_size() {
        let val = CssValue::Multiple(vec![px(16.0), num(1.5), kw("cursive")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("font-size", px(16.0)));
        assert_eq!(result[1], decl("line-height", num(1.5)));
        assert_eq!(result[2], decl("font-family", kw("cursive")));
    }

    #[test]
    fn font_percentage_size() {
        let val = CssValue::Multiple(vec![pct(120.0), kw("fantasy")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], decl("font-size", pct(120.0)));
        assert_eq!(result[1], decl("font-family", kw("fantasy")));
    }

    #[test]
    fn font_normal_skipped() {
        // "normal" is ambiguous and should be skipped.
        let val = CssValue::Multiple(vec![kw("normal"), px(14.0), kw("serif")]);
        let result = expand("font", val);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], decl("font-size", px(14.0)));
        assert_eq!(result[1], decl("font-family", kw("serif")));
    }

    #[test]
    fn font_unknown_single_passthrough() {
        let result = expand("font", kw("caption"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("font", kw("caption")));
    }

    #[test]
    fn font_oblique() {
        let val = CssValue::Multiple(vec![kw("oblique"), px(10.0), kw("monospace")]);
        let result = expand("font", val);
        assert!(
            result
                .iter()
                .any(|d| d.property == "font-style" && d.value == kw("oblique"))
        );
    }

    // -- overflow passthrough -------------------------------------

    #[test]
    fn overflow_passes_through() {
        let result = expand("overflow", kw("hidden"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("overflow", kw("hidden")));
    }

    // -- unknown property passthrough -----------------------------

    #[test]
    fn unknown_property_passes_through() {
        let result = expand("color", kw("blue"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], decl("color", kw("blue")));
    }

    // -- multiple declarations ------------------------------------

    #[test]
    fn multiple_shorthand_declarations() {
        let decls = vec![
            decl("margin", px(10.0)),
            decl("padding", px(5.0)),
            decl("color", kw("red")),
        ];
        let result = expand_shorthands(decls);
        // 4 margin + 4 padding + 1 color = 9
        assert_eq!(result.len(), 9);
        assert_eq!(result[0].property, "margin-top");
        assert_eq!(result[4].property, "padding-top");
        assert_eq!(result[8].property, "color");
    }

    // -- is_border_style ------------------------------------------

    #[test]
    fn border_style_recognition() {
        let styles = [
            "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset",
            "outset",
        ];
        for s in &styles {
            assert!(is_border_style(s), "{} should be a border style", s);
        }
        assert!(!is_border_style("fancy"));
        assert!(!is_border_style(""));
    }

    // -- gradient parsing -----------------------------------------

    #[test]
    fn gradient_to_right_two_colors() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Ident("to".into()),
            CssToken::Ident("right".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.direction, GradientDirection::ToRight);
        assert_eq!(grad.stops.len(), 2);
        assert!((grad.stops[0].position - 0.0).abs() < 0.001);
        assert!((grad.stops[1].position - 1.0).abs() < 0.001);
    }

    #[test]
    fn gradient_angle_deg() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Dimension(45.0, "deg".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.direction, GradientDirection::Angle(45.0));
    }

    #[test]
    fn gradient_angle_turn() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Dimension(0.25, "turn".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.direction, GradientDirection::Angle(90.0));
    }

    #[test]
    fn gradient_default_direction() {
        use crate::css::values::GradientDirection;

        // No direction specified -- first group is a color.
        let tokens = vec![
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.direction, GradientDirection::ToBottom);
    }

    #[test]
    fn gradient_with_percentage_stops_returns_none() {
        // Named colors with position tokens in the same group
        // cause try_parse_color to fail (it requires exactly
        // one non-whitespace token for ident colors). This
        // documents the current limitation.
        let tokens = vec![
            CssToken::Ident("to".into()),
            CssToken::Ident("top".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Percentage(25.0),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
            CssToken::Percentage(75.0),
        ];
        assert!(parse_linear_gradient(&tokens).is_none());
    }

    #[test]
    fn gradient_three_stops_evenly_spaced() {
        let tokens = vec![
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("green".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.stops.len(), 3);
        assert!((grad.stops[0].position - 0.0).abs() < 0.001);
        assert!((grad.stops[1].position - 0.5).abs() < 0.001);
        assert!((grad.stops[2].position - 1.0).abs() < 0.001);
    }

    #[test]
    fn gradient_strips_function_and_close_paren() {
        let tokens = vec![
            CssToken::Function("linear-gradient".into()),
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
            CssToken::CloseParen,
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.stops.len(), 2);
    }

    #[test]
    fn gradient_too_few_stops() {
        let tokens = vec![CssToken::Ident("red".into())];
        assert!(parse_linear_gradient(&tokens).is_none());
    }

    #[test]
    fn gradient_whitespace_ignored() {
        let tokens = vec![
            CssToken::Whitespace,
            CssToken::Ident("red".into()),
            CssToken::Whitespace,
            CssToken::Comma,
            CssToken::Whitespace,
            CssToken::Ident("blue".into()),
            CssToken::Whitespace,
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.stops.len(), 2);
    }

    #[test]
    fn gradient_to_left() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Ident("to".into()),
            CssToken::Ident("left".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        assert_eq!(grad.direction, GradientDirection::ToLeft);
    }

    #[test]
    fn gradient_rad_unit() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Dimension(std::f32::consts::PI, "rad".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        if let GradientDirection::Angle(a) = grad.direction {
            assert!(
                (a - 180.0).abs() < 0.1,
                "pi rad should be ~180deg, got {}",
                a
            );
        } else {
            panic!("expected Angle direction");
        }
    }

    #[test]
    fn gradient_grad_unit() {
        use crate::css::values::GradientDirection;

        let tokens = vec![
            CssToken::Dimension(100.0, "grad".into()),
            CssToken::Comma,
            CssToken::Ident("red".into()),
            CssToken::Comma,
            CssToken::Ident("blue".into()),
        ];
        let grad = parse_linear_gradient(&tokens).unwrap();
        if let GradientDirection::Angle(a) = grad.direction {
            assert!((a - 90.0).abs() < 0.1, "100grad should be 90deg, got {}", a);
        } else {
            panic!("expected Angle direction");
        }
    }

    // -- edge cases -----------------------------------------------

    #[test]
    fn empty_declarations_vec() {
        let result = expand_shorthands(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn box_shorthand_with_em_units() {
        let em = CssValue::Length(2.0, LengthUnit::Em);
        let result = expand("margin", em.clone());
        assert_eq!(result.len(), 4);
        for d in &result {
            assert_eq!(d.value, em);
        }
    }

    #[test]
    fn border_all_styles_recognized() {
        let styles = [
            "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset",
            "outset",
        ];
        for s in &styles {
            let result = expand("border", kw(s));
            assert_eq!(
                result[1].property, "border-style",
                "failed for style '{}'",
                s
            );
            assert_eq!(result[1].value, kw(s), "style '{}' not recognized", s);
        }
    }

    #[test]
    fn border_width_keywords() {
        let result = expand("border-width", kw("thin"));
        assert_eq!(result.len(), 4);
        for d in &result {
            assert_eq!(d.value, kw("thin"));
        }
    }

    #[test]
    fn flex_zero() {
        let result = expand("flex", num(0.0));
        // 0.0 is a Number but the check is > 0.0 in font,
        // for flex it's just a number so it produces grow=0.
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], decl("flex-grow", num(0.0)));
        assert_eq!(result[1], decl("flex-shrink", num(1.0)));
        assert_eq!(result[2], decl("flex-basis", px(0.0)));
    }
}

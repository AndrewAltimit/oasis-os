//! Border shorthand expansion.

use crate::css::helpers::named_color;
use crate::css::parser::{CssValue, Declaration};

pub(super) fn expand_border(value: &CssValue, important: bool) -> Vec<Declaration> {
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
pub(super) fn expand_border_side(
    property: &str,
    value: &CssValue,
    important: bool,
) -> Vec<Declaration> {
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

pub(super) fn is_border_style(s: &str) -> bool {
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

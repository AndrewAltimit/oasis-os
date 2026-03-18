//! Border shorthand expansion.

use crate::css::helpers::named_color;
use crate::css::parser::{CssValue, Declaration, PropertyId};

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
            property_id: PropertyId::from_name("border-width"),
        },
        Declaration {
            property: "border-style".into(),
            value: style,
            important,
            property_id: PropertyId::from_name("border-style"),
        },
        Declaration {
            property: "border-color".into(),
            value: color,
            important,
            property_id: PropertyId::from_name("border-color"),
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
            property_id: PropertyId::Other,
        },
        Declaration {
            property: format!("{property}-style"),
            value: style,
            important,
            property_id: PropertyId::Other,
        },
        Declaration {
            property: format!("{property}-color"),
            value: color,
            important,
            property_id: PropertyId::Other,
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

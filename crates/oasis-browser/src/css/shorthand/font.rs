//! Font shorthand expansion.

use crate::css::parser::{CssValue, Declaration, PropertyId};

pub(super) fn expand_font(value: &CssValue, important: bool) -> Vec<Declaration> {
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
                    property_id: PropertyId::from_name("font-size"),
                });
            },
            CssValue::Number(n) if *n > 0.0 => {
                // Could be line-height if we already have font-size, or font-weight.
                if result.iter().any(|d| d.property == "font-size") {
                    result.push(Declaration {
                        property: "line-height".into(),
                        value: v.clone(),
                        important,
                        property_id: PropertyId::from_name("line-height"),
                    });
                } else if *n >= 100.0 {
                    result.push(Declaration {
                        property: "font-weight".into(),
                        value: v.clone(),
                        important,
                        property_id: PropertyId::from_name("font-weight"),
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
                            property_id: PropertyId::from_name("font-weight"),
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
                            property_id: PropertyId::from_name("font-style"),
                        });
                    },
                    "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" => {
                        result.push(Declaration {
                            property: "font-family".into(),
                            value: CssValue::Keyword(kw_lower),
                            important,
                            property_id: PropertyId::from_name("font-family"),
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
            property_id: PropertyId::from_name("font"),
        });
    }

    result
}

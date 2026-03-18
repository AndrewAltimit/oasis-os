//! List-style shorthand expansion.

use crate::css::parser::{CssValue, Declaration, PropertyId};

pub(super) fn expand_list_style(value: &CssValue, important: bool) -> Vec<Declaration> {
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
                        property_id: PropertyId::from_name("list-style-type"),
                    });
                },
                "inside" | "outside" => {
                    result.push(Declaration {
                        property: "list-style-position".into(),
                        value: CssValue::Keyword(kw_lower),
                        important,
                        property_id: PropertyId::from_name("list-style-position"),
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
            property_id: PropertyId::from_name("list-style-type"),
        });
    }

    result
}

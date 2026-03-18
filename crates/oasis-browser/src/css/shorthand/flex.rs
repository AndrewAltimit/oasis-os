//! Flex shorthand expansion.

use crate::css::parser::{CssValue, Declaration, LengthUnit, PropertyId};

pub(super) fn expand_flex(value: &CssValue, important: bool) -> Vec<Declaration> {
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
                        property_id: PropertyId::from_name("flex-grow"),
                    },
                    Declaration {
                        property: "flex-shrink".into(),
                        value: CssValue::Number(0.0),
                        important,
                        property_id: PropertyId::from_name("flex-shrink"),
                    },
                    Declaration {
                        property: "flex-basis".into(),
                        value: CssValue::Keyword("auto".into()),
                        important,
                        property_id: PropertyId::from_name("flex-basis"),
                    },
                ]
            },
            CssValue::Number(n) => {
                vec![
                    Declaration {
                        property: "flex-grow".into(),
                        value: CssValue::Number(*n),
                        important,
                        property_id: PropertyId::from_name("flex-grow"),
                    },
                    Declaration {
                        property: "flex-shrink".into(),
                        value: CssValue::Number(1.0),
                        important,
                        property_id: PropertyId::from_name("flex-shrink"),
                    },
                    Declaration {
                        property: "flex-basis".into(),
                        value: CssValue::Length(0.0, LengthUnit::Px),
                        important,
                        property_id: PropertyId::from_name("flex-basis"),
                    },
                ]
            },
            _ => vec![Declaration {
                property: "flex".into(),
                value: value.clone(),
                important,
                property_id: PropertyId::from_name("flex"),
            }],
        },
        _ => vec![Declaration {
            property: "flex".into(),
            value: value.clone(),
            important,
            property_id: PropertyId::from_name("flex"),
        }],
    }
}

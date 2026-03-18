//! Background shorthand expansion.

use crate::css::helpers::named_color;
use crate::css::parser::{CssColor, CssValue, Declaration, PropertyId};

pub(super) fn expand_background(value: &CssValue, important: bool) -> Vec<Declaration> {
    // Simple heuristic: if the value is a color, set background-color.
    // If the value is a url(), set background-image.
    match value {
        CssValue::Url(_) | CssValue::Gradient(_) | CssValue::RadialGradient(_) => {
            vec![Declaration {
                property: "background-image".into(),
                value: value.clone(),
                important,
                property_id: PropertyId::from_name("background-image"),
            }]
        },
        CssValue::Color(_) | CssValue::Var(..) => {
            vec![Declaration {
                property: "background-color".into(),
                value: value.clone(),
                important,
                property_id: PropertyId::from_name("background-color"),
            }]
        },
        CssValue::Multiple(vs) => {
            let mut decls = Vec::new();
            for v in vs {
                if matches!(
                    v,
                    CssValue::Url(_) | CssValue::Gradient(_) | CssValue::RadialGradient(_)
                ) {
                    decls.push(Declaration {
                        property: "background-image".into(),
                        value: v.clone(),
                        important,
                        property_id: PropertyId::from_name("background-image"),
                    });
                } else if matches!(v, CssValue::Color(_) | CssValue::Var(..)) {
                    decls.push(Declaration {
                        property: "background-color".into(),
                        value: v.clone(),
                        important,
                        property_id: PropertyId::from_name("background-color"),
                    });
                }
            }
            if decls.is_empty() {
                decls.push(Declaration {
                    property: "background".into(),
                    value: value.clone(),
                    important,
                    property_id: PropertyId::from_name("background"),
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
                    property_id: PropertyId::from_name("background-color"),
                }]
            } else if let Some(c) = named_color(name) {
                vec![Declaration {
                    property: "background-color".into(),
                    value: CssValue::Color(c),
                    important,
                    property_id: PropertyId::from_name("background-color"),
                }]
            } else {
                vec![Declaration {
                    property: "background".into(),
                    value: value.clone(),
                    important,
                    property_id: PropertyId::from_name("background"),
                }]
            }
        },
        _ => {
            vec![Declaration {
                property: "background".into(),
                value: value.clone(),
                important,
                property_id: PropertyId::from_name("background"),
            }]
        },
    }
}

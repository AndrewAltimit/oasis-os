//! Box-model shorthand expansion (margin, padding).

use crate::css::parser::{CssValue, Declaration, PropertyId};

pub(super) fn expand_box_shorthand(
    prefix: &str,
    value: &CssValue,
    important: bool,
) -> Vec<Declaration> {
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

    let top_prop = format!("{}-top", prefix);
    let right_prop = format!("{}-right", prefix);
    let bottom_prop = format!("{}-bottom", prefix);
    let left_prop = format!("{}-left", prefix);
    vec![
        Declaration {
            property_id: PropertyId::from_name(&top_prop),
            property: top_prop,
            value: top,
            important,
        },
        Declaration {
            property_id: PropertyId::from_name(&right_prop),
            property: right_prop,
            value: right,
            important,
        },
        Declaration {
            property_id: PropertyId::from_name(&bottom_prop),
            property: bottom_prop,
            value: bottom,
            important,
        },
        Declaration {
            property_id: PropertyId::from_name(&left_prop),
            property: left_prop,
            value: left,
            important,
        },
    ]
}

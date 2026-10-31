//! Four-side shorthand expansion: `margin`, `padding`, `border-width`,
//! `border-style`, `border-color`.

use crate::parser::{CssValue, Declaration, PropertyId};

/// The `[top, right, bottom, left]` longhands of four-side shorthand
/// `property`, or `None` if it is not one.
fn side_longhands(property: &str) -> Option<[&'static str; 4]> {
    Some(match property {
        "margin" => ["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "padding" => [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        "border-width" => [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
        "border-style" => [
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ],
        "border-color" => [
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
        _ => return None,
    })
}

/// Expand four-side shorthand `property` (1-4 values: all; vertical
/// horizontal; top horizontal bottom; top right bottom left). `None` if
/// `property` is not a four-side shorthand. Any other value count is
/// invalid and expands to nothing (the declaration is dropped).
pub(super) fn expand_four_sides(
    property: &str,
    value: &CssValue,
    important: bool,
) -> Option<Vec<Declaration>> {
    let longhands = side_longhands(property)?;
    let values: &[CssValue] = match value {
        CssValue::Multiple(vs) => vs,
        other => std::slice::from_ref(other),
    };
    let [top, right, bottom, left] = match values {
        [a] => [a, a, a, a],
        [tb, lr] => [tb, lr, tb, lr],
        [t, lr, b] => [t, lr, b, lr],
        [t, r, b, l] => [t, r, b, l],
        _ => return Some(Vec::new()),
    };
    Some(
        longhands
            .into_iter()
            .zip([top, right, bottom, left])
            .map(|(prop, value)| Declaration {
                property: prop.into(),
                value: value.clone(),
                important,
                property_id: PropertyId::from_name(prop),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::LengthUnit;

    fn px(n: f32) -> CssValue {
        CssValue::Length(n, LengthUnit::Px)
    }

    fn values(decls: &[Declaration]) -> Vec<(&str, &CssValue)> {
        decls
            .iter()
            .map(|d| (d.property.as_str(), &d.value))
            .collect()
    }

    #[test]
    fn three_values_mirror_right_to_left() {
        let v = CssValue::Multiple(vec![px(1.0), px(2.0), px(3.0)]);
        let out = expand_four_sides("border-width", &v, false).expect("shorthand");
        assert_eq!(
            values(&out),
            [
                ("border-top-width", &px(1.0)),
                ("border-right-width", &px(2.0)),
                ("border-bottom-width", &px(3.0)),
                ("border-left-width", &px(2.0)),
            ]
        );
    }

    #[test]
    fn more_than_four_values_is_invalid() {
        let v = CssValue::Multiple((0..5).map(|i| px(i as f32)).collect());
        assert_eq!(expand_four_sides("margin", &v, false), Some(Vec::new()));
        assert_eq!(
            expand_four_sides("margin", &CssValue::Multiple(vec![]), false),
            Some(vec![])
        );
        assert!(expand_four_sides("margin-top", &px(1.0), false).is_none());
    }
}

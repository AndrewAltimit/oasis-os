//! Form-control painting tests.

use super::*;
use crate::css::values::ComputedStyle;
use crate::layout::box_model::{BoxType, Dimensions, LayoutBox, ReplacedContent};

fn make_text_input_box(value: &str, focused: bool, caret: Option<Color>) -> (LayoutBox, bool) {
    let style = ComputedStyle {
        font_size: 12.0,
        color: Color::rgb(0, 0, 0),
        caret_color: caret,
        ..Default::default()
    };
    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::TextInput {
            value: value.to_string(),
            placeholder: String::new(),
            size: 20,
            is_password: false,
        }),
        style,
        Some(7),
    );
    lb.dimensions = Dimensions {
        content: crate::layout::box_model::Rect::new(0.0, 0.0, 200.0, 20.0),
        ..Dimensions::default()
    };
    (lb, focused)
}

fn record_input_items(value: &str, focused: bool, caret: Option<Color>) -> Vec<DisplayItem> {
    let (lb, focused) = make_text_input_box(value, focused, caret);
    let mut dl = DisplayList::new();
    record_text_input(&lb, &mut dl, 0, 0, value, "", false, focused);
    dl.items().to_vec()
}

fn count_caret_with_color(items: &[DisplayItem], color: Color) -> usize {
    items
        .iter()
        .filter(|it| {
            matches!(
                it,
                DisplayItem::FillRect { w: 1, color: c, .. } if *c == color
            )
        })
        .count()
}

#[test]
fn caret_drawn_when_focused() {
    let items = record_input_items("abc", true, None);
    // Default caret-color falls back to text color (black).
    assert!(count_caret_with_color(&items, Color::rgb(0, 0, 0)) >= 1);
}

#[test]
fn caret_not_drawn_when_unfocused() {
    let items = record_input_items("abc", false, None);
    assert_eq!(count_caret_with_color(&items, Color::rgb(0, 0, 0)), 0);
}

#[test]
fn caret_color_overrides_text_color() {
    let blue = Color::rgb(0, 0, 255);
    let items = record_input_items("abc", true, Some(blue));
    assert!(count_caret_with_color(&items, blue) >= 1);
    // No black caret in this case (text color is still black, but
    // the 1-pixel caret rect should be blue).
    assert_eq!(count_caret_with_color(&items, Color::rgb(0, 0, 0)), 0);
}

#[test]
fn checkbox_uses_accent_color_when_checked() {
    let mut style = ComputedStyle::default();
    let pink = Color::rgb(255, 0, 128);
    style.accent_color = Some(pink);
    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::Checkbox { checked: true }),
        style,
        Some(11),
    );
    lb.dimensions = Dimensions {
        content: crate::layout::box_model::Rect::new(0.0, 0.0, 13.0, 13.0),
        ..Dimensions::default()
    };
    let mut dl = DisplayList::new();
    record_checkbox(&lb, &mut dl, 0, 0, true);
    // The first FillRect is the background and should now be pink.
    let first_bg = dl.items().iter().find_map(|it| match it {
        DisplayItem::FillRect { color, w, h, .. } if *w == 13 && *h == 13 => Some(*color),
        _ => None,
    });
    assert_eq!(first_bg, Some(pink));
}

#[test]
fn radio_uses_accent_color_for_dot() {
    let mut style = ComputedStyle::default();
    let teal = Color::rgb(0, 200, 200);
    style.accent_color = Some(teal);
    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::RadioButton { checked: true }),
        style,
        Some(12),
    );
    lb.dimensions = Dimensions {
        content: crate::layout::box_model::Rect::new(0.0, 0.0, 13.0, 13.0),
        ..Dimensions::default()
    };
    let mut dl = DisplayList::new();
    record_radio_button(&lb, &mut dl, 0, 0, true);
    // The dot is the rect at the inset; ensure at least one rect
    // uses the teal accent.
    let any_teal = dl
        .items()
        .iter()
        .any(|it| matches!(it, DisplayItem::FillRect { color, .. } if *color == teal));
    assert!(any_teal, "radio dot should use accent-color");
}

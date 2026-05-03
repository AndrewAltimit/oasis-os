//! Tests for `apply_declaration` and the surrounding helper layer.
#![allow(clippy::unwrap_used)]

use oasis_types::backend::Color;

use super::super::computed::ComputedStyle;
use super::super::types::{
    BackfaceVisibility, BackgroundBox, BlendMode, ContentVisibility, FontStretch, Hyphens,
    ImageRendering, JustifySelf, OverscrollBehavior, ScrollBehavior, TextAlignLast, TextDirection,
    TextShadow, TimingFunction, TransformStyle,
};
use super::parsers::parse_time;
use crate::css::parser::{CssColor, CssValue, LengthUnit};
use crate::css::values::types::ROOT_FONT_SIZE;
use crate::css::values::{
    BackgroundImage, BorderStyle, Dimension, Display, FontWeight, GradientDirection, GradientStop,
    LinearGradient,
};

#[test]
fn apply_keyword_display() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("display", &CssValue::Keyword("block".into()), 16.0);
    assert_eq!(s.display, Display::Block);
}

#[test]
fn apply_px_margin() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("margin", &CssValue::Length(10.0, LengthUnit::Px), 16.0);
    assert!((s.margin_top - 10.0).abs() < f32::EPSILON);
    assert!((s.margin_right - 10.0).abs() < f32::EPSILON);
    assert!((s.margin_bottom - 10.0).abs() < f32::EPSILON);
    assert!((s.margin_left - 10.0).abs() < f32::EPSILON);
}

#[test]
fn apply_em_padding() {
    let mut s = ComputedStyle::default();
    // 1.5em with parent font-size 20px = 30px.
    s.apply_declaration("padding-top", &CssValue::Length(1.5, LengthUnit::Em), 20.0);
    assert!((s.padding_top - 30.0).abs() < f32::EPSILON);
}

#[test]
fn apply_color_keyword() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("color", &CssValue::Keyword("red".into()), 16.0);
    assert_eq!(s.color, Color::rgb(255, 0, 0));
}

#[test]
fn apply_color_value() {
    let mut s = ComputedStyle::default();
    let c = CssColor {
        r: 10,
        g: 20,
        b: 30,
        a: 255,
    };
    s.apply_declaration("color", &CssValue::Color(c), 16.0);
    assert_eq!(s.color, Color::rgb(10, 20, 30));
}

#[test]
fn apply_font_size_updates_line_height() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("font-size", &CssValue::Length(20.0, LengthUnit::Px), 16.0);
    assert!((s.font_size - 20.0).abs() < f32::EPSILON);
    // Line height should be recomputed: 20 * 1.5 = 30.
    assert!((s.line_height - 30.0).abs() < 0.01);
}

#[test]
fn apply_font_weight_bold_keyword() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("font-weight", &CssValue::Keyword("bold".into()), 16.0);
    assert_eq!(s.font_weight, FontWeight::BOLD);
}

#[test]
fn apply_font_weight_bold_number() {
    // The CSS parser normalises "bold" to Number(700.0).
    let mut s = ComputedStyle::default();
    s.apply_declaration("font-weight", &CssValue::Number(700.0), 16.0);
    assert_eq!(s.font_weight, FontWeight::BOLD);
}

#[test]
fn apply_font_weight_normal_number() {
    let mut s = ComputedStyle::default();
    s.font_weight = FontWeight::BOLD;
    s.apply_declaration("font-weight", &CssValue::Number(400.0), 16.0);
    assert_eq!(s.font_weight, FontWeight::NORMAL);
}

#[test]
fn apply_font_weight_numeric() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("font-weight", &CssValue::Number(300.0), 16.0);
    assert_eq!(s.font_weight, FontWeight(300));
    assert!(!s.font_weight.is_bold());
    s.apply_declaration("font-weight", &CssValue::Number(600.0), 16.0);
    assert_eq!(s.font_weight, FontWeight(600));
    assert!(s.font_weight.is_bold());
}

#[test]
fn apply_dimension_percent() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("width", &CssValue::Percentage(50.0), 16.0);
    assert_eq!(s.width, Dimension::Percent(50.0));
}

#[test]
fn apply_dimension_auto() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("width", &CssValue::Keyword("auto".into()), 16.0);
    assert_eq!(s.width, Dimension::Auto);
}

#[test]
fn apply_border_shorthand() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("border-style", &CssValue::Keyword("solid".into()), 16.0);
    assert_eq!(s.border_top_style, BorderStyle::Solid);
    assert_eq!(s.border_right_style, BorderStyle::Solid);
    assert_eq!(s.border_bottom_style, BorderStyle::Solid);
    assert_eq!(s.border_left_style, BorderStyle::Solid);
}

#[test]
fn apply_background_color() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("background-color", &CssValue::Keyword("white".into()), 16.0);
    assert_eq!(s.background_color, Color::WHITE);
}

#[test]
fn apply_unknown_property_is_noop() {
    let mut s = ComputedStyle::default();
    let before = s.clone();
    s.apply_declaration("unknown-prop", &CssValue::Keyword("something".into()), 16.0);
    // Nothing should have changed.
    assert_eq!(s.display, before.display);
    assert_eq!(s.color, before.color);
}

#[test]
fn resolve_font_size_keywords() {
    let mut s = ComputedStyle::default();
    let parent = ROOT_FONT_SIZE;
    s.apply_declaration("font-size", &CssValue::Keyword("small".into()), parent);
    // Absolute-size keywords anchor to the CSS "medium" default
    // (16 px per CSS 2.1 §15.7), not the PSP-tuned `ROOT_FONT_SIZE`.
    // Changed because old.reddit leans on `small`/`x-small` for its
    // taglines and buttons and the 8px-anchored resolution made
    // links unreadable/unclickable on desktop viewports.
    let expected_small = 16.0_f32 * 0.8125;
    assert!((s.font_size - expected_small).abs() < f32::EPSILON);

    s.apply_declaration("font-size", &CssValue::Keyword("larger".into()), parent);
    // `larger` still scales against the parent computed size, not
    // the absolute-size anchor.
    let expected_larger = parent * 1.2;
    assert!((s.font_size - expected_larger).abs() < 0.01);
}

#[test]
fn resolve_line_height_number_multiplier() {
    let mut s = ComputedStyle::default();
    s.font_size = 20.0;
    s.apply_declaration("line-height", &CssValue::Number(1.5), 16.0);
    // 1.5 * 20.0 = 30.0
    assert!((s.line_height - 30.0).abs() < f32::EPSILON);
}

#[test]
fn test_margin_auto_vertical_flags() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("margin-top", &CssValue::Keyword("auto".into()), 16.0);
    assert!(s.margin_top_auto, "margin-top: auto should set flag");
    assert_eq!(s.margin_top, 0.0);

    s.apply_declaration("margin-bottom", &CssValue::Keyword("auto".into()), 16.0);
    assert!(s.margin_bottom_auto, "margin-bottom: auto should set flag");
    assert_eq!(s.margin_bottom, 0.0);
}

#[test]
fn test_margin_shorthand_preserves_auto() {
    use crate::css::parser::LengthUnit;

    let mut s = ComputedStyle::default();
    // margin: 0 auto => top/bottom=0, left/right=auto
    // The shorthand is expanded by the parser, but here we test
    // individual property application after expansion.
    s.apply_declaration("margin-top", &CssValue::Length(0.0, LengthUnit::Px), 16.0);
    s.apply_declaration("margin-right", &CssValue::Keyword("auto".into()), 16.0);
    s.apply_declaration(
        "margin-bottom",
        &CssValue::Length(0.0, LengthUnit::Px),
        16.0,
    );
    s.apply_declaration("margin-left", &CssValue::Keyword("auto".into()), 16.0);

    assert!(s.margin_left_auto);
    assert!(s.margin_right_auto);
    assert!(!s.margin_top_auto);
    assert!(!s.margin_bottom_auto);
}

#[test]
fn test_currentcolor_resolves_to_element_color() {
    let mut s = ComputedStyle::default();
    s.color = Color::rgb(255, 0, 0);
    s.apply_declaration(
        "border-top-color",
        &CssValue::Keyword("currentcolor".into()),
        16.0,
    );
    assert_eq!(
        s.border_top_color,
        Color::rgb(255, 0, 0),
        "currentcolor should resolve to element's color",
    );
}

#[test]
fn text_shadow_parsed() {
    let mut s = ComputedStyle::default();
    let value = CssValue::Multiple(vec![
        CssValue::Length(2.0, LengthUnit::Px),
        CssValue::Length(3.0, LengthUnit::Px),
        CssValue::Length(1.0, LengthUnit::Px),
        CssValue::Color(CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }),
    ]);
    s.apply_declaration("text-shadow", &value, 16.0);
    let ts = s.text_shadow.expect("should parse text-shadow");
    assert_eq!(ts.offset_x, 2.0);
    assert_eq!(ts.offset_y, 3.0);
    assert_eq!(ts.blur, 1.0);
    assert_eq!(ts.color, Color::rgba(0, 0, 0, 255));
}

#[test]
fn text_shadow_none() {
    let mut s = ComputedStyle::default();
    s.text_shadow = Some(TextShadow {
        offset_x: 1.0,
        offset_y: 1.0,
        blur: 0.0,
        color: Color::rgb(0, 0, 0),
    });
    let value = CssValue::Keyword("none".into());
    s.apply_declaration("text-shadow", &value, 16.0);
    assert!(s.text_shadow.is_none());
}

#[test]
fn multi_layer_background_image_takes_first_layer() {
    // `background-image: url(a), url(b)` parses as
    // `CssValue::Multiple([Url(a), Url(b)])`. The engine only
    // supports single-layer semantics, so the first layer should
    // win instead of the whole declaration being dropped.
    let mut s = ComputedStyle::default();
    let value = CssValue::Multiple(vec![
        CssValue::Url("a.png".into()),
        CssValue::Url("b.png".into()),
    ]);
    s.apply_declaration("background-image", &value, 16.0);
    assert_eq!(s.background_image, BackgroundImage::Url("a.png".into()));
}

#[test]
fn multi_layer_mask_image_takes_first_layer() {
    // Same behaviour for mask-image: without the `Multiple` arm
    // the fallthrough left `mask_image = None`, silently
    // removing the mask on any page using the multi-layer form.
    let mut s = ComputedStyle::default();
    let value = CssValue::Multiple(vec![
        CssValue::Url("mask-a.png".into()),
        CssValue::Url("mask-b.png".into()),
    ]);
    s.apply_declaration("mask-image", &value, 16.0);
    assert_eq!(s.mask_image, BackgroundImage::Url("mask-a.png".into()));
}

#[test]
fn gradient_background_image_applied() {
    let mut s = ComputedStyle::default();
    let grad = LinearGradient {
        direction: GradientDirection::ToRight,
        stops: vec![
            GradientStop {
                color: Color::rgb(255, 0, 0),
                position: 0.0,
            },
            GradientStop {
                color: Color::rgb(0, 0, 255),
                position: 1.0,
            },
        ],
        repeating: false,
    };
    let value = CssValue::Gradient(grad.clone());
    s.apply_declaration("background-image", &value, 16.0);
    assert_eq!(s.background_image, BackgroundImage::Gradient(grad));
}

mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// apply_declaration with unknown property is a no-op.
        #[test]
        fn apply_unknown_property_noop(
            prop_name in "[a-z\\-]{1,20}",
        ) {
            // Filter out known properties.
            if matches!(
                prop_name.as_str(),
                "display" | "color" | "margin" | "padding"
                | "width" | "height" | "font-size"
                | "background-color" | "background"
                | "border-width" | "border-style"
                | "border-color" | "overflow" | "position"
                | "float" | "clear" | "visibility"
                | "text-align" | "text-decoration"
                | "text-indent" | "text-transform"
                | "white-space" | "text-wrap" | "line-height"
                | "letter-spacing" | "word-spacing"
                | "font-weight" | "font-style" | "font-family"
                | "list-style-type" | "list-style-position"
                | "border-collapse" | "border-spacing"
                | "z-index" | "flex-direction" | "flex-wrap"
                | "justify-content" | "align-items" | "align-content"
                | "align-self" | "order"
                | "flex-grow" | "flex-shrink" | "flex-basis"
                | "gap" | "row-gap" | "column-gap"
                | "grid-template-columns" | "grid-template-rows"
                | "grid-column" | "grid-column-start" | "grid-column-end"
                | "grid-row" | "grid-row-start" | "grid-row-end"
                | "grid-gap" | "grid-row-gap" | "grid-column-gap"
                | "grid-auto-rows" | "grid-auto-columns"
                | "top" | "right" | "bottom" | "left"
                | "max-width" | "min-width"
                | "max-height" | "min-height"
                | "transform-origin" | "filter"
                | "counter-reset" | "counter-increment"
                | "will-change" | "tab-size" | "column-count" | "column-width"
                | "columns" | "grid-auto-flow" | "grid-template-areas"
                | "grid-area" | "table-layout"
            ) {
                return Ok(());
            }
            let mut s = ComputedStyle::default();
            let before_color = s.color;
            s.apply_declaration(
                &prop_name,
                &CssValue::Keyword("x".into()),
                16.0,
            );
            prop_assert_eq!(s.color, before_color);
        }
    }
}

// -- Transition parsing tests ----------------------------------------

#[test]
fn parse_transition_all_ease() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transition",
        &CssValue::String("all 0.3s ease".into()),
        16.0,
    );
    assert_eq!(s.transitions.len(), 1);
    let t = &s.transitions[0];
    assert_eq!(t.property, "all");
    assert!((t.duration_ms - 300.0).abs() < 0.1);
    assert_eq!(t.timing, TimingFunction::Ease);
    assert!((t.delay_ms).abs() < f32::EPSILON);
}

#[test]
fn parse_transition_ms_with_delay() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transition",
        &CssValue::String("color 200ms linear 50ms".into()),
        16.0,
    );
    assert_eq!(s.transitions.len(), 1);
    let t = &s.transitions[0];
    assert_eq!(t.property, "color");
    assert!((t.duration_ms - 200.0).abs() < 0.1);
    assert_eq!(t.timing, TimingFunction::Linear);
    assert!((t.delay_ms - 50.0).abs() < 0.1);
}

#[test]
fn parse_transition_ease_in_out() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transition",
        &CssValue::String("opacity 1s ease-in-out".into()),
        16.0,
    );
    assert_eq!(s.transitions.len(), 1);
    let t = &s.transitions[0];
    assert_eq!(t.property, "opacity");
    assert!((t.duration_ms - 1000.0).abs() < 0.1);
    assert_eq!(t.timing, TimingFunction::EaseInOut);
}

#[test]
fn parse_time_seconds() {
    assert!((parse_time("0.3s").unwrap() - 300.0).abs() < 0.1);
    assert!((parse_time("1s").unwrap() - 1000.0).abs() < 0.1);
}

#[test]
fn parse_time_milliseconds() {
    assert!((parse_time("200ms").unwrap() - 200.0).abs() < 0.1);
    assert!((parse_time("50ms").unwrap() - 50.0).abs() < 0.1);
}

// -- Extended property coverage tests --------------------------------

#[test]
fn parse_scroll_behavior_smooth() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("scroll-behavior", &CssValue::Keyword("smooth".into()), 16.0);
    assert_eq!(s.scroll_behavior, ScrollBehavior::Smooth);
}

#[test]
fn parse_clip_path_inset_four_values() {
    use super::super::types::{ClipLength, ClipPath};
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "clip-path",
        &CssValue::Keyword("inset(10px 20px 30px 40px)".into()),
        16.0,
    );
    match s.clip_path {
        Some(ClipPath::Inset {
            top,
            right,
            bottom,
            left,
        }) => {
            assert_eq!(top, ClipLength::Px(10.0));
            assert_eq!(right, ClipLength::Px(20.0));
            assert_eq!(bottom, ClipLength::Px(30.0));
            assert_eq!(left, ClipLength::Px(40.0));
        },
        other => panic!("expected Inset, got {other:?}"),
    }
}

#[test]
fn parse_clip_path_inset_shorthand_one_value() {
    use super::super::types::{ClipLength, ClipPath};
    let mut s = ComputedStyle::default();
    s.apply_declaration("clip-path", &CssValue::Keyword("inset(5%)".into()), 16.0);
    match s.clip_path {
        Some(ClipPath::Inset {
            top,
            right,
            bottom,
            left,
        }) => {
            assert_eq!(top, ClipLength::Frac(0.05));
            assert_eq!(right, ClipLength::Frac(0.05));
            assert_eq!(bottom, ClipLength::Frac(0.05));
            assert_eq!(left, ClipLength::Frac(0.05));
        },
        other => panic!("expected Inset, got {other:?}"),
    }
}

#[test]
fn parse_clip_path_circle_with_at() {
    use super::super::types::{ClipLength, ClipPath};
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "clip-path",
        &CssValue::Keyword("circle(50% at 25% 75%)".into()),
        16.0,
    );
    match s.clip_path {
        Some(ClipPath::Circle { cx, cy, r }) => {
            assert_eq!(r, ClipLength::Frac(0.5));
            assert_eq!(cx, ClipLength::Frac(0.25));
            assert_eq!(cy, ClipLength::Frac(0.75));
        },
        other => panic!("expected Circle, got {other:?}"),
    }
}

#[test]
fn parse_clip_path_circle_single_coordinate_at() {
    use super::super::types::{ClipLength, ClipPath};
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "clip-path",
        &CssValue::Keyword("circle(50% at 25%)".into()),
        16.0,
    );
    match s.clip_path {
        Some(ClipPath::Circle { cx, cy, r }) => {
            assert_eq!(r, ClipLength::Frac(0.5));
            assert_eq!(cx, ClipLength::Frac(0.25));
            assert_eq!(cy, ClipLength::Frac(0.5));
        },
        other => panic!("expected Circle, got {other:?}"),
    }
}

#[test]
fn parse_clip_path_ellipse_default_center() {
    use super::super::types::{ClipLength, ClipPath};
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "clip-path",
        &CssValue::Keyword("ellipse(40px 20px)".into()),
        16.0,
    );
    match s.clip_path {
        Some(ClipPath::Ellipse { cx, cy, rx, ry }) => {
            assert_eq!(rx, ClipLength::Px(40.0));
            assert_eq!(ry, ClipLength::Px(20.0));
            assert_eq!(cx, ClipLength::Frac(0.5));
            assert_eq!(cy, ClipLength::Frac(0.5));
        },
        other => panic!("expected Ellipse, got {other:?}"),
    }
}

#[test]
fn parse_clip_path_none_clears() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("clip-path", &CssValue::Keyword("inset(10px)".into()), 16.0);
    assert!(s.clip_path.is_some());
    s.apply_declaration("clip-path", &CssValue::Keyword("none".into()), 16.0);
    assert!(s.clip_path.is_none());
}

#[test]
fn parse_mix_blend_mode() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "mix-blend-mode",
        &CssValue::Keyword("multiply".into()),
        16.0,
    );
    assert_eq!(s.mix_blend_mode, BlendMode::Multiply);
}

#[test]
fn parse_background_clip_text() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("background-clip", &CssValue::Keyword("text".into()), 16.0);
    assert_eq!(s.background_clip, BackgroundBox::Text);
}

#[test]
fn parse_image_rendering_pixelated() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "image-rendering",
        &CssValue::Keyword("pixelated".into()),
        16.0,
    );
    assert_eq!(s.image_rendering, ImageRendering::Pixelated);
}

#[test]
fn parse_font_stretch_condensed() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("font-stretch", &CssValue::Keyword("condensed".into()), 16.0);
    assert_eq!(s.font_stretch, FontStretch::Condensed);
}

#[test]
fn parse_hyphens_auto() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("hyphens", &CssValue::Keyword("auto".into()), 16.0);
    assert_eq!(s.hyphens, Hyphens::Auto);
}

#[test]
fn parse_text_align_last() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("text-align-last", &CssValue::Keyword("center".into()), 16.0);
    assert_eq!(s.text_align_last, TextAlignLast::Center);
}

#[test]
fn parse_text_decoration_thickness_px() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "text-decoration-thickness",
        &CssValue::Length(2.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.text_decoration_thickness, Some(2.0));
}

#[test]
fn parse_text_decoration_thickness_auto() {
    let mut s = ComputedStyle::default();
    s.text_decoration_thickness = Some(3.0);
    s.apply_declaration(
        "text-decoration-thickness",
        &CssValue::Keyword("auto".into()),
        16.0,
    );
    assert_eq!(s.text_decoration_thickness, None);
}

#[test]
fn parse_perspective_length() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "perspective",
        &CssValue::Length(500.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.perspective, Some(500.0));
}

#[test]
fn parse_backface_visibility_hidden() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "backface-visibility",
        &CssValue::Keyword("hidden".into()),
        16.0,
    );
    assert_eq!(s.backface_visibility, BackfaceVisibility::Hidden);
}

#[test]
fn parse_transform_origin_three_value_includes_z() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transform-origin",
        &CssValue::String("25% 75% 40px".into()),
        16.0,
    );
    let origin = s.transform_origin.expect("transform-origin parsed");
    assert_eq!(origin.x_pct, Some(0.25));
    assert_eq!(origin.y_pct, Some(0.75));
    assert!((origin.z - 40.0).abs() < 1e-4);
}

#[test]
fn parse_perspective_origin_to_structured_value() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "perspective-origin",
        &CssValue::String("left center".into()),
        16.0,
    );
    let origin = s.perspective_origin.expect("perspective-origin parsed");
    assert_eq!(origin.x_pct, Some(0.0));
    assert_eq!(origin.y_pct, Some(0.5));
}

#[test]
fn parse_transform_3d_functions() {
    use super::super::types::TransformFunction;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transform",
        &CssValue::String(
            "translate3d(10px, 20px, 30px) rotateX(45deg) rotateY(60deg) scale3d(1, 2, 3) \
                 perspective(500px)"
                .into(),
        ),
        16.0,
    );
    assert_eq!(s.transforms.len(), 5);
    assert!(matches!(
        s.transforms[0],
        TransformFunction::Translate3d(10.0, 20.0, 30.0)
    ));
    assert!(matches!(s.transforms[1], TransformFunction::RotateX(d) if (d - 45.0).abs() < 1e-4));
    assert!(matches!(s.transforms[2], TransformFunction::RotateY(d) if (d - 60.0).abs() < 1e-4));
    assert!(matches!(
        s.transforms[3],
        TransformFunction::Scale3d(1.0, 2.0, 3.0)
    ));
    assert!(
        matches!(s.transforms[4], TransformFunction::Perspective(d) if (d - 500.0).abs() < 1e-4)
    );
}

#[test]
fn parse_transform_rotate3d_and_matrix3d() {
    use super::super::types::TransformFunction;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transform",
        &CssValue::String(
            "rotate3d(0, 1, 0, 90deg) matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 5,6,7,1)".into(),
        ),
        16.0,
    );
    assert_eq!(s.transforms.len(), 2);
    assert!(matches!(
        s.transforms[0],
        TransformFunction::Rotate3d(0.0, 1.0, 0.0, d) if (d - 90.0).abs() < 1e-4
    ));
    if let TransformFunction::Matrix3d(values) = &s.transforms[1] {
        assert_eq!(values[12], 5.0);
        assert_eq!(values[13], 6.0);
        assert_eq!(values[14], 7.0);
        assert_eq!(values[15], 1.0);
    } else {
        panic!("expected Matrix3d, got {:?}", s.transforms[1]);
    }
}

#[test]
fn parse_transform_style_preserve_3d() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "transform-style",
        &CssValue::Keyword("preserve-3d".into()),
        16.0,
    );
    assert_eq!(s.transform_style, TransformStyle::Preserve3d);
}

#[test]
fn parse_overscroll_behavior_shorthand_sets_both_axes() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "overscroll-behavior",
        &CssValue::Keyword("contain".into()),
        16.0,
    );
    assert_eq!(s.overscroll_behavior_x, OverscrollBehavior::Contain);
    assert_eq!(s.overscroll_behavior_y, OverscrollBehavior::Contain);
}

#[test]
fn parse_content_visibility_auto() {
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "content-visibility",
        &CssValue::Keyword("auto".into()),
        16.0,
    );
    assert_eq!(s.content_visibility, ContentVisibility::Auto);
}

#[test]
fn parse_justify_self_center() {
    let mut s = ComputedStyle::default();
    s.apply_declaration("justify-self", &CssValue::Keyword("center".into()), 16.0);
    assert_eq!(s.justify_self, JustifySelf::Center);
}

#[test]
fn parse_inset_shorthand_sets_all_four_sides() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.apply_declaration("inset", &CssValue::Length(10.0, LengthUnit::Px), 16.0);
    assert_eq!(s.top, Dimension::Px(10.0));
    assert_eq!(s.right, Dimension::Px(10.0));
    assert_eq!(s.bottom, Dimension::Px(10.0));
    assert_eq!(s.left, Dimension::Px(10.0));
}

// -- RTL-aware logical properties ----------------------------------

#[test]
fn margin_inline_start_resolves_to_left_in_ltr() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    // direction defaults to LTR
    s.apply_declaration(
        "margin-inline-start",
        &CssValue::Length(10.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.margin_left, 10.0, "inline-start → left in LTR");
    assert_eq!(s.margin_right, 0.0, "right should be untouched");
}

#[test]
fn margin_inline_start_resolves_to_right_in_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "margin-inline-start",
        &CssValue::Length(10.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.margin_right, 10.0, "inline-start → right in RTL");
    assert_eq!(s.margin_left, 0.0, "left should be untouched");
}

#[test]
fn margin_inline_end_resolves_to_right_in_ltr() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "margin-inline-end",
        &CssValue::Length(20.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.margin_right, 20.0, "inline-end → right in LTR");
}

#[test]
fn margin_inline_end_resolves_to_left_in_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "margin-inline-end",
        &CssValue::Length(20.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.margin_left, 20.0, "inline-end → left in RTL");
}

#[test]
fn padding_inline_start_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "padding-inline-start",
        &CssValue::Length(8.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(s.padding_right, 8.0, "inline-start → right in RTL");
    assert_eq!(s.padding_left, 0.0, "left should be untouched");
}

#[test]
fn border_inline_start_width_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "border-inline-start-width",
        &CssValue::Length(2.0, LengthUnit::Px),
        16.0,
    );
    assert_eq!(
        s.border_right_width, 2.0,
        "border-inline-start-width → right in RTL",
    );
    assert_eq!(s.border_left_width, 0.0);
}

#[test]
fn inset_inline_start_resolves_to_left_in_ltr() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    // direction defaults to LTR
    s.apply_declaration(
        "inset-inline-start",
        &CssValue::Length(12.0, LengthUnit::Px),
        16.0,
    );
    assert!(
        matches!(s.left, Dimension::Px(v) if (v - 12.0).abs() < f32::EPSILON),
        "inset-inline-start → left in LTR, got {:?}",
        s.left,
    );
    assert!(
        matches!(s.right, Dimension::Auto),
        "right should be untouched in LTR, got {:?}",
        s.right,
    );
}

#[test]
fn inset_inline_start_resolves_to_right_in_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "inset-inline-start",
        &CssValue::Length(12.0, LengthUnit::Px),
        16.0,
    );
    assert!(
        matches!(s.right, Dimension::Px(v) if (v - 12.0).abs() < f32::EPSILON),
        "inset-inline-start → right in RTL, got {:?}",
        s.right,
    );
    assert!(
        matches!(s.left, Dimension::Auto),
        "left should be untouched in RTL, got {:?}",
        s.left,
    );
}

#[test]
fn inset_inline_end_resolves_to_right_in_ltr() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.apply_declaration(
        "inset-inline-end",
        &CssValue::Length(24.0, LengthUnit::Px),
        16.0,
    );
    assert!(
        matches!(s.right, Dimension::Px(v) if (v - 24.0).abs() < f32::EPSILON),
        "inset-inline-end → right in LTR, got {:?}",
        s.right,
    );
    assert!(
        matches!(s.left, Dimension::Auto),
        "left should be untouched in LTR, got {:?}",
        s.left,
    );
}

#[test]
fn inset_inline_end_resolves_to_left_in_rtl() {
    use crate::css::parser::LengthUnit;
    let mut s = ComputedStyle::default();
    s.direction = TextDirection::Rtl;
    s.apply_declaration(
        "inset-inline-end",
        &CssValue::Length(24.0, LengthUnit::Px),
        16.0,
    );
    assert!(
        matches!(s.left, Dimension::Px(v) if (v - 24.0).abs() < f32::EPSILON),
        "inset-inline-end → left in RTL, got {:?}",
        s.left,
    );
    assert!(
        matches!(s.right, Dimension::Auto),
        "right should be untouched in RTL, got {:?}",
        s.right,
    );
}

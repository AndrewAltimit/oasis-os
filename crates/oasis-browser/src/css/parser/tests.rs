//! Tests for the CSS parser.

use super::*;

// -- helper -------------------------------------------------------

fn parse(css: &str) -> Stylesheet {
    Stylesheet::parse(css)
}

fn first_decls(css: &str) -> Vec<Declaration> {
    let sheet = parse(css);
    assert!(!sheet.rules.is_empty(), "expected at least one rule");
    sheet.rules[0].declarations.clone()
}

fn first_selectors(css: &str) -> SelectorList {
    let sheet = parse(css);
    sheet.rules[0].selectors.clone()
}

// -- test 1: simple rule -----------------------------------------

#[test]
fn simple_rule() {
    let sheet = parse("p { color: red; }");
    assert_eq!(sheet.rules.len(), 1);
    let rule = &sheet.rules[0];
    let sel = &rule.selectors.selectors[0];
    assert_eq!(sel.parts[0].0.parts, vec![SimpleSelector::Type("p".into())]);
    assert_eq!(rule.declarations.len(), 1);
    assert_eq!(rule.declarations[0].property, "color");
    assert_eq!(
        rule.declarations[0].value,
        CssValue::Color(CssColor::new(255, 0, 0, 255))
    );
}

// -- test 2: class selector --------------------------------------

#[test]
fn class_selector() {
    let sheet = parse(".intro { font-size: 14px; }");
    let sel = &sheet.rules[0].selectors.selectors[0];
    assert_eq!(
        sel.parts[0].0.parts,
        vec![SimpleSelector::Class("intro".into())]
    );
    assert_eq!(
        sheet.rules[0].declarations[0].value,
        CssValue::Length(14.0, LengthUnit::Px)
    );
}

// -- test 3: id selector -----------------------------------------

#[test]
fn id_selector() {
    let sheet = parse("#header { background-color: #333; }");
    let sel = &sheet.rules[0].selectors.selectors[0];
    assert_eq!(
        sel.parts[0].0.parts,
        vec![SimpleSelector::Id("header".into())]
    );
    assert_eq!(
        sheet.rules[0].declarations[0].value,
        CssValue::Color(CssColor::new(0x33, 0x33, 0x33, 255))
    );
}

// -- test 4: descendant selector ---------------------------------

#[test]
fn descendant_selector() {
    let decls = first_decls("div p { margin: 10px; }");
    // Should expand margin shorthand.
    assert_eq!(decls.len(), 4);
    assert_eq!(decls[0].property, "margin-top");
}

// -- test 5: child selector --------------------------------------

#[test]
fn child_selector() {
    let sels = first_selectors("div > p { color: blue; }");
    let sel = &sels.selectors[0];
    assert_eq!(sel.parts.len(), 2);
    assert_eq!(sel.parts[1].1, Some(Combinator::Child));
}

// -- test 6: grouped selectors -----------------------------------

#[test]
fn grouped_selectors() {
    let sheet = parse("h1, h2, h3 { font-weight: bold; }");
    assert_eq!(sheet.rules[0].selectors.selectors.len(), 3);
}

// -- test 7: compound selector -----------------------------------

#[test]
fn compound_selector() {
    let sels = first_selectors("p.intro#first { color: green; }");
    let parts = &sels.selectors[0].parts[0].0.parts;
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], SimpleSelector::Type("p".into()));
    assert_eq!(parts[1], SimpleSelector::Class("intro".into()));
    assert_eq!(parts[2], SimpleSelector::Id("first".into()));
}

// -- test 8: multiple declarations -------------------------------

#[test]
fn multiple_declarations() {
    let sheet = parse("p { color: red; font-size: 12px; display: block; }");
    assert_eq!(sheet.rules[0].declarations.len(), 3);
}

// -- test 9: shorthand expansion ---------------------------------

#[test]
fn shorthand_margin_two_values() {
    let decls = first_decls("div { margin: 10px 20px; }");
    assert_eq!(decls.len(), 4);
    assert_eq!(decls[0].property, "margin-top");
    assert_eq!(decls[0].value, CssValue::Length(10.0, LengthUnit::Px));
    assert_eq!(decls[1].property, "margin-right");
    assert_eq!(decls[1].value, CssValue::Length(20.0, LengthUnit::Px));
    assert_eq!(decls[2].property, "margin-bottom");
    assert_eq!(decls[2].value, CssValue::Length(10.0, LengthUnit::Px));
    assert_eq!(decls[3].property, "margin-left");
    assert_eq!(decls[3].value, CssValue::Length(20.0, LengthUnit::Px));
}

#[test]
fn shorthand_margin_three_values() {
    let decls = first_decls("div { margin: 10px 20px 30px; }");
    assert_eq!(decls.len(), 4);
    assert_eq!(decls[0].value, CssValue::Length(10.0, LengthUnit::Px));
    assert_eq!(decls[1].value, CssValue::Length(20.0, LengthUnit::Px));
    assert_eq!(decls[2].value, CssValue::Length(30.0, LengthUnit::Px));
    assert_eq!(decls[3].value, CssValue::Length(20.0, LengthUnit::Px));
}

#[test]
fn shorthand_margin_four_values() {
    let decls = first_decls("div { margin: 10px 20px 30px 40px; }");
    assert_eq!(decls.len(), 4);
    assert_eq!(decls[3].value, CssValue::Length(40.0, LengthUnit::Px));
}

#[test]
fn shorthand_padding() {
    let decls = first_decls("div { padding: 5px; }");
    assert_eq!(decls.len(), 4);
    for d in &decls {
        assert!(d.property.starts_with("padding-"));
        assert_eq!(d.value, CssValue::Length(5.0, LengthUnit::Px));
    }
}

#[test]
fn shorthand_border() {
    let decls = first_decls("div { border: 1px solid black; }");
    assert!(
        decls
            .iter()
            .any(|d| d.property == "border-width"
                && d.value == CssValue::Length(1.0, LengthUnit::Px))
    );
    assert!(
        decls
            .iter()
            .any(|d| d.property == "border-style" && d.value == CssValue::Keyword("solid".into()))
    );
    assert!(
        decls.iter().any(|d| d.property == "border-color"
            && d.value == CssValue::Color(CssColor::new(0, 0, 0, 255)))
    );
}

#[test]
fn shorthand_background_color() {
    let decls = first_decls("div { background: #fff; }");
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].property, "background-color");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(255, 255, 255, 255))
    );
}

// -- test 10: colour parsing -------------------------------------

#[test]
fn color_named() {
    let decls = first_decls("p { color: red; }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(255, 0, 0, 255))
    );
}

#[test]
fn color_hex_short() {
    let decls = first_decls("p { background-color: #abc; }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(0xaa, 0xbb, 0xcc, 255))
    );
}

#[test]
fn color_hex_long() {
    let decls = first_decls("p { color: #11aa33; }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(0x11, 0xaa, 0x33, 255))
    );
}

#[test]
fn color_hex_with_alpha() {
    let decls = first_decls("p { color: #11aa3380; }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(0x11, 0xaa, 0x33, 0x80))
    );
}

#[test]
fn color_rgb_function() {
    let decls = first_decls("p { color: rgb(100, 200, 50); }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(100, 200, 50, 255))
    );
}

#[test]
fn color_rgba_function() {
    let decls = first_decls("p { color: rgba(100, 200, 50, 0.5); }");
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(100, 200, 50, 127))
    );
}

#[test]
fn color_transparent() {
    let decls = first_decls("p { color: transparent; }");
    assert_eq!(decls[0].value, CssValue::Color(CssColor::new(0, 0, 0, 0)));
}

// -- test 11: specificity ----------------------------------------

#[test]
fn specificity_type_only() {
    let sels = first_selectors("p { color: red; }");
    assert_eq!(
        sels.selectors[0].specificity(),
        Specificity {
            inline: 0,
            ids: 0,
            classes: 0,
            types: 1,
        }
    );
}

#[test]
fn specificity_class() {
    let sels = first_selectors(".foo { color: red; }");
    assert_eq!(
        sels.selectors[0].specificity(),
        Specificity {
            inline: 0,
            ids: 0,
            classes: 1,
            types: 0,
        }
    );
}

#[test]
fn specificity_id() {
    let sels = first_selectors("#bar { color: red; }");
    assert_eq!(
        sels.selectors[0].specificity(),
        Specificity {
            inline: 0,
            ids: 1,
            classes: 0,
            types: 0,
        }
    );
}

#[test]
fn specificity_compound() {
    // p.intro#first => types=1, classes=1, ids=1
    let sels = first_selectors("p.intro#first { color: red; }");
    assert_eq!(
        sels.selectors[0].specificity(),
        Specificity {
            inline: 0,
            ids: 1,
            classes: 1,
            types: 1,
        }
    );
}

#[test]
fn specificity_descendant() {
    // div p => types=2
    let sels = first_selectors("div p { color: red; }");
    assert_eq!(
        sels.selectors[0].specificity(),
        Specificity {
            inline: 0,
            ids: 0,
            classes: 0,
            types: 2,
        }
    );
}

#[test]
fn specificity_ordering() {
    let a = Specificity {
        inline: 0,
        ids: 1,
        classes: 0,
        types: 0,
    };
    let b = Specificity {
        inline: 0,
        ids: 0,
        classes: 10,
        types: 10,
    };
    assert!(a > b, "ID selector should outrank classes + types");
}

// -- test 12: !important -----------------------------------------

#[test]
fn important_flag() {
    let decls = first_decls("p { color: red !important; }");
    assert!(decls[0].important);
    assert_eq!(
        decls[0].value,
        CssValue::Color(CssColor::new(255, 0, 0, 255))
    );
}

#[test]
fn not_important() {
    let decls = first_decls("p { color: red; }");
    assert!(!decls[0].important);
}

// -- test 13: inline style parsing -------------------------------

#[test]
fn inline_style() {
    let decls = parse_inline_style("color: red; font-size: 16px;");
    assert_eq!(decls.len(), 2);
    assert_eq!(decls[0].property, "color");
    assert_eq!(decls[1].property, "font-size");
}

// -- test 14: malformed input recovery ---------------------------

#[test]
fn malformed_recovery_bad_declaration() {
    // Missing colon -- the bad declaration should be skipped.
    let sheet = parse("p { color red; font-size: 12px; }");
    // At least font-size should survive.
    let decls = &sheet.rules[0].declarations;
    assert!(
        decls.iter().any(|d| d.property == "font-size"),
        "should recover and parse font-size"
    );
}

#[test]
fn malformed_recovery_unclosed_brace() {
    // Unclosed rule should not panic.
    let sheet = parse("p { color: red; ");
    // May or may not produce a rule, but must not panic.
    let _ = sheet;
}

#[test]
fn malformed_recovery_extra_close_brace() {
    let sheet = parse("} p { color: red; }");
    assert!(
        !sheet.rules.is_empty(),
        "should recover after stray close-brace"
    );
}

// -- font-weight normalisation -----------------------------------

#[test]
fn font_weight_bold() {
    let decls = first_decls("p { font-weight: bold; }");
    assert_eq!(decls[0].value, CssValue::Number(700.0));
}

#[test]
fn font_weight_normal() {
    let decls = first_decls("p { font-weight: normal; }");
    assert_eq!(decls[0].value, CssValue::Number(400.0));
}

// -- multiple rules ---------------------------------------------

#[test]
fn multiple_rules() {
    let sheet = parse("p { color: red; } div { color: blue; }");
    assert_eq!(sheet.rules.len(), 2);
}

// -- at-rule skipping -------------------------------------------

#[test]
fn at_rule_skipped() {
    let sheet = parse("@import url('a.css'); p { color: red; }");
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(
        sheet.rules[0].selectors.selectors[0].parts[0].0.parts[0],
        SimpleSelector::Type("p".into())
    );
}

#[test]
fn at_media_screen_parsed() {
    let sheet = parse(
        "@media screen { body { color: red; } } \
         p { color: blue; }",
    );
    // @media screen matches, so body rule is included alongside p rule.
    assert_eq!(sheet.rules.len(), 2);
}

#[test]
fn at_media_print_skipped() {
    let sheet = parse(
        "@media print { body { color: red; } } \
         p { color: blue; }",
    );
    // @media print does not match screen, so only p rule remains.
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_max_width_match() {
    // 480 <= 600, so this should match.
    let sheet = parse("@media (max-width: 600px) { p { color: red; } }");
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_max_width_no_match() {
    // 480 > 320, so this should NOT match.
    let sheet = parse("@media (max-width: 320px) { p { color: red; } }");
    assert_eq!(sheet.rules.len(), 0);
}

// -- @media with custom viewport ----------------------------------

#[test]
fn at_media_min_height_match() {
    let vp = MediaViewport {
        width: 480.0,
        height: 272.0,
    };
    let sheet =
        Stylesheet::parse_with_viewport("@media (min-height: 200px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_min_height_no_match() {
    let vp = MediaViewport {
        width: 480.0,
        height: 272.0,
    };
    let sheet =
        Stylesheet::parse_with_viewport("@media (min-height: 600px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 0);
}

#[test]
fn at_media_max_height_match() {
    let vp = MediaViewport {
        width: 480.0,
        height: 272.0,
    };
    let sheet =
        Stylesheet::parse_with_viewport("@media (max-height: 400px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_max_height_no_match() {
    let vp = MediaViewport {
        width: 480.0,
        height: 272.0,
    };
    let sheet =
        Stylesheet::parse_with_viewport("@media (max-height: 200px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 0);
}

#[test]
fn at_media_custom_viewport_width() {
    let vp = MediaViewport {
        width: 1024.0,
        height: 768.0,
    };
    // With 1024px viewport, max-width: 320 should NOT match.
    let sheet =
        Stylesheet::parse_with_viewport("@media (max-width: 320px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 0);
    // But min-width: 800 SHOULD match.
    let sheet =
        Stylesheet::parse_with_viewport("@media (min-width: 800px) { p { color: red; } }", vp);
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_screen_and_min_width() {
    let vp = MediaViewport {
        width: 800.0,
        height: 600.0,
    };
    let sheet = Stylesheet::parse_with_viewport(
        "@media screen and (min-width: 480px) { p { color: red; } }",
        vp,
    );
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_all_matches() {
    let sheet = parse("@media all { p { color: red; } }");
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_prefers_color_scheme_dark() {
    let sheet = parse("@media (prefers-color-scheme: dark) { p { color: white; } }");
    // Dark mode is always false.
    assert_eq!(sheet.rules.len(), 0);
}

#[test]
fn at_media_prefers_color_scheme_light() {
    let sheet = parse("@media (prefers-color-scheme: light) { p { color: black; } }");
    // Light mode is always true.
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn at_media_compound_width_and_height() {
    let vp = MediaViewport {
        width: 800.0,
        height: 600.0,
    };
    let sheet = Stylesheet::parse_with_viewport(
        "@media (min-width: 480px) and (min-height: 400px) { \
         p { color: red; } }",
        vp,
    );
    assert_eq!(sheet.rules.len(), 1);
    // Fail on height.
    let sheet = Stylesheet::parse_with_viewport(
        "@media (min-width: 480px) and (min-height: 800px) { \
         p { color: red; } }",
        vp,
    );
    assert_eq!(sheet.rules.len(), 0);
}

// -- pseudo-class ------------------------------------------------

#[test]
fn pseudo_class_selector() {
    let sels = first_selectors("a:hover { color: red; }");
    let parts = &sels.selectors[0].parts[0].0.parts;
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], SimpleSelector::Type("a".into()));
    assert_eq!(parts[1], SimpleSelector::PseudoClass("hover".into()));
}

// -- universal selector ------------------------------------------

#[test]
fn universal_selector() {
    let sels = first_selectors("* { margin: 0; }");
    assert_eq!(
        sels.selectors[0].parts[0].0.parts[0],
        SimpleSelector::Universal
    );
}

// -- empty stylesheet -------------------------------------------

#[test]
fn empty_stylesheet() {
    let sheet = parse("");
    assert!(sheet.rules.is_empty());
}

#[test]
fn whitespace_only_stylesheet() {
    let sheet = parse("   \n\t  ");
    assert!(sheet.rules.is_empty());
}

// -- robustness / edge cases ----------------------------------------

#[test]
fn unclosed_rule_block() {
    let sheet = parse("p { color: red;");
    // Should not panic; may or may not produce a rule.
    let _ = sheet;
}

#[test]
fn unclosed_value() {
    let sheet = parse("p { color: ");
    let _ = sheet;
}

#[test]
fn missing_colon() {
    let sheet = parse("p { color red; }");
    // Malformed declaration -- parser should skip gracefully.
    let _ = sheet;
}

#[test]
fn missing_semicolon_between_declarations() {
    let sheet = parse("p { color: red background: blue; }");
    let _ = sheet;
}

#[test]
fn empty_selector() {
    let sheet = parse("{ color: red; }");
    let _ = sheet;
}

#[test]
fn empty_declaration_block() {
    let sheet = parse("p { }");
    assert_eq!(sheet.rules.len(), 1);
    assert!(sheet.rules[0].declarations.is_empty());
}

#[test]
fn very_long_property_value() {
    let val = "x".repeat(10_000);
    let css = format!("p {{ content: \"{val}\"; }}");
    let sheet = parse(&css);
    assert!(!sheet.rules.is_empty());
}

#[test]
fn very_long_selector_chain() {
    // div > div > div > ... (100 levels)
    let sel: String = (0..100).map(|_| "div").collect::<Vec<_>>().join(" > ");
    let css = format!("{sel} {{ color: red; }}");
    let sheet = parse(&css);
    assert!(!sheet.rules.is_empty());
}

#[test]
fn many_rules() {
    let css: String = (0..500)
        .map(|i| format!(".c{i} {{ color: red; }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let sheet = parse(&css);
    assert_eq!(sheet.rules.len(), 500);
}

#[test]
fn nested_braces() {
    // CSS doesn't normally nest, but parser should handle gracefully.
    let sheet = parse("p { color: red; { nested: bad; } }");
    let _ = sheet;
}

#[test]
fn unmatched_closing_brace() {
    let sheet = parse("} p { color: red; }");
    let _ = sheet;
}

#[test]
fn at_rule_unknown() {
    let sheet = parse("@unknown { p { color: red; } }");
    let _ = sheet;
}

#[test]
fn at_media_rule() {
    let sheet = parse("@media screen { p { color: red; } }");
    // @media screen matches, inner rules are extracted.
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn comments_in_css() {
    let sheet = parse("/* comment */ p { color: red; /* inline */ }");
    assert!(!sheet.rules.is_empty());
}

#[test]
fn multiple_selectors_comma_separated() {
    let sheet = parse("h1, h2, h3 { color: blue; }");
    assert_eq!(sheet.rules.len(), 1);
    assert!(sheet.rules[0].selectors.selectors.len() >= 3);
}

#[test]
fn selector_with_pseudo_class() {
    let sheet = parse("a:hover { color: red; }");
    let _ = sheet; // Should not panic.
}

#[test]
fn selector_with_pseudo_element() {
    let sheet = parse("p::before { content: 'x'; }");
    let _ = sheet;
}

#[test]
fn null_bytes_in_css() {
    let sheet = parse("p { color: re\0d; }");
    let _ = sheet;
}

#[test]
fn extremely_specific_selector() {
    // #id.c1.c2.c3...c50
    let classes: String = (0..50).map(|i| format!(".c{i}")).collect();
    let css = format!("#id{classes} {{ color: red; }}");
    let sheet = parse(&css);
    assert!(!sheet.rules.is_empty());
}

#[test]
fn numeric_property_values() {
    let sheet = parse("p { width: 100px; height: 50%; margin: 0; }");
    let decls = &sheet.rules[0].declarations;
    // margin: 0 may be expanded into 4 longhand properties.
    assert!(decls.len() >= 3);
}

#[test]
fn shorthand_property() {
    let sheet = parse("p { margin: 10px 20px 30px 40px; }");
    assert!(!sheet.rules.is_empty());
}

#[test]
fn color_hex_values() {
    let sheet = parse("p { color: #fff; background: #aabbcc; border-color: #12345678; }");
    // border-color is expanded into 4 longhand properties.
    assert_eq!(sheet.rules[0].declarations.len(), 6);
}

#[test]
fn trailing_garbage_after_rules() {
    let sheet = parse("p { color: red; } garbage here");
    // The first rule should still parse.
    assert!(!sheet.rules.is_empty());
}

// -- CSS custom properties / var() parsing tests ----------------------

#[test]
fn var_function_parsed() {
    let decls = first_decls("p { color: var(--my-color); }");
    assert_eq!(decls[0].property, "color");
    assert_eq!(decls[0].value, CssValue::Var("--my-color".into(), None));
}

#[test]
fn var_function_with_fallback() {
    let decls = first_decls("p { color: var(--my-color, blue); }");
    assert_eq!(
        decls[0].value,
        CssValue::Var("--my-color".into(), Some("blue".into()))
    );
}

#[test]
fn var_function_with_hex_fallback() {
    let decls = first_decls("p { color: var(--my-color, #202122); }");
    assert_eq!(
        decls[0].value,
        CssValue::Var("--my-color".into(), Some("#202122".into()))
    );
}

#[test]
fn custom_property_stored_as_raw_text() {
    let decls = first_decls(":root { --color: #202122; }");
    assert_eq!(decls[0].property, "--color");
    assert_eq!(decls[0].value, CssValue::String("#202122".into()));
}

#[test]
fn custom_property_complex_value() {
    let decls = first_decls(":root { --border: 1px solid red; }");
    assert_eq!(decls[0].property, "--border");
    assert_eq!(decls[0].value, CssValue::String("1px solid red".into()));
}

#[test]
fn var_in_multiple_value_property() {
    let decls = first_decls("p { border: 1px solid var(--color); }");
    // The border shorthand should expand, and var() should end up
    // in border-color.
    let bc = decls.iter().find(|d| d.property == "border-color");
    assert!(bc.is_some(), "border-color should exist");
    assert!(
        matches!(&bc.unwrap().value, CssValue::Var(name, None) if name == "--color"),
        "border-color should be var(--color)"
    );
}

#[test]
fn linear_gradient_to_right() {
    let css = "div { background: linear-gradient(to right, red, blue); }";
    let sheet = parse(css);
    let decls = &sheet.rules[0].declarations;
    let bg_image = decls
        .iter()
        .find(|d| d.property == "background-image")
        .expect("should have background-image");
    assert!(
        matches!(&bg_image.value, CssValue::Gradient(_)),
        "should parse as gradient"
    );
    if let CssValue::Gradient(ref g) = bg_image.value {
        assert_eq!(g.direction, crate::css::values::GradientDirection::ToRight);
        assert_eq!(g.stops.len(), 2);
    }
}

#[test]
fn linear_gradient_default_direction() {
    let css = "div { background-image: linear-gradient(red, blue); }";
    let sheet = parse(css);
    let decls = &sheet.rules[0].declarations;
    let bg_image = decls
        .iter()
        .find(|d| d.property == "background-image")
        .expect("should have background-image");
    assert!(
        matches!(&bg_image.value, CssValue::Gradient(_)),
        "expected gradient"
    );
    let CssValue::Gradient(g) = &bg_image.value else {
        unreachable!()
    };
    assert_eq!(g.direction, crate::css::values::GradientDirection::ToBottom);
    assert_eq!(g.stops.len(), 2);
}

mod prop {
    use super::*;
    use crate::css::helpers::named_color;
    use crate::css::helpers::parse_hex_color;
    use proptest::prelude::*;

    proptest! {
        /// Parsing arbitrary CSS never panics.
        #[test]
        fn parse_never_panics(input in "[ -~]{0,120}") {
            let _ = Stylesheet::parse(&input);
        }

        /// Parsing inline styles never panics.
        #[test]
        fn parse_inline_never_panics(input in "[ -~]{0,80}") {
            let _ = parse_inline_style(&input);
        }

        /// Valid 3-digit hex colors parse successfully.
        #[test]
        fn hex_color_3_digit(
            r in "[0-9a-fA-F]",
            g in "[0-9a-fA-F]",
            b in "[0-9a-fA-F]",
        ) {
            let hex = format!("#{r}{g}{b}");
            let color = parse_hex_color(&hex);
            prop_assert!(
                color.is_some(),
                "valid 3-digit hex '{hex}' should parse",
            );
        }

        /// Valid 6-digit hex colors parse successfully.
        #[test]
        fn hex_color_6_digit(
            r in "[0-9a-fA-F]{2}",
            g in "[0-9a-fA-F]{2}",
            b in "[0-9a-fA-F]{2}",
        ) {
            let hex = format!("#{r}{g}{b}");
            let color = parse_hex_color(&hex);
            prop_assert!(
                color.is_some(),
                "valid 6-digit hex '{hex}' should parse",
            );
        }

        /// Invalid hex strings (wrong length) return None.
        #[test]
        fn hex_color_bad_length(
            s in "[0-9a-f]{1,2}|[0-9a-f]{5}|[0-9a-f]{7}|[0-9a-f]{9,12}",
        ) {
            let hex = format!("#{s}");
            prop_assert!(
                parse_hex_color(&hex).is_none(),
                "invalid-length hex '{hex}' should not parse",
            );
        }

        /// Named color lookup is case-insensitive.
        #[test]
        fn named_color_case_insensitive(
            name in proptest::sample::select(vec![
                "red".to_string(), "Red".to_string(), "RED".to_string(),
                "blue".to_string(), "Blue".to_string(), "BLUE".to_string(),
                "green".to_string(), "Green".to_string(), "GREEN".to_string(),
                "white".to_string(), "White".to_string(),
                "black".to_string(), "Black".to_string(), "BLACK".to_string(),
            ]),
        ) {
            prop_assert!(
                named_color(&name).is_some(),
                "named color '{}' should be recognized", name,
            );
        }

        /// A valid rule with random property name parses without panic.
        #[test]
        fn rule_with_random_property(
            prop_name in "[a-z\\-]{1,20}",
            value in "[a-z0-9]{1,10}",
        ) {
            let css = format!("p {{ {prop_name}: {value}; }}");
            let sheet = Stylesheet::parse(&css);
            // Should parse the rule (property may not be recognized,
            // but shouldn't panic).
            prop_assert!(!sheet.rules.is_empty());
        }
    }
}

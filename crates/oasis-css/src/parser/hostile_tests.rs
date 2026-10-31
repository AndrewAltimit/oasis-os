//! Parser regressions for hostile and real-world input: nesting depth,
//! lookahead cost, selector explosion, layer naming.

use super::*;

fn parse(css: &str) -> Stylesheet {
    Stylesheet::parse(css)
}

/// `@layer{@media{@supports{...` recursed without the nesting cap and
/// overflowed the stack.
#[test]
fn deeply_nested_at_rule_blocks_do_not_overflow() {
    for open in [
        "@layer{",
        "@media screen{",
        "@supports (color:red){",
        "@scope{",
        "@container (min-width: 1px){",
    ] {
        let css = format!("{}p{{color:red}}", open.repeat(100_000));
        let sheet = parse(&css);
        assert!(sheet.rules.is_empty(), "{open}");
    }
    // Shallow nesting still works.
    let sheet = parse("@layer a{@media screen{@supports (color:red){p{color:red}}}}");
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn long_not_chains_in_supports_do_not_overflow() {
    let css = format!(
        "@supports {}(color:red){{p{{color:red}}}}",
        "not ".repeat(100_000)
    );
    // An even number of `not`s: the condition holds.
    assert_eq!(parse(&css).rules.len(), 1);
}

/// A `Function` token did not open a group in the declaration/rule
/// lookahead, so `&:not(.x) { .. }` read as a declaration (losing the
/// rule and the next declaration, and leaking the parent's `}`).
#[test]
fn nested_rule_with_functional_pseudo_class() {
    let sheet = parse(".p { &:not(.x) { color: red } margin: 1px } .q { color: blue }");
    let props: Vec<&str> = sheet
        .rules
        .iter()
        .flat_map(|r| r.declarations.iter().map(|d| d.property.as_str()))
        .collect();
    assert!(props.contains(&"color"), "{props:?}");
    assert!(props.contains(&"margin-top"), "{props:?}");
    assert_eq!(sheet.rules.len(), 3, "{props:?}");
}

/// The same bug made every declaration containing a function scan to
/// the end of the stylesheet: quadratic parsing.
#[test]
fn declarations_with_functions_parse_in_linear_time() {
    let css = format!(".a {{ {} }}", "color: rgb(1, 2, 3); ".repeat(20_000));
    let start = std::time::Instant::now();
    let sheet = parse(&css);
    assert_eq!(sheet.rules[0].declarations.len(), 20_000);
    assert!(start.elapsed() < std::time::Duration::from_secs(20));
}

/// Declarations written directly inside a nested `@media` were dropped.
#[test]
fn bare_declarations_in_nested_media_apply_to_the_parent() {
    let sheet = parse(".a { @media screen { color: red } }");
    let rule = sheet
        .rules
        .iter()
        .find(|r| r.declarations.iter().any(|d| d.property == "color"))
        .expect("nested @media declarations kept");
    assert_eq!(rule.selectors, parse_selector_string(".a").unwrap());
}

/// At-rules inside a top-level `@media` / `@supports` were skipped.
#[test]
fn at_rules_nested_in_media_and_supports_are_parsed() {
    let sheet = parse("@media screen { @supports (color: red) { p { color: red } } }");
    assert_eq!(sheet.rules.len(), 1);
    let sheet = parse("@supports (color: red) { @layer x { p { color: red } } }");
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(sheet.layers, vec!["x".to_string()]);
}

/// `a,b{a,b{...` flattened as a Cartesian product: 2^depth selectors.
#[test]
fn nesting_selector_explosion_is_capped() {
    let depth = 30;
    let css = format!("{}color:red{}", "a,b{".repeat(depth), "}".repeat(depth));
    let sheet = parse(&css);
    let total: usize = sheet
        .rules
        .iter()
        .map(|r| r.selectors.selectors.len())
        .sum();
    assert!(total <= 4096, "{total}");
    let css = format!("p{{{}color:red{}}}", "& & & &{".repeat(12), "}".repeat(12));
    for rule in &parse(&css).rules {
        for sel in &rule.selectors.selectors {
            assert!(sel.parts.len() <= 64, "{}", sel.parts.len());
        }
    }
}

#[test]
fn nested_layer_names_are_qualified() {
    let sheet = parse("@layer a { @layer b { p { color: red } } } @layer b { p { color: blue } }");
    assert_eq!(sheet.layers, vec!["a", "a.b", "b"]);
    let sheet = parse("@layer a { @layer b; } @layer a.b { p { color: red } }");
    assert_eq!(sheet.layers, vec!["a", "a.b"]);
}

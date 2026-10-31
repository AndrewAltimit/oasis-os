//! Unit tests for statement splitting and parsing.

use super::*;

fn texts(src: &str) -> Vec<String> {
    split_statements(src).into_iter().map(|s| s.text).collect()
}

#[test]
fn split_on_newlines_and_semicolons() {
    assert_eq!(
        texts("echo a; echo b\necho c"),
        ["echo a", "echo b", "echo c"]
    );
}

#[test]
fn split_respects_quotes() {
    assert_eq!(
        texts("echo 'a;b'; echo \"c;d\""),
        ["echo 'a;b'", "echo \"c;d\""]
    );
}

#[test]
fn split_keeps_braces_and_substitutions() {
    assert_eq!(
        texts("function f() { echo a; echo b }; echo $(echo x; echo y)"),
        ["function f() { echo a; echo b }", "echo $(echo x; echo y)"]
    );
}

#[test]
fn split_emits_double_semicolon() {
    assert_eq!(
        texts("a) echo x ;; b) echo y;;"),
        ["a) echo x", ";;", "b) echo y", ";;"]
    );
}

#[test]
fn split_strips_comments_but_not_dollar_hash() {
    assert_eq!(texts("# comment\necho $# # trailing"), ["echo $#"]);
    assert_eq!(texts("echo a#b"), ["echo a#b"]);
}

#[test]
fn split_tracks_line_numbers() {
    let stmts = split_statements("echo a\n\n# c\necho b; echo c");
    let lines: Vec<usize> = stmts.iter().map(|s| s.line).collect();
    assert_eq!(lines, [1, 4, 4]);
}

#[test]
fn split_line_continuation() {
    assert_eq!(texts("echo a \\\nb"), ["echo a b"]);
}

#[test]
fn is_compound_detects_keywords_only_at_statement_start() {
    assert!(is_compound("if true; then echo x; fi"));
    assert!(is_compound("echo a; for x in 1; do echo $x; done"));
    assert!(!is_compound("echo if then fi"));
    assert!(!is_compound("echo 'a; if b'"));
}

#[test]
fn parse_nested_if_with_elif() {
    let nodes = parse_script(split_statements(
        "if a; then if b; then c; fi; elif d; then e; else f; fi",
    ))
    .unwrap();
    assert_eq!(nodes.len(), 1);
    let Node::If {
        branches,
        else_body,
    } = &nodes[0]
    else {
        panic!("expected if");
    };
    assert_eq!(branches.len(), 2);
    assert!(matches!(branches[0].1[0], Node::If { .. }));
    assert!(else_body.is_some());
}

#[test]
fn parse_errors_on_missing_terminators() {
    assert!(parse_script(split_statements("if a; then b")).is_err());
    assert!(parse_script(split_statements("while a; do b")).is_err());
    assert!(parse_script(split_statements("case x in a) b ;;")).is_err());
    assert!(parse_script(split_statements("echo a; fi")).is_err());
}

#[test]
fn find_keyword_basic() {
    assert_eq!(find_keyword("foo then bar", "then"), Some(4));
    assert_eq!(find_keyword("undone", "done"), None);
    assert_eq!(find_keyword("echo 'then' fi", "then"), None);
}

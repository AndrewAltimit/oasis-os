//! Minimal html5lib-tests tree-construction harness.
//!
//! Loads `.dat` files from `tests/fixtures/html5lib/`, runs the HTML
//! parser (tokenizer + tree builder) on each `#data` block, and
//! compares the result against the `#document` dump in the same
//! html5lib pipe-indented format.
//!
//! This is intentionally self-contained — we vendor a small curated
//! subset of fixtures rather than pulling the full html5lib-tests
//! repository (~20k tests, many of which exercise features we
//! deliberately don't implement: full SVG namespacing with camelCase
//! fixup, adoption agency corner cases beyond the 8-iteration limit,
//! SVG integration points, MathML `<annotation-xml>`, etc.). Fixtures
//! focus on the features we *do* claim to support: basic tree
//! construction, the full adoption agency algorithm, foreign content
//! breakout, and `<template>` parsing.
//!
//! The harness fails with a unified diff on mismatch so regressions
//! are easy to read. To extend coverage, drop additional `.dat` files
//! into `tests/fixtures/html5lib/` and list them in `FIXTURE_FILES`.

use oasis_browser::internals::{Document, NodeKind, Tokenizer, TreeBuilder};

// `NodeId` is a type alias for `usize` in the DOM arena. The internals
// module doesn't re-export it, so mirror the alias here.
type NodeId = usize;

const FIXTURE_FILES: &[&str] = &["tree_construction_basic.dat"];

#[derive(Debug)]
struct TestCase {
    data: String,
    expected: String,
    /// Human-readable label for failure messages — the fixture file
    /// name plus 1-based test index within that file.
    label: String,
}

/// Parse a single `.dat` file into its list of test cases. The format
/// is the html5lib-tests tree-construction format:
///
/// ```text
/// #data
/// <input>
/// #errors
/// #document
/// | <html>
/// |   <body>
/// ```
///
/// Sections are separated by blank lines. We support the minimum
/// subset we need: `#data`, `#errors` (ignored), and `#document`.
fn parse_dat(contents: &str, file: &str) -> Vec<TestCase> {
    let mut cases = Vec::new();
    // Split on blank lines that precede a `#data` marker. We can't
    // just split on `\n\n` because a `#document` section may itself
    // contain blank lines between nodes (rare but legal). Instead we
    // walk line by line and assemble tests manually.
    let mut lines = contents.lines().peekable();
    let mut idx = 0usize;
    while lines.peek().is_some() {
        // Skip blank lines between tests.
        while let Some(&line) = lines.peek() {
            if line.is_empty() {
                lines.next();
            } else {
                break;
            }
        }
        if lines.peek().is_none() {
            break;
        }
        idx += 1;
        let label = format!("{file}#{idx}");
        // Expect `#data`.
        let first = lines.next().expect("already peeked non-empty");
        assert_eq!(first, "#data", "expected `#data` at start of test {label}");
        let mut data = String::new();
        let mut in_data = true;
        let mut expected = String::new();
        let mut in_document = false;
        while let Some(line) = lines.next() {
            match line {
                "#errors" => {
                    in_data = false;
                },
                "#new-errors" => {
                    // Newer html5lib-tests dialect — treat like errors.
                    in_data = false;
                },
                "#document" => {
                    in_data = false;
                    in_document = true;
                },
                "#document-fragment" => {
                    in_document = true;
                    in_data = false;
                },
                "#script-off" | "#script-on" => {
                    in_data = false;
                },
                _ if line == "#data" => {
                    // Start of the next test — unread it by
                    // pretending we haven't consumed it. The outer
                    // loop will handle it if we break.
                    // We can't actually unread a std::iter::Peekable
                    // after .next(), so prepend via a small trick:
                    // stash it in a local and handle at the top.
                    // Instead, finalize this test and break.
                    cases.push(TestCase {
                        data: data.clone(),
                        expected: expected.clone(),
                        label: label.clone(),
                    });
                    // Re-inject by recursing on the remainder.
                    let mut rest = String::from(line);
                    rest.push('\n');
                    for l in lines.by_ref() {
                        rest.push_str(l);
                        rest.push('\n');
                    }
                    let mut tail = parse_dat(&rest, file);
                    // The first case in tail was generated starting
                    // at index 1 — rebase its label so the index
                    // stays monotonic.
                    for (off, c) in tail.iter_mut().enumerate() {
                        c.label = format!("{file}#{}", idx + 1 + off);
                    }
                    cases.extend(tail);
                    return cases;
                },
                _ => {
                    if in_data {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(line);
                    } else if in_document {
                        expected.push_str(line);
                        expected.push('\n');
                    }
                    // Errors section — ignored.
                },
            }
        }
        cases.push(TestCase {
            data,
            expected,
            label,
        });
    }
    cases
}

/// Dump a parsed document in the html5lib pipe-indented format.
/// Only the subset of node kinds our tree builder produces is
/// emitted; attributes are rendered as `key="value"` sorted by name
/// to match the html5lib convention.
fn dump_document(doc: &Document) -> String {
    let mut out = String::new();
    // The html5lib format starts the dump at the Document node and
    // uses 2-space indentation per level, with each line prefixed by
    // "| ". Our Document root is synthetic and not printed; its
    // children begin at depth 1.
    for &child in &doc.get(doc.root).children {
        dump_node(doc, child, 0, &mut out);
    }
    out
}

fn dump_node(doc: &Document, id: NodeId, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match &doc.get(id).kind {
        NodeKind::Element(data) => {
            out.push_str("| ");
            out.push_str(&indent);
            out.push('<');
            out.push_str(data.tag.as_str());
            out.push('>');
            out.push('\n');
            // Attributes: html5lib emits them sorted by name one per
            // line, indented one level deeper than the element.
            let mut attrs = data.attributes.clone();
            attrs.sort_by(|a, b| a.name.cmp(&b.name));
            let attr_indent = "  ".repeat(depth + 1);
            for attr in &attrs {
                out.push_str("| ");
                out.push_str(&attr_indent);
                out.push_str(&attr.name);
                out.push_str("=\"");
                out.push_str(&attr.value);
                out.push_str("\"\n");
            }
            for &c in &doc.get(id).children {
                dump_node(doc, c, depth + 1, out);
            }
        },
        NodeKind::Text(s) => {
            out.push_str("| ");
            out.push_str(&indent);
            out.push('"');
            out.push_str(s);
            out.push_str("\"\n");
        },
        NodeKind::Comment(s) => {
            out.push_str("| ");
            out.push_str(&indent);
            out.push_str("<!-- ");
            out.push_str(s);
            out.push_str(" -->\n");
        },
        NodeKind::Document => {
            for &c in &doc.get(id).children {
                dump_node(doc, c, depth, out);
            }
        },
    }
}

/// Line-oriented diff: print the mismatched lines side by side.
fn diff_lines(expected: &str, actual: &str) -> String {
    let e_lines: Vec<&str> = expected.lines().collect();
    let a_lines: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    out.push_str("--- expected\n");
    out.push_str("+++ actual\n");
    let max = e_lines.len().max(a_lines.len());
    for i in 0..max {
        let e = e_lines.get(i).copied().unwrap_or("");
        let a = a_lines.get(i).copied().unwrap_or("");
        if e == a {
            out.push_str("  ");
            out.push_str(e);
            out.push('\n');
        } else {
            if !e.is_empty() {
                out.push_str("- ");
                out.push_str(e);
                out.push('\n');
            }
            if !a.is_empty() {
                out.push_str("+ ");
                out.push_str(a);
                out.push('\n');
            }
        }
    }
    out
}

fn run_fixture(file: &str) -> (usize, Vec<String>) {
    let path = format!(
        "{}/tests/fixtures/html5lib/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let contents =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let cases = parse_dat(&contents, file);
    let mut failures = Vec::new();
    let total = cases.len();
    for case in cases {
        let tokens = Tokenizer::new(&case.data).tokenize();
        let doc = TreeBuilder::build(tokens);
        let actual = dump_document(&doc);
        // Normalize trailing whitespace so fixture EOF conventions
        // don't drive false failures.
        let expected_n = case.expected.trim_end().to_string();
        let actual_n = actual.trim_end().to_string();
        if actual_n != expected_n {
            let diff = diff_lines(&expected_n, &actual_n);
            failures.push(format!(
                "\n==== FAIL {} ====\ninput: {:?}\n{diff}",
                case.label, case.data
            ));
        }
    }
    (total, failures)
}

#[test]
fn html5lib_tree_construction_fixtures_pass() {
    let mut total = 0usize;
    let mut all_failures = Vec::new();
    for file in FIXTURE_FILES {
        let (n, failures) = run_fixture(file);
        total += n;
        all_failures.extend(failures);
    }
    assert!(total > 0, "no tests loaded");
    if !all_failures.is_empty() {
        let passed = total - all_failures.len();
        panic!(
            "html5lib tree-construction: {}/{} passed\n{}",
            passed,
            total,
            all_failures.join("\n")
        );
    }
}

#[test]
fn dat_parser_smoke() {
    let raw = "#data\n<p>x</p>\n#errors\n#document\n| <html>\n|   <head>\n|   <body>\n|     <p>\n|       \"x\"\n";
    let cases = parse_dat(raw, "smoke.dat");
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].data, "<p>x</p>");
    assert!(cases[0].expected.contains("<html>"));
}

//! Highlighter tests (all languages).

use super::*;

// -- File type detection --

#[test]
fn detect_rust() {
    assert_eq!(detect_file_type("/foo/bar.rs"), FileType::Rust);
    assert_eq!(detect_file_type("main.c"), FileType::C);
    assert_eq!(detect_file_type("util.hpp"), FileType::C);
}

#[test]
fn detect_toml() {
    assert_eq!(detect_file_type("Cargo.toml"), FileType::Toml);
}

#[test]
fn detect_html() {
    assert_eq!(detect_file_type("index.html"), FileType::Html);
    assert_eq!(detect_file_type("page.htm"), FileType::Html);
}

#[test]
fn detect_css() {
    assert_eq!(detect_file_type("style.css"), FileType::Css);
}

#[test]
fn detect_javascript() {
    assert_eq!(detect_file_type("app.js"), FileType::JavaScript);
    assert_eq!(detect_file_type("module.ts"), FileType::JavaScript);
    assert_eq!(detect_file_type("component.tsx"), FileType::JavaScript);
}

#[test]
fn detect_markdown() {
    assert_eq!(detect_file_type("README.md"), FileType::Markdown);
}

#[test]
fn detect_shell() {
    assert_eq!(detect_file_type("build.sh"), FileType::Shell);
    assert_eq!(detect_file_type("init.bash"), FileType::Shell);
}

#[test]
fn detect_plain() {
    assert_eq!(detect_file_type("notes.txt"), FileType::Plain);
    assert_eq!(detect_file_type("data"), FileType::Plain);
}

#[test]
fn detect_case_insensitive_extension() {
    assert_eq!(detect_file_type("FILE.RS"), FileType::Rust);
    assert_eq!(detect_file_type("PAGE.HTML"), FileType::Html);
}

// -- Plain highlighting --

#[test]
fn plain_returns_single_span() {
    let (spans, _) = highlight_line("hello world", FileType::Plain, false);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, SyntaxKind::Normal);
    assert_eq!(spans[0].start, 0);
    assert_eq!(spans[0].end, 11);
}

// -- Rust highlighting --

#[test]
fn rust_keyword() {
    let (spans, _) = highlight_line("fn main() {", FileType::Rust, false);
    assert_eq!(spans[0].kind, SyntaxKind::Keyword);
    assert_eq!(&"fn main() {"[spans[0].start..spans[0].end], "fn");
}

#[test]
fn rust_type() {
    let (spans, _) = highlight_line("let x: u32 = 42;", FileType::Rust, false);
    // Find the u32 span.
    let u32_span = spans
        .iter()
        .find(|s| &"let x: u32 = 42;"[s.start..s.end] == "u32");
    assert!(u32_span.is_some());
    assert_eq!(u32_span.unwrap().kind, SyntaxKind::Type);
}

#[test]
fn rust_string() {
    let (spans, _) = highlight_line("let s = \"hello\";", FileType::Rust, false);
    let str_span = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
    assert!(str_span.is_some());
    assert_eq!(
        &"let s = \"hello\";"[str_span.unwrap().start..str_span.unwrap().end],
        "\"hello\""
    );
}

#[test]
fn rust_number() {
    let (spans, _) = highlight_line("let x = 42;", FileType::Rust, false);
    let num = spans.iter().find(|s| s.kind == SyntaxKind::Number);
    assert!(num.is_some());
    assert_eq!(&"let x = 42;"[num.unwrap().start..num.unwrap().end], "42");
}

#[test]
fn rust_line_comment() {
    let (spans, _) = highlight_line("// this is a comment", FileType::Rust, false);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, SyntaxKind::Comment);
}

#[test]
fn rust_block_comment_single_line() {
    let (spans, in_comment) = highlight_line("x /* comment */ y", FileType::Rust, false);
    assert!(!in_comment);
    let comment = spans.iter().find(|s| s.kind == SyntaxKind::Comment);
    assert!(comment.is_some());
}

#[test]
fn rust_block_comment_spans_lines() {
    let (spans1, in_comment) = highlight_line("x /* start", FileType::Rust, false);
    assert!(in_comment);
    let comment1 = spans1.iter().find(|s| s.kind == SyntaxKind::Comment);
    assert!(comment1.is_some());

    let (spans2, in_comment) = highlight_line("still comment */ y", FileType::Rust, true);
    assert!(!in_comment);
    assert_eq!(spans2[0].kind, SyntaxKind::Comment);
}

#[test]
fn rust_attribute() {
    let (spans, _) = highlight_line("#[derive(Debug)]", FileType::Rust, false);
    assert_eq!(spans[0].kind, SyntaxKind::Attribute);
}

// -- TOML highlighting --

#[test]
fn toml_section() {
    let (spans, _) = highlight_line("[package]", FileType::Toml, false);
    assert_eq!(spans[0].kind, SyntaxKind::Section);
}

#[test]
fn toml_comment() {
    let (spans, _) = highlight_line("# comment", FileType::Toml, false);
    assert_eq!(spans[0].kind, SyntaxKind::Comment);
}

#[test]
fn toml_key_value() {
    let (spans, _) = highlight_line("name = \"oasis\"", FileType::Toml, false);
    assert_eq!(spans[0].kind, SyntaxKind::Keyword); // key
    assert_eq!(spans[1].kind, SyntaxKind::Operator); // =
}

// -- HTML highlighting --

#[test]
fn html_tag() {
    let (spans, _) = highlight_line("<div class=\"main\">", FileType::Html, false);
    assert_eq!(spans[0].kind, SyntaxKind::Tag); // <div
    let attr = spans.iter().find(|s| s.kind == SyntaxKind::TagAttribute);
    assert!(attr.is_some());
    let str_span = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
    assert!(str_span.is_some());
}

// -- Markdown highlighting --

#[test]
fn markdown_heading() {
    let (spans, _) = highlight_line("# Hello", FileType::Markdown, false);
    assert_eq!(spans[0].kind, SyntaxKind::Heading);
}

#[test]
fn markdown_code_span() {
    let (spans, _) = highlight_line("use `code` here", FileType::Markdown, false);
    let code = spans.iter().find(|s| s.kind == SyntaxKind::CodeSpan);
    assert!(code.is_some());
}

// -- Shell highlighting --

#[test]
fn shell_keyword() {
    let (spans, _) = highlight_line("if true; then", FileType::Shell, false);
    assert_eq!(spans[0].kind, SyntaxKind::Keyword);
    assert_eq!(&"if true; then"[spans[0].start..spans[0].end], "if");
}

#[test]
fn shell_variable() {
    let (spans, _) = highlight_line("echo $HOME", FileType::Shell, false);
    let var = spans.iter().find(|s| s.kind == SyntaxKind::Attribute);
    assert!(var.is_some());
}

#[test]
fn shell_comment() {
    let (spans, _) = highlight_line("# comment", FileType::Shell, false);
    assert_eq!(spans[0].kind, SyntaxKind::Comment);
}

// -- JavaScript highlighting --

#[test]
fn js_keyword() {
    let (spans, _) = highlight_line("const x = 42;", FileType::JavaScript, false);
    assert_eq!(spans[0].kind, SyntaxKind::Keyword);
    assert_eq!(&"const x = 42;"[spans[0].start..spans[0].end], "const");
}

#[test]
fn js_template_literal() {
    let (spans, _) = highlight_line("let s = `hello`;", FileType::JavaScript, false);
    let tpl = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
    assert!(tpl.is_some());
}

// -- SyntaxTheme --

#[test]
fn theme_color_for_returns_correct_color() {
    let theme = SyntaxTheme::default();
    assert_eq!(theme.color_for(SyntaxKind::Keyword), theme.keyword);
    assert_eq!(theme.color_for(SyntaxKind::Normal), theme.normal);
    assert_eq!(theme.color_for(SyntaxKind::Comment), theme.comment);
}

// -- Empty lines --

#[test]
fn empty_line_returns_empty_spans() {
    for ft in [
        FileType::Rust,
        FileType::Toml,
        FileType::Html,
        FileType::Css,
        FileType::JavaScript,
        FileType::Markdown,
        FileType::Shell,
        FileType::Plain,
    ] {
        let (spans, _) = highlight_line("", ft, false);
        assert!(spans.is_empty(), "non-empty spans for {ft:?}");
    }
}

// -- Spans cover full line --

#[test]
fn rust_spans_cover_full_line() {
    let line = "fn main() { let x: u32 = 42; }";
    let (spans, _) = highlight_line(line, FileType::Rust, false);
    assert_spans_contiguous(&spans, line.len());
}

#[test]
fn toml_spans_cover_full_line() {
    let line = "name = \"oasis\"";
    let (spans, _) = highlight_line(line, FileType::Toml, false);
    assert_spans_contiguous(&spans, line.len());
}

#[test]
fn html_spans_cover_full_line() {
    let line = "<div class=\"x\">hello</div>";
    let (spans, _) = highlight_line(line, FileType::Html, false);
    assert_spans_contiguous(&spans, line.len());
}

fn assert_spans_contiguous(spans: &[ColorSpan], expected_len: usize) {
    if expected_len == 0 {
        assert!(spans.is_empty());
        return;
    }
    let mut expected_start = 0;
    for span in spans {
        assert_eq!(
            span.start, expected_start,
            "gap before span at {}",
            span.start
        );
        assert!(span.end > span.start, "empty span at {}", span.start);
        expected_start = span.end;
    }
    assert_eq!(expected_start, expected_len, "spans don't cover full line");
}

#[test]
fn c_preprocessor_keywords_and_block_comments() {
    let (spans, _) = highlight_line("#include <stdio.h>", FileType::C, false);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, SyntaxKind::Attribute);
    let (spans, _) = highlight_line("int main(void) {", FileType::C, false);
    assert_eq!(spans[0].kind, SyntaxKind::Type);
    let (_, open) = highlight_line("/* multi", FileType::C, false);
    assert!(open);
    // A '#' inside a block comment is not a directive.
    let (spans, open) = highlight_line("# still */ return", FileType::C, true);
    assert!(!open);
    assert_eq!(spans[0].kind, SyntaxKind::Comment);
}

// -- Every language terminates and tiles every line --

/// Lines that stress each scanner: lone `/` (CSS `font: 12px/1.5`,
/// `grid-area: 1 / 2` used to spin forever: `/` was neither an operator
/// nor a word byte, so the CSS scanner never advanced), unterminated
/// strings and comments, stray quotes, multi-byte text.
const TRICKY_LINES: &[&str] = &[
    "font: 12px/1.5 serif;",
    "grid-area: 1 / 2 / 3 / 4;",
    "a/b c/ /d / //x",
    "/",
    "*/ end of comment",
    "\"unterminated",
    "'single",
    "url(a/b.png) @media (min-width: 10em) { .x { y: 50% } }",
    "-- @ # $ ` ~ ^ & | \\ ! ? < > = + -",
    "<div a=\"1\" b='2' c=3>text</div><!-- c --> &amp;",
    "[section] key = \"v\" # comment",
    "**bold** _em_ `code` [link](http://x) # h",
    "echo \"$HOME\" 'x' # c | grep -v y > /dev/null",
    "fn é() { let s = \"ü\"; /* ö */ } // ß",
    "<p é=1 ü>Grüße</p> <!-- ö -->",
    "0x1F 1.5e3 .5 10px 99% 3.",
];

fn all_file_types() -> [FileType; 9] {
    [
        FileType::Rust,
        FileType::C,
        FileType::Toml,
        FileType::Html,
        FileType::Css,
        FileType::JavaScript,
        FileType::Markdown,
        FileType::Shell,
        FileType::Plain,
    ]
}

#[test]
fn every_highlighter_terminates_and_tiles_tricky_lines() {
    // On a worker thread so a scanner that stops advancing fails the test
    // instead of hanging the suite.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for ft in all_file_types() {
            for line in TRICKY_LINES {
                for open in [false, true] {
                    let (spans, _) = highlight_line(line, ft, open);
                    assert_spans_contiguous(&spans, line.len());
                    for s in &spans {
                        assert!(line.is_char_boundary(s.start) && line.is_char_boundary(s.end));
                    }
                }
            }
        }
        let _ = tx.send(());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(()) => {},
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("a highlighter panicked (see the thread's output)")
        },
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("a highlighter stopped advancing on one of TRICKY_LINES")
        },
    }
}

fn css_kinds(line: &str, open: bool) -> (Vec<(&str, SyntaxKind)>, bool) {
    let (spans, still_open) = highlight_line(line, FileType::Css, open);
    let kinds = spans
        .iter()
        .filter(|s| s.kind != SyntaxKind::Normal || !line[s.start..s.end].trim().is_empty())
        .map(|s| (&line[s.start..s.end], s.kind))
        .collect();
    (kinds, still_open)
}

#[test]
fn css_rule_tokens() {
    let (k, open) = css_kinds(
        "@media screen { .a { margin: 10px 50%; content: \"x\"; } }",
        false,
    );
    assert!(!open);
    assert_eq!(k[0], ("@media", SyntaxKind::Keyword));
    assert!(k.contains(&("10px", SyntaxKind::Number)));
    assert!(k.contains(&("50%", SyntaxKind::Number)));
    assert!(k.contains(&("\"x\"", SyntaxKind::StringLiteral)));
    assert!(k.contains(&("{", SyntaxKind::Operator)));
    assert!(k.contains(&("margin", SyntaxKind::Normal)));
    // `/` divides (`font` shorthand) and is an operator.
    let (k, _) = css_kinds("font: 12px/1.5 serif", false);
    assert!(k.contains(&("/", SyntaxKind::Operator)), "{k:?}");
    assert!(k.contains(&("1.5", SyntaxKind::Number)));
}

#[test]
fn css_block_comments_span_lines() {
    let (k, open) = css_kinds("a { color: red; } /* starts", false);
    assert!(open);
    assert_eq!(k.last().map(|t| t.1), Some(SyntaxKind::Comment));
    let (k, open) = css_kinds("still inside", true);
    assert!(open);
    assert_eq!(k, vec![("still inside", SyntaxKind::Comment)]);
    let (k, open) = css_kinds("ends */ b { }", true);
    assert!(!open);
    assert_eq!(k[0], ("ends */", SyntaxKind::Comment));
    assert_eq!(k[1], ("b", SyntaxKind::Normal));
}

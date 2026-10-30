//! Unit tests for the shell syntax highlighter.

use std::collections::HashSet;

use super::*;

/// Helper: build a `HashSet` from a slice of string slices.
fn cmd_set(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

/// Helper: collect `(text, kind)` pairs.
fn text_kinds<'a>(spans: &[HighlightSpan], input: &'a str) -> Vec<(&'a str, HighlightKind)> {
    spans
        .iter()
        .map(|s| (&input[s.start..s.end], s.kind))
        .collect()
}

// --- 1. Empty input ---

#[test]
fn empty_input() {
    let spans = highlight("", &cmd_set(&[]));
    assert!(spans.is_empty());
}

// --- 2. Simple valid command ---

#[test]
fn simple_valid_command() {
    let input = "echo";
    let spans = highlight(input, &cmd_set(&["echo"]));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, HighlightKind::ValidCommand);
    assert_eq!(&input[spans[0].start..spans[0].end], "echo");
}

// --- 3. Simple unknown command ---

#[test]
fn simple_unknown_command() {
    let input = "foobar";
    let spans = highlight(input, &cmd_set(&["echo"]));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, HighlightKind::UnknownCommand);
}

// --- 4. Command with arguments ---

#[test]
fn command_with_arguments() {
    let input = "echo hello world";
    let spans = highlight(input, &cmd_set(&["echo"]));
    let tk = text_kinds(&spans, input);
    assert_eq!(tk[0], ("echo", HighlightKind::ValidCommand));
    // whitespace
    assert_eq!(tk[1].1, HighlightKind::Normal);
    // "hello" is a normal argument
    assert_eq!(tk[2], ("hello", HighlightKind::Normal));
    assert_eq!(tk[4], ("world", HighlightKind::Normal));
}

// --- 5. Pipe chain with command checking ---

#[test]
fn pipe_chain_commands() {
    let input = "ls | grep foo | wc";
    let known = cmd_set(&["ls", "grep", "wc"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert_eq!(tk[0], ("ls", HighlightKind::ValidCommand));
    // |
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "|" && *k == HighlightKind::Pipe)
    );
    // grep should be valid command after pipe
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "grep" && *k == HighlightKind::ValidCommand)
    );
    // wc should be valid
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "wc" && *k == HighlightKind::ValidCommand)
    );
}

// --- 6. Unknown command after pipe ---

#[test]
fn unknown_command_after_pipe() {
    let input = "echo hi | nonexistent";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "nonexistent" && *k == HighlightKind::UnknownCommand)
    );
}

// --- 7. Single-quoted string ---

#[test]
fn single_quoted_string() {
    let input = "echo 'hello world'";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "'hello world'" && *k == HighlightKind::StringLiteral)
    );
}

// --- 8. Double-quoted string ---

#[test]
fn double_quoted_string() {
    let input = "echo \"hello world\"";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "\"hello world\"" && *k == HighlightKind::StringLiteral)
    );
}

// --- 9. Variable highlighting ---

#[test]
fn variable_dollar_var() {
    let input = "echo $HOME";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "$HOME" && *k == HighlightKind::Variable)
    );
}

#[test]
fn variable_braced() {
    let input = "echo ${USER}";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "${USER}" && *k == HighlightKind::Variable)
    );
}

#[test]
fn variable_special() {
    let input = "echo $? $# $0";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "$?" && *k == HighlightKind::Variable)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "$#" && *k == HighlightKind::Variable)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "$0" && *k == HighlightKind::Variable)
    );
}

// --- 10. Redirect operators ---

#[test]
fn redirect_operators() {
    let input = "echo hi > out.txt";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == ">" && *k == HighlightKind::Redirect)
    );
}

#[test]
fn redirect_append() {
    let input = "echo hi >> out.txt";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == ">>" && *k == HighlightKind::Redirect)
    );
}

#[test]
fn redirect_stderr() {
    let input = "cmd 2> err.log";
    let known = cmd_set(&["cmd"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "2>" && *k == HighlightKind::Redirect)
    );
}

// --- 11. Chain operators ---

#[test]
fn chain_operators() {
    let input = "echo a && echo b || echo c ; echo d";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "&&" && *k == HighlightKind::Chain)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "||" && *k == HighlightKind::Chain)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == ";" && *k == HighlightKind::Chain)
    );
    // Each `echo` after a chain should be ValidCommand.
    let echo_spans: Vec<_> = tk
        .iter()
        .filter(|(t, k)| *t == "echo" && *k == HighlightKind::ValidCommand)
        .collect();
    assert_eq!(echo_spans.len(), 4);
}

// --- 12. Comments ---

#[test]
fn comment_line() {
    let input = "# this is a comment";
    let spans = highlight(input, &cmd_set(&[]));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, HighlightKind::Comment);
    assert_eq!(&input[spans[0].start..spans[0].end], input);
}

#[test]
fn inline_comment() {
    let input = "echo hi #comment";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(tk.iter().any(|(_, k)| *k == HighlightKind::Comment));
}

// --- 13. Keywords ---

#[test]
fn keyword_in_command_position() {
    let input = "if true";
    let known = cmd_set(&["true"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert_eq!(tk[0], ("if", HighlightKind::Keyword));
}

#[test]
fn keyword_then_done() {
    let input = "then echo hi ; done";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert_eq!(tk[0], ("then", HighlightKind::Keyword));
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "done" && *k == HighlightKind::Keyword)
    );
}

#[test]
fn all_keywords_recognised() {
    for kw in KEYWORDS {
        assert!(is_keyword(kw), "{kw} should be recognised as a keyword");
    }
    assert!(!is_keyword("echo"));
    assert!(!is_keyword("ls"));
}

// --- 14. Glob patterns ---

#[test]
fn glob_patterns() {
    let input = "ls *.rs";
    let known = cmd_set(&["ls"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "*.rs" && *k == HighlightKind::Glob)
    );
}

#[test]
fn glob_question_mark() {
    let input = "ls file?.txt";
    let known = cmd_set(&["ls"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "file?.txt" && *k == HighlightKind::Glob)
    );
}

// --- 15. Flags ---

#[test]
fn flag_tokens() {
    let input = "grep -i -r pattern";
    let known = cmd_set(&["grep"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "-i" && *k == HighlightKind::Flag)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "-r" && *k == HighlightKind::Flag)
    );
}

#[test]
fn long_flag() {
    let input = "ls --all";
    let known = cmd_set(&["ls"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "--all" && *k == HighlightKind::Flag)
    );
}

// --- 16. Numeric literals ---

#[test]
fn numeric_literal() {
    let input = "sleep 5";
    let known = cmd_set(&["sleep"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "5" && *k == HighlightKind::Number)
    );
}

// --- 17. Mixed complex input ---

#[test]
fn mixed_complex_input() {
    let input = "echo \"hello $USER\" | grep -i test > output.txt";
    let known = cmd_set(&["echo", "grep"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);

    // echo is valid command
    assert_eq!(tk[0], ("echo", HighlightKind::ValidCommand));
    // double-quoted string
    assert!(
        tk.iter()
            .any(|(t, k)| t.starts_with('"') && *k == HighlightKind::StringLiteral)
    );
    // pipe
    assert!(tk.iter().any(|(_, k)| *k == HighlightKind::Pipe));
    // grep is valid command
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "grep" && *k == HighlightKind::ValidCommand)
    );
    // -i is flag
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "-i" && *k == HighlightKind::Flag)
    );
    // > is redirect
    assert!(
        tk.iter()
            .any(|(t, k)| *t == ">" && *k == HighlightKind::Redirect)
    );
}

// --- 18. Multiple chained commands with varying validity ---

#[test]
fn chained_valid_and_invalid() {
    let input = "echo hi ; badcmd arg ; ls";
    let known = cmd_set(&["echo", "ls"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);

    assert!(
        tk.iter()
            .any(|(t, k)| *t == "echo" && *k == HighlightKind::ValidCommand)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "badcmd" && *k == HighlightKind::UnknownCommand)
    );
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "ls" && *k == HighlightKind::ValidCommand)
    );
}

// --- 19. Spans cover full input ---

#[test]
fn spans_cover_full_input() {
    let input = "echo 'str' $VAR | grep -v 42 > out";
    let known = cmd_set(&["echo", "grep"]);
    let spans = highlight(input, &known);

    // Verify spans are contiguous and cover entire input.
    let mut expected_start = 0;
    for span in &spans {
        assert_eq!(
            span.start, expected_start,
            "gap before span at {}",
            span.start
        );
        assert!(span.end > span.start, "empty span");
        expected_start = span.end;
    }
    assert_eq!(expected_start, input.len());
}

// --- 20. Colorize produces correct segments ---

#[test]
fn colorize_produces_correct_text() {
    let input = "echo hello";
    let known = cmd_set(&["echo"]);
    let spans = highlight(input, &known);
    let theme = HighlightTheme::default();
    let colored = colorize(&spans, input, &theme);

    // Concatenation of all text segments should equal the input.
    let reconstructed: String = colored.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(reconstructed, input);

    // First segment (echo) should have valid_command color.
    assert_eq!(colored[0].1, theme.valid_command);
}

// --- 21. Light theme has different colors from dark ---

#[test]
fn light_theme_differs_from_dark() {
    let dark = HighlightTheme::default();
    let light = HighlightTheme::light();
    assert_ne!(dark.valid_command, light.valid_command);
    assert_ne!(dark.normal, light.normal);
}

// --- 22. is_keyword checks ---

#[test]
fn is_keyword_rejects_non_keywords() {
    assert!(!is_keyword("echo"));
    assert!(!is_keyword("grep"));
    assert!(!is_keyword(""));
    assert!(!is_keyword("IF")); // case-sensitive
}

// --- 23. Redirect input ---

#[test]
fn redirect_input() {
    let input = "sort < data.txt";
    let known = cmd_set(&["sort"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "<" && *k == HighlightKind::Redirect)
    );
}

// --- 24. Whitespace-only input ---

#[test]
fn whitespace_only() {
    let input = "   ";
    let spans = highlight(input, &cmd_set(&[]));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, HighlightKind::Normal);
}

// --- 25. 2>> redirect ---

#[test]
fn redirect_stderr_append() {
    let input = "cmd 2>> err.log";
    let known = cmd_set(&["cmd"]);
    let spans = highlight(input, &known);
    let tk = text_kinds(&spans, input);
    assert!(
        tk.iter()
            .any(|(t, k)| *t == "2>>" && *k == HighlightKind::Redirect)
    );
}

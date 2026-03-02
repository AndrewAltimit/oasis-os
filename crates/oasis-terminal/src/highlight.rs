//! Syntax highlighting for shell input.
//!
//! Tokenizes a shell input line and classifies each region by syntax role
//! (command, keyword, string, variable, operator, etc.). A theme maps each
//! role to a color so the terminal can render colored spans.

use std::collections::HashSet;

use oasis_types::backend::Color;

/// Syntax element types for shell input highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// Normal text / unknown.
    Normal,
    /// Valid command name (green).
    ValidCommand,
    /// Unknown command name (red).
    UnknownCommand,
    /// Shell builtin or keyword (blue).
    /// `if`, `then`, `else`, `fi`, `while`, `for`, `do`, `done`, `function`.
    Keyword,
    /// String literal in quotes.
    StringLiteral,
    /// Variable reference (`$VAR`, `${VAR}`).
    Variable,
    /// Pipe operator (`|`).
    Pipe,
    /// Redirect operator (`>`, `>>`, `2>`, etc.).
    Redirect,
    /// Chain operator (`&&`, `||`, `;`).
    Chain,
    /// Comment (`#` to end of line).
    Comment,
    /// Glob pattern (`*`, `?`, `[...]`).
    Glob,
    /// Argument/flag (starts with `-`).
    Flag,
    /// Numeric literal.
    Number,
}

/// A highlighted region of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Byte offset of the start of this span.
    pub start: usize,
    /// Byte offset of the end of this span (exclusive).
    pub end: usize,
    /// The syntax element type for this span.
    pub kind: HighlightKind,
}

/// Color mapping for each highlight kind.
pub struct HighlightTheme {
    /// Color for normal / unclassified text.
    pub normal: Color,
    /// Color for valid command names.
    pub valid_command: Color,
    /// Color for unknown command names.
    pub unknown_command: Color,
    /// Color for shell keywords.
    pub keyword: Color,
    /// Color for string literals.
    pub string_literal: Color,
    /// Color for variable references.
    pub variable: Color,
    /// Color for the pipe operator.
    pub pipe: Color,
    /// Color for redirect operators.
    pub redirect: Color,
    /// Color for chain operators.
    pub chain: Color,
    /// Color for comments.
    pub comment: Color,
    /// Color for glob patterns.
    pub glob: Color,
    /// Color for flags / options.
    pub flag: Color,
    /// Color for numeric literals.
    pub number: Color,
}

impl Default for HighlightTheme {
    /// Dark theme colors suitable for a dark terminal background.
    fn default() -> Self {
        Self {
            normal: Color::rgb(204, 204, 204),
            valid_command: Color::rgb(80, 200, 80),
            unknown_command: Color::rgb(220, 60, 60),
            keyword: Color::rgb(80, 140, 220),
            string_literal: Color::rgb(200, 170, 80),
            variable: Color::rgb(180, 120, 220),
            pipe: Color::rgb(200, 200, 80),
            redirect: Color::rgb(200, 200, 80),
            chain: Color::rgb(200, 200, 80),
            comment: Color::rgb(120, 120, 120),
            glob: Color::rgb(100, 200, 200),
            flag: Color::rgb(160, 200, 240),
            number: Color::rgb(220, 140, 80),
        }
    }
}

impl HighlightTheme {
    /// Light theme colors suitable for a light terminal background.
    pub fn light() -> Self {
        Self {
            normal: Color::rgb(40, 40, 40),
            valid_command: Color::rgb(0, 128, 0),
            unknown_command: Color::rgb(180, 0, 0),
            keyword: Color::rgb(0, 0, 180),
            string_literal: Color::rgb(160, 100, 0),
            variable: Color::rgb(120, 60, 180),
            pipe: Color::rgb(140, 140, 0),
            redirect: Color::rgb(140, 140, 0),
            chain: Color::rgb(140, 140, 0),
            comment: Color::rgb(140, 140, 140),
            glob: Color::rgb(0, 140, 140),
            flag: Color::rgb(60, 100, 160),
            number: Color::rgb(180, 80, 0),
        }
    }

    /// Return the color for a given highlight kind.
    pub fn color_for(&self, kind: HighlightKind) -> Color {
        match kind {
            HighlightKind::Normal => self.normal,
            HighlightKind::ValidCommand => self.valid_command,
            HighlightKind::UnknownCommand => self.unknown_command,
            HighlightKind::Keyword => self.keyword,
            HighlightKind::StringLiteral => self.string_literal,
            HighlightKind::Variable => self.variable,
            HighlightKind::Pipe => self.pipe,
            HighlightKind::Redirect => self.redirect,
            HighlightKind::Chain => self.chain,
            HighlightKind::Comment => self.comment,
            HighlightKind::Glob => self.glob,
            HighlightKind::Flag => self.flag,
            HighlightKind::Number => self.number,
        }
    }
}

/// Shell keywords recognised by the highlighter.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "do", "done", "for", "in", "case", "esac",
    "function", "return", "break", "continue",
];

/// Check whether `word` is a shell keyword.
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// Tokenize and highlight a shell input line.
///
/// `known_commands` should contain the names of all registered commands,
/// aliases, and functions so the highlighter can distinguish valid from
/// unknown command names.
///
/// Returns a list of non-overlapping spans covering the entire input.
pub fn highlight(input: &str, known_commands: &HashSet<String>) -> Vec<HighlightSpan> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let mut spans: Vec<HighlightSpan> = Vec::new();
    let mut pos: usize = 0;
    // `true` when the next non-whitespace token is a command position.
    let mut expect_command = true;

    while pos < len {
        // Skip whitespace, emitting Normal spans.
        if bytes[pos].is_ascii_whitespace() {
            let start = pos;
            while pos < len && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            spans.push(HighlightSpan {
                start,
                end: pos,
                kind: HighlightKind::Normal,
            });
            continue;
        }

        // Comment: `#` to end of line.
        if bytes[pos] == b'#' {
            spans.push(HighlightSpan {
                start: pos,
                end: len,
                kind: HighlightKind::Comment,
            });
            pos = len;
            continue;
        }

        // Single-quoted string.
        if bytes[pos] == b'\'' {
            let start = pos;
            pos += 1; // skip opening quote
            while pos < len && bytes[pos] != b'\'' {
                pos += 1;
            }
            if pos < len {
                pos += 1; // skip closing quote
            }
            spans.push(HighlightSpan {
                start,
                end: pos,
                kind: HighlightKind::StringLiteral,
            });
            expect_command = false;
            continue;
        }

        // Double-quoted string.
        if bytes[pos] == b'"' {
            let start = pos;
            pos += 1; // skip opening quote
            while pos < len && bytes[pos] != b'"' {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 2; // skip escaped char
                } else {
                    pos += 1;
                }
            }
            if pos < len {
                pos += 1; // skip closing quote
            }
            spans.push(HighlightSpan {
                start,
                end: pos,
                kind: HighlightKind::StringLiteral,
            });
            expect_command = false;
            continue;
        }

        // Variable: $VAR, ${VAR}, $?, $#, $0-$9
        if bytes[pos] == b'$' && pos + 1 < len {
            let start = pos;
            pos += 1; // skip $
            if pos < len && bytes[pos] == b'{' {
                pos += 1; // skip {
                while pos < len && bytes[pos] != b'}' {
                    pos += 1;
                }
                if pos < len {
                    pos += 1; // skip }
                }
            } else if pos < len
                && (bytes[pos] == b'?'
                    || bytes[pos] == b'#'
                    || bytes[pos] == b'@'
                    || bytes[pos] == b'!'
                    || bytes[pos].is_ascii_digit())
            {
                pos += 1; // single special char
            } else {
                // Bare name: alphanumeric and underscore.
                while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
            }
            // Only emit if we consumed more than the bare `$`.
            if pos > start + 1 {
                spans.push(HighlightSpan {
                    start,
                    end: pos,
                    kind: HighlightKind::Variable,
                });
            } else {
                spans.push(HighlightSpan {
                    start,
                    end: pos,
                    kind: HighlightKind::Normal,
                });
            }
            expect_command = false;
            continue;
        }

        // Pipe operator.
        if bytes[pos] == b'|' && !(pos + 1 < len && bytes[pos + 1] == b'|') {
            spans.push(HighlightSpan {
                start: pos,
                end: pos + 1,
                kind: HighlightKind::Pipe,
            });
            pos += 1;
            expect_command = true;
            continue;
        }

        // Chain operators: &&, ||, ;
        if bytes[pos] == b';' {
            spans.push(HighlightSpan {
                start: pos,
                end: pos + 1,
                kind: HighlightKind::Chain,
            });
            pos += 1;
            expect_command = true;
            continue;
        }
        if bytes[pos] == b'&' && pos + 1 < len && bytes[pos + 1] == b'&' {
            spans.push(HighlightSpan {
                start: pos,
                end: pos + 2,
                kind: HighlightKind::Chain,
            });
            pos += 2;
            expect_command = true;
            continue;
        }
        if bytes[pos] == b'|' && pos + 1 < len && bytes[pos + 1] == b'|' {
            spans.push(HighlightSpan {
                start: pos,
                end: pos + 2,
                kind: HighlightKind::Chain,
            });
            pos += 2;
            expect_command = true;
            continue;
        }

        // Redirect operators: >>, 2>>, 2>, >, <
        if let Some(redir_len) = try_redirect(bytes, pos) {
            spans.push(HighlightSpan {
                start: pos,
                end: pos + redir_len,
                kind: HighlightKind::Redirect,
            });
            pos += redir_len;
            expect_command = false;
            continue;
        }

        // Regular token (word). Collect until whitespace or operator.
        let start = pos;
        while pos < len && !is_token_break(bytes, pos) {
            if bytes[pos] == b'\\' && pos + 1 < len {
                pos += 2; // skip escaped char
            } else {
                pos += 1;
            }
        }
        let word = &input[start..pos];

        let kind = classify_word(word, expect_command, known_commands);
        spans.push(HighlightSpan {
            start,
            end: pos,
            kind,
        });
        // After the first token in a command position, subsequent tokens
        // are arguments until a chain/pipe resets the state.
        if expect_command {
            expect_command = false;
        }
    }

    spans
}

/// Try to match a redirect operator at `pos`. Returns the length of the
/// operator in bytes, or `None` if no redirect is found.
fn try_redirect(bytes: &[u8], pos: usize) -> Option<usize> {
    let len = bytes.len();
    // 2>> (3 chars)
    if pos + 2 < len && bytes[pos] == b'2' && bytes[pos + 1] == b'>' && bytes[pos + 2] == b'>' {
        return Some(3);
    }
    // 2> (2 chars)
    if pos + 1 < len && bytes[pos] == b'2' && bytes[pos + 1] == b'>' {
        return Some(2);
    }
    // >> (2 chars)
    if pos + 1 < len && bytes[pos] == b'>' && bytes[pos + 1] == b'>' {
        return Some(2);
    }
    // > (1 char)
    if bytes[pos] == b'>' {
        return Some(1);
    }
    // < (1 char)
    if bytes[pos] == b'<' {
        return Some(1);
    }
    None
}

/// Check whether `pos` is at a token boundary character.
fn is_token_break(bytes: &[u8], pos: usize) -> bool {
    let b = bytes[pos];
    let len = bytes.len();

    if b.is_ascii_whitespace() {
        return true;
    }
    // Single-char operators that always break a token.
    if matches!(b, b'|' | b';' | b'\'' | b'"' | b'#' | b'<') {
        return true;
    }
    // `>` always breaks (redirect).
    if b == b'>' {
        return true;
    }
    // `&` followed by `&` is a chain; bare `&` also breaks.
    if b == b'&' {
        return true;
    }
    // `$` starts a variable reference.
    if b == b'$' {
        return true;
    }
    // `2>` redirect while inside a word: only break when followed by `>`.
    if b == b'2' && pos + 1 < len && bytes[pos + 1] == b'>' {
        return true;
    }
    false
}

/// Classify a bare word token.
fn classify_word(
    word: &str,
    is_command_position: bool,
    known_commands: &HashSet<String>,
) -> HighlightKind {
    if is_command_position {
        if is_keyword(word) {
            return HighlightKind::Keyword;
        }
        if known_commands.contains(word) {
            return HighlightKind::ValidCommand;
        }
        return HighlightKind::UnknownCommand;
    }

    // Keywords can appear in non-command positions too
    // (e.g. `then`, `do`, `done` after a command).
    if is_keyword(word) {
        return HighlightKind::Keyword;
    }

    // Flags: tokens starting with `-`.
    if word.starts_with('-') && word.len() > 1 {
        return HighlightKind::Flag;
    }

    // Glob patterns: contain `*`, `?`, or `[`.
    if word.contains('*') || word.contains('?') || word.contains('[') {
        return HighlightKind::Glob;
    }

    // Pure numeric literal.
    if is_numeric(word) {
        return HighlightKind::Number;
    }

    HighlightKind::Normal
}

/// Check whether a word is a pure numeric literal.
///
/// Accepts optional leading sign and digits, including a single decimal
/// point (e.g. `42`, `-7`, `3.14`).
fn is_numeric(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let bytes = word.as_bytes();
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        // A bare sign is not a number (and `-flag` handled before this).
        if bytes.len() == 1 {
            return false;
        }
        i = 1;
    }
    let mut saw_dot = false;
    let mut saw_digit = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            saw_digit = true;
        } else if bytes[i] == b'.' && !saw_dot {
            saw_dot = true;
        } else {
            return false;
        }
        i += 1;
    }
    saw_digit
}

/// Convert highlight spans into colored text segments.
///
/// Each entry in the returned vector is a `(text, color)` pair. The
/// concatenation of all text segments reproduces the original `input`.
pub fn colorize(
    spans: &[HighlightSpan],
    input: &str,
    theme: &HighlightTheme,
) -> Vec<(String, Color)> {
    spans
        .iter()
        .map(|span| {
            let text = input.get(span.start..span.end).unwrap_or("").to_string();
            let color = theme.color_for(span.kind);
            (text, color)
        })
        .collect()
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}

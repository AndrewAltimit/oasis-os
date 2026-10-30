//! The shell-input tokenizer behind [`highlight`]: splits a line into
//! [`HighlightSpan`]s and classifies each word.

use std::collections::HashSet;

use super::{HighlightKind, HighlightSpan, is_keyword};

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

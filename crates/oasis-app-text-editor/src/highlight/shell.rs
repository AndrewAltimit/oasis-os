//! Shell script highlighter.

use super::scan::*;
use super::{ColorSpan, SyntaxKind};

pub(super) const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "do", "done", "for", "in", "case", "esac",
    "function", "return", "break", "continue", "export", "source", "local", "readonly", "declare",
    "set", "unset", "shift", "exit", "exec", "eval", "trap",
];

/// Scan a shell variable reference: `$VAR` or `${VAR}`.
/// `pos` must point at the `$`. Returns the position after the variable.
pub(super) fn scan_shell_variable(bytes: &[u8], pos: usize) -> usize {
    let len = bytes.len();
    let mut p = pos + 1;
    if p < len && bytes[p] == b'{' {
        p += 1;
        while p < len && bytes[p] != b'}' {
            p += 1;
        }
        if p < len {
            p += 1;
        }
    } else {
        while p < len && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_') {
            p += 1;
        }
    }
    p
}

/// Scan a shell operator (|, ;, &&, ||, >, <, >>).
/// Returns the position after the operator.
pub(super) fn scan_shell_operator(bytes: &[u8], pos: usize) -> usize {
    let len = bytes.len();
    let b = bytes[pos];
    let mut p = pos + 1;
    if p < len
        && ((b == b'&' && bytes[p] == b'&')
            || (b == b'|' && bytes[p] == b'|')
            || (b == b'>' && bytes[p] == b'>'))
    {
        p += 1;
    }
    p
}

pub(super) fn highlight_shell(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Comment.
        if bytes[pos] == b'#' {
            spans.push(ColorSpan {
                start: pos,
                end: len,
                kind: SyntaxKind::Comment,
            });
            pos = len;
            continue;
        }

        // String: double-quoted (with escapes), single-quoted (no escapes).
        if matches!(bytes[pos], b'"' | b'\'') {
            let start = pos;
            let with_escapes = bytes[pos] == b'"';
            pos = scan_quoted(bytes, pos, bytes[pos], with_escapes);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Variable: $VAR, ${VAR}.
        if bytes[pos] == b'$' && pos + 1 < len {
            let start = pos;
            pos = scan_shell_variable(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Attribute,
            });
            continue;
        }

        // Whitespace.
        if bytes[pos].is_ascii_whitespace() {
            let start = pos;
            pos = scan_whitespace(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
            continue;
        }

        // Operators: |, ;, &&, ||, >, <, >>.
        if matches!(bytes[pos], b'|' | b';' | b'&' | b'>' | b'<') {
            let start = pos;
            pos = scan_shell_operator(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Operator,
            });
            continue;
        }

        // Word (identifier / keyword).
        if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' || bytes[pos] == b'-' {
            let start = pos;
            pos = scan_word(bytes, pos, b"-");
            let word = &line[start..pos];
            let kind = if SHELL_KEYWORDS.contains(&word) {
                SyntaxKind::Keyword
            } else {
                SyntaxKind::Normal
            };
            spans.push(ColorSpan {
                start,
                end: pos,
                kind,
            });
            continue;
        }

        // Fallback: single character (advance by full UTF-8 char width).
        let end = advance_utf8_char(line, pos);
        spans.push(ColorSpan {
            start: pos,
            end,
            kind: SyntaxKind::Normal,
        });
        pos = end;
    }

    spans
}

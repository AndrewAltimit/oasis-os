//! CSS highlighter.

use super::scan::*;
use super::{ColorSpan, SyntaxKind};

pub(super) const CSS_AT_RULES: &[&str] = &[
    "@media",
    "@import",
    "@keyframes",
    "@font-face",
    "@charset",
    "@supports",
    "@namespace",
    "@page",
    "@layer",
];

pub(super) fn highlight_css(line: &str, mut in_block_comment: bool) -> (Vec<ColorSpan>, bool) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return (Vec::new(), in_block_comment);
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        if in_block_comment {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, false);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Block comment.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, true);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // At-rule.
        if bytes[pos] == b'@' {
            let start = pos;
            pos += 1;
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'-') {
                pos += 1;
            }
            // All @-rules are highlighted as keywords.
            let _ = CSS_AT_RULES; // keyword list available for future refinement
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Keyword,
            });
            continue;
        }

        // String.
        if matches!(bytes[pos], b'"' | b'\'') {
            let start = pos;
            pos = scan_quoted(bytes, pos, bytes[pos], true);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Number (including units like `10px`).
        if bytes[pos].is_ascii_digit()
            || (bytes[pos] == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            let start = pos;
            while pos < len && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
                pos += 1;
            }
            // CSS units.
            while pos < len && bytes[pos].is_ascii_alphabetic() {
                pos += 1;
            }
            // Percent.
            if pos < len && bytes[pos] == b'%' {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Number,
            });
            continue;
        }

        // Braces / operators. A `/` that does not open a comment divides
        // (`font: 12px/1.5`, `grid-area: 1 / 2`); it ends words below, so
        // without this arm the scanner never advanced past it.
        if matches!(
            bytes[pos],
            b'{' | b'}' | b':' | b';' | b',' | b'(' | b')' | b'/'
        ) {
            spans.push(ColorSpan {
                start: pos,
                end: pos + 1,
                kind: SyntaxKind::Operator,
            });
            pos += 1;
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

        // Selector / property / value word.
        let start = pos;
        while pos < len
            && !bytes[pos].is_ascii_whitespace()
            && !matches!(
                bytes[pos],
                b'{' | b'}' | b':' | b';' | b',' | b'(' | b')' | b'"' | b'\'' | b'/'
            )
        {
            pos += 1;
        }
        if pos > start {
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
        }
    }

    (spans, in_block_comment)
}

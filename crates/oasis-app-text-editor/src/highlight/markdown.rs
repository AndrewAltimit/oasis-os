//! Markdown highlighter.

use super::{ColorSpan, SyntaxKind};

pub(super) fn highlight_markdown(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = line.trim_start();

    // Heading: lines starting with `#`.
    if trimmed.starts_with('#') {
        return vec![ColorSpan {
            start: 0,
            end: len,
            kind: SyntaxKind::Heading,
        }];
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Inline code: `...`.
        if bytes[pos] == b'`' {
            let start = pos;
            pos += 1;
            while pos < len && bytes[pos] != b'`' {
                pos += 1;
            }
            if pos < len {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::CodeSpan,
            });
            continue;
        }

        // Bold/italic markers: `**`, `__`, `*`, `_`.
        if bytes[pos] == b'*' || bytes[pos] == b'_' {
            let marker = bytes[pos];
            let start = pos;
            while pos < len && bytes[pos] == marker {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Emphasis,
            });
            continue;
        }

        // Link: `[text](url)`.
        if bytes[pos] == b'[' {
            let start = pos;
            pos += 1;
            while pos < len && bytes[pos] != b']' {
                pos += 1;
            }
            if pos < len {
                pos += 1; // `]`
            }
            if pos < len && bytes[pos] == b'(' {
                pos += 1;
                while pos < len && bytes[pos] != b')' {
                    pos += 1;
                }
                if pos < len {
                    pos += 1;
                }
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Link,
            });
            continue;
        }

        // Regular text.
        let start = pos;
        while pos < len && !matches!(bytes[pos], b'`' | b'*' | b'_' | b'[') {
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

    spans
}

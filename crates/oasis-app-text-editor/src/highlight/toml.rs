//! TOML highlighter.

use super::{ColorSpan, SyntaxKind};

pub(super) fn highlight_toml(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = line.trim_start();
    let leading_ws = len - trimmed.len();

    // Comment line.
    if trimmed.starts_with('#') {
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(ColorSpan {
                start: 0,
                end: leading_ws,
                kind: SyntaxKind::Normal,
            });
        }
        spans.push(ColorSpan {
            start: leading_ws,
            end: len,
            kind: SyntaxKind::Comment,
        });
        return spans;
    }

    // Section header: `[section]` or `[[array]]`.
    if trimmed.starts_with('[') {
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(ColorSpan {
                start: 0,
                end: leading_ws,
                kind: SyntaxKind::Normal,
            });
        }
        spans.push(ColorSpan {
            start: leading_ws,
            end: len,
            kind: SyntaxKind::Section,
        });
        return spans;
    }

    // Key = value line.
    if let Some(eq_pos) = line.find('=') {
        let mut spans = Vec::new();
        // Key part (before `=`).
        spans.push(ColorSpan {
            start: 0,
            end: eq_pos,
            kind: SyntaxKind::Keyword,
        });
        // `=` sign.
        spans.push(ColorSpan {
            start: eq_pos,
            end: eq_pos + 1,
            kind: SyntaxKind::Operator,
        });
        // Value part (after `=`).
        let rest = &line[eq_pos + 1..];
        let rest_trimmed = rest.trim_start();
        let val_start = eq_pos + 1 + (rest.len() - rest_trimmed.len());
        if val_start < len {
            // Whitespace between = and value.
            if val_start > eq_pos + 1 {
                spans.push(ColorSpan {
                    start: eq_pos + 1,
                    end: val_start,
                    kind: SyntaxKind::Normal,
                });
            }
            let kind = if rest_trimmed.starts_with('"') || rest_trimmed.starts_with('\'') {
                SyntaxKind::StringLiteral
            } else if rest_trimmed == "true" || rest_trimmed == "false" {
                SyntaxKind::Keyword
            } else if rest_trimmed
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
            {
                SyntaxKind::Number
            } else {
                SyntaxKind::Normal
            };
            spans.push(ColorSpan {
                start: val_start,
                end: len,
                kind,
            });
        }
        return spans;
    }

    vec![ColorSpan {
        start: 0,
        end: len,
        kind: SyntaxKind::Normal,
    }]
}

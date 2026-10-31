//! Rust, C / C++ and JavaScript: one C-family scanner with per-language
//! keyword tables.

use super::scan::*;
use super::{ColorSpan, SyntaxKind};

pub(super) const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

pub(super) const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    "HashMap", "HashSet", "BTreeMap", "BTreeSet",
];

pub(super) const C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "class",
    "const",
    "constexpr",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "extern",
    "false",
    "for",
    "goto",
    "if",
    "inline",
    "namespace",
    "new",
    "nullptr",
    "private",
    "protected",
    "public",
    "register",
    "return",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "true",
    "typedef",
    "typename",
    "union",
    "using",
    "virtual",
    "volatile",
    "while",
    "NULL",
];

pub(super) const C_TYPES: &[&str] = &[
    "bool",
    "char",
    "double",
    "float",
    "int",
    "long",
    "short",
    "signed",
    "unsigned",
    "void",
    "size_t",
    "ssize_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "uintptr_t",
    "FILE",
];

pub(super) const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

pub(super) fn highlight_c_family(
    line: &str,
    keywords: &[&str],
    types: &[&str],
    mut in_block_comment: bool,
) -> (Vec<ColorSpan>, bool) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return (Vec::new(), in_block_comment);
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Inside a block comment: scan for `*/`.
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

        // Line comment: `//` to end of line.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            spans.push(ColorSpan {
                start: pos,
                end: len,
                kind: SyntaxKind::Comment,
            });
            pos = len;
            continue;
        }

        // Block comment start: `/*`.
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

        // Attribute: `#[...]` or `#![...]` (Rust).
        if bytes[pos] == b'#'
            && pos + 1 < len
            && (bytes[pos + 1] == b'['
                || (bytes[pos + 1] == b'!' && pos + 2 < len && bytes[pos + 2] == b'['))
        {
            let start = pos;
            // Find matching `]`.
            let mut depth = 0;
            while pos < len {
                if bytes[pos] == b'[' {
                    depth += 1;
                } else if bytes[pos] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        pos += 1;
                        break;
                    }
                }
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Attribute,
            });
            continue;
        }

        // String literals: double-quoted, single-quoted, backtick (template).
        if matches!(bytes[pos], b'"' | b'\'' | b'`') {
            let start = pos;
            pos = scan_quoted(bytes, pos, bytes[pos], true);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Numeric literal.
        if bytes[pos].is_ascii_digit()
            || (bytes[pos] == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            let start = pos;
            pos = scan_c_number(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Number,
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

        // Operator / punctuation.
        if is_operator(bytes[pos]) {
            let start = pos;
            while pos < len && is_operator(bytes[pos]) {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Operator,
            });
            continue;
        }

        // Word (identifier / keyword).
        if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' {
            let start = pos;
            pos = scan_word(bytes, pos, &[]);
            let word = &line[start..pos];
            let kind = if keywords.contains(&word) {
                SyntaxKind::Keyword
            } else if types.contains(&word) {
                SyntaxKind::Type
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

    (spans, in_block_comment)
}

/// C / C++: preprocessor lines (`#include`, `#define`, ...) are one
/// attribute span; everything else uses the C-family scanner.
pub(super) fn highlight_c(line: &str, in_block_comment: bool) -> (Vec<ColorSpan>, bool) {
    if !in_block_comment && line.trim_start().starts_with('#') {
        let span = ColorSpan {
            start: 0,
            end: line.len(),
            kind: SyntaxKind::Attribute,
        };
        return (vec![span], false);
    }
    highlight_c_family(line, C_KEYWORDS, C_TYPES, in_block_comment)
}

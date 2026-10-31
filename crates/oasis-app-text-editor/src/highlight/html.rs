//! HTML highlighter.

use super::{ColorSpan, SyntaxKind};

pub(super) fn highlight_html(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // HTML comment: `<!-- ... -->`.
        // Compared as bytes: slicing `line` four bytes on from an
        // arbitrary position panicked inside a multi-byte character.
        if bytes[pos..].starts_with(b"<!--") {
            let start = pos;
            if let Some(end_pos) = line[pos..].find("-->") {
                pos += end_pos + 3;
            } else {
                pos = len;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Tag: `<...>`.
        if bytes[pos] == b'<' {
            let start = pos;
            pos += 1;
            // Skip `/` for closing tags.
            if pos < len && bytes[pos] == b'/' {
                pos += 1;
            }
            // Tag name.
            let tag_name_start = pos;
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'-') {
                pos += 1;
            }
            if pos > tag_name_start {
                // Opening bracket + optional slash + tag name.
                spans.push(ColorSpan {
                    start,
                    end: pos,
                    kind: SyntaxKind::Tag,
                });
            } else {
                spans.push(ColorSpan {
                    start,
                    end: pos.max(start + 1),
                    kind: SyntaxKind::Operator,
                });
            }

            // Attributes within the tag until `>`.
            while pos < len && bytes[pos] != b'>' {
                if bytes[pos].is_ascii_whitespace() {
                    let ws_start = pos;
                    while pos < len && bytes[pos].is_ascii_whitespace() {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: ws_start,
                        end: pos,
                        kind: SyntaxKind::Normal,
                    });
                } else if bytes[pos] == b'=' {
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + 1,
                        kind: SyntaxKind::Operator,
                    });
                    pos += 1;
                } else if bytes[pos] == b'"' || bytes[pos] == b'\'' {
                    let q = bytes[pos];
                    let str_start = pos;
                    pos += 1;
                    while pos < len && bytes[pos] != q {
                        pos += 1;
                    }
                    if pos < len {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: str_start,
                        end: pos,
                        kind: SyntaxKind::StringLiteral,
                    });
                } else if bytes[pos] == b'/' {
                    // Self-closing slash.
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + 1,
                        kind: SyntaxKind::Tag,
                    });
                    pos += 1;
                } else if bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'-'
                    || bytes[pos] == b'_'
                {
                    let attr_start = pos;
                    while pos < len
                        && (bytes[pos].is_ascii_alphanumeric()
                            || bytes[pos] == b'-'
                            || bytes[pos] == b'_')
                    {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: attr_start,
                        end: pos,
                        kind: SyntaxKind::TagAttribute,
                    });
                } else {
                    // One whole character: spans must end on a char
                    // boundary (the renderer slices the line with them).
                    let ch_len = line[pos..].chars().next().map_or(1, char::len_utf8);
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + ch_len,
                        kind: SyntaxKind::Normal,
                    });
                    pos += ch_len;
                }
            }
            // Closing `>`.
            if pos < len && bytes[pos] == b'>' {
                spans.push(ColorSpan {
                    start: pos,
                    end: pos + 1,
                    kind: SyntaxKind::Tag,
                });
                pos += 1;
            }
            continue;
        }

        // Plain text between tags.
        let start = pos;
        while pos < len && bytes[pos] != b'<' {
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

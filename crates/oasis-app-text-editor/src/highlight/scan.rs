//! Byte-level scanners shared by the per-language highlighters.

/// Scan a quoted string starting at `pos` (which must point to the opening
/// quote character). Handles backslash escapes when `with_escapes` is true.
/// Returns the position after the closing quote (or end of bytes).
pub(super) fn scan_quoted(bytes: &[u8], pos: usize, quote: u8, with_escapes: bool) -> usize {
    let len = bytes.len();
    let mut p = pos + 1;
    while p < len && bytes[p] != quote {
        if with_escapes && bytes[p] == b'\\' && p + 1 < len {
            p += 2;
        } else {
            p += 1;
        }
    }
    if p < len {
        p + 1 // skip closing quote
    } else {
        p
    }
}

/// Scan a `/* ... */` block comment. `pos` should point to the first byte
/// to scan (either the `/*` opener or continuation from previous line).
/// When `include_opener` is true, `pos` points at `/*` and we skip 2 bytes.
/// Returns `(end_pos, still_in_comment)`.
pub(super) fn scan_block_comment(
    bytes: &[u8],
    mut pos: usize,
    include_opener: bool,
) -> (usize, bool) {
    let len = bytes.len();
    if include_opener {
        pos += 2;
    }
    loop {
        if pos + 1 < len && bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
            return (pos + 2, false);
        }
        pos += 1;
        if pos >= len {
            return (len, true);
        }
    }
}

/// Scan contiguous ASCII whitespace starting at `pos`.
/// Returns the position after the last whitespace byte.
pub(super) fn scan_whitespace(bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    while pos < len && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

/// Scan a C-family numeric literal (decimal, hex, float with type suffix).
/// Returns the position after the number.
pub(super) fn scan_c_number(bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    // Hex: 0x...
    if bytes[pos] == b'0' && pos + 1 < len && (bytes[pos + 1] == b'x' || bytes[pos + 1] == b'X') {
        pos += 2;
        while pos < len && (bytes[pos].is_ascii_hexdigit() || bytes[pos] == b'_') {
            pos += 1;
        }
    } else {
        while pos < len
            && (bytes[pos].is_ascii_digit()
                || bytes[pos] == b'.'
                || bytes[pos] == b'_'
                || bytes[pos] == b'e'
                || bytes[pos] == b'E')
        {
            pos += 1;
        }
    }
    // Type suffix (e.g., `42u32`, `1.0f64`).
    while pos < len && bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    pos
}

/// Scan an identifier/word. Returns the position after the word.
/// `extra_chars` lists additional bytes (beyond alphanumeric/underscore)
/// that are part of a word (e.g., `b'-'` for shell).
pub(super) fn scan_word(bytes: &[u8], mut pos: usize, extra_chars: &[u8]) -> usize {
    let len = bytes.len();
    while pos < len
        && (bytes[pos].is_ascii_alphanumeric()
            || bytes[pos] == b'_'
            || extra_chars.contains(&bytes[pos]))
    {
        pos += 1;
    }
    pos
}

/// Advance past a single UTF-8 character, returning the position after it.
pub(super) fn advance_utf8_char(line: &str, pos: usize) -> usize {
    pos + line[pos..].chars().next().map_or(1, |c| c.len_utf8())
}

pub(super) fn is_operator(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'!'
            | b'<'
            | b'>'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b';'
            | b':'
            | b','
            | b'.'
            | b'?'
            | b'@'
    )
}

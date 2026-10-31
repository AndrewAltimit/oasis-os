//! UTF-8 char-boundary, word-motion and common-prefix helpers used by
//! the line editor.

/// Return the byte offset of the previous character boundary, or 0.
pub(super) fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Return the byte offset of the next character boundary, or `s.len()`.
pub(super) fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// True if `ch` is a word-separating character.
fn is_word_sep(ch: char) -> bool {
    ch.is_whitespace() || ch.is_ascii_punctuation()
}

/// Find the byte offset of the start of the word to the left of `pos`.
pub(super) fn word_start(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let bytes = s.as_bytes();
    let mut p = prev_char_boundary(s, pos);

    // Skip trailing separators.
    while p > 0 && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if !is_word_sep(ch) {
            break;
        }
        p = prev_char_boundary(s, p);
    }

    // Walk back through the word body.
    loop {
        if p == 0 {
            break;
        }
        let prev = prev_char_boundary(s, p);
        let ch = char_at(bytes, prev);
        if is_word_sep(ch) {
            break;
        }
        p = prev;
    }

    p
}

/// Find the byte offset of the end of the word to the right of `pos`.
pub(super) fn word_end(s: &str, pos: usize) -> usize {
    let len = s.len();
    if pos >= len {
        return len;
    }
    let bytes = s.as_bytes();
    let mut p = pos;

    // Skip leading separators.
    while p < len && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if !is_word_sep(ch) {
            break;
        }
        p = next_char_boundary(s, p);
    }

    // Walk forward through the word body.
    while p < len && s.is_char_boundary(p) {
        let ch = char_at(bytes, p);
        if is_word_sep(ch) {
            break;
        }
        p = next_char_boundary(s, p);
    }

    p
}

/// Read the char that starts at byte offset `pos` in the given bytes.
///
/// Assumes `pos` is a valid char boundary.
fn char_at(bytes: &[u8], pos: usize) -> char {
    // Decode one UTF-8 character starting at `pos`.
    let rest = &bytes[pos..];
    let s = core::str::from_utf8(rest).unwrap_or("\0");
    s.chars().next().unwrap_or('\0')
}

/// Find the longest common prefix among a set of strings.
pub(super) fn common_prefix(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let first = &candidates[0];
    let mut prefix_len = first.len();
    for c in &candidates[1..] {
        prefix_len = prefix_len.min(c.len());
        for (i, (a, b)) in first.bytes().zip(c.bytes()).enumerate() {
            if a != b {
                prefix_len = prefix_len.min(i);
                break;
            }
        }
    }
    // Make sure we land on a char boundary.
    let bounded = first.floor_char_boundary(prefix_len);
    Some(first[..bounded].to_string())
}

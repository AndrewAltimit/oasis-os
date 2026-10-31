//! Input-scanning and path helpers shared by the completers.

use super::{CompletionContext, CompletionResult, longest_common_prefix};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a `CompletionResult` from a partial string and candidate list.
pub(super) fn build_result(partial: &str, candidates: &[String]) -> CompletionResult {
    match candidates.len() {
        0 => CompletionResult {
            replacement: partial.to_string(),
            start: 0,
            end: partial.len(),
            candidates: Vec::new(),
            is_complete: false,
        },
        1 => CompletionResult {
            replacement: candidates[0].clone(),
            start: 0,
            end: partial.len(),
            candidates: candidates.to_vec(),
            is_complete: true,
        },
        _ => {
            let prefix = longest_common_prefix(candidates);
            CompletionResult {
                replacement: prefix,
                start: 0,
                end: partial.len(),
                candidates: candidates.to_vec(),
                is_complete: false,
            }
        },
    }
}

/// Extract the partial text that should be completed, plus its start offset
/// in the input.
pub(super) fn extract_partial(
    input: &str,
    cursor: usize,
    context: &CompletionContext,
) -> (String, usize) {
    let before = &input[..cursor];

    match context {
        CompletionContext::Variable => {
            // Walk backwards from cursor to find the `$` or `${`.
            if let Some(dollar_pos) = find_variable_start(before) {
                let after_dollar = dollar_pos + 1;
                let skip_brace = if before.as_bytes().get(after_dollar) == Some(&b'{') {
                    after_dollar + 1
                } else {
                    after_dollar
                };
                let partial = &before[skip_brace..];
                (partial.to_string(), skip_brace)
            } else {
                (String::new(), cursor)
            }
        },
        CompletionContext::Command => {
            // The first token up to cursor.
            let start = 0;
            let partial = before.trim_start();
            let trimmed_start = before.len() - partial.len();
            (partial.to_string(), trimmed_start + start)
        },
        CompletionContext::FilePath => {
            // Find the start of the current argument (last unquoted space).
            let arg_start = last_arg_start(before);
            let partial = &before[arg_start..];
            (partial.to_string(), arg_start)
        },
        CompletionContext::None => (String::new(), cursor),
    }
}

/// Find the byte offset of the `$` that starts a variable reference
/// touching the end of `before`, or `None` if we are not in a variable
/// context.
pub(super) fn find_variable_start(before: &str) -> Option<usize> {
    let bytes = before.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    // Scan backwards from end.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'$' => return Some(i),
            // Inside `${...}` -- keep scanning past identifier chars and `{`.
            b'{' if i > 0 && bytes[i - 1] == b'$' => return Some(i - 1),
            c if c.is_ascii_alphanumeric() || c == b'_' => continue,
            _ => return None,
        }
    }
    None
}

/// Find the start byte offset of the last argument: after the last
/// whitespace that is not quoted or backslash-escaped.
pub(super) fn last_arg_start(before: &str) -> usize {
    let (mut in_single, mut in_double, mut escaped) = (false, false, false);
    let mut start = 0;
    for (i, c) in before.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => escaped = true,
            ' ' | '\t' if !in_single && !in_double => start = i + 1,
            _ => {},
        }
    }
    start
}

/// An argument's typed text with quotes and escapes removed, and the
/// quote it is still open in (`"my f` -> (`my f`, Some('"'))).
pub(super) fn unquote_word(raw: &str) -> (String, Option<char>) {
    let mut out = String::with_capacity(raw.len());
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => out.extend(chars.next()),
            (Some(_), c) => out.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => out.extend(chars.next()),
            (None, c) => out.push(c),
        }
    }
    (out, quote)
}

/// Characters a completed path escapes outside quotes.
const SHELL_SPECIAL: &[char] = &[
    ' ', '\t', '\'', '"', '\\', '$', '&', '|', ';', '<', '>', '(', ')', '*', '?', '[', '#', '`',
    '{', '}', '~',
];

/// Encode completed path text back into shell syntax: inside the quote
/// `quote` the word reopens it (and closes it when `complete` and not a
/// directory); unquoted, specials are backslash-escaped.
pub(super) fn requote(text: &str, quote: Option<char>, complete: bool) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    match quote {
        Some(q) => {
            out.push(q);
            for c in text.chars() {
                if q == '"' && matches!(c, '"' | '\\' | '$' | '`') {
                    out.push('\\');
                }
                // A `'` cannot appear inside single quotes: close, escape,
                // reopen.
                if q == '\'' && c == '\'' {
                    out.push_str("'\\''");
                    continue;
                }
                out.push(c);
            }
            if complete && !text.ends_with('/') {
                out.push(q);
            }
        },
        None => {
            for c in text.chars() {
                if SHELL_SPECIAL.contains(&c) {
                    out.push('\\');
                }
                out.push(c);
            }
        },
    }
    out
}

/// Split a user-typed path into (directory_prefix, filename_prefix).
///
/// Examples:
/// - `"foo"` -> `("", "foo")`
/// - `"/home/us"` -> `("/home/", "us")`
/// - `"sub/"` -> `("sub/", "")`
/// - `""` -> `("", "")`
pub(super) fn split_path(path: &str) -> (String, String) {
    match path.rfind('/') {
        Some(pos) => {
            let dir = &path[..=pos];
            let file = &path[pos + 1..];
            (dir.to_string(), file.to_string())
        },
        None => (String::new(), path.to_string()),
    }
}

/// Resolve a relative path against cwd (simplified, no `.` / `..` handling
/// beyond basic normalization).
pub(super) fn resolve_simple(cwd: &str, relative: &str) -> String {
    let combined = if cwd == "/" {
        format!("/{relative}")
    } else {
        format!("{cwd}/{relative}")
    };
    normalize_path(&combined)
}

/// Normalize a path: collapse `.`, `..`, and double slashes.
pub(super) fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {},
            ".." => {
                parts.pop();
            },
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

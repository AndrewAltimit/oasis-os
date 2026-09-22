//! Token expansion: tokenization, brace expansion, glob expansion, and path resolution.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

/// Tokenize a command line respecting quotes and backslash escapes.
///
/// - Single-quoted strings preserve all characters literally.
/// - Double-quoted strings allow `$VAR` expansion (done before tokenize).
/// - Backslash escapes the next character outside of quotes.
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
        } else if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(&next) = chars.peek()
            {
                match next {
                    '"' | '\\' | '$' => {
                        if let Some(escaped) = chars.next() {
                            current.push(escaped);
                        }
                    },
                    _ => {
                        current.push('\\');
                    },
                }
            } else if ch == '\\' {
                current.push('\\');
            } else {
                current.push(ch);
            }
        } else {
            match ch {
                '\'' => in_single = true,
                '"' => in_double = true,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                },
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                },
                _ => current.push(ch),
            }
        }
    }

    if in_single {
        return Err(OasisError::Command("unterminated single quote".into()));
    }
    if in_double {
        return Err(OasisError::Command("unterminated double quote".into()));
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

/// Expand brace patterns (`{a,b,c}`) in tokens.
///
/// A token like `file.{rs,toml}` expands to `["file.rs", "file.toml"]`.
/// Nested braces are not supported. If the token contains no braces or
/// the braces are malformed, it is returned as-is.
pub(crate) fn expand_braces(tokens: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for token in tokens {
        if let Some(expanded) = expand_one_brace(token) {
            result.extend(expanded);
        } else {
            result.push(token.clone());
        }
    }
    result
}

/// Maximum nesting depth for brace expansion to prevent stack overflow.
const MAX_BRACE_DEPTH: usize = 16;

/// Expand a single brace expression. Returns `None` if no valid brace
/// pattern is found.
fn expand_one_brace(token: &str) -> Option<Vec<String>> {
    expand_one_brace_inner(token, 0)
}

fn expand_one_brace_inner(token: &str, depth: usize) -> Option<Vec<String>> {
    if depth >= MAX_BRACE_DEPTH {
        return None;
    }
    let open = token.find('{')?;
    let close = token[open..].find('}').map(|i| i + open)?;
    let prefix = &token[..open];
    let suffix = &token[close + 1..];
    let alternatives = &token[open + 1..close];

    // Must contain at least one comma to be a brace expansion.
    if !alternatives.contains(',') {
        return None;
    }

    let parts: Vec<&str> = alternatives.split(',').collect();
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let expanded = format!("{prefix}{part}{suffix}");
        // Recursively expand nested braces in the suffix.
        if let Some(nested) = expand_one_brace_inner(&expanded, depth + 1) {
            result.extend(nested);
        } else {
            result.push(expanded);
        }
    }
    Some(result)
}

/// Expand glob patterns (`*`, `?`, `[...]`) in tokens against VFS.
pub(crate) fn expand_globs(tokens: &[String], vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    let mut result = Vec::new();
    for token in tokens {
        if token.contains('*') || token.contains('?') || token.contains('[') {
            let expanded = expand_one_glob(token, vfs, cwd);
            if expanded.is_empty() {
                // No matches: pass the pattern through as-is.
                result.push(token.clone());
            } else {
                result.extend(expanded);
            }
        } else {
            result.push(token.clone());
        }
    }
    result
}

/// Expand a single glob pattern against the VFS.
fn expand_one_glob(pattern: &str, vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    // Split into directory and filename parts.
    let full_pattern = if pattern.starts_with('/') {
        pattern.to_string()
    } else if cwd == "/" {
        format!("/{pattern}")
    } else {
        format!("{cwd}/{pattern}")
    };

    let (dir, file_pattern) = match full_pattern.rsplit_once('/') {
        Some((d, f)) => {
            let dir = if d.is_empty() { "/" } else { d };
            (dir.to_string(), f.to_string())
        },
        None => (cwd.to_string(), full_pattern),
    };

    // Don't expand if the directory part also has globs (simple impl).
    if dir.contains('*') || dir.contains('?') {
        return Vec::new();
    }

    let entries = match vfs.readdir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut matches: Vec<String> = entries
        .iter()
        .filter(|e| glob_match(&file_pattern, &e.name))
        .map(|e| {
            if dir == "/" {
                format!("/{}", e.name)
            } else {
                format!("{}/{}", dir, e.name)
            }
        })
        .collect();
    matches.sort();
    matches
}

/// Glob matching: `*` matches any string, `?` matches one char,
/// `[abc]` matches one of the listed chars, `[a-z]` matches a range,
/// `[!abc]` or `[^abc]` matches any char NOT in the set.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();

    // Iterative wildcard matching with single-star backtracking: O(p * t)
    // worst case, no recursion (so no depth limit on long names and no
    // exponential blow-up on patterns like `*a*a*a*b`).
    let (mut pi, mut ti) = (0usize, 0usize);
    // Position after the most recent `*` and the text index it resumes at.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        let step = match p.get(pi) {
            Some('*') => {
                star = Some((pi + 1, ti));
                pi += 1;
                continue;
            },
            Some('?') => Some(pi + 1),
            Some('[') => match parse_char_class(&p, pi) {
                Some((negate, chars, end_pi)) => {
                    (chars.contains(&t[ti]) != negate).then_some(end_pi)
                },
                // Malformed bracket: treat '[' as a literal.
                None => (t[ti] == '[').then_some(pi + 1),
            },
            Some(&c) => (c == t[ti]).then_some(pi + 1),
            None => None,
        };
        match (step, star) {
            (Some(next), _) => {
                pi = next;
                ti += 1;
            },
            (None, Some((star_pi, star_ti))) => {
                // Let the last `*` absorb one more character and retry.
                pi = star_pi;
                ti = star_ti + 1;
                star = Some((star_pi, star_ti + 1));
            },
            (None, None) => return false,
        }
    }
    // Only trailing stars may remain.
    p[pi..].iter().all(|&c| c == '*')
}

/// Parse a character class starting at `p[pi]` which is `[`.
///
/// Returns `(negate, chars, end_index)` where `end_index` is the
/// position after the closing `]`. Returns `None` if malformed.
fn parse_char_class(p: &[char], pi: usize) -> Option<(bool, Vec<char>, usize)> {
    debug_assert!(p[pi] == '[');
    let mut i = pi + 1;
    if i >= p.len() {
        return None;
    }

    // Check for negation.
    let negate = p[i] == '!' || p[i] == '^';
    if negate {
        i += 1;
    }

    let mut chars = Vec::new();

    while i < p.len() && p[i] != ']' {
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            // Range: a-z.
            let start = p[i];
            let end = p[i + 2];
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            // Cap range expansion to prevent huge allocations from
            // broad Unicode ranges (e.g. [\0-\u{10FFFF}]).
            let range_len = (hi as u32).saturating_sub(lo as u32) + 1;
            if range_len > 256 {
                return None;
            }
            for c in lo..=hi {
                chars.push(c);
            }
            i += 3;
        } else {
            chars.push(p[i]);
            i += 1;
        }
    }

    if i < p.len() && p[i] == ']' {
        Some((negate, chars, i + 1))
    } else {
        None // No closing bracket.
    }
}

// ---------------------------------------------------------------------------
// Case pattern matching
// ---------------------------------------------------------------------------

/// Match a case pattern against a value.
///
/// Supports glob wildcards (`*`, `?`, `[...]`), `|` (alternation), and
/// literal matching.
pub(crate) fn case_pattern_matches(value: &str, pattern: &str) -> bool {
    for alt in pattern.split('|') {
        let alt = alt.trim();
        if alt == "*" {
            return true;
        }
        if alt == value {
            return true;
        }
        if glob_match_simple(value, alt) {
            return true;
        }
    }
    false
}

/// Glob matching with the operands in `(value, pattern)` order.
///
/// Formerly a prefix/suffix special case that mishandled patterns with more
/// than one `*` (e.g. `*alpha*` never matched); now full [`glob_match`]
/// semantics (`*`, `?`, `[...]`).
pub(crate) fn glob_match_simple(value: &str, pattern: &str) -> bool {
    glob_match(pattern, value)
}

// ---------------------------------------------------------------------------
// Path resolution helper
// ---------------------------------------------------------------------------

/// Resolve a possibly-relative path against the current working directory.
pub fn resolve_path(cwd: &str, input: &str) -> String {
    let raw = if input.starts_with('/') {
        input.to_string()
    } else if cwd == "/" {
        format!("/{input}")
    } else {
        format!("{cwd}/{input}")
    };

    let mut parts: Vec<&str> = Vec::new();
    for component in raw.split('/') {
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

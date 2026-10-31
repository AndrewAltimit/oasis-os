//! Word splitting and expansion: quote removal, brace expansion, glob
//! expansion and path resolution.
//!
//! [`split_words`] is the one quote-aware word scanner. The registry's
//! expander (`CommandRegistry::expand_words_of`) drives it with a `$`
//! callback that substitutes variables and `$(...)` output *as words*: an
//! expansion's value is never parsed as shell syntax again, it is only
//! split into fields on whitespace when unquoted.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

/// Tokenize a command line respecting quotes and backslash escapes.
///
/// - Single-quoted strings preserve all characters literally.
/// - Inside double quotes a backslash escapes `"`, `\` and `$`.
/// - Backslash escapes the next character outside of quotes.
/// - A quoted empty string (`""`, `''`) is an (empty) word.
/// - `$` is a plain character (no expansion happens here).
#[cfg(test)]
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    Ok(tokenize_words(input)?.into_iter().map(|w| w.text).collect())
}

/// One word of a command line after quote removal and expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    /// The text with quotes and escapes removed.
    pub(crate) text: String,
    /// The word is not plain unquoted source text: some part of it was
    /// quoted, escaped or came from a `$VAR` / `$(...)` expansion. Such a
    /// word gets no brace expansion and no alias lookup.
    pub(crate) quoted: bool,
    /// Pathname-expansion pattern, present when the word holds a glob
    /// metacharacter (`*`, `?`, `[`) from an unquoted source (literal text
    /// or an unquoted expansion). Metacharacters and backslashes from
    /// quoted parts are backslash-escaped in it, so `"*"*.txt` only
    /// matches names that start with a star.
    pub(crate) glob: Option<String>,
}

/// Characters that make a word a pathname pattern.
fn is_glob_meta(c: char) -> bool {
    matches!(c, '*' | '?' | '[')
}

/// Incrementally builds [`Word`]s for [`split_words`] and its `$`
/// callbacks.
#[derive(Default)]
pub(crate) struct WordBuilder {
    words: Vec<Word>,
    text: String,
    /// Glob pattern for the current word (see [`Word::glob`]).
    pattern: String,
    /// The current word has begun (it may be empty: `""`).
    started: bool,
    quoted: bool,
    /// The current word holds an active glob metacharacter.
    globbing: bool,
    /// Bytes of word text produced so far (all words).
    len: usize,
}

impl WordBuilder {
    /// Append one character. `quoted`: it came from a quoted or escaped
    /// source, so it is literal for pathname expansion.
    pub(crate) fn push_char(&mut self, c: char, quoted: bool) {
        self.started = true;
        self.text.push(c);
        self.len += c.len_utf8();
        if c == '\\' || quoted && is_glob_meta(c) {
            self.pattern.push('\\');
        } else if is_glob_meta(c) {
            self.globbing = true;
        }
        self.pattern.push(c);
    }

    /// Mark the current word as quoted (`""` starts an empty word).
    fn mark_quoted(&mut self) {
        self.started = true;
        self.quoted = true;
    }

    /// Append the value of an expansion. Inside double quotes it joins the
    /// current word verbatim; unquoted it is split into fields on
    /// whitespace. That is the only re-interpretation a value gets:
    /// quotes, `$`, `;`, `|` and the like in it are plain characters.
    pub(crate) fn push_expansion(&mut self, value: &str, in_double: bool) {
        if in_double {
            self.mark_quoted();
            for c in value.chars() {
                self.push_char(c, true);
            }
            return;
        }
        for c in value.chars() {
            if c.is_whitespace() {
                self.finish_word();
            } else {
                self.push_char(c, false);
                self.quoted = true;
            }
        }
    }

    /// Append `"$@"`: each field a separate quoted word, the first joining
    /// the current word and the last continuing into what follows. No
    /// fields (and no other text) produce no word at all.
    pub(crate) fn push_quoted_fields(&mut self, fields: &[String]) {
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                self.finish_word();
            }
            self.mark_quoted();
            for c in field.chars() {
                self.push_char(c, true);
            }
        }
    }

    /// End the current word (if one has begun).
    fn finish_word(&mut self) {
        if self.started {
            let glob = self.globbing.then(|| std::mem::take(&mut self.pattern));
            self.words.push(Word {
                text: std::mem::take(&mut self.text),
                quoted: self.quoted,
                glob,
            });
        }
        self.pattern.clear();
        self.started = false;
        self.quoted = false;
        self.globbing = false;
    }

    /// Total bytes of word text produced so far.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Finish and return the words.
    fn finish(mut self) -> Vec<Word> {
        self.finish_word();
        self.words
    }
}

/// The `$` hook of [`split_words`]: called with the input, the index of an
/// unquoted-or-double-quoted `$`, whether it is inside double quotes, and
/// the builder. Returns the index just past the expansion it consumed, or
/// `None` to keep the `$` as a literal character.
pub(crate) type DollarHook<'a> =
    dyn FnMut(&[char], usize, bool, &mut WordBuilder) -> Result<Option<usize>> + 'a;

/// Split `input` into words: quote removal, backslash escapes, `$`
/// expansions through `on_dollar` (nothing inside `'...'` is expanded),
/// and tilde expansion: an unquoted `~` that starts a word and ends it or
/// precedes a `/` becomes `home` (when given).
pub(crate) fn split_words(
    input: &str,
    home: Option<&str>,
    on_dollar: &mut DollarHook<'_>,
) -> Result<Vec<Word>> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = WordBuilder::default();
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        i += 1;
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                out.push_char(ch, true);
            }
            continue;
        }
        if ch == '$'
            && let Some(next) = on_dollar(&chars, i - 1, in_double, &mut out)?
        {
            i = next;
            continue;
        }
        if in_double {
            match ch {
                '"' => in_double = false,
                '\\' if matches!(chars.get(i), Some('"' | '\\' | '$')) => {
                    out.push_char(chars[i], true);
                    i += 1;
                },
                c => out.push_char(c, true),
            }
            continue;
        }
        match ch {
            '\'' => {
                in_single = true;
                out.mark_quoted();
            },
            '"' => {
                in_double = true;
                out.mark_quoted();
            },
            '\\' => {
                out.mark_quoted();
                if let Some(&next) = chars.get(i) {
                    out.push_char(next, true);
                    i += 1;
                }
            },
            c if c.is_whitespace() => out.finish_word(),
            '~' if !out.started
                && chars.get(i).is_none_or(|&n| n == '/' || n.is_whitespace())
                && let Some(home) = home =>
            {
                // The home directory is literal text (no globbing).
                out.mark_quoted();
                for c in home.chars() {
                    out.push_char(c, true);
                }
            },
            c => out.push_char(c, false),
        }
    }

    if in_single {
        return Err(OasisError::Command("unterminated single quote".into()));
    }
    if in_double {
        return Err(OasisError::Command("unterminated double quote".into()));
    }
    Ok(out.finish())
}

/// [`tokenize`], keeping the per-word quoting and glob information.
#[cfg(test)]
pub(crate) fn tokenize_words(input: &str) -> Result<Vec<Word>> {
    split_words(input, None, &mut |_, _, _, _| Ok(None))
}

/// Brace- then glob-expand the words: brace expansion applies to plain
/// unquoted words only, pathname expansion to words with an active glob
/// pattern (see [`Word::glob`]). Everything else passes through verbatim.
pub(crate) fn expand_words(words: Vec<Word>, vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(words.len());
    for word in words {
        if !word.quoted {
            for braced in expand_braces(std::slice::from_ref(&word.text)) {
                if braced.chars().any(is_glob_meta) {
                    // A plain word is its own pattern; a backslash in it
                    // is a literal character.
                    let pattern = braced.replace('\\', "\\\\");
                    out.extend(glob_or_literal(&pattern, braced, vfs, cwd));
                } else {
                    out.push(braced);
                }
            }
        } else if let Some(pattern) = &word.glob {
            out.extend(glob_or_literal(pattern, word.text, vfs, cwd));
        } else {
            out.push(word.text);
        }
    }
    out
}

/// The paths matching `pattern`, or `literal` alone when nothing matches.
fn glob_or_literal(pattern: &str, literal: String, vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    let matches = expand_one_glob(pattern, vfs, cwd);
    if matches.is_empty() {
        vec![literal]
    } else {
        matches
    }
}

/// Remove the backslash escapes of a glob pattern (for its directory part).
fn unescape_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// Whether `pattern` holds an unescaped glob metacharacter.
fn has_active_meta(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            },
            c if is_glob_meta(c) => return true,
            _ => {},
        }
    }
    false
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

/// Maximum words one token may brace-expand to. `{0,1,...,9}` repeated
/// eight times would otherwise produce 10^8 strings.
const MAX_BRACE_RESULTS: usize = 4096;

/// Expand a single brace expression. Returns `None` if no valid brace
/// pattern is found, or if the expansion would exceed
/// [`MAX_BRACE_RESULTS`] words (the token is then left as-is).
fn expand_one_brace(token: &str) -> Option<Vec<String>> {
    let mut budget = MAX_BRACE_RESULTS;
    let result = expand_one_brace_inner(token, 0, &mut budget)?;
    (budget > 0).then_some(result)
}

fn expand_one_brace_inner(token: &str, depth: usize, budget: &mut usize) -> Option<Vec<String>> {
    if depth >= MAX_BRACE_DEPTH || *budget == 0 {
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
        if *budget == 0 {
            return Some(result);
        }
        let expanded = format!("{prefix}{part}{suffix}");
        // Recursively expand nested braces in the suffix.
        if let Some(nested) = expand_one_brace_inner(&expanded, depth + 1, budget) {
            result.extend(nested);
        } else if *budget > 0 {
            *budget -= 1;
            result.push(expanded);
        }
    }
    Some(result)
}

/// Expand a single glob pattern (backslash escapes allowed, see
/// [`glob_match`]) against the VFS. Returns no paths when nothing matches.
fn expand_one_glob(pattern: &str, vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    // Split into directory and filename parts. The working directory is
    // literal text, so its own metacharacters are escaped.
    let full_pattern = if pattern.starts_with('/') {
        pattern.to_string()
    } else if cwd == "/" {
        format!("/{pattern}")
    } else {
        let mut escaped_cwd = String::with_capacity(cwd.len());
        for c in cwd.chars() {
            if c == '\\' || is_glob_meta(c) {
                escaped_cwd.push('\\');
            }
            escaped_cwd.push(c);
        }
        format!("{escaped_cwd}/{pattern}")
    };

    let (dir, file_pattern) = match full_pattern.rsplit_once('/') {
        Some((d, f)) => (if d.is_empty() { "/" } else { d }, f),
        None => ("/", full_pattern.as_str()),
    };

    // Only the last path component may be a pattern.
    if has_active_meta(dir) {
        return Vec::new();
    }
    let dir = unescape_pattern(dir);

    let entries = match vfs.readdir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    // A leading `.` must be matched explicitly (`*` skips dotfiles).
    let dot_ok = file_pattern.starts_with('.') || file_pattern.starts_with("\\.");
    let mut matches: Vec<String> = entries
        .iter()
        .filter(|e| (dot_ok || !e.name.starts_with('.')) && glob_match(file_pattern, &e.name))
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
/// `[!abc]` or `[^abc]` matches any char NOT in the set, and `\c`
/// matches the character `c` literally.
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
            Some('\\') => match p.get(pi + 1) {
                Some(&c) => (c == t[ti]).then_some(pi + 2),
                // A trailing backslash is literal.
                None => (t[ti] == '\\').then_some(pi + 1),
            },
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

/// Split a `case` arm pattern on its top-level `|` (not inside quotes or
/// after a backslash): `a|"b|c"` is `a` and `"b|c"`. Each alternative is
/// returned trimmed and still quoted; the executor expands it.
pub(crate) fn split_case_alternatives(pattern: &str) -> Vec<&str> {
    let crate::pipeline::Scan { top, .. } = crate::pipeline::scan(pattern);
    let mut alts = Vec::new();
    let mut start = 0;
    for (i, b) in pattern.bytes().enumerate() {
        if b == b'|' && top[i] {
            alts.push(pattern[start..i].trim());
            start = i + 1;
        }
    }
    alts.push(pattern[start..].trim());
    alts
}

/// The glob pattern an expanded `case` alternative stands for: its glob
/// form when it holds an unquoted metacharacter, else its text with every
/// metacharacter escaped (matched literally).
pub(crate) fn case_glob(words: &[Word]) -> String {
    let mut pattern = String::new();
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            pattern.push(' ');
        }
        match &word.glob {
            Some(glob) => pattern.push_str(glob),
            None => {
                for c in word.text.chars() {
                    if c == '\\' || is_glob_meta(c) {
                        pattern.push('\\');
                    }
                    pattern.push(c);
                }
            },
        }
    }
    pattern
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

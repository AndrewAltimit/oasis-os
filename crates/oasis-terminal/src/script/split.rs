//! Statement splitting: [`split_statements`] and the keyword / word
//! scanners the parser shares.

/// One logical statement produced by [`split_statements`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stmt {
    /// Statement text (trimmed, without the separator).
    pub(crate) text: String,
    /// 1-based source line the statement starts on.
    pub(crate) line: usize,
}

/// Split shell source into logical statements.
///
/// Separators are newlines and unquoted `;`. A `;;` (case-arm terminator)
/// is emitted as its own `";;"` statement. Separators inside quotes,
/// `{ ... }` bodies and `$( ... )` substitutions are kept verbatim. A `#`
/// at the start of a word begins a comment that runs to end of line, and
/// a backslash-newline joins two physical lines.
pub(crate) fn split_statements(src: &str) -> Vec<Stmt> {
    fn flush(cur: &mut String, out: &mut Vec<Stmt>, line: usize) {
        let t = cur.trim();
        if !t.is_empty() {
            out.push(Stmt {
                text: t.to_string(),
                line,
            });
        }
        cur.clear();
    }

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut line = 1usize;
    let mut start_line = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut chars = src.chars().peekable();

    while let Some(c) = chars.next() {
        if cur.is_empty() {
            if c == ' ' || c == '\t' || c == '\r' {
                continue;
            }
            start_line = line;
        }
        if in_single {
            cur.push(c);
            match c {
                '\'' => in_single = false,
                '\n' => line += 1,
                _ => {},
            }
            continue;
        }
        if in_double {
            cur.push(c);
            match c {
                '"' => in_double = false,
                '\\' => {
                    if let Some(n) = chars.next() {
                        if n == '\n' {
                            line += 1;
                        }
                        cur.push(n);
                    }
                },
                '\n' => line += 1,
                _ => {},
            }
            continue;
        }
        let nested = brace_depth > 0 || paren_depth > 0;
        match c {
            '\'' => {
                in_single = true;
                cur.push(c);
            },
            '"' => {
                in_double = true;
                cur.push(c);
            },
            '\\' => match chars.next() {
                // Line continuation.
                Some('\n') => line += 1,
                Some(n) => {
                    cur.push(c);
                    cur.push(n);
                },
                None => cur.push(c),
            },
            '#' if !nested && cur.chars().last().is_none_or(char::is_whitespace) => {
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            },
            '{' => {
                brace_depth += 1;
                cur.push(c);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                cur.push(c);
            },
            '(' => {
                if paren_depth > 0 || cur.ends_with('$') {
                    paren_depth += 1;
                }
                cur.push(c);
            },
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                cur.push(c);
            },
            '\n' => {
                line += 1;
                if nested {
                    cur.push(c);
                } else {
                    flush(&mut cur, &mut out, start_line);
                }
            },
            // A lone `&` ends a statement (keeping the `&`, which the
            // chain executor turns into a background job): `do cmd & done`.
            '&' if !nested
                && chars.peek().is_none_or(|&n| n != '&' && n != '>')
                && !cur.ends_with(['&', '>', '<']) =>
            {
                cur.push(c);
                flush(&mut cur, &mut out, start_line);
            },
            ';' if !nested => {
                flush(&mut cur, &mut out, start_line);
                if chars.peek() == Some(&';') {
                    chars.next();
                    out.push(Stmt {
                        text: ";;".to_string(),
                        line,
                    });
                }
            },
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut out, start_line);
    out
}

/// First whitespace-delimited word of `s` (empty for blank input).
pub(super) fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

/// Keywords that open a compound command.
const COMPOUND_KEYWORDS: [&str; 5] = ["if", "while", "until", "for", "case"];

/// Whether `line` contains a compound command (`if`, `while`, `until`,
/// `for`, `case`) at statement level and must go through the script
/// engine rather than the plain chain executor.
pub(crate) fn is_compound(line: &str) -> bool {
    // Cheap pre-filter: every compound keyword appears as a substring.
    if !COMPOUND_KEYWORDS.iter().any(|kw| line.contains(kw)) {
        return false;
    }
    split_statements(line)
        .iter()
        .any(|s| COMPOUND_KEYWORDS.contains(&first_word(&s.text)))
}

/// Find a whitespace-delimited keyword in a string, ignoring quoted text
/// and `{ ... }` groups. Returns the byte offset of the keyword.
pub(crate) fn find_keyword(input: &str, keyword: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let kw_bytes = keyword.as_bytes();
    let kw_len = kw_bytes.len();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth: usize = 0;

    let mut i = 0;
    while i + kw_len <= bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' {
                i += 1;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                _ if depth == 0 => {
                    // A keyword is a whole word: whitespace (or the edge)
                    // on both sides (`case foo.in in` matches the second).
                    let at_start = i == 0 || bytes[i - 1].is_ascii_whitespace();
                    let at_end =
                        i + kw_len >= bytes.len() || bytes[i + kw_len].is_ascii_whitespace();
                    if at_start && at_end && &bytes[i..i + kw_len] == kw_bytes {
                        return Some(i);
                    }
                },
                _ => {},
            }
        }
        i += 1;
    }
    None
}

/// Byte offset of the first `)` outside quotes (case-arm pattern end).
pub(super) fn find_pattern_end(s: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ')' if !in_single && !in_double => return Some(i),
            _ => {},
        }
    }
    None
}

use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

use super::expander::resolve_path;
use super::interpreter::CommandOutput;

// ---------------------------------------------------------------------------
// Lexical scan: which bytes are shell operators?
// ---------------------------------------------------------------------------

/// Result of [`scan`]: for every byte of the input, whether it sits at the
/// top level of the command line, where an operator byte (`;`, `&`, `|`,
/// `<`, `>`) acts as an operator.
pub(crate) struct Scan {
    /// `top[i]`: byte `i` is outside quotes, not escaped by a backslash,
    /// and not inside `{ ... }` or `$( ... )`. Quote characters and
    /// backslashes themselves are never top-level.
    pub(crate) top: Vec<bool>,
    /// Every quote was closed.
    pub(crate) balanced: bool,
}

/// The one quoting / nesting scanner the chain, pipe, redirect and
/// background splitters share (they used to carry three slightly different
/// copies, none of which knew about `$( ... )`, so `echo $(ls | wc -l)` was
/// split into two pipeline stages).
///
/// Rules: `'...'` is literal; inside `"..."` a backslash escapes the next
/// character; outside quotes a backslash escapes the next character; `{`
/// / `}` nest (function bodies, `${VAR}`); `$(` opens a substitution in
/// which `(` / `)` nest. Operators are ASCII, so byte positions reported
/// as top-level are always char boundaries.
pub(crate) fn scan(input: &str) -> Scan {
    let b = input.as_bytes();
    let mut top = vec![false; b.len()];
    let (mut in_single, mut in_double) = (false, false);
    let (mut braces, mut parens) = (0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_single {
            in_single = c != b'\'';
        } else if in_double {
            match c {
                b'"' => in_double = false,
                b'\\' => i += 1,
                _ => {},
            }
        } else {
            match c {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'\\' => i += 1,
                b'$' if b.get(i + 1) == Some(&b'(') => {
                    parens += 1;
                    i += 1;
                },
                b'(' if parens > 0 => parens += 1,
                b')' if parens > 0 => parens -= 1,
                b'{' => braces += 1,
                b'}' => braces = braces.saturating_sub(1),
                _ => top[i] = braces == 0 && parens == 0,
            }
        }
        i += 1;
    }
    Scan {
        top,
        balanced: !in_single && !in_double,
    }
}

// ---------------------------------------------------------------------------
// Chain splitting: ;, &&, ||
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainOp {
    /// First command or after `;`.
    Always,
    /// After `&&` -- run only if previous succeeded.
    And,
    /// After `||` -- run only if previous failed.
    Or,
}

pub(crate) struct ChainSegment {
    pub(crate) command: String,
    pub(crate) chain_op: ChainOp,
    /// The and-or list this pipeline belongs to ends in `&`: queue it as a
    /// background job instead of running it.
    pub(crate) background: bool,
}

/// Whether the top-level `&` at byte `i` is the background operator (not
/// half of `&&`, not part of a `>&` / `<&` / `&>` redirection).
fn is_background_amp(b: &[u8], top: &[bool], i: usize) -> bool {
    let next_amp = b.get(i + 1) == Some(&b'&') && top.get(i + 1) == Some(&true);
    let prev_amp = i > 0 && b[i - 1] == b'&' && top[i - 1];
    let redirect = (i > 0 && matches!(b[i - 1], b'>' | b'<')) || b.get(i + 1) == Some(&b'>');
    !next_amp && !prev_amp && !redirect
}

/// Split a command line on top-level `;`, `&`, `&&` and `||` (see
/// [`scan`]). Empty segments (`a ;; b`) are skipped. A `&` marks every
/// pipeline of the and-or list it ends as [`ChainSegment::background`].
pub(crate) fn split_chains(input: &str) -> Result<Vec<ChainSegment>> {
    let Scan { top, .. } = scan(input);
    let b = input.as_bytes();
    let mut segments: Vec<ChainSegment> = Vec::new();
    let push = |segments: &mut Vec<ChainSegment>, text: &str, chain_op: ChainOp| {
        let command = text.trim();
        if !command.is_empty() {
            segments.push(ChainSegment {
                command: command.to_string(),
                chain_op,
                background: false,
            });
        }
    };
    let (mut start, mut op, mut i) = (0, ChainOp::Always, 0);
    // Index of the first segment of the current and-or list.
    let mut list_start = 0;
    while i < b.len() {
        let doubled = b.get(i + 1) == Some(&b[i]) && top.get(i + 1) == Some(&true);
        let next_op = match b[i] {
            _ if !top[i] => None,
            b';' => Some((ChainOp::Always, 1)),
            b'&' if doubled => Some((ChainOp::And, 2)),
            b'|' if doubled => Some((ChainOp::Or, 2)),
            b'&' if is_background_amp(b, &top, i) => {
                push(&mut segments, &input[start..i], op);
                for s in &mut segments[list_start..] {
                    s.background = true;
                }
                list_start = segments.len();
                op = ChainOp::Always;
                i += 1;
                start = i;
                continue;
            },
            _ => None,
        };
        if let Some((next, len)) = next_op {
            push(&mut segments, &input[start..i], op);
            if next == ChainOp::Always {
                list_start = segments.len();
            }
            op = next;
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    push(&mut segments, &input[start..], op);
    Ok(segments)
}

/// Remove `#` comments: an unquoted, top-level `#` at the start of a word
/// comments out the rest of its line (`echo a#b` and `$#` are not
/// comments; nor is a `#` inside `{ ... }` or `$( ... )`, whose text is
/// executed, and stripped, later).
pub(crate) fn strip_comments(input: &str) -> std::borrow::Cow<'_, str> {
    if !input.contains('#') {
        return std::borrow::Cow::Borrowed(input);
    }
    let Scan { top, .. } = scan(input);
    let b = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let (mut i, mut copied, mut stripped) = (0, 0, false);
    while i < b.len() {
        if b[i] == b'#' && top[i] && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            out.push_str(&input[copied..i]);
            // Skip to the end of the line (keep the newline).
            i = input[i..].find('\n').map_or(b.len(), |n| i + n);
            copied = i;
            stripped = true;
            continue;
        }
        i += 1;
    }
    if !stripped {
        return std::borrow::Cow::Borrowed(input);
    }
    out.push_str(&input[copied..]);
    std::borrow::Cow::Owned(out)
}

/// If `input` ends with an unquoted, unescaped single `&` (background
/// operator, not `&&` or a `>&` redirect), return the command before it.
pub(crate) fn strip_background(input: &str) -> Option<&str> {
    let trimmed = input.trim_end();
    let body = trimmed.strip_suffix('&')?;
    if body.ends_with(['&', '>', '<']) {
        return None;
    }
    let scanned = scan(trimmed);
    if !scanned.balanced || scanned.top.last() != Some(&true) {
        return None;
    }
    let cmd = body.trim_end();
    (!cmd.is_empty()).then_some(cmd)
}

// ---------------------------------------------------------------------------
// Pipe splitting
// ---------------------------------------------------------------------------

/// Split on top-level `|` (single pipe, not `||`; see [`scan`]).
///
/// An empty stage (`a | | b`, `a |`, `| b`) is a syntax error.
pub(crate) fn split_pipes(input: &str) -> Result<Vec<String>> {
    let Scan { top, .. } = scan(input);
    let b = input.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if top[i] && b[i] == b'|' {
            if b.get(i + 1) == Some(&b'|') {
                i += 2;
                continue;
            }
            segments.push(input[start..i].trim().to_string());
            start = i + 1;
        }
        i += 1;
    }
    segments.push(input[start..].trim().to_string());
    if segments.len() > 1 && segments.iter().any(String::is_empty) {
        return Err(OasisError::Command(
            "syntax error: empty command in pipeline".into(),
        ));
    }
    segments.retain(|s| !s.is_empty());
    Ok(segments)
}

// ---------------------------------------------------------------------------
// Redirection parsing
// ---------------------------------------------------------------------------

/// A single output redirection.
pub(crate) struct Redirect<'a> {
    pub(crate) path: &'a str,
    pub(crate) append: bool,
}

/// Parsed redirections for a command.
#[derive(Default)]
pub(crate) struct Redirections<'a> {
    /// Stdout redirect (`>` / `>>`).
    pub(crate) stdout: Option<Redirect<'a>>,
    /// Stderr redirect (`2>` / `2>>`).
    pub(crate) stderr: Option<Redirect<'a>>,
    /// Stdin redirect (`<`).
    pub(crate) stdin: Option<&'a str>,
    /// Merge stderr into stdout (`2>&1`).
    pub(crate) stderr_to_stdout: bool,
}

impl Redirections<'_> {
    /// No redirection at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.stdout.is_none()
            && self.stderr.is_none()
            && self.stdin.is_none()
            && !self.stderr_to_stdout
    }
}

/// Parse redirect operators from a command string.
///
/// Supports:
/// - `> file` / `>> file`   (stdout)
/// - `2> file` / `2>> file` (stderr)
/// - `2>&1`                 (merge stderr into stdout)
/// - `< file`               (stdin)
///
/// Only top-level operators count (see [`scan`]): `\>`, `">"` and
/// `$(cat < f)` are not redirections of this command.
///
/// Returns `(command_part, redirections)`.
pub(crate) fn parse_redirect(input: &str) -> (&str, Redirections<'_>) {
    let Scan { top, .. } = scan(input);
    let bytes = input.as_bytes();
    let mut i = 0;

    // Collected redirect entries: (position, fd, append, merge_target).
    // fd: 0 = stdin, 1 = stdout, 2 = stderr; merge = `2>&1`.
    let mut redirects: Vec<(usize, u8, bool, bool)> = Vec::new();

    while i < bytes.len() {
        if !top[i] {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'2' if bytes.get(i + 1) == Some(&b'>')
                && (i == 0 || bytes[i - 1].is_ascii_whitespace()) =>
            {
                if bytes.get(i + 2) == Some(&b'&') && bytes.get(i + 3) == Some(&b'1') {
                    redirects.push((i, 2, false, true));
                    i += 3;
                } else if bytes.get(i + 2) == Some(&b'>') {
                    redirects.push((i, 2, true, false));
                    i += 2;
                } else {
                    redirects.push((i, 2, false, false));
                    i += 1;
                }
            },
            b'>' => {
                let append = bytes.get(i + 1) == Some(&b'>');
                redirects.push((i, 1, append, false));
                if append {
                    i += 1;
                }
            },
            b'<' => redirects.push((i, 0, false, false)),
            _ => {},
        }
        i += 1;
    }

    let mut parsed = Redirections::default();
    let Some(&(first, ..)) = redirects.first() else {
        return (input, parsed);
    };

    for (idx, &(pos, fd, append, merge)) in redirects.iter().enumerate() {
        if merge {
            parsed.stderr_to_stdout = true;
            continue;
        }
        // The path runs from after the operator to the next redirect or
        // the end of the line.
        let op_len = match (fd, append) {
            (2, true) => 3,
            (2, false) | (1, true) => 2,
            _ => 1,
        };
        let path_end = redirects.get(idx + 1).map_or(input.len(), |r| r.0);
        let path = input[pos + op_len..path_end].trim();
        match fd {
            0 => parsed.stdin = Some(path),
            1 => parsed.stdout = Some(Redirect { path, append }),
            _ => parsed.stderr = Some(Redirect { path, append }),
        }
    }

    (&input[..first], parsed)
}

/// Extract text content from a `CommandOutput`.
pub(crate) fn output_to_text(output: &CommandOutput) -> String {
    match output {
        CommandOutput::Text(t) => t.clone(),
        CommandOutput::Table { headers, rows } => {
            let mut out = headers.join(" | ");
            for row in rows {
                out.push('\n');
                out.push_str(&row.join(" | "));
            }
            out
        },
        CommandOutput::Status { output, .. } => output_to_text(output),
        _ => String::new(),
    }
}

/// Write text to a VFS file, handling append mode.
pub(crate) fn write_redirect(
    text: &str,
    raw_path: &str,
    append: bool,
    cwd: &str,
    vfs: &mut dyn Vfs,
) -> Result<()> {
    let path = resolve_path(cwd, raw_path.trim());
    if append {
        // Work on raw bytes so appending never rewrites (lossily re-encodes)
        // existing non-UTF-8 content.
        let mut combined = if vfs.exists(&path) {
            vfs.read(&path)?
        } else {
            Vec::new()
        };
        if !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(text.as_bytes());
        vfs.write(&path, &combined)?;
    } else {
        vfs.write(&path, text.as_bytes())?;
    }
    Ok(())
}

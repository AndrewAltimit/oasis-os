use std::borrow::Cow;

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

/// Byte offset of the first top-level `&&` or `||` in `input`.
pub(crate) fn find_and_or(input: &str) -> Option<usize> {
    let Scan { top, .. } = scan(input);
    let b = input.as_bytes();
    (0..b.len().saturating_sub(1))
        .find(|&i| matches!(b[i], b'&' | b'|') && b[i + 1] == b[i] && top[i] && top[i + 1])
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

/// A single output redirection target (`> f`, `2>> f`, `&> f`).
pub(crate) struct Redirect<'a> {
    pub(crate) path: &'a str,
    pub(crate) append: bool,
}

/// Where an output stream (stdout or stderr) ends up after every
/// redirection of a command is applied, left to right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StreamDest {
    /// The command's original stdout (the terminal, or the next pipe stage).
    Stdout,
    /// The command's original stderr (shown on the terminal).
    Stderr,
    /// An index into [`Redirections::files`].
    File(usize),
}

/// A command's stdin source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StdinSource<'a> {
    /// `< path`.
    File(&'a str),
    /// `<<< word`: the expanded word (no trailing newline, like the text
    /// every command of this shell passes on).
    HereString(&'a str),
}

/// Parsed redirections for a command.
///
/// Operators apply left to right like in a POSIX shell, so
/// `cmd 2>&1 > f` sends stderr to the terminal and only stdout to `f`,
/// while `cmd > f 2>&1` sends both to `f`.
pub(crate) struct Redirections<'a> {
    /// Every output file opened, in order. A file that no stream ends up
    /// in (`> a > b` leaves `a`) is still created / truncated.
    pub(crate) files: Vec<Redirect<'a>>,
    /// Where stdout goes.
    pub(crate) stdout: StreamDest,
    /// Where stderr goes.
    pub(crate) stderr: StreamDest,
    /// Stdin source (`<` / `<<<`; the last one wins).
    pub(crate) stdin: Option<StdinSource<'a>>,
    /// An operator the shell does not implement (`<<` here-documents).
    pub(crate) unsupported: Option<&'static str>,
}

impl Default for Redirections<'_> {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            stdout: StreamDest::Stdout,
            stderr: StreamDest::Stderr,
            stdin: None,
            unsupported: None,
        }
    }
}

impl Redirections<'_> {
    /// No redirection at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.stdin.is_none()
            && self.unsupported.is_none()
            && self.stdout == StreamDest::Stdout
            && self.stderr == StreamDest::Stderr
    }

    /// The file stdout ends up in, if any.
    pub(crate) fn stdout_file(&self) -> Option<&Redirect<'_>> {
        match self.stdout {
            StreamDest::File(i) => self.files.get(i),
            _ => None,
        }
    }

    /// The file stderr ends up in, if any.
    pub(crate) fn stderr_file(&self) -> Option<&Redirect<'_>> {
        match self.stderr {
            StreamDest::File(i) => self.files.get(i),
            _ => None,
        }
    }
}

/// One redirection operator found by [`parse_redirect`].
#[derive(Clone, Copy)]
enum RedirOp {
    /// `>` / `>>` / `1>` / `1>>`.
    Out { append: bool },
    /// `2>` / `2>>`.
    Err { append: bool },
    /// `&>` / `&>>`: stdout and stderr to one file.
    Both { append: bool },
    /// `<`.
    In,
    /// `<<<`.
    HereString,
    /// `<<` (here-document; unsupported).
    HereDoc,
    /// `2>&1`.
    ErrToOut,
    /// `>&2` / `1>&2`.
    OutToErr,
    /// `>&1` / `2>&2` (no effect).
    Noop,
}

/// The redirection operator at byte `i` (a top-level byte) and its length.
fn redirect_op_at(b: &[u8], i: usize) -> Option<(RedirOp, usize)> {
    let word_start = i == 0 || b[i - 1].is_ascii_whitespace();
    let at = |k: usize| b.get(i + k).copied();
    // The operator after an optional fd digit: `>`, `>>`, `>&N`.
    let out_op = |k: usize, fd: u8| -> Option<(RedirOp, usize)> {
        match (at(k + 1), at(k + 2)) {
            (Some(b'>'), _) => Some((
                if fd == 2 {
                    RedirOp::Err { append: true }
                } else {
                    RedirOp::Out { append: true }
                },
                k + 2,
            )),
            (Some(b'&'), Some(b'1')) if fd == 2 => Some((RedirOp::ErrToOut, k + 3)),
            (Some(b'&'), Some(b'2')) if fd == 1 => Some((RedirOp::OutToErr, k + 3)),
            (Some(b'&'), Some(b'1' | b'2')) => Some((RedirOp::Noop, k + 3)),
            _ => Some((
                if fd == 2 {
                    RedirOp::Err { append: false }
                } else {
                    RedirOp::Out { append: false }
                },
                k + 1,
            )),
        }
    };
    match b[i] {
        b'2' if word_start && at(1) == Some(b'>') => out_op(1, 2),
        b'1' if word_start && at(1) == Some(b'>') => out_op(1, 1),
        b'&' if at(1) == Some(b'>') => {
            let append = at(2) == Some(b'>');
            Some((RedirOp::Both { append }, if append { 3 } else { 2 }))
        },
        b'>' => out_op(0, 1),
        b'<' if at(1) == Some(b'<') && at(2) == Some(b'<') => Some((RedirOp::HereString, 3)),
        b'<' if at(1) == Some(b'<') => Some((RedirOp::HereDoc, 2)),
        b'<' => Some((RedirOp::In, 1)),
        _ => None,
    }
}

/// Open output file `path` and return the [`StreamDest`] naming it.
fn open<'a>(files: &mut Vec<Redirect<'a>>, path: &'a str, append: bool) -> StreamDest {
    files.push(Redirect { path, append });
    StreamDest::File(files.len() - 1)
}

/// Parse redirect operators from a command string.
///
/// Supports `> f`, `>> f`, `1> f`, `2> f`, `2>> f`, `&> f`, `&>> f`,
/// `< f`, `<<< word`, `2>&1` and `>&2`, applied left to right. Only
/// top-level operators count (see [`scan`]): `\>`, `">"` and
/// `$(cat < f)` are not redirections of this command. A target is the
/// one word after its operator; words after it are arguments again
/// (`echo a > f b` writes `a b`).
///
/// Returns `(command_part, redirections)`.
pub(crate) fn parse_redirect(input: &str) -> (Cow<'_, str>, Redirections<'_>) {
    let Scan { top, .. } = scan(input);
    let b = input.as_bytes();

    // (operator start, operator end, operator)
    let mut ops: Vec<(usize, usize, RedirOp)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if top[i]
            && let Some((op, len)) = redirect_op_at(b, i)
        {
            ops.push((i, i + len, op));
            i += len;
            continue;
        }
        i += 1;
    }

    let mut parsed = Redirections::default();
    let Some(&(first, ..)) = ops.first() else {
        return (Cow::Borrowed(input), parsed);
    };

    let mut leftovers: Vec<&str> = Vec::new();
    for (idx, &(_, op_end, op)) in ops.iter().enumerate() {
        let region_end = ops.get(idx + 1).map_or(input.len(), |r| r.0);
        let region = &input[op_end..region_end];
        let takes_path = matches!(
            op,
            RedirOp::Out { .. }
                | RedirOp::Err { .. }
                | RedirOp::Both { .. }
                | RedirOp::In
                | RedirOp::HereString
                | RedirOp::HereDoc
        );
        let (path, rest) = if takes_path {
            // The target is the first top-level word of the region.
            let start = op_end + (region.len() - region.trim_start().len());
            let end = (start..region_end)
                .find(|&j| top[j] && b[j].is_ascii_whitespace())
                .unwrap_or(region_end);
            (&input[start..end], &input[end..region_end])
        } else {
            ("", region)
        };
        if !rest.trim().is_empty() {
            leftovers.push(rest.trim());
        }
        let files = &mut parsed.files;
        match op {
            RedirOp::In => parsed.stdin = Some(StdinSource::File(path)),
            RedirOp::HereString => parsed.stdin = Some(StdinSource::HereString(path)),
            RedirOp::HereDoc => {
                parsed.unsupported = Some("here-documents (<<) are not supported; use <<< or <");
            },
            RedirOp::Out { append } => parsed.stdout = open(files, path, append),
            RedirOp::Err { append } => parsed.stderr = open(files, path, append),
            RedirOp::Both { append } => {
                let dest = open(files, path, append);
                parsed.stdout = dest;
                parsed.stderr = dest;
            },
            RedirOp::ErrToOut => parsed.stderr = parsed.stdout,
            RedirOp::OutToErr => parsed.stdout = parsed.stderr,
            RedirOp::Noop => {},
        }
    }

    let head = &input[..first];
    if leftovers.is_empty() {
        return (Cow::Borrowed(head), parsed);
    }
    let mut cmd = head.trim_end().to_string();
    for rest in leftovers {
        cmd.push(' ');
        cmd.push_str(rest);
    }
    (Cow::Owned(cmd), parsed)
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

/// SGR the shell itself puts around directory names (`ls`, `tree`):
/// bright blue. The redundant `24` (underline off) makes the sequence one
/// that files practically never contain, so [`capture_text`] can remove
/// exactly the shell's colouring and leave other ANSI text alone.
pub(crate) const DIR_COLOR_ON: &str = "\u{1b}[94;24m";
/// Ends [`DIR_COLOR_ON`] (default foreground).
pub(crate) const DIR_COLOR_OFF: &str = "\u{1b}[39;24m";

/// `name` in the shell's directory colour.
pub(crate) fn dir_colored(name: &str) -> String {
    format!("{DIR_COLOR_ON}{name}{DIR_COLOR_OFF}")
}

/// The text of `output` as a pipe, `$(...)` or redirect receives it:
/// [`output_to_text`] without the shell's own colouring (`ls` colours
/// directory names for the terminal, which `for d in $(ls)` must not see).
/// Other escape sequences pass through: `cat art.ans > copy.ans` copies
/// the file byte for byte.
pub(crate) fn capture_text(output: &CommandOutput) -> String {
    let text = output_to_text(output);
    match strip_ansi(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(stripped) => stripped,
    }
}

/// Remove the shell's colour sequences ([`DIR_COLOR_ON`] /
/// [`DIR_COLOR_OFF`]) from `text`.
pub(crate) fn strip_ansi(text: &str) -> Cow<'_, str> {
    if !text.contains(DIR_COLOR_ON) && !text.contains(DIR_COLOR_OFF) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace(DIR_COLOR_ON, "").replace(DIR_COLOR_OFF, ""))
}

/// The path a redirect to which discards its text.
const DEV_NULL: &str = "/dev/null";

/// Write text to a VFS file, handling append mode. `/dev/null` (no VFS
/// has a `/dev`) swallows the text.
pub(crate) fn write_redirect(
    text: &str,
    raw_path: &str,
    append: bool,
    cwd: &str,
    vfs: &mut dyn Vfs,
) -> Result<()> {
    let path = resolve_path(cwd, raw_path.trim());
    if path == DEV_NULL {
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_target_is_one_word() {
        let (cmd, r) = parse_redirect("echo a > f b");
        assert_eq!(cmd, "echo a b");
        assert_eq!(r.stdout_file().map(|o| o.path), Some("f"));
        let (cmd, r) = parse_redirect("echo a > \"my file\" b");
        assert_eq!(cmd, "echo a b");
        assert_eq!(r.stdout_file().map(|o| o.path), Some("\"my file\""));
    }

    #[test]
    fn fd_duplication_forms() {
        let (cmd, r) = parse_redirect("echo err >&2");
        assert_eq!(cmd.trim(), "echo err");
        assert_eq!(r.stdout, StreamDest::Stderr);
        assert!(r.files.is_empty());
        let (cmd, r) = parse_redirect("echo x &> f");
        assert_eq!(cmd.trim(), "echo x");
        assert_eq!(r.stderr, r.stdout);
        assert_eq!(
            r.stdout_file().map(|o| (o.path, o.append)),
            Some(("f", false))
        );
        let (_, r) = parse_redirect("echo x &>> f");
        assert_eq!(r.stdout_file().map(|o| o.append), Some(true));
        let (cmd, r) = parse_redirect("echo x 1> f");
        assert_eq!(cmd.trim(), "echo x");
        assert_eq!(r.stdout_file().map(|o| o.path), Some("f"));
    }

    #[test]
    fn redirections_apply_in_order() {
        // `2>&1 > f`: stderr keeps the old stdout; `> f 2>&1`: both in f.
        let (_, r) = parse_redirect("cmd 2>&1 > f");
        assert_eq!(
            (r.stdout, r.stderr),
            (StreamDest::File(0), StreamDest::Stdout)
        );
        let (_, r) = parse_redirect("cmd > f 2>&1");
        assert_eq!(
            (r.stdout, r.stderr),
            (StreamDest::File(0), StreamDest::File(0))
        );
        let (_, r) = parse_redirect("cmd >&2 2> e");
        assert_eq!(
            (r.stdout, r.stderr),
            (StreamDest::Stderr, StreamDest::File(0))
        );
        let (cmd, r) = parse_redirect("cat <<< 'a b' x");
        assert_eq!(cmd, "cat x");
        assert_eq!(r.stdin, Some(StdinSource::HereString("'a b'")));
        assert!(parse_redirect("cat << EOF").1.unsupported.is_some());
    }

    #[test]
    fn strip_ansi_removes_only_the_shells_colouring() {
        assert_eq!(
            strip_ansi(&format!("{} file", dir_colored("dir/"))),
            "dir/ file"
        );
        assert_eq!(strip_ansi("plain"), "plain");
        // File contents with their own escapes are left alone.
        let art = "\u{1b}[31mred\u{1b}[0m \u{1b}[";
        assert_eq!(strip_ansi(art), art);
    }
}

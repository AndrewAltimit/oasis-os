use oasis_types::error::Result;
use oasis_vfs::Vfs;

use super::expander::resolve_path;
use super::interpreter::CommandOutput;

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
}

/// Split a command line on `;`, `&&`, and `||` (respecting quotes).
pub(crate) fn split_chains(input: &str) -> Result<Vec<ChainSegment>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chain_op = ChainOp::Always;
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;

    while let Some(ch) = chars.next() {
        if in_single {
            current.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(ch);
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(next) = chars.next()
            {
                current.push(next);
            }
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                current.push(ch);
            },
            '"' => {
                in_double = true;
                current.push(ch);
            },
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            },
            '{' => {
                brace_depth += 1;
                current.push(ch);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                current.push(ch);
            },
            ';' if brace_depth > 0 => {
                // Inside braces: don't split, keep as part of the body.
                current.push(ch);
            },
            ';' => {
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::Always;
            },
            '&' if brace_depth > 0 => current.push(ch),
            '&' if chars.peek() == Some(&'&') => {
                chars.next(); // consume second &
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::And;
            },
            '|' if brace_depth > 0 && chars.peek() == Some(&'|') => {
                current.push(ch);
                // peek() returned Some above, so next() is guaranteed to yield a value.
                current.push(chars.next().expect("peek() confirmed char available"));
            },
            '|' if chars.peek() == Some(&'|') => {
                chars.next(); // consume second |
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::Or;
            },
            _ => current.push(ch),
        }
    }

    let cmd = current.trim().to_string();
    if !cmd.is_empty() {
        segments.push(ChainSegment {
            command: cmd,
            chain_op,
        });
    }

    Ok(segments)
}

// ---------------------------------------------------------------------------
// Pipe splitting
// ---------------------------------------------------------------------------

/// Split on `|` (single pipe, not `||`), respecting quotes.
pub(crate) fn split_pipes(input: &str) -> Result<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;

    while let Some(ch) = chars.next() {
        if in_single {
            current.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(ch);
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(next) = chars.next()
            {
                current.push(next);
            }
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                current.push(ch);
            },
            '"' => {
                in_double = true;
                current.push(ch);
            },
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            },
            '{' => {
                brace_depth += 1;
                current.push(ch);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                current.push(ch);
            },
            '|' if brace_depth == 0 && chars.peek() != Some(&'|') => {
                segments.push(current.trim().to_string());
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        segments.push(remaining);
    }

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

/// Parse redirect operators from a command string.
///
/// Supports:
/// - `> file` / `>> file`   (stdout)
/// - `2> file` / `2>> file` (stderr)
/// - `2>&1`                 (merge stderr into stdout)
///
/// Returns `(command_part, redirections)`.
pub(crate) fn parse_redirect(input: &str) -> (&str, Redirections<'_>) {
    let bytes = input.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;
    let mut i = 0;
    let mut first_redirect_pos: Option<usize> = None;

    // Collected redirect entries: (position, fd, append, merge_target).
    // fd: 1 = stdout, 2 = stderr.
    // merge_target: Some("1") for `2>&1`.
    let mut redirects: Vec<(usize, u8, bool, bool)> = Vec::new();

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' {
                i += 1; // skip next
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b'2' if brace_depth == 0
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'>'
                    && (i == 0 || bytes[i - 1].is_ascii_whitespace()) =>
                {
                    if first_redirect_pos.is_none() {
                        first_redirect_pos = Some(i);
                    }
                    // Check for 2>> or 2>&1.
                    if i + 3 < bytes.len() && bytes[i + 2] == b'&' && bytes[i + 3] == b'1' {
                        // 2>&1 -- merge stderr into stdout.
                        redirects.push((i, 2, false, true));
                        i += 3;
                    } else if i + 2 < bytes.len() && bytes[i + 2] == b'>' {
                        // 2>>
                        redirects.push((i, 2, true, false));
                        i += 2;
                    } else {
                        // 2>
                        redirects.push((i, 2, false, false));
                        i += 1;
                    }
                },
                b'>' if brace_depth == 0 => {
                    if first_redirect_pos.is_none() {
                        first_redirect_pos = Some(i);
                    }
                    if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                        redirects.push((i, 1, true, false));
                        i += 1;
                    } else {
                        redirects.push((i, 1, false, false));
                    }
                },
                // fd 0 = stdin redirect (`< file`).
                b'<' if brace_depth == 0 => {
                    if first_redirect_pos.is_none() {
                        first_redirect_pos = Some(i);
                    }
                    redirects.push((i, 0, false, false));
                },
                _ => {},
            }
        }
        i += 1;
    }

    if redirects.is_empty() {
        return (
            input,
            Redirections {
                stdout: None,
                stderr: None,
                stdin: None,
                stderr_to_stdout: false,
            },
        );
    }

    let cmd_part = &input[..first_redirect_pos.unwrap_or(input.len())];

    let mut stdout_redirect: Option<Redirect<'_>> = None;
    let mut stderr_redirect: Option<Redirect<'_>> = None;
    let mut stdin_redirect: Option<&str> = None;
    let mut stderr_to_stdout = false;

    for (idx, &(pos, fd, append, merge)) in redirects.iter().enumerate() {
        if merge {
            stderr_to_stdout = true;
            continue;
        }

        // Path is everything from after the operator to the next redirect
        // or end of string.
        let op_len = if fd == 2 {
            if append { 3 } else { 2 } // "2>>" = 3, "2>" = 2
        } else if append {
            2
        } else {
            1
        }; // ">>" = 2, ">" = 1, "<" = 1

        let path_start = pos + op_len;
        let path_end = if idx + 1 < redirects.len() {
            redirects[idx + 1].0
        } else {
            input.len()
        };
        let path = input[path_start..path_end].trim();

        if fd == 0 {
            stdin_redirect = Some(path);
        } else {
            let redir = Redirect { path, append };
            if fd == 2 {
                stderr_redirect = Some(redir);
            } else {
                stdout_redirect = Some(redir);
            }
        }
    }

    (
        cmd_part,
        Redirections {
            stdout: stdout_redirect,
            stderr: stderr_redirect,
            stdin: stdin_redirect,
            stderr_to_stdout,
        },
    )
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
        let existing = if vfs.exists(&path) {
            let data = vfs.read(&path)?;
            String::from_utf8_lossy(&data).into_owned()
        } else {
            String::new()
        };
        let combined = if existing.is_empty() {
            text.to_string()
        } else {
            format!("{existing}\n{text}")
        };
        vfs.write(&path, combined.as_bytes())?;
    } else {
        vfs.write(&path, text.as_bytes())?;
    }
    Ok(())
}

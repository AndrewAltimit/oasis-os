//! Text processing commands: head, tail, wc, grep, sort, uniq, tee, tr, cut, diff.
//!
//! Each command family lives in its own submodule:
//!
//! - `lines` -- `head`, `tail`, `wc`
//! - `filter` -- `grep`, `sort`, `uniq`
//! - `stream` -- `tee`, `tr`, `cut`
//! - `diff` -- `diff`
//!
//! This module holds [`register_text_commands`] and the input helpers the
//! commands share.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{CommandRegistry, Environment, resolve_path};

mod diff;
mod filter;
mod lines;
mod stream;
#[cfg(test)]
mod tests;

/// Register the text processing commands (`head`, `tail`, `wc`, `grep`,
/// `sort`, `uniq`, `tee`, `tr`, `cut`, `diff`).
pub fn register_text_commands(reg: &mut CommandRegistry) {
    lines::register(reg);
    filter::register(reg);
    stream::register(reg);
    diff::register(reg);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `head` / `tail` arguments: `-n N`, `-nN` or the historical `-N`,
/// and at most one file. Returns (count, optional file path).
fn parse_n_flag<'a>(
    args: &[&'a str],
    default: usize,
    usage: &str,
) -> Result<(usize, Option<&'a str>)> {
    let bad_count = |v: &str| OasisError::Command(format!("invalid line count: {v}").into());
    let mut n = None;
    // Only plain digits: `usize::from_str` takes `+5`, which would make
    // `tail -n +5` (POSIX "from line 5") print the last 5 lines.
    let count = |v: &str| -> Result<usize> {
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad_count(v));
        }
        v.parse().map_err(|_| bad_count(v))
    };
    let mut rest = Vec::with_capacity(args.len());
    let mut operands_only = false;
    for &arg in args {
        if operands_only {
            rest.push(arg);
            continue;
        }
        match arg.strip_prefix('-') {
            // After `--` a `-5` is a file name (`head -- -5`).
            Some("-") => {
                operands_only = true;
                rest.push(arg);
            },
            Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                n = Some(digits.parse().map_err(|_| bad_count(digits))?);
            },
            _ => rest.push(arg),
        }
    }
    let spec = crate::cmd_helpers::FlagSpec {
        switches: "",
        valued: "n",
        long: &[("lines", 'n')],
    };
    let flags = crate::cmd_helpers::parse_flags(&rest, &spec, usage)?;
    if let Some(v) = flags.value('n') {
        n = Some(count(v)?);
    }
    match flags.positional[..] {
        [] => Ok((n.unwrap_or(default), None)),
        [file] => Ok((n.unwrap_or(default), Some(file))),
        _ => Err(OasisError::Command(format!("usage: {usage}").into())),
    }
}
/// Read text from a file path or stdin.
fn read_text_input(file: Option<&str>, env: &mut Environment<'_>) -> Result<String> {
    if let Some(path) = file.filter(|&p| p != "-") {
        let full = resolve_path(&env.cwd, path);
        let data = env.vfs.read(&full)?;
        Ok(String::from_utf8_lossy(&data).into_owned())
    } else if let Some(ref stdin) = env.stdin {
        Ok(stdin.clone())
    } else {
        Ok(String::new())
    }
}

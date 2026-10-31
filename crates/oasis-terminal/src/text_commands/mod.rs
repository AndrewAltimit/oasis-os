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

/// Parse `-n N` flag from args, returning (count, optional file path).
fn parse_n_flag<'a>(args: &[&'a str], default: usize) -> Result<(usize, Option<&'a str>)> {
    let mut n = default;
    let mut file = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-n" {
            i += 1;
            if i < args.len() {
                n = args[i]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
            }
        } else {
            file = Some(args[i]);
        }
        i += 1;
    }
    Ok((n, file))
}

/// Read text from a file path or stdin.
fn read_text_input(file: Option<&str>, env: &mut Environment<'_>) -> Result<String> {
    if let Some(path) = file {
        let full = resolve_path(&env.cwd, path);
        let data = env.vfs.read(&full)?;
        Ok(String::from_utf8_lossy(&data).into_owned())
    } else if let Some(ref stdin) = env.stdin {
        Ok(stdin.clone())
    } else {
        Ok(String::new())
    }
}

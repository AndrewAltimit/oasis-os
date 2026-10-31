//! Shared helpers for command argument validation.
//!
//! Eliminates repeated argument-count checks and usage-string error
//! construction across the 90+ commands in oasis-terminal.

use oasis_types::error::{OasisError, Result};

/// Require at least `min` arguments, returning a usage error if fewer are provided.
///
/// ```ignore
/// require_args(args, 1, "cat <file>")?;
/// ```
pub fn require_args(args: &[&str], min: usize, usage: &str) -> Result<()> {
    if args.len() < min {
        return Err(OasisError::Command(format!("usage: {usage}").into()));
    }
    Ok(())
}

/// Require exactly `n` arguments, returning a usage error otherwise.
///
/// ```ignore
/// require_args_exact(args, 2, "cp <src> <dst>")?;
/// ```
pub fn require_args_exact(args: &[&str], n: usize, usage: &str) -> Result<()> {
    if args.len() != n {
        return Err(OasisError::Command(format!("usage: {usage}").into()));
    }
    Ok(())
}

/// Which options a command accepts (see [`parse_flags`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct FlagSpec<'s> {
    /// Single-letter switches without a value (`"inv"` = `-i`, `-n`, `-v`).
    pub switches: &'s str,
    /// Single-letter options that take a value (`"n"` = `-n 5` / `-n5`).
    pub valued: &'s str,
    /// Long options and the letter each one is an alias of
    /// (`("decode", 'd')` = `--decode`; a valued letter takes
    /// `--name=value` or `--name value`).
    pub long: &'s [(&'s str, char)],
}

impl<'s> FlagSpec<'s> {
    /// A spec with switches only.
    pub const fn switches(switches: &'s str) -> Self {
        Self {
            switches,
            valued: "",
            long: &[],
        }
    }
}

/// Command-line options parsed by [`parse_flags`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flags<'a> {
    switches: Vec<char>,
    values: Vec<(char, &'a str)>,
    /// Arguments that are not options, in order.
    pub positional: Vec<&'a str>,
}

impl<'a> Flags<'a> {
    /// Whether switch (or valued option) `c` was given.
    pub fn has(&self, c: char) -> bool {
        self.switches.contains(&c) || self.values.iter().any(|(k, _)| *k == c)
    }

    /// The value of option `c` (the last one when given several times).
    pub fn value(&self, c: char) -> Option<&'a str> {
        self.values
            .iter()
            .rev()
            .find(|(k, _)| *k == c)
            .map(|(_, v)| *v)
    }
}

/// Parse POSIX-style options out of `args`.
///
/// - `-abc` is `-a -b -c`; a valued letter takes the rest of its word or
///   the next argument (`-n5`, `-n 5`, `-an5`).
/// - `--name` / `--name=value` map to letters through [`FlagSpec::long`].
/// - `--` ends the options; `-` alone is an operand (stdin by
///   convention). Options may appear anywhere among the operands.
/// - An unknown option, or a valued one with no value, is an error that
///   carries the `usage` line (whose first word names the command).
pub fn parse_flags<'a>(args: &[&'a str], spec: &FlagSpec<'_>, usage: &str) -> Result<Flags<'a>> {
    let name = usage.split_whitespace().next().unwrap_or("");
    let bad = |what: String| OasisError::Command(format!("{name}: {what}\nusage: {usage}").into());
    let mut flags = Flags::default();
    let mut iter = args.iter().copied();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            flags.positional.extend(iter.by_ref());
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (key, inline) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (long, None),
            };
            let Some(&(_, c)) = spec.long.iter().find(|(n, _)| *n == key) else {
                return Err(bad(format!("unknown option '--{key}'")));
            };
            if spec.valued.contains(c) {
                let value = inline
                    .or_else(|| iter.next())
                    .ok_or_else(|| bad(format!("option '--{key}' needs a value")))?;
                flags.values.push((c, value));
            } else if inline.is_some() {
                return Err(bad(format!("option '--{key}' takes no value")));
            } else {
                flags.switches.push(c);
            }
            continue;
        }
        let Some(letters) = arg.strip_prefix('-').filter(|l| !l.is_empty()) else {
            flags.positional.push(arg);
            continue;
        };
        for (i, c) in letters.char_indices() {
            if spec.valued.contains(c) {
                let rest = &letters[i + c.len_utf8()..];
                let value = if rest.is_empty() {
                    iter.next()
                        .ok_or_else(|| bad(format!("option '-{c}' needs a value")))?
                } else {
                    rest
                };
                flags.values.push((c, value));
                break;
            }
            if !spec.switches.contains(c) {
                return Err(bad(format!("unknown option '-{c}'")));
            }
            flags.switches.push(c);
        }
    }
    Ok(flags)
}

/// Platform-safe seed for PRNG. Uses `TimeService` when available (required on
/// WASM where `std::time::SystemTime::now()` panics), falls back to std on native.
pub fn time_seed(env: &crate::Environment<'_>) -> u64 {
    if let Some(time) = env.time
        && let Ok(now) = time.now()
    {
        return (now.year as u64) << 40
            | (now.month as u64) << 32
            | (now.day as u64) << 24
            | (now.hour as u64) << 16
            | (now.minute as u64) << 8
            | (now.second as u64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    }
    #[cfg(target_arch = "wasm32")]
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_args_ok_when_enough() {
        assert!(require_args(&["a", "b"], 1, "test <arg>").is_ok());
        assert!(require_args(&["a", "b"], 2, "test <a> <b>").is_ok());
    }

    #[test]
    fn require_args_err_when_too_few() {
        let err = require_args(&[], 1, "cat <file>").unwrap_err();
        assert!(format!("{err}").contains("usage: cat <file>"));
    }

    #[test]
    fn require_args_zero_always_ok() {
        assert!(require_args(&[], 0, "test").is_ok());
    }

    #[test]
    fn require_args_exact_ok() {
        assert!(require_args_exact(&["a", "b"], 2, "cp <src> <dst>").is_ok());
    }

    #[test]
    fn require_args_exact_too_few() {
        let err = require_args_exact(&["a"], 2, "cp <src> <dst>").unwrap_err();
        assert!(format!("{err}").contains("usage: cp <src> <dst>"));
    }

    const SPEC: FlagSpec<'static> = FlagSpec {
        switches: "abc",
        valued: "n",
        long: &[("all", 'a'), ("lines", 'n')],
    };

    #[test]
    fn parse_flags_combined_and_valued() {
        let f = parse_flags(&["-ab", "x", "-n5", "y", "-cn", "7"], &SPEC, "t [-abc]").unwrap();
        assert!(f.has('a') && f.has('b') && f.has('c'));
        assert_eq!(f.value('n'), Some("7"));
        assert_eq!(f.positional, vec!["x", "y"]);
        let f = parse_flags(&["-n", "3", "--all", "--lines=9"], &SPEC, "t").unwrap();
        assert!(f.has('a'));
        assert_eq!(f.value('n'), Some("9"));
    }

    #[test]
    fn parse_flags_double_dash_and_lone_dash() {
        let f = parse_flags(&["-a", "--", "-b", "-"], &SPEC, "t").unwrap();
        assert!(f.has('a') && !f.has('b'));
        assert_eq!(f.positional, vec!["-b", "-"]);
        let f = parse_flags(&["-"], &SPEC, "t").unwrap();
        assert_eq!(f.positional, vec!["-"]);
    }

    #[test]
    fn parse_flags_rejects_unknown_and_missing_value() {
        let err = parse_flags(&["-az"], &SPEC, "tool [-abc] <x>").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("tool: unknown option '-z'"), "{msg}");
        assert!(msg.contains("usage: tool [-abc] <x>"), "{msg}");
        assert!(parse_flags(&["-n"], &SPEC, "t").is_err());
        assert!(parse_flags(&["--nope"], &SPEC, "t").is_err());
        assert!(parse_flags(&["--all=1"], &SPEC, "t").is_err());
    }

    #[test]
    fn require_args_exact_too_many() {
        let err = require_args_exact(&["a", "b", "c"], 2, "cp <src> <dst>").unwrap_err();
        assert!(format!("{err}").contains("usage:"));
    }
}

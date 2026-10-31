//! `grep`, `sort` and `uniq`.

use oasis_types::error::OasisError;

use super::read_text_input;
use crate::cmd_helpers::{FlagSpec, parse_flags};
use crate::interpreter::CommandOutput;

const GREP_USAGE: &str = "grep [-cinqv] <pattern> [file]";
const SORT_USAGE: &str = "sort [-nru] [file]";
const UNIQ_USAGE: &str = "uniq [-c] [file]";

// ---------------------------------------------------------------------------
// grep
// ---------------------------------------------------------------------------

define_command!(
    GrepCmd,
    "grep",
    "Search for pattern in text",
    GREP_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("cinqv"), GREP_USAGE)?;
        let (case_insensitive, show_numbers) = (flags.has('i'), flags.has('n'));
        let (invert, count_only, quiet) = (flags.has('v'), flags.has('c'), flags.has('q'));
        let Some(&pattern) = flags.positional.first() else {
            return Err(OasisError::Command(format!("usage: {GREP_USAGE}").into()));
        };
        let text = read_text_input(flags.positional.get(1).copied(), env)?;

        let pat = if case_insensitive {
            pattern.to_ascii_lowercase()
        } else {
            pattern.to_string()
        };

        let mut matches = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let haystack = if case_insensitive {
                line.to_ascii_lowercase()
            } else {
                line.to_string()
            };
            let found = haystack.contains(&pat);
            if found != invert {
                if show_numbers {
                    matches.push(format!("{}:{line}", i + 1));
                } else {
                    matches.push(line.to_string());
                }
            }
        }

        // Exit status 1 when nothing matched. No placeholder text: the
        // output feeds pipes and `$(...)` (`grep x f | wc -l` counts 0).
        let code = i32::from(matches.is_empty());
        let output = if quiet || (matches.is_empty() && !count_only) {
            CommandOutput::None
        } else if count_only {
            CommandOutput::Text(matches.len().to_string())
        } else {
            CommandOutput::Text(matches.join("\n"))
        };
        Ok(CommandOutput::status(code, output))
    }
);

// ---------------------------------------------------------------------------
// sort
// ---------------------------------------------------------------------------

define_command!(
    SortCmd,
    "sort",
    "Sort lines of text",
    SORT_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("nru"), SORT_USAGE)?;
        let text = read_text_input(flags.positional.first().copied(), env)?;
        let mut lines: Vec<&str> = text.lines().collect();

        if flags.has('n') {
            let key = |s: &str| -> f64 {
                s.split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0)
            };
            lines.sort_by(|a, b| {
                key(a)
                    .partial_cmp(&key(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        } else {
            lines.sort();
        }
        if flags.has('r') {
            lines.reverse();
        }
        if flags.has('u') {
            lines.dedup();
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// uniq
// ---------------------------------------------------------------------------

define_command!(
    UniqCmd,
    "uniq",
    "Remove adjacent duplicate lines",
    UNIQ_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("c"), UNIQ_USAGE)?;
        let show_count = flags.has('c');
        let text = read_text_input(flags.positional.first().copied(), env)?;
        let mut result = Vec::new();
        let mut last: Option<&str> = None;
        let mut count = 0usize;

        let mut emit = |prev: &str, count: usize| {
            if show_count {
                result.push(format!("{count:>7} {prev}"));
            } else {
                result.push(prev.to_string());
            }
        };
        for line in text.lines() {
            if last == Some(line) {
                count += 1;
            } else {
                if let Some(prev) = last {
                    emit(prev, count);
                }
                last = Some(line);
                count = 1;
            }
        }
        if let Some(prev) = last {
            emit(prev, count);
        }
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

register_commands!(register, [GrepCmd, SortCmd, UniqCmd]);

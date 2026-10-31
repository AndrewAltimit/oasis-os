//! `grep`, `sort` and `uniq`.

use oasis_types::error::OasisError;

use super::read_text_input;
use crate::interpreter::CommandOutput;

// ---------------------------------------------------------------------------
// grep
// ---------------------------------------------------------------------------

define_command!(
    GrepCmd,
    "grep",
    "Search for pattern in text",
    "grep [-i] [-n] [-v] [-c] <pattern> [file]",
    "text",
    |args, env| {
        let mut case_insensitive = false;
        let mut show_numbers = false;
        let mut invert = false;
        let mut count_only = false;
        let mut positional = Vec::new();

        for &arg in args {
            match arg {
                "-i" => case_insensitive = true,
                "-n" => show_numbers = true,
                "-v" => invert = true,
                "-c" => count_only = true,
                _ => positional.push(arg),
            }
        }
        if positional.is_empty() {
            return Err(OasisError::Command(
                "usage: grep [-i] [-n] [-v] [-c] <pattern> [file]".into(),
            ));
        }
        let pattern = positional[0];
        let text = read_text_input(positional.get(1).copied(), env)?;

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

        if count_only {
            let code = i32::from(matches.is_empty());
            Ok(CommandOutput::status(
                code,
                CommandOutput::Text(format!("{}", matches.len())),
            ))
        } else if matches.is_empty() {
            Ok(CommandOutput::status(
                1,
                CommandOutput::Text("(no matches)".to_string()),
            ))
        } else {
            Ok(CommandOutput::Text(matches.join("\n")))
        }
    }
);

// ---------------------------------------------------------------------------
// sort
// ---------------------------------------------------------------------------

define_command!(
    SortCmd,
    "sort",
    "Sort lines of text",
    "sort [-r] [-n] [file]",
    "text",
    |args, env| {
        let mut reverse = false;
        let mut numeric = false;
        let mut file_arg = None;
        for &arg in args {
            match arg {
                "-r" => reverse = true,
                "-n" => numeric = true,
                _ => file_arg = Some(arg),
            }
        }
        let text = read_text_input(file_arg, env)?;
        let mut lines: Vec<&str> = text.lines().collect();

        if numeric {
            lines.sort_by(|a, b| {
                let na: f64 = a
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let nb: f64 = b
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            });
        } else {
            lines.sort();
        }
        if reverse {
            lines.reverse();
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
    "uniq [-c] [file]",
    "text",
    |args, env| {
        let mut show_count = false;
        let mut file_arg = None;
        for &arg in args {
            match arg {
                "-c" => show_count = true,
                _ => file_arg = Some(arg),
            }
        }
        let text = read_text_input(file_arg, env)?;
        let mut result = Vec::new();
        let mut last: Option<&str> = None;
        let mut count = 0usize;

        for line in text.lines() {
            if last == Some(line) {
                count += 1;
            } else {
                if let Some(prev) = last {
                    if show_count {
                        result.push(format!("{count:>7} {prev}"));
                    } else {
                        result.push(prev.to_string());
                    }
                }
                last = Some(line);
                count = 1;
            }
        }
        if let Some(prev) = last {
            if show_count {
                result.push(format!("{count:>7} {prev}"));
            } else {
                result.push(prev.to_string());
            }
        }
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

register_commands!(register, [GrepCmd, SortCmd, UniqCmd]);

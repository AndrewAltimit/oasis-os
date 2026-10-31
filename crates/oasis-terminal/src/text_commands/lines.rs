//! `head`, `tail` and `wc`.

use super::{parse_n_flag, read_text_input};
use crate::cmd_helpers::{FlagSpec, parse_flags};
use crate::interpreter::CommandOutput;

const HEAD_USAGE: &str = "head [-n N] [file]";
const TAIL_USAGE: &str = "tail [-n N] [file]";
const WC_USAGE: &str = "wc [-clw] [file]";

// ---------------------------------------------------------------------------
// head
// ---------------------------------------------------------------------------

define_command!(
    HeadCmd,
    "head",
    "Show first N lines of a file",
    HEAD_USAGE,
    "text",
    |args, env| {
        let (n, file) = parse_n_flag(args, 10, HEAD_USAGE)?;
        let text = read_text_input(file, env)?;
        let result: Vec<&str> = text.lines().take(n).collect();
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// tail
// ---------------------------------------------------------------------------

define_command!(
    TailCmd,
    "tail",
    "Show last N lines of a file",
    TAIL_USAGE,
    "text",
    |args, env| {
        let (n, file) = parse_n_flag(args, 10, TAIL_USAGE)?;
        let text = read_text_input(file, env)?;
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        Ok(CommandOutput::Text(lines[start..].join("\n")))
    }
);

// ---------------------------------------------------------------------------
// wc
// ---------------------------------------------------------------------------

define_command!(
    WcCmd,
    "wc",
    "Count lines, words, and bytes",
    WC_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("clw"), WC_USAGE)?;
        let text = read_text_input(flags.positional.first().copied(), env)?;
        let counts = [
            ('l', text.lines().count()),
            ('w', text.split_whitespace().count()),
            ('c', text.len()),
        ];
        let picked: Vec<usize> = counts
            .iter()
            .filter(|(c, _)| flags.has(*c))
            .map(|&(_, n)| n)
            .collect();
        let text = match picked[..] {
            // One count alone prints bare (`wc -l` feeds arithmetic).
            [n] => n.to_string(),
            [] => counts.map(|(_, n)| format!("{n:>8}")).join(" "),
            _ => picked
                .iter()
                .map(|n| format!("{n:>8}"))
                .collect::<Vec<_>>()
                .join(" "),
        };
        Ok(CommandOutput::Text(text))
    }
);

register_commands!(register, [HeadCmd, TailCmd, WcCmd]);

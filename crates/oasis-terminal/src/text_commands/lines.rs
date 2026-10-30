//! `head`, `tail` and `wc`.

use super::{parse_n_flag, read_text_input};
use crate::interpreter::CommandOutput;

// ---------------------------------------------------------------------------
// head
// ---------------------------------------------------------------------------

define_command!(
    HeadCmd,
    "head",
    "Show first N lines of a file",
    "head [-n N] <file>",
    "text",
    |args, env| {
        let (n, file) = parse_n_flag(args, 10)?;
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
    "tail [-n N] <file>",
    "text",
    |args, env| {
        let (n, file) = parse_n_flag(args, 10)?;
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
    "wc [-l|-w|-c] <file>",
    "text",
    |args, env| {
        let mut mode = "all";
        let mut file_args = Vec::new();
        for &arg in args {
            match arg {
                "-l" => mode = "lines",
                "-w" => mode = "words",
                "-c" => mode = "bytes",
                _ => file_args.push(arg),
            }
        }
        let text = read_text_input(file_args.first().copied(), env)?;
        let lines = text.lines().count();
        let words = text.split_whitespace().count();
        let bytes = text.len();
        match mode {
            "lines" => Ok(CommandOutput::Text(format!("{lines}"))),
            "words" => Ok(CommandOutput::Text(format!("{words}"))),
            "bytes" => Ok(CommandOutput::Text(format!("{bytes}"))),
            _ => Ok(CommandOutput::Text(format!(
                "{lines:>8} {words:>8} {bytes:>8}"
            ))),
        }
    }
);

register_commands!(register, [HeadCmd, TailCmd, WcCmd]);

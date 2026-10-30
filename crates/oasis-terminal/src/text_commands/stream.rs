//! `tee`, `tr` and `cut`.

use oasis_types::error::{OasisError, Result};

use super::read_text_input;
use crate::interpreter::{CommandOutput, resolve_path};

// ---------------------------------------------------------------------------
// tee
// ---------------------------------------------------------------------------

define_command!(
    TeeCmd,
    "tee",
    "Read stdin, write to file and stdout",
    "tee [-a] <file>",
    "text",
    |args, env| {
        let mut append = false;
        let mut file_arg = None;
        for &arg in args {
            match arg {
                "-a" => append = true,
                _ if file_arg.is_none() => file_arg = Some(arg),
                _ => {},
            }
        }
        let file = file_arg.ok_or_else(|| OasisError::Command("usage: tee [-a] <file>".into()))?;
        let path = resolve_path(&env.cwd, file);
        let input = env.stdin.clone().unwrap_or_default();

        if append && env.vfs.exists(&path) {
            let existing = env.vfs.read(&path)?;
            let mut data = existing;
            data.extend_from_slice(input.as_bytes());
            env.vfs.write(&path, &data)?;
        } else {
            env.vfs.write(&path, input.as_bytes())?;
        }
        Ok(CommandOutput::Text(input))
    }
);

// ---------------------------------------------------------------------------
// tr
// ---------------------------------------------------------------------------

define_command!(
    TrCmd,
    "tr",
    "Translate or delete characters",
    "tr [-d] <set1> [set2]",
    "text",
    |args, env| {
        let mut delete = false;
        let mut positional = Vec::new();
        for &arg in args {
            if arg == "-d" {
                delete = true;
            } else {
                positional.push(arg);
            }
        }
        if positional.is_empty() {
            return Err(OasisError::Command("usage: tr [-d] <set1> [set2]".into()));
        }
        let set1 = positional[0];
        let text = read_text_input(None, env)?;

        if delete {
            let result: String = text.chars().filter(|c| !set1.contains(*c)).collect();
            Ok(CommandOutput::Text(result))
        } else {
            let set2 = positional.get(1).copied().unwrap_or("");
            let set1_chars: Vec<char> = set1.chars().collect();
            let set2_chars: Vec<char> = set2.chars().collect();
            let result: String = text
                .chars()
                .map(|c| {
                    if let Some(pos) = set1_chars.iter().position(|&s| s == c) {
                        set2_chars
                            .get(pos)
                            .copied()
                            .unwrap_or(*set2_chars.last().unwrap_or(&c))
                    } else {
                        c
                    }
                })
                .collect();
            Ok(CommandOutput::Text(result))
        }
    }
);

// ---------------------------------------------------------------------------
// cut
// ---------------------------------------------------------------------------

define_command!(
    CutCmd,
    "cut",
    "Extract fields or columns",
    "cut -d <delim> -f <fields> [file]",
    "text",
    |args, env| {
        let mut delim = "\t";
        let mut fields_str = "";
        let mut file_arg = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" => {
                    i += 1;
                    if i < args.len() {
                        delim = args[i];
                    }
                },
                "-f" => {
                    i += 1;
                    if i < args.len() {
                        fields_str = args[i];
                    }
                },
                _ => file_arg = Some(args[i]),
            }
            i += 1;
        }
        if fields_str.is_empty() {
            return Err(OasisError::Command(
                "usage: cut -d <delim> -f <fields> [file]".into(),
            ));
        }
        let fields = parse_field_spec(fields_str)?;
        let text = read_text_input(file_arg, env)?;

        let mut result = Vec::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.split(delim).collect();
            let selected: Vec<&str> = fields
                .iter()
                .filter_map(|&f| {
                    if f > 0 {
                        parts.get(f - 1).copied()
                    } else {
                        None
                    }
                })
                .collect();
            result.push(selected.join(delim));
        }
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

/// Parse a field spec like "1,3" or "2-4" into a list of 1-based indices.
fn parse_field_spec(spec: &str) -> Result<Vec<usize>> {
    let mut fields = Vec::new();
    for part in spec.split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let start: usize = a
                .parse()
                .map_err(|_| OasisError::Command(format!("bad field: {part}").into()))?;
            let end: usize = b
                .parse()
                .map_err(|_| OasisError::Command(format!("bad field: {part}").into()))?;
            for f in start..=end {
                fields.push(f);
            }
        } else {
            let f: usize = part
                .parse()
                .map_err(|_| OasisError::Command(format!("bad field: {part}").into()))?;
            fields.push(f);
        }
    }
    Ok(fields)
}

register_commands!(register, [TeeCmd, TrCmd, CutCmd]);

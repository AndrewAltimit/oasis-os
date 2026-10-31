//! `tee`, `tr` and `cut`.

use oasis_types::error::{OasisError, Result};

use super::read_text_input;
use crate::cmd_helpers::{FlagSpec, parse_flags};
use crate::interpreter::{CommandOutput, resolve_path};

const TEE_USAGE: &str = "tee [-a] <file>";
const TR_USAGE: &str = "tr [-d] <set1> [set2]";
const CUT_USAGE: &str = "cut [-d <delim>] -f <fields> [file]";

// ---------------------------------------------------------------------------
// tee
// ---------------------------------------------------------------------------

define_command!(
    TeeCmd,
    "tee",
    "Read stdin, write to file and stdout",
    TEE_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("a"), TEE_USAGE)?;
        let [file] = flags.positional[..] else {
            return Err(OasisError::Command(format!("usage: {TEE_USAGE}").into()));
        };
        let path = resolve_path(&env.cwd, file);
        let input = env.stdin.clone().unwrap_or_default();

        if flags.has('a') && env.vfs.exists(&path) {
            let mut data = env.vfs.read(&path)?;
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

/// Expand a `tr` set: `a-z` ranges (ascending, at most 256 characters)
/// and the escapes `\n`, `\t`, `\\` and `\-`.
fn expand_tr_set(set: &str) -> Result<Vec<char>> {
    // (character, came from an escape) -- an escaped `-` is never a range.
    let mut tokens = Vec::new();
    let mut it = set.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            tokens.push((c, false));
            continue;
        }
        let escaped = match it.next() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some(other) => other,
            None => '\\',
        };
        tokens.push((escaped, true));
    }
    let mut chars = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let lo = tokens[i].0;
        match (tokens.get(i + 1), tokens.get(i + 2)) {
            (Some(&('-', false)), Some(&(hi, _))) => {
                if hi < lo || (hi as u32 - lo as u32) >= 256 {
                    return Err(OasisError::Command(
                        format!("tr: invalid range '{lo}-{hi}'").into(),
                    ));
                }
                chars.extend(lo..=hi);
                i += 3;
            },
            _ => {
                chars.push(lo);
                i += 1;
            },
        }
    }
    Ok(chars)
}

define_command!(
    TrCmd,
    "tr",
    "Translate or delete characters",
    TR_USAGE,
    "text",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("d"), TR_USAGE)?;
        let Some(&set1) = flags.positional.first() else {
            return Err(OasisError::Command(format!("usage: {TR_USAGE}").into()));
        };
        let set1 = expand_tr_set(set1)?;
        let text = read_text_input(None, env)?;

        if flags.has('d') {
            let result: String = text.chars().filter(|c| !set1.contains(c)).collect();
            return Ok(CommandOutput::Text(result));
        }
        let set2 = expand_tr_set(flags.positional.get(1).copied().unwrap_or(""))?;
        let result: String = text
            .chars()
            .map(|c| match set1.iter().position(|&s| s == c) {
                // A short set2 repeats its last character.
                Some(pos) => set2.get(pos).or(set2.last()).copied().unwrap_or(c),
                None => c,
            })
            .collect();
        Ok(CommandOutput::Text(result))
    }
);

// ---------------------------------------------------------------------------
// cut
// ---------------------------------------------------------------------------

define_command!(
    CutCmd,
    "cut",
    "Extract fields or columns",
    CUT_USAGE,
    "text",
    |args, env| {
        let spec = FlagSpec {
            switches: "",
            valued: "df",
            long: &[],
        };
        let flags = parse_flags(args, &spec, CUT_USAGE)?;
        let delim = flags.value('d').unwrap_or("\t");
        if delim.is_empty() {
            return Err(OasisError::Command("cut: empty delimiter".into()));
        }
        let Some(fields_str) = flags.value('f') else {
            return Err(OasisError::Command(format!("usage: {CUT_USAGE}").into()));
        };
        let fields = parse_field_spec(fields_str)?;
        let text = read_text_input(flags.positional.first().copied(), env)?;

        let mut result = Vec::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.split(delim).collect();
            let selected: Vec<&str> = fields
                .iter()
                .filter_map(|&f| f.checked_sub(1).and_then(|i| parts.get(i).copied()))
                .collect();
            result.push(selected.join(delim));
        }
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

/// Parse a field spec like "1,3" or "2-4" into a list of 1-based indices.
fn parse_field_spec(spec: &str) -> Result<Vec<usize>> {
    /// Upper bound on one range (`1-99999999` would allocate gigabytes).
    const MAX_FIELDS: usize = 4096;
    let bad = |part: &str| OasisError::Command(format!("bad field: {part}").into());
    let mut fields = Vec::new();
    for part in spec.split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let start: usize = a.parse().map_err(|_| bad(part))?;
            let end: usize = b.parse().map_err(|_| bad(part))?;
            if end < start || end - start >= MAX_FIELDS {
                return Err(bad(part));
            }
            fields.extend(start..=end);
        } else {
            fields.push(part.parse().map_err(|_| bad(part))?);
        }
    }
    Ok(fields)
}

register_commands!(register, [TeeCmd, TrCmd, CutCmd]);

#[cfg(test)]
mod tests {
    use super::expand_tr_set;

    #[test]
    fn tr_sets_expand_ranges_and_escapes() {
        let s = |t: &str| expand_tr_set(t).unwrap().into_iter().collect::<String>();
        assert_eq!(s("a-e"), "abcde");
        assert_eq!(s("a-cx"), "abcx");
        assert_eq!(s("-a"), "-a");
        assert_eq!(s("a-"), "a-");
        assert_eq!(s("\\n\\-x"), "\n-x");
        assert!(expand_tr_set("z-a").is_err());
    }
}

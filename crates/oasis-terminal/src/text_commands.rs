//! Text processing commands: head, tail, wc, grep, sort, uniq, tee, tr, cut, diff.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{CommandOutput, Environment, resolve_path};

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
            Ok(CommandOutput::Text(format!("{}", matches.len())))
        } else if matches.is_empty() {
            Ok(CommandOutput::Text("(no matches)".to_string()))
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

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

define_command!(
    DiffCmd,
    "diff",
    "Compare two files line by line",
    "diff <file1> <file2>",
    "text",
    |args, env| {
        if args.len() < 2 {
            return Err(OasisError::Command("usage: diff <file1> <file2>".into()));
        }
        let path1 = resolve_path(&env.cwd, args[0]);
        let path2 = resolve_path(&env.cwd, args[1]);
        let data1 = env.vfs.read(&path1)?;
        let data2 = env.vfs.read(&path2)?;
        let text1 = String::from_utf8_lossy(&data1);
        let text2 = String::from_utf8_lossy(&data2);

        let lines1: Vec<&str> = text1.lines().collect();
        let lines2: Vec<&str> = text2.lines().collect();

        let mut output = Vec::new();
        let max = lines1.len().max(lines2.len());

        for i in 0..max {
            let l1 = lines1.get(i).copied();
            let l2 = lines2.get(i).copied();
            match (l1, l2) {
                (Some(a), Some(b)) if a == b => {},
                (Some(a), Some(b)) => {
                    output.push(format!("{}c{}", i + 1, i + 1));
                    output.push(format!("< {a}"));
                    output.push("---".to_string());
                    output.push(format!("> {b}"));
                },
                (Some(a), None) => {
                    output.push(format!("{}d", i + 1));
                    output.push(format!("< {a}"));
                },
                (None, Some(b)) => {
                    output.push(format!("{}a", i + 1));
                    output.push(format!("> {b}"));
                },
                (None, None) => {},
            }
        }
        if output.is_empty() {
            Ok(CommandOutput::Text("Files are identical.".to_string()))
        } else {
            Ok(CommandOutput::Text(output.join("\n")))
        }
    }
);

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

register_commands!(
    register_text_commands,
    [
        HeadCmd, TailCmd, WcCmd, GrepCmd, SortCmd, UniqCmd, TeeCmd, TrCmd, CutCmd, DiffCmd,
    ]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_text_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/tmp/test.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon")
            .unwrap();
        (reg, vfs)
    }

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn head_default() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "head /tmp/test.txt").unwrap());
        assert!(s.contains("alpha"));
        assert!(s.contains("epsilon"));
    }

    #[test]
    fn head_n2() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "head -n 2 /tmp/test.txt").unwrap());
        assert!(s.contains("alpha"));
        assert!(s.contains("beta"));
        assert!(!s.contains("gamma"));
    }

    #[test]
    fn tail_n2() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "tail -n 2 /tmp/test.txt").unwrap());
        assert!(!s.contains("gamma"));
        assert!(s.contains("delta"));
        assert!(s.contains("epsilon"));
    }

    #[test]
    fn wc_all() {
        let (reg, mut vfs) = setup();
        assert!(assert_text!(exec(&reg, &mut vfs, "wc /tmp/test.txt").unwrap()).contains("5"));
    }

    #[test]
    fn wc_lines() {
        let (reg, mut vfs) = setup();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, "wc -l /tmp/test.txt").unwrap()).trim(),
            "5"
        );
    }

    #[test]
    fn grep_basic() {
        let (reg, mut vfs) = setup();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, "grep alpha /tmp/test.txt").unwrap()).trim(),
            "alpha"
        );
    }

    #[test]
    fn grep_case_insensitive() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/ci.txt", b"Hello\nhello\nHELLO").unwrap();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, "grep -i hello /tmp/ci.txt").unwrap())
                .lines()
                .count(),
            3
        );
    }

    #[test]
    fn grep_invert() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "grep -v alpha /tmp/test.txt").unwrap());
        assert!(!s.contains("alpha"));
        assert!(s.contains("beta"));
    }

    #[test]
    fn grep_count() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "grep -c a /tmp/test.txt").unwrap());
        let n: usize = s.trim().parse().unwrap();
        assert!(n >= 3); // alpha, gamma, delta, epsilon all contain 'a'
    }

    #[test]
    fn sort_basic() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/unsorted.txt", b"cherry\napple\nbanana")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "sort /tmp/unsorted.txt").unwrap());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, ["apple", "banana", "cherry"]);
    }

    #[test]
    fn sort_reverse() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/unsorted.txt", b"a\nb\nc").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "sort -r /tmp/unsorted.txt").unwrap());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, ["c", "b", "a"]);
    }

    #[test]
    fn sort_numeric() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/nums.txt", b"10\n2\n1\n20").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "sort -n /tmp/nums.txt").unwrap());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, ["1", "2", "10", "20"]);
    }

    #[test]
    fn uniq_basic() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/dup.txt", b"a\na\nb\nb\nb\nc").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "uniq /tmp/dup.txt").unwrap());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn uniq_count() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/dup.txt", b"x\nx\nx\ny").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "uniq -c /tmp/dup.txt").unwrap());
        assert!(s.contains("3"));
        assert!(s.contains("x"));
        assert!(s.contains("1"));
        assert!(s.contains("y"));
    }

    #[test]
    fn diff_identical() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/a.txt", b"hello\nworld").unwrap();
        vfs.write("/tmp/b.txt", b"hello\nworld").unwrap();
        assert!(
            assert_text!(exec(&reg, &mut vfs, "diff /tmp/a.txt /tmp/b.txt").unwrap())
                .contains("identical")
        );
    }

    #[test]
    fn diff_different() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/a.txt", b"hello\nworld").unwrap();
        vfs.write("/tmp/b.txt", b"hello\nearth").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "diff /tmp/a.txt /tmp/b.txt").unwrap());
        assert!(s.contains("< world"));
        assert!(s.contains("> earth"));
    }

    #[test]
    fn cut_fields() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/csv.txt", b"a,b,c\nd,e,f").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "cut -d , -f 1,3 /tmp/csv.txt").unwrap());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, ["a,c", "d,f"]);
    }

    #[test]
    fn tr_translate() {
        let mut reg = CommandRegistry::new();
        register_text_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: Some("hello".to_string()),
            stderr: String::new(),
        };
        assert_eq!(
            assert_text!(reg.execute("tr elo ELO", &mut env).unwrap()),
            "hELLO"
        );
    }

    #[test]
    fn tr_delete() {
        let mut reg = CommandRegistry::new();
        register_text_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: Some("hello world".to_string()),
            stderr: String::new(),
        };
        assert_eq!(
            assert_text!(reg.execute("tr -d lo", &mut env).unwrap()),
            "he wrd"
        );
    }
}

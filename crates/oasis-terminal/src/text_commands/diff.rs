//! `diff`: line-by-line file comparison.

use oasis_types::error::OasisError;

use crate::interpreter::{CommandOutput, resolve_path};

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
            // Exit 1 when the files differ, as POSIX diff does.
            Ok(CommandOutput::status(
                1,
                CommandOutput::Text(output.join("\n")),
            ))
        }
    }
);

register_commands!(register, [DiffCmd]);

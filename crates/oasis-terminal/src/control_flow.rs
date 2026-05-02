//! Shell control flow structures: if/then/else/fi, for/in/do/done, while/do/done.

use crate::interpreter::{CommandOutput, CommandRegistry, Environment};
use oasis_types::error::{OasisError, Result};

/// Execute an `if ... then ... [else ...] fi` block.
///
/// The condition is a command whose exit code determines which branch runs.
pub fn execute_if(
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
    condition: &str,
    then_body: &[String],
    else_body: &[String],
) -> Result<CommandOutput> {
    let cond_result = registry.execute(condition, env);
    let exit_code = registry.last_exit_code();

    // Check condition: exit code 0 = true, non-zero = false.
    // Also check for `test` command output of "false" since test returns Ok("false").
    let condition_true = if exit_code != 0 {
        false
    } else {
        !matches!(&cond_result, Ok(CommandOutput::Text(t)) if t.trim() == "false")
    };

    let body = if condition_true { then_body } else { else_body };

    let mut last_output = cond_result.unwrap_or(CommandOutput::None);
    for line in body {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        last_output = registry.execute(trimmed, env)?;
        if registry.break_flag() || registry.continue_flag() || registry.return_flag() {
            break;
        }
    }
    Ok(last_output)
}

/// Execute a `for VAR in WORDS... do BODY done` loop.
pub fn execute_for(
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
    var_name: &str,
    words: &[String],
    body: &[String],
) -> Result<CommandOutput> {
    let mut last_output = CommandOutput::None;
    for word in words {
        registry.set_variable(var_name, word);
        for line in body {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            last_output = registry.execute(trimmed, env)?;
            if registry.break_flag() || registry.return_flag() {
                break;
            }
            if registry.continue_flag() {
                registry.clear_continue();
                break;
            }
        }
        if registry.break_flag() {
            registry.clear_break();
            break;
        }
        if registry.return_flag() {
            break;
        }
    }
    Ok(last_output)
}

/// Execute a `while CONDITION do BODY done` loop.
pub fn execute_while(
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
    condition: &str,
    body: &[String],
    max_iterations: usize,
) -> Result<CommandOutput> {
    let mut last_output = CommandOutput::None;
    let mut iterations = 0;

    loop {
        if iterations >= max_iterations {
            return Err(OasisError::Command(
                format!("while loop exceeded {max_iterations} iterations").into(),
            ));
        }
        iterations += 1;

        let cond_result = registry.execute(condition, env);
        let exit_code = registry.last_exit_code();
        let condition_true = if exit_code != 0 {
            false
        } else {
            !matches!(&cond_result, Ok(CommandOutput::Text(t)) if t.trim() == "false")
        };
        if !condition_true {
            break;
        }

        for line in body {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            last_output = registry.execute(trimmed, env)?;
            if registry.break_flag() || registry.return_flag() {
                break;
            }
            if registry.continue_flag() {
                registry.clear_continue();
                break;
            }
        }
        if registry.break_flag() {
            registry.clear_break();
            break;
        }
        if registry.return_flag() {
            break;
        }
    }
    Ok(last_output)
}

/// Parse multi-line control flow from a single input string.
///
/// Recognizes:
/// - `if COND; then BODY; [else BODY;] fi`
/// - `for VAR in WORDS; do BODY; done`
/// - `while COND; do BODY; done`
///
/// Returns `None` if the input doesn't match a control flow pattern.
pub fn parse_and_execute(
    input: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Option<Result<CommandOutput>> {
    let trimmed = input.trim();

    if trimmed.starts_with("if ") {
        Some(parse_if(trimmed, registry, env))
    } else if trimmed.starts_with("for ") {
        Some(parse_for(trimmed, registry, env))
    } else if trimmed.starts_with("while ") {
        Some(parse_while(trimmed, registry, env))
    } else {
        None
    }
}

/// Parse and execute an if block.
fn parse_if(
    input: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Result<CommandOutput> {
    // Strip "if " prefix.
    let rest = &input[3..];

    // Find "then" keyword.
    let then_pos = find_keyword(rest, "then")
        .ok_or_else(|| OasisError::Command("if: missing 'then'".into()))?;
    let condition = rest[..then_pos].trim().trim_end_matches(';').trim();

    let after_then = rest[then_pos + 4..].trim();

    // Find "fi" keyword (must be present).
    let fi_pos = find_keyword(after_then, "fi")
        .ok_or_else(|| OasisError::Command("if: missing 'fi'".into()))?;
    let body_part = &after_then[..fi_pos];

    // Split body on "else" keyword.
    let (then_body, else_body) = if let Some(else_pos) = find_keyword(body_part, "else") {
        let then_str = &body_part[..else_pos];
        let else_str = &body_part[else_pos + 4..];
        (split_body(then_str), split_body(else_str))
    } else {
        (split_body(body_part), Vec::new())
    };

    execute_if(registry, env, condition, &then_body, &else_body)
}

/// Parse and execute a for loop.
fn parse_for(
    input: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Result<CommandOutput> {
    let rest = &input[4..];

    // Find "in" keyword.
    let in_pos =
        find_keyword(rest, "in").ok_or_else(|| OasisError::Command("for: missing 'in'".into()))?;
    let var_name = rest[..in_pos].trim();

    let after_in = rest[in_pos + 2..].trim();

    // Find "do" keyword.
    let do_pos = find_keyword(after_in, "do")
        .ok_or_else(|| OasisError::Command("for: missing 'do'".into()))?;
    let words_str = after_in[..do_pos].trim().trim_end_matches(';').trim();

    let after_do = after_in[do_pos + 2..].trim();

    // Find "done" keyword.
    let done_pos = find_keyword(after_do, "done")
        .ok_or_else(|| OasisError::Command("for: missing 'done'".into()))?;
    let body_str = &after_do[..done_pos];

    let words: Vec<String> = words_str.split_whitespace().map(String::from).collect();
    let body = split_body(body_str);

    execute_for(registry, env, var_name, &words, &body)
}

/// Parse and execute a while loop.
fn parse_while(
    input: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Result<CommandOutput> {
    let rest = &input[6..];

    // Find "do" keyword.
    let do_pos = find_keyword(rest, "do")
        .ok_or_else(|| OasisError::Command("while: missing 'do'".into()))?;
    let condition = rest[..do_pos].trim().trim_end_matches(';').trim();

    let after_do = rest[do_pos + 2..].trim();

    // Find "done" keyword.
    let done_pos = find_keyword(after_do, "done")
        .ok_or_else(|| OasisError::Command("while: missing 'done'".into()))?;
    let body_str = &after_do[..done_pos];

    let body = split_body(body_str);

    // Max 1000 iterations to prevent infinite loops.
    execute_while(registry, env, condition, &body, 1000)
}

/// Find a keyword in a string at a word boundary.
fn find_keyword(input: &str, keyword: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let kw_bytes = keyword.as_bytes();
    let kw_len = kw_bytes.len();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth: usize = 0;

    let mut i = 0;
    while i + kw_len <= bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' {
                i += 1;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                _ if depth == 0 => {
                    // Check for keyword at word boundary.
                    let at_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                    let at_end =
                        i + kw_len >= bytes.len() || !bytes[i + kw_len].is_ascii_alphanumeric();
                    if at_start && at_end && &bytes[i..i + kw_len] == kw_bytes {
                        return Some(i);
                    }
                },
                _ => {},
            }
        }
        i += 1;
    }
    None
}

/// Split a block body on `;` or newlines into individual command lines.
fn split_body(body: &str) -> Vec<String> {
    body.split(';')
        .flat_map(|s| s.split('\n'))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_keyword_basic() {
        assert_eq!(find_keyword("foo then bar", "then"), Some(4));
    }

    #[test]
    fn find_keyword_at_start() {
        assert_eq!(find_keyword("then bar", "then"), Some(0));
    }

    #[test]
    fn find_keyword_at_end() {
        assert_eq!(find_keyword("foo fi", "fi"), Some(4));
    }

    #[test]
    fn find_keyword_not_substring() {
        // "done" should not match inside "undone"
        assert_eq!(find_keyword("undone", "done"), None);
    }

    #[test]
    fn find_keyword_in_quotes_ignored() {
        assert_eq!(find_keyword("echo 'then' fi", "then"), None);
    }

    #[test]
    fn find_keyword_none() {
        assert_eq!(find_keyword("no match here", "then"), None);
    }

    #[test]
    fn split_body_semicolons() {
        let result = split_body("echo a; echo b; echo c");
        assert_eq!(result, vec!["echo a", "echo b", "echo c"]);
    }

    #[test]
    fn split_body_empty() {
        let result = split_body("  ;  ;  ");
        assert!(result.is_empty());
    }

    #[test]
    fn split_body_newlines() {
        let result = split_body("echo a\necho b");
        assert_eq!(result, vec!["echo a", "echo b"]);
    }

    // Integration tests with CommandRegistry.
    use crate::interpreter::{Command, CommandRegistry};
    use oasis_vfs::MemoryVfs;

    // Minimal echo command for tests.
    struct EchoCmd;
    impl Command for EchoCmd {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Print arguments"
        }
        fn usage(&self) -> &str {
            "echo [text...]"
        }
        fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
            Ok(CommandOutput::Text(args.join(" ")))
        }
    }

    fn make_reg() -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::dev_commands::register_dev_commands(&mut reg);
        reg
    }

    fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
        Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        }
    }

    #[test]
    fn if_then_fi_true_branch() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        let result = parse_and_execute("if test 1 -eq 1; then echo yes; fi", &reg, &mut env);
        assert!(result.is_some());
        let output = result.unwrap().unwrap();
        let CommandOutput::Text(t) = output else {
            panic!("expected Text, got {output:?}");
        };
        assert_eq!(t, "yes");
    }

    #[test]
    fn if_then_else_fi_false_branch() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        let result = parse_and_execute(
            "if test 1 -eq 2; then echo yes; else echo no; fi",
            &reg,
            &mut env,
        );
        assert!(result.is_some());
        let output = result.unwrap().unwrap();
        let CommandOutput::Text(t) = output else {
            panic!("expected Text, got {output:?}");
        };
        assert_eq!(t, "no");
    }

    #[test]
    fn for_loop_basic() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        let result = parse_and_execute("for x in a b c; do echo $x; done", &reg, &mut env);
        assert!(result.is_some());
        // Last iteration echoes "c".
        let output = result.unwrap().unwrap();
        let CommandOutput::Text(t) = output else {
            panic!("expected Text, got {output:?}");
        };
        assert_eq!(t, "c");
    }

    #[test]
    fn non_control_flow_returns_none() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        assert!(parse_and_execute("echo hello", &reg, &mut env).is_none());
    }

    #[test]
    fn if_missing_fi_errors() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        let result = parse_and_execute("if true; then echo yes", &reg, &mut env);
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn for_missing_done_errors() {
        let mut vfs = MemoryVfs::new();
        let reg = make_reg();
        let mut env = make_env(&mut vfs);

        let result = parse_and_execute("for x in a b; do echo $x", &reg, &mut env);
        assert!(result.unwrap().is_err());
    }
}

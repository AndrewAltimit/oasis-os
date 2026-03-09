//! Script execution engine for [`CommandRegistry`].
//!
//! Provides the `run` command, control flow (`if`/`elif`/`else`/`fi`,
//! `while`/`do`/`done`, `for`/`in`/`do`/`done`, `case`/`esac`), and
//! condition evaluation.

use oasis_types::error::{OasisError, Result};

use crate::expander::{case_pattern_matches, resolve_path};
use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

impl CommandRegistry {
    /// Built-in `run` implementation that executes scripts through the registry.
    pub(crate) fn execute_run(
        &self,
        args: &[&str],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let path = args
            .first()
            .copied()
            .ok_or_else(|| OasisError::Command("usage: run <path>".into()))?;

        let full_path = resolve_path(&env.cwd, path);

        if !env.vfs.exists(&full_path) {
            return Err(OasisError::Command(
                format!("script not found: {full_path}").into(),
            ));
        }

        let data = env.vfs.read(&full_path)?;
        let source = String::from_utf8_lossy(&data);
        let lines: Vec<String> = source
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();

        if lines.is_empty() {
            return Ok(CommandOutput::Text("(empty script)".to_string()));
        }

        let mut output = Vec::new();
        let mut pos = 0;
        self.execute_script_block(&lines, &mut pos, env, &mut output);

        if output.is_empty() {
            Ok(CommandOutput::Text(format!(
                "Script {full_path}: {} commands executed.",
                lines.len()
            )))
        } else {
            Ok(CommandOutput::Text(output.join("\n")))
        }
    }

    /// Execute a block of script lines with control flow support.
    ///
    /// Handles `if`/`then`/`else`/`fi`, `while`/`do`/`done`, and
    /// `for`/`in`/`do`/`done` constructs.
    pub(crate) fn execute_script_block(
        &self,
        lines: &[String],
        pos: &mut usize,
        env: &mut Environment<'_>,
        output: &mut Vec<String>,
    ) {
        const MAX_ITERATIONS: usize = 1000;

        while *pos < lines.len() {
            // Check flow-control flags early.
            if self.return_flag.get() || self.break_flag.get() || self.continue_flag.get() {
                return;
            }

            let line = &lines[*pos];
            let first_word = line.split_whitespace().next().unwrap_or("");

            match first_word {
                "if" => {
                    self.execute_if_block(lines, pos, env, output);
                },
                "case" => {
                    self.execute_case_block(lines, pos, env, output);
                },
                "while" => {
                    let condition = line.strip_prefix("while").unwrap_or("").trim().to_string();
                    *pos += 1;
                    let body = self.collect_loop_body(lines, pos);

                    let mut iterations = 0;
                    while self.eval_condition(&condition, env) && iterations < MAX_ITERATIONS {
                        self.break_flag.set(false);
                        self.continue_flag.set(false);
                        let mut sub_pos = 0;
                        self.execute_script_block(&body, &mut sub_pos, env, output);
                        if self.break_flag.get() {
                            self.break_flag.set(false);
                            break;
                        }
                        self.continue_flag.set(false);
                        iterations += 1;
                    }
                    if iterations >= MAX_ITERATIONS {
                        output.push(format!(
                            "warning: while loop terminated after \
                             {MAX_ITERATIONS} iterations (limit reached)"
                        ));
                    }
                },
                "for" => {
                    let rest = line.strip_prefix("for").unwrap_or("").trim();
                    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                    let var_name = parts.first().copied().unwrap_or("_");
                    let items_str = if parts.get(1) == Some(&"in") {
                        parts.get(2).copied().unwrap_or("")
                    } else {
                        ""
                    };
                    let items: Vec<&str> = items_str.split_whitespace().collect();

                    *pos += 1;
                    let body = self.collect_loop_body(lines, pos);

                    for item in &items {
                        self.set_variable(var_name, item);
                        self.break_flag.set(false);
                        self.continue_flag.set(false);
                        let mut sub_pos = 0;
                        self.execute_script_block(&body, &mut sub_pos, env, output);
                        if self.break_flag.get() {
                            self.break_flag.set(false);
                            break;
                        }
                        self.continue_flag.set(false);
                    }
                },
                // Stop tokens -- return to parent.
                "fi" | "done" | "else" | "elif" | "then" | "esac" => {
                    *pos += 1;
                    return;
                },
                _ => {
                    self.execute_script_line(line, env, output, *pos);
                    *pos += 1;
                },
            }
        }
    }

    /// Collect loop body lines (skipping `do`, until matching `done`), advancing `pos` past `done`.
    pub(crate) fn collect_loop_body(&self, lines: &[String], pos: &mut usize) -> Vec<String> {
        // Skip "do" keyword.
        if *pos < lines.len() && lines[*pos].split_whitespace().next() == Some("do") {
            *pos += 1;
        }
        let body_start = *pos;
        let mut depth = 0;
        while *pos < lines.len() {
            let fw = lines[*pos].split_whitespace().next().unwrap_or("");
            if fw == "while" || fw == "for" {
                depth += 1;
            }
            if fw == "done" {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            *pos += 1;
        }
        let body: Vec<String> = lines[body_start..*pos].to_vec();
        if *pos < lines.len() {
            *pos += 1; // Skip "done".
        }
        body
    }

    /// Execute an `if`/`elif`/`else`/`fi` block with full elif support.
    ///
    /// Collects branches as `(Option<condition>, body_lines)` pairs, then
    /// evaluates and executes only the first matching branch.
    pub(crate) fn execute_if_block(
        &self,
        lines: &[String],
        pos: &mut usize,
        env: &mut Environment<'_>,
        output: &mut Vec<String>,
    ) {
        // branches: Vec<(Option<condition_string>, Vec<body_lines>)>
        // None condition = else branch.
        let mut branches: Vec<(Option<String>, Vec<String>)> = Vec::new();

        // Parse the initial "if COND" line.
        let first_cond = lines[*pos]
            .strip_prefix("if")
            .unwrap_or("")
            .trim()
            .to_string();
        *pos += 1;
        // Skip "then".
        if *pos < lines.len() && lines[*pos].split_whitespace().next() == Some("then") {
            *pos += 1;
        }

        let mut current_cond: Option<String> = Some(first_cond);
        let mut current_body: Vec<String> = Vec::new();
        let mut depth = 0;

        while *pos < lines.len() {
            let l = &lines[*pos];
            let fw = l.split_whitespace().next().unwrap_or("");

            if fw == "if" || fw == "case" {
                depth += 1;
            }
            if (fw == "fi" || fw == "esac") && depth > 0 {
                depth -= 1;
                current_body.push(l.clone());
                *pos += 1;
                continue;
            }

            if depth == 0 && fw == "fi" {
                // End of entire if block.
                branches.push((current_cond.take(), current_body));
                *pos += 1;
                break;
            }
            if depth == 0 && fw == "elif" {
                branches.push((current_cond.take(), current_body));
                current_body = Vec::new();
                current_cond = Some(l.strip_prefix("elif").unwrap_or("").trim().to_string());
                *pos += 1;
                // Skip "then".
                if *pos < lines.len() && lines[*pos].split_whitespace().next() == Some("then") {
                    *pos += 1;
                }
                continue;
            }
            if depth == 0 && fw == "else" {
                branches.push((current_cond.take(), current_body));
                current_body = Vec::new();
                current_cond = None; // else branch
                *pos += 1;
                continue;
            }
            if depth == 0 && fw == "then" {
                *pos += 1;
                continue;
            }

            current_body.push(l.clone());
            *pos += 1;
        }

        // Execute the first matching branch.
        for (cond, body) in &branches {
            let matches = match cond {
                Some(c) => self.eval_condition(c, env),
                None => true, // else branch always matches
            };
            if matches {
                if !body.is_empty() {
                    let mut sub_pos = 0;
                    self.execute_script_block(body, &mut sub_pos, env, output);
                }
                return;
            }
        }
    }

    /// Execute a `case EXPR in ... esac` block.
    pub(crate) fn execute_case_block(
        &self,
        lines: &[String],
        pos: &mut usize,
        env: &mut Environment<'_>,
        output: &mut Vec<String>,
    ) {
        // Parse "case EXPR in".
        let case_line = &lines[*pos];
        let rest = case_line.strip_prefix("case").unwrap_or("").trim();
        // Extract the expression (everything before "in").
        let expr = if let Some(idx) = rest.rfind(" in") {
            rest[..idx].trim()
        } else {
            rest.trim_end_matches(" in").trim()
        };
        // Expand command substitutions and variables in the expression.
        let expr_subst = self.expand_substitutions(expr, env);
        let value = self.expand_variables(&expr_subst, &env.cwd);
        *pos += 1;

        // Collect pattern/body pairs until "esac".
        // Format: PATTERN) BODY ;; or PATTERN)\nBODY\n;;
        let mut matched = false;
        let mut depth = 0;

        while *pos < lines.len() {
            let l = lines[*pos].trim().to_string();

            if l == "esac" && depth == 0 {
                *pos += 1;
                break;
            }

            // Track nested case blocks.
            if l.starts_with("case ") {
                depth += 1;
            }
            if l == "esac" && depth > 0 {
                depth -= 1;
                *pos += 1;
                continue;
            }

            if depth > 0 || matched {
                *pos += 1;
                continue;
            }

            // Look for "PATTERN)" or "PATTERN|PATTERN)".
            if let Some(paren_idx) = l.find(')') {
                let pattern = l[..paren_idx].trim();
                // Body might be on the same line after ")".
                let after = l[paren_idx + 1..].trim().to_string();

                // Collect body lines until ";;".
                let mut body_lines: Vec<String> = Vec::new();
                if !after.is_empty() {
                    // Inline body: "pattern) cmd1; cmd2 ;;"
                    let trimmed = after.trim_end_matches(";;").trim();
                    if !trimmed.is_empty() {
                        // Split on ';' for inline commands.
                        for cmd in trimmed.split(';') {
                            let cmd = cmd.trim();
                            if !cmd.is_empty() {
                                body_lines.push(cmd.to_string());
                            }
                        }
                    }
                    if after.contains(";;") {
                        *pos += 1;
                    } else {
                        *pos += 1;
                        // Continue collecting until ";;".
                        while *pos < lines.len() {
                            let bl = lines[*pos].trim();
                            if bl == ";;" || bl.ends_with(";;") {
                                let trimmed = bl.trim_end_matches(";;").trim();
                                if !trimmed.is_empty() {
                                    body_lines.push(trimmed.to_string());
                                }
                                *pos += 1;
                                break;
                            }
                            body_lines.push(bl.to_string());
                            *pos += 1;
                        }
                    }
                } else {
                    *pos += 1;
                    while *pos < lines.len() {
                        let bl = lines[*pos].trim();
                        if bl == ";;" || bl.ends_with(";;") {
                            let trimmed = bl.trim_end_matches(";;").trim();
                            if !trimmed.is_empty() {
                                body_lines.push(trimmed.to_string());
                            }
                            *pos += 1;
                            break;
                        }
                        body_lines.push(bl.to_string());
                        *pos += 1;
                    }
                }

                if case_pattern_matches(&value, pattern) {
                    matched = true;
                    if !body_lines.is_empty() {
                        let mut sub_pos = 0;
                        self.execute_script_block(&body_lines, &mut sub_pos, env, output);
                    }
                }
            } else {
                *pos += 1;
            }
        }
    }

    /// Execute a single script line and collect output.
    pub(crate) fn execute_script_line(
        &self,
        line: &str,
        env: &mut Environment<'_>,
        output: &mut Vec<String>,
        line_num: usize,
    ) {
        match self.execute_single_cmd(line, env) {
            Ok(CommandOutput::Text(text)) => {
                for l in text.lines() {
                    output.push(l.to_string());
                }
            },
            Ok(CommandOutput::Table { headers, rows }) => {
                output.push(headers.join(" | "));
                for row in &rows {
                    output.push(row.join(" | "));
                }
            },
            Ok(CommandOutput::Clear) => {
                output.push("(clear)".to_string());
            },
            Ok(CommandOutput::None) => {},
            Ok(_) => {
                output.push("(signal command skipped in script)".to_string());
            },
            Err(e) => {
                output.push(format!("error at line {}: {e}", line_num + 1));
            },
        }
    }

    /// Evaluate a condition string for if/while.
    ///
    /// Runs the condition as a command. If it succeeds and outputs "true",
    /// the condition is true. If it errors or outputs "false", it's false.
    pub(crate) fn eval_condition(&self, condition: &str, env: &mut Environment<'_>) -> bool {
        match self.execute_single_cmd(condition, env) {
            Ok(CommandOutput::Text(text)) => {
                let t = text.trim();
                t == "true" || t == "0" || (!t.is_empty() && t != "false" && t != "1")
            },
            Ok(CommandOutput::None) => true,
            _ => false,
        }
    }
}

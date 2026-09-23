//! Command execution pipeline for [`CommandRegistry`].
//!
//! Contains the main `execute()` entry point plus pipeline, redirection,
//! command substitution, and single-command dispatch.

use oasis_types::error::{OasisError, Result};

use crate::expander::{expand_braces, expand_globs, resolve_path, tokenize};
use crate::pipeline::{
    ChainOp, output_to_text, parse_redirect, split_chains, split_pipes, write_redirect,
};
use crate::types::{CommandOutput, Environment};

use crate::registry::CommandRegistry;

/// Flatten the outputs of several commands into one.
///
/// `None` entries are dropped. A single output is returned as-is. Multiple
/// outputs have consecutive
/// `Text` entries merged (newline-joined) and are wrapped in
/// [`CommandOutput::Multi`] so signals survive alongside text.
pub(crate) fn merge_outputs(outputs: Vec<CommandOutput>) -> CommandOutput {
    let mut merged: Vec<CommandOutput> = Vec::new();
    for output in outputs {
        if matches!(output, CommandOutput::None) {
            continue;
        }
        if let CommandOutput::Text(ref new_text) = output
            && let Some(CommandOutput::Text(prev)) = merged.last_mut()
        {
            prev.push('\n');
            prev.push_str(new_text);
            continue;
        }
        merged.push(output);
    }
    match merged.len() {
        0 => CommandOutput::None,
        1 => merged.pop().unwrap_or(CommandOutput::None),
        _ => CommandOutput::Multi(merged),
    }
}

impl CommandRegistry {
    /// Parse and execute a command line.
    ///
    /// Supports quoting, variable expansion, command substitution
    /// (`$(...)`), aliases, command chaining (`;`, `&&`, `||`),
    /// pipes (`|`), input redirection (`<`), and output redirection
    /// (`>`, `>>`). Command names are case-insensitive.
    ///
    /// Lines containing compound commands (`if`, `while`, `until`, `for`,
    /// `case`) run through the script engine. A trailing unquoted `&`
    /// queues the line as a background job instead of running it (see
    /// [`CommandRegistry::poll_jobs`]).
    ///
    /// Only top-level calls (not function bodies, `$(...)`, scripts or
    /// jobs) are recorded in the history.
    pub fn execute(&self, line: &str, env: &mut Environment<'_>) -> Result<CommandOutput> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(CommandOutput::None);
        }

        // History expansion: !! and !n
        let expanded = self.expand_history(trimmed)?;
        let line = if expanded != trimmed {
            expanded
        } else {
            trimmed.to_string()
        };

        let depth = self.exec_depth.get();
        if depth == 0 {
            // Push to history (after history expansion, before execution).
            self.push_history(&line);
        }

        // Update $CWD before variable expansion.
        self.set_variable("CWD", &env.cwd);

        if let Some(cmd) = crate::pipeline::strip_background(&line) {
            return Ok(self.queue_job(cmd));
        }

        self.exec_depth.set(depth + 1);
        let result = if crate::script::is_compound(&line) {
            self.execute_compound(&line, env)
        } else {
            self.execute_chain(&line, env)
        };
        self.exec_depth.set(depth);
        if depth == 0 {
            // A stray `break` / `continue` outside any loop must not leak
            // into the next command line.
            self.break_flag.set(false);
            self.continue_flag.set(false);
        }
        result
    }

    /// Execute a line without history or job handling, as a nested
    /// (non-top-level) call.
    pub(crate) fn execute_nested(
        &self,
        line: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let depth = self.exec_depth.get();
        self.exec_depth.set(depth + 1);
        let result = self.execute(line, env);
        self.exec_depth.set(depth);
        result
    }

    /// Execute a `;` / `&&` / `||` chain of pipelines (no compound
    /// commands, no history).
    pub(crate) fn execute_chain(
        &self,
        line: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        // Split into chained segments (;, &&, ||).
        let segments = split_chains(line)?;
        let single_command = segments.len() == 1;
        let mut all_outputs: Vec<CommandOutput> = Vec::new();

        for segment in &segments {
            // Check chain condition.
            let should_run = match segment.chain_op {
                ChainOp::Always => true,
                ChainOp::And => self.last_exit_code.get() == 0,
                ChainOp::Or => self.last_exit_code.get() != 0,
            };
            if !should_run {
                continue;
            }

            // Reset exit code before pipeline so we can detect if the
            // pipeline sets a non-zero code (e.g. redirect capturing
            // an error via was_error). `$?` keeps the previous command's
            // status until this one finishes (it used to be reset to 0
            // here, so `cmd; echo $?` always printed 0).
            self.last_exit_code.set(0);
            match self.execute_pipeline(&segment.command, env) {
                Ok(output) => {
                    self.set_variable("?", &self.last_exit_code.get().to_string());
                    match output {
                        CommandOutput::None => {},
                        other => all_outputs.push(other),
                    }
                    // Stop executing further segments if `return`
                    // was called.
                    if self.return_flag.get() {
                        break;
                    }
                },
                Err(e) => {
                    self.last_exit_code.set(1);
                    self.set_variable("?", "1");
                    // For single commands, propagate errors directly.
                    if single_command {
                        return Err(e);
                    }
                    all_outputs.push(CommandOutput::Text(format!("error: {e}")));
                },
            }
        }

        Ok(merge_outputs(all_outputs))
    }

    /// Execute a pipeline: `cmd1 | cmd2 | cmd3`.
    fn execute_pipeline(
        &self,
        pipeline_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let pipe_segments = split_pipes(pipeline_str)?;

        if pipe_segments.len() == 1 {
            // No pipes -- just execute the single command with
            // redirection.
            return self.execute_with_redirect(&pipe_segments[0], env);
        }

        // Pipeline: chain stdout -> stdin.
        let mut stdin: Option<String> = env.stdin.take();

        for segment in &pipe_segments {
            env.stdin = stdin.take();
            // All segments get redirection parsing so `>` / `>>` is
            // stripped instead of being passed as literal arguments.
            let result = self.execute_with_redirect(segment, env)?;

            stdin = match result {
                CommandOutput::Text(text) => Some(text),
                CommandOutput::Table { headers, rows } => {
                    let mut out = headers.join(" | ");
                    for row in &rows {
                        out.push('\n');
                        out.push_str(&row.join(" | "));
                    }
                    Some(out)
                },
                _ => None,
            };
        }

        // Return the final output.
        match stdin {
            Some(text) => Ok(CommandOutput::Text(text)),
            None => Ok(CommandOutput::None),
        }
    }

    /// Mark the current command as failed (exit status 1) without an error
    /// message.
    fn set_failed_status(&self) {
        self.last_exit_code.set(1);
        self.set_variable("?", "1");
    }

    /// Expand a raw redirect target: `$VAR` / `${VAR}` substitution, then
    /// quote removal. Falls back to the trimmed raw text when it does not
    /// tokenize to exactly one word.
    fn redirect_target(&self, raw: &str, cwd: &str) -> String {
        let expanded = self.expand_variables(raw.trim(), cwd);
        match tokenize(&expanded) {
            Ok(mut tokens) if tokens.len() == 1 => tokens.pop().unwrap_or(expanded),
            _ => expanded.trim().to_string(),
        }
    }

    /// Execute a command, handling output redirection (`>`, `>>`,
    /// `2>`, `2>>`, `2>&1`).
    fn execute_with_redirect(
        &self,
        cmd_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let (cmd_part, redirections) = parse_redirect(cmd_str);
        let has_stderr_handling = redirections.stderr.is_some() || redirections.stderr_to_stdout;
        // Redirect targets get the same variable expansion and quote
        // removal as arguments (`> $DIR/out.txt`, `> "my file"`).
        let stdin_target = redirections
            .stdin
            .map(|p| self.redirect_target(p, &env.cwd));
        let stdout_target = redirections
            .stdout
            .as_ref()
            .map(|r| (self.redirect_target(r.path, &env.cwd), r.append));
        let stderr_target = redirections
            .stderr
            .as_ref()
            .map(|r| (self.redirect_target(r.path, &env.cwd), r.append));

        // Clear stderr before each command.
        env.stderr.clear();

        // Handle stdin redirect: read file contents into env.stdin.
        if let Some(stdin_path) = stdin_target {
            let resolved = resolve_path(&env.cwd, &stdin_path);
            match env.vfs.read(&resolved) {
                Ok(data) => {
                    env.stdin = Some(String::from_utf8_lossy(&data).into_owned());
                },
                Err(e) => {
                    return Err(OasisError::Command(
                        format!(
                            "cannot redirect stdin from \
                         '{stdin_path}': {e}"
                        )
                        .into(),
                    ));
                },
            }
        }

        let result = self.execute_single_cmd(cmd_part.trim(), env);

        // If no stderr redirect/merge, propagate errors normally.
        if !has_stderr_handling {
            let result = result?;
            if let Some((path, append)) = stdout_target {
                let text = output_to_text(&result);
                write_redirect(&text, &path, append, &env.cwd, env.vfs)?;
                return Ok(CommandOutput::None);
            }
            return Ok(result);
        }

        // Capture error messages into stderr.
        let (result, captured_stderr, was_error) = match result {
            Ok(output) => (output, std::mem::take(&mut env.stderr), false),
            Err(e) => {
                let mut stderr_text = std::mem::take(&mut env.stderr);
                if !stderr_text.is_empty() {
                    stderr_text.push('\n');
                }
                stderr_text.push_str(&e.to_string());
                (CommandOutput::None, stderr_text, true)
            },
        };

        // If 2>&1, merge stderr into stdout.
        let (result, captured_stderr) = if redirections.stderr_to_stdout {
            if captured_stderr.is_empty() {
                (result, String::new())
            } else {
                let merged = match result {
                    CommandOutput::Text(t) if !t.is_empty() => {
                        CommandOutput::Text(format!("{t}\n{captured_stderr}"))
                    },
                    CommandOutput::Text(_) | CommandOutput::None => {
                        CommandOutput::Text(captured_stderr)
                    },
                    other => other,
                };
                (merged, String::new())
            }
        } else {
            (result, captured_stderr)
        };

        // Handle stdout redirect.
        let result = if let Some((path, append)) = stdout_target {
            let text = output_to_text(&result);
            write_redirect(&text, &path, append, &env.cwd, env.vfs)?;
            CommandOutput::None
        } else {
            result
        };

        // Handle stderr redirect.
        if let Some((path, append)) = stderr_target {
            write_redirect(&captured_stderr, &path, append, &env.cwd, env.vfs)?;
        }

        // Preserve exit code: if command errored, keep it as exit
        // code 1 even though we captured the error text.
        if was_error {
            self.last_exit_code.set(1);
            self.set_variable("?", "1");
        }

        Ok(result)
    }

    /// Execute a single command (after chaining, piping, and
    /// redirection).
    pub(crate) fn execute_single_cmd(
        &self,
        cmd_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let trimmed = cmd_str.trim();
        if trimmed.is_empty() {
            return Ok(CommandOutput::None);
        }

        // Intercept `function` before variable expansion so the body
        // is stored literally (variables expand at call time).
        if trimmed.starts_with("function ")
            || trimmed.starts_with("function\t")
            || trimmed == "function"
        {
            // Guarded by starts_with("function") / == "function" above.
            let rest = trimmed.strip_prefix("function").unwrap_or("").trim();
            return self.execute_function_def_raw(rest);
        }

        // Expand command substitutions ($(...)).
        let after_subst = self.expand_substitutions(trimmed, env);

        // Expand variables.
        let expanded = self.expand_variables(&after_subst, &env.cwd);

        // Tokenize with quote handling.
        let tokens = tokenize(&expanded)?;
        if tokens.is_empty() {
            return Ok(CommandOutput::None);
        }

        // Expand aliases (first token only).
        let tokens = self.expand_alias(tokens);
        if tokens.is_empty() {
            return Ok(CommandOutput::None);
        }

        // Expand braces ({a,b,c}).
        let tokens = expand_braces(&tokens);

        // Expand globs.
        let tokens = expand_globs(&tokens, env.vfs, &env.cwd);

        let name_lower = tokens[0].to_ascii_lowercase();
        let arg_strings: Vec<String> = tokens[1..].to_vec();
        let args: Vec<&str> = arg_strings.iter().map(|s| s.as_str()).collect();

        // Intercept built-in commands that need registry access.
        match name_lower.as_str() {
            "help" => return self.execute_help(&args),
            "run" => return self.execute_run(&args, env),
            "history" => return self.execute_history_cmd(&args),
            "set" => return self.execute_set(&args),
            "unset" => return self.execute_unset(&args),
            "env" => return self.execute_env(),
            "alias" => return self.execute_alias(&args),
            "unalias" => return self.execute_unalias(&args),
            "which" => return self.execute_which(&args),
            "return" => return self.execute_return(&args),
            "break" => return self.execute_break(),
            "continue" => return self.execute_continue(),
            "local" => return self.execute_local(&args),
            "jobs" => return self.execute_jobs(),
            "fg" => return self.execute_fg(&args, env),
            "bg" => return self.execute_bg(&args),
            "kill" if args.iter().any(|a| a.starts_with('%')) => {
                return self.execute_kill(&args);
            },
            "true" => return Ok(CommandOutput::None),
            "false" => {
                self.set_failed_status();
                return Ok(CommandOutput::None);
            },
            _ => {},
        }

        // Check registered commands first, then user-defined
        // functions.
        if let Some(cmd) = self.commands.get(name_lower.as_str()) {
            let result = cmd.execute(&args, env);
            // `test` reports its verdict as text; a `false` verdict is also
            // a failing exit status so `test ... && cmd` / `||` behave.
            if name_lower == "test" && matches!(&result, Ok(CommandOutput::Text(t)) if t == "false")
            {
                self.set_failed_status();
            }
            return result;
        }

        // Check user-defined functions.
        if self.functions.borrow().contains_key(name_lower.as_str()) {
            return self.call_function(&name_lower, &args, env);
        }

        Err(OasisError::Command(
            format!("unknown command: {}", tokens[0]).into(),
        ))
    }

    // -- Command substitution --

    /// Expand `$(command)` substitutions in the input string.
    ///
    /// Executes the inner command, captures its output, and replaces
    /// the `$(...)` expression with the output (trailing newlines
    /// trimmed). Supports one level of nesting
    /// (e.g. `$(echo $(echo hi))`).
    pub(crate) fn expand_substitutions(&self, input: &str, env: &mut Environment<'_>) -> String {
        let chars: Vec<char> = input.chars().collect();
        let mut result = String::with_capacity(input.len());
        let mut i = 0;

        while i < chars.len() {
            // Skip single-quoted strings (no substitution inside).
            if chars[i] == '\'' {
                result.push('\'');
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    result.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    result.push('\'');
                    i += 1;
                }
                continue;
            }

            if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                // Find matching closing paren, respecting nesting.
                let start = i + 2;
                let mut depth = 1;
                let mut j = start;
                let mut in_sq = false;
                let mut in_dq = false;

                while j < chars.len() && depth > 0 {
                    if in_sq {
                        if chars[j] == '\'' {
                            in_sq = false;
                        }
                    } else if in_dq {
                        if chars[j] == '"' {
                            in_dq = false;
                        } else if chars[j] == '\\' {
                            j += 1; // skip escaped char
                        }
                    } else {
                        match chars[j] {
                            '\'' => in_sq = true,
                            '"' => in_dq = true,
                            '(' if j > 0 && chars[j - 1] == '$' => {
                                depth += 1;
                            },
                            '(' => {},
                            ')' => depth -= 1,
                            _ => {},
                        }
                    }
                    if depth > 0 {
                        j += 1;
                    }
                }

                if depth == 0 {
                    let inner: String = chars[start..j].iter().collect();
                    let output = match self.execute(&inner, env) {
                        Ok(ref out) => output_to_text(out),
                        Err(e) => {
                            env.stderr.push_str(&format!("command substitution: {e}"));
                            String::new()
                        },
                    };
                    // Trim trailing newlines (like bash).
                    result.push_str(output.trim_end_matches('\n'));
                    i = j + 1;
                    continue;
                }
                // Unmatched paren -- pass through literally.
                result.push('$');
                i += 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        result
    }
}

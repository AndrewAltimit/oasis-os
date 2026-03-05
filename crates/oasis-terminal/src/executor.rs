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

impl CommandRegistry {
    /// Parse and execute a command line.
    ///
    /// Supports quoting, variable expansion, command substitution
    /// (`$(...)`), aliases, command chaining (`;`, `&&`, `||`),
    /// pipes (`|`), input redirection (`<`), and output redirection
    /// (`>`, `>>`). Command names are case-insensitive.
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

        // Push to history (after history expansion, before execution).
        self.push_history(&line);

        // Update $CWD before variable expansion.
        self.set_variable("CWD", &env.cwd);
        self.last_exit_code.set(self.last_exit_code.get());

        // Split into chained segments (;, &&, ||).
        let segments = split_chains(&line)?;
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
            // an error via was_error).
            self.last_exit_code.set(0);
            self.set_variable("?", "0");
            match self.execute_pipeline(&segment.command, env) {
                Ok(output) => {
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

        // Flatten: if only one output, return it directly. If
        // multiple, merge consecutive text outputs and wrap in Multi
        // so signals are preserved alongside text.
        if all_outputs.is_empty() {
            Ok(CommandOutput::None)
        } else if all_outputs.len() == 1 {
            // SAFETY: guarded by len() == 1 check above.
            Ok(all_outputs.into_iter().next().unwrap())
        } else {
            // Merge consecutive Text entries to reduce Multi size.
            let mut merged: Vec<CommandOutput> = Vec::new();
            for output in all_outputs {
                if let CommandOutput::Text(ref new_text) = output
                    && let Some(CommandOutput::Text(prev)) = merged.last_mut()
                {
                    prev.push('\n');
                    prev.push_str(new_text);
                    continue;
                }
                merged.push(output);
            }
            if merged.len() == 1 {
                // SAFETY: guarded by len() == 1 check above.
                Ok(merged.into_iter().next().unwrap())
            } else {
                Ok(CommandOutput::Multi(merged))
            }
        }
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

    /// Execute a command, handling output redirection (`>`, `>>`,
    /// `2>`, `2>>`, `2>&1`).
    fn execute_with_redirect(
        &self,
        cmd_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let (cmd_part, redirections) = parse_redirect(cmd_str);
        let has_stderr_handling = redirections.stderr.is_some() || redirections.stderr_to_stdout;

        // Clear stderr before each command.
        env.stderr.clear();

        // Handle stdin redirect: read file contents into env.stdin.
        if let Some(stdin_path) = redirections.stdin {
            let resolved = resolve_path(&env.cwd, stdin_path);
            match env.vfs.read(&resolved) {
                Ok(data) => {
                    env.stdin = Some(String::from_utf8_lossy(&data).into_owned());
                },
                Err(e) => {
                    return Err(OasisError::Command(format!(
                        "cannot redirect stdin from \
                         '{stdin_path}': {e}"
                    )));
                },
            }
        }

        let result = self.execute_single_cmd(cmd_part.trim(), env);

        // If no stderr redirect/merge, propagate errors normally.
        if !has_stderr_handling {
            let result = result?;
            if let Some(redir) = redirections.stdout {
                let text = output_to_text(&result);
                write_redirect(&text, redir.path, redir.append, &env.cwd, env.vfs)?;
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
        let result = if let Some(redir) = redirections.stdout {
            let text = output_to_text(&result);
            write_redirect(&text, redir.path, redir.append, &env.cwd, env.vfs)?;
            CommandOutput::None
        } else {
            result
        };

        // Handle stderr redirect.
        if let Some(redir) = redirections.stderr {
            write_redirect(
                &captured_stderr,
                redir.path,
                redir.append,
                &env.cwd,
                env.vfs,
            )?;
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

        // Intercept control flow structures (if/for/while) before
        // expansion.
        if let Some(result) = crate::control_flow::parse_and_execute(trimmed, self, env) {
            return result;
        }

        // Intercept `function` before variable expansion so the body
        // is stored literally (variables expand at call time).
        if trimmed.starts_with("function ")
            || trimmed.starts_with("function\t")
            || trimmed == "function"
        {
            // SAFETY: guarded by starts_with("function") /
            // == "function" above.
            let rest = trimmed.strip_prefix("function").unwrap().trim();
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
            _ => {},
        }

        // Check registered commands first, then user-defined
        // functions.
        if let Some(cmd) = self.commands.get(name_lower.as_str()) {
            return cmd.execute(&args, env);
        }

        // Check user-defined functions.
        if self.functions.borrow().contains_key(name_lower.as_str()) {
            return self.call_function(&name_lower, &args, env);
        }

        Err(OasisError::Command(format!(
            "unknown command: {}",
            tokens[0]
        )))
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

//! Command execution pipeline for [`CommandRegistry`].
//!
//! Contains the main `execute()` entry point plus pipeline, redirection,
//! command substitution, and single-command dispatch.

use oasis_types::error::{OasisError, Result};

use crate::pipeline::{
    ChainOp, capture_text, parse_redirect, split_chains, split_pipes, strip_comments,
};
use crate::types::{CommandOutput, Environment, MAX_EXEC_DEPTH};

use crate::registry::CommandRegistry;

/// The error for exceeding [`MAX_EXEC_DEPTH`] (out of line: `execute`
/// recurses and its frame should stay small).
#[cold]
#[inline(never)]
fn nesting_error() -> OasisError {
    OasisError::Command(format!("maximum nesting depth ({MAX_EXEC_DEPTH}) exceeded").into())
}

/// Flatten the outputs of several commands into one.
///
/// `None` entries are dropped. A single output is returned as-is. Multiple
/// outputs have consecutive `Text` entries merged (newline-joined) and are
/// wrapped in [`CommandOutput::Multi`] so signals survive alongside text.
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

/// Split a leading `!` (pipeline negation) off `cmd`.
fn strip_bang(cmd: &str) -> (bool, &str) {
    match cmd.strip_prefix('!') {
        Some(rest) if rest.starts_with(char::is_whitespace) => (true, rest.trim_start()),
        _ => (false, cmd),
    }
}

impl CommandRegistry {
    /// Parse and execute a command line.
    ///
    /// Supports quoting, variable expansion, command substitution
    /// (`$(...)`), aliases, `#` comments, command chaining (`;`, `&&`,
    /// `||`), pipeline negation (`! cmd`), pipes (`|`), input redirection
    /// (`<`), and output redirection (`>`, `>>`). Command names are
    /// case-insensitive.
    ///
    /// Every command finishes with an exit status (0 = success; see
    /// [`CommandOutput::Status`]) that `&&`, `||`, `!`, `if`, `while`,
    /// `until` and `$?` act on. A command returning `Err` has status 1.
    ///
    /// Lines containing compound commands (`if`, `while`, `until`, `for`,
    /// `case`) run through the script engine. A command list ended by an
    /// unquoted `&` is queued as a background job instead of running (see
    /// [`CommandRegistry::poll_jobs`]), anywhere: `a & b`, inside loops and
    /// scripts.
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
            self.begin_top_level();
        } else if depth >= MAX_EXEC_DEPTH {
            return Err(nesting_error());
        }

        let line = strip_comments(&line);
        if line.trim().is_empty() {
            return Ok(CommandOutput::None);
        }

        // Update $CWD before variable expansion.
        self.set_variable("CWD", &env.cwd);

        let compound = crate::script::is_compound(&line);
        // A compound line ending in `&` is one job (`for ...; done &`);
        // plain lists background per `&` in the chain executor.
        if compound && let Some(cmd) = crate::pipeline::strip_background(&line) {
            return self.queue_job(&self.freeze_parameters(cmd, &env.cwd));
        }

        self.exec_depth.set(depth + 1);
        let result = if compound {
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

    /// Record `code` as the exit status of the last command (`$?`).
    pub(crate) fn set_status(&self, code: i32) {
        self.last_exit_code.set(code);
        self.set_variable("?", &code.to_string());
    }

    /// Execute a `;` / `&` / `&&` / `||` list of pipelines (no compound
    /// commands, no history).
    pub(crate) fn execute_chain(
        &self,
        line: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let segments = split_chains(line)?;
        let single_command = segments.len() == 1 && !segments[0].background;
        let mut all_outputs: Vec<CommandOutput> = Vec::new();

        let mut i = 0;
        while i < segments.len() {
            let segment = &segments[i];
            if segment.background {
                // The whole and-or list up to the `&` becomes one job.
                let end = segments[i + 1..]
                    .iter()
                    .position(|s| !s.background || s.chain_op == ChainOp::Always)
                    .map_or(segments.len(), |n| i + 1 + n);
                let mut text = segment.command.clone();
                for s in &segments[i + 1..end] {
                    text.push_str(if s.chain_op == ChainOp::And {
                        " && "
                    } else {
                        " || "
                    });
                    text.push_str(&s.command);
                }
                i = end;
                match self.queue_job(&self.freeze_parameters(&text, &env.cwd)) {
                    Ok(out) => {
                        self.set_status(0);
                        all_outputs.push(out);
                    },
                    Err(e) => {
                        self.set_status(1);
                        if segments.len() == 1 {
                            return Err(e);
                        }
                        all_outputs.push(CommandOutput::Text(format!("error: {e}")));
                    },
                }
                continue;
            }
            i += 1;

            // Check chain condition.
            let should_run = match segment.chain_op {
                ChainOp::Always => true,
                ChainOp::And => self.last_exit_code.get() == 0,
                ChainOp::Or => self.last_exit_code.get() != 0,
            };
            if !should_run {
                continue;
            }

            // Each pipeline starts from success; commands that fail set
            // their status through `CommandOutput::Status` or `Err`. `$?`
            // keeps the previous status until this pipeline finishes.
            self.last_exit_code.set(0);
            let (negate, pipeline) = strip_bang(&segment.command);
            let result = match self.operator_alias(pipeline) {
                Some((name, line)) => {
                    self.alias_stack.borrow_mut().push(name);
                    let result = self.execute_nested(&line, env);
                    self.alias_stack.borrow_mut().pop();
                    result
                },
                None => self.execute_pipeline(pipeline, env),
            };
            match result {
                Ok(output) => {
                    let code = self.last_exit_code.get();
                    self.set_status(if negate { i32::from(code == 0) } else { code });
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
                    self.set_status(i32::from(!negate));
                    // For single commands, propagate errors directly.
                    if single_command && !negate {
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
            // A pipeline's status is its last command's (`false | echo hi`
            // succeeds), so each segment starts from success.
            self.last_exit_code.set(0);
            // All segments get redirection parsing so `>` / `>>` is
            // stripped instead of being passed as literal arguments.
            let result = self.execute_with_redirect(segment, env);
            // The piped text belongs to this stage only: it must not leak
            // into the commands after the pipeline (`a | b; wc`).
            env.stdin = None;
            let result = result?;

            stdin = match result {
                CommandOutput::Text(_) | CommandOutput::Table { .. } => Some(capture_text(&result)),
                _ => None,
            };
        }

        // Return the final output.
        match stdin {
            Some(text) => Ok(CommandOutput::Text(text)),
            None => Ok(CommandOutput::None),
        }
    }

    /// Consume a [`CommandOutput::Status`] wrapper: record its exit code
    /// (so `&&` / `||` / `$?` / `if` see it) and return the inner output.
    /// Callers above the executor never see the wrapper.
    fn apply_status(&self, result: Result<CommandOutput>) -> Result<CommandOutput> {
        let mut output = result?;
        while let CommandOutput::Status {
            code,
            output: inner,
        } = output
        {
            self.last_exit_code.set(code);
            self.set_variable("?", &code.to_string());
            output = *inner;
        }
        Ok(output)
    }

    /// Execute a command, handling output redirection (`>`, `>>`,
    /// `2>`, `2>>`, `2>&1`).
    fn execute_with_redirect(
        &self,
        cmd_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let (cmd_part, redirections) = parse_redirect(cmd_str);
        if redirections.is_empty() {
            // The common case, kept lean: this frame recurses through
            // functions and `$(...)`.
            env.stderr.clear();
            return self.execute_single_cmd(cmd_part.trim(), env);
        }
        self.execute_redirected(&cmd_part, &redirections, env)
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
        self.charge_command()?;

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

        let words = self.expand_args(trimmed, env)?;
        self.run_words(&words, env)
    }

    /// Dispatch an already-expanded command: shell builtins (they need
    /// registry access) win over registered commands, then registered
    /// commands, then user-defined functions. The first word is the
    /// (case-insensitive) command name. `xargs` uses this to run its
    /// command without re-parsing its arguments.
    pub(crate) fn run_words(
        &self,
        words: &[String],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let Some(first) = words.first() else {
            return Ok(CommandOutput::None);
        };
        let name_lower = first.to_ascii_lowercase();
        let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();

        if let Some(b) = crate::builtins::builtin(&name_lower) {
            return self.apply_status((b.run)(self, &args, env));
        }
        if let Some(cmd) = self.commands.get(name_lower.as_str()) {
            if name_lower == "cd" {
                return self.run_cd(cmd.as_ref(), &args, env);
            }
            return self.apply_status(cmd.execute(&args, env));
        }
        if self.functions.borrow().contains_key(name_lower.as_str()) {
            return self.call_function(&name_lower, &args, env);
        }
        Err(OasisError::Command(
            format!("unknown command: {first}").into(),
        ))
    }

    /// Run the `cd` command with the forms that need shell variables
    /// resolved: no argument is `$HOME` (`/` when unset), `-` is `$OLDPWD`
    /// (printing the new directory, as POSIX shells do). A successful
    /// change records the previous directory in `OLDPWD`.
    fn run_cd(
        &self,
        cmd: &dyn crate::interpreter::Command,
        args: &[&str],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let var = |name: &str| self.get_variable(name).filter(|v| !v.is_empty());
        let (target, announce) = match args {
            [] => (var("HOME").unwrap_or_else(|| "/".to_string()), false),
            ["-"] => (
                var("OLDPWD").ok_or_else(|| OasisError::Command("cd: OLDPWD not set".into()))?,
                true,
            ),
            _ => (String::new(), false),
        };
        let previous = env.cwd.clone();
        let output = if target.is_empty() {
            cmd.execute(args, env)?
        } else {
            cmd.execute(&[target.as_str()], env)?
        };
        if env.cwd != previous {
            self.set_variable("OLDPWD", &previous);
        }
        if announce {
            return Ok(CommandOutput::Text(env.cwd.clone()));
        }
        self.apply_status(Ok(output))
    }
}

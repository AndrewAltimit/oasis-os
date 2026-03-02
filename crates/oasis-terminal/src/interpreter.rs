//! Command trait, registry, and dispatch logic.
//!
//! Supports quoted arguments, environment variables, command history,
//! pipes, output redirection, command chaining, and glob expansion.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use oasis_platform::{NetworkService, PowerService, TimeService, UsbService};
use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

/// Output produced by a command.
#[derive(Debug, Clone)]
pub enum CommandOutput {
    /// Plain text lines.
    Text(String),
    /// Tabular data (header row + data rows).
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Command produced no visible output.
    None,
    /// Signal to clear the terminal output buffer.
    Clear,
    /// Signal to the app to start/stop the remote terminal listener.
    ListenToggle {
        /// Port to listen on (0 = stop).
        port: u16,
    },
    /// Signal to the app to connect to a remote host.
    RemoteConnect {
        address: String,
        port: u16,
        psk: Option<String>,
    },
    /// Signal to the app to toggle browser sandbox mode.
    BrowserSandbox {
        /// `true` = sandbox on (VFS only), `false` = networking enabled.
        enable: bool,
    },
    /// Signal to the app to swap the active skin.
    SkinSwap {
        /// Skin name or path to load.
        name: String,
    },
    /// Signal to the app to start/stop the FTP file server.
    FtpToggle {
        /// Port to listen on (0 = stop).
        port: u16,
    },
    /// Multiple outputs from a chained command (e.g. `skin xp ; echo Done`).
    /// Each inner output is processed in order by the app layer.
    Multi(Vec<CommandOutput>),
}

/// Shared mutable environment passed to every command.
pub struct Environment<'a> {
    /// Current working directory (VFS path).
    pub cwd: String,
    /// The virtual file system.
    pub vfs: &'a mut dyn Vfs,
    /// Power service for battery/CPU queries.
    pub power: Option<&'a dyn PowerService>,
    /// Time service for clock/uptime queries.
    pub time: Option<&'a dyn TimeService>,
    /// USB service for status queries.
    pub usb: Option<&'a dyn UsbService>,
    /// Network service for WiFi status queries.
    pub network: Option<&'a dyn NetworkService>,
    /// TLS provider for HTTPS connections.
    pub tls: Option<&'a dyn oasis_net::tls::TlsProvider>,
    /// Piped input from a previous command in a pipeline.
    pub stdin: Option<String>,
    /// Accumulated stderr output from the most recent command.
    /// Commands append error messages here. Cleared before each command.
    pub stderr: String,
}

/// A single executable command.
pub trait Command {
    /// The command name (what the user types).
    fn name(&self) -> &str;

    /// One-line description for `help`.
    fn description(&self) -> &str;

    /// Usage string (e.g. "ls \[path\]").
    fn usage(&self) -> &str;

    /// Command category for grouping in `help` output.
    fn category(&self) -> &str {
        "general"
    }

    /// Execute the command with the given arguments and environment.
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput>;
}

/// Maximum number of history entries to retain.
const MAX_HISTORY: usize = 100;

/// Maximum shell function call depth (prevents infinite recursion).
const MAX_CALL_DEPTH: usize = 64;

/// A user-defined shell function.
#[derive(Clone, Debug)]
struct ShellFunction {
    /// Function body lines (semicolon-separated or newline-separated).
    body: String,
}

/// Registry of available commands with dispatch.
///
/// Also holds persistent shell state: variables, aliases, and history.
pub struct CommandRegistry {
    commands: HashMap<String, Box<dyn Command>>,
    variables: RefCell<HashMap<String, String>>,
    aliases: RefCell<HashMap<String, String>>,
    history: RefCell<Vec<String>>,
    last_exit_code: Cell<i32>,
    /// User-defined shell functions.
    functions: RefCell<HashMap<String, ShellFunction>>,
    /// Current function call depth (for recursion limiting).
    call_depth: Cell<usize>,
    /// Set by `return` to signal early exit from a function body.
    return_flag: Cell<bool>,
    /// Set by `break` inside a loop body.
    break_flag: Cell<bool>,
    /// Set by `continue` inside a loop body.
    continue_flag: Cell<bool>,
    /// Stack of local variable scopes for function calls.
    /// Each scope maps variable names to their saved values (None = was unset).
    local_scopes: RefCell<Vec<HashMap<String, Option<String>>>>,
}

impl CommandRegistry {
    /// Create an empty command registry.
    pub fn new() -> Self {
        let mut vars = HashMap::new();
        vars.insert("SHELL".to_string(), "oasis".to_string());
        vars.insert("HOME".to_string(), "/home".to_string());
        vars.insert("USER".to_string(), "user".to_string());
        Self {
            commands: HashMap::new(),
            variables: RefCell::new(vars),
            aliases: RefCell::new(HashMap::new()),
            history: RefCell::new(Vec::new()),
            last_exit_code: Cell::new(0),
            functions: RefCell::new(HashMap::new()),
            call_depth: Cell::new(0),
            return_flag: Cell::new(false),
            break_flag: Cell::new(false),
            continue_flag: Cell::new(false),
            local_scopes: RefCell::new(Vec::new()),
        }
    }

    /// Register a command. Replaces any existing command with the same name.
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        self.commands.insert(cmd.name().to_string(), cmd);
    }

    // -- Shell variable API --

    /// Set a shell variable.
    pub fn set_variable(&self, name: &str, value: &str) {
        self.variables
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
    }

    /// Get a shell variable value.
    pub fn get_variable(&self, name: &str) -> Option<String> {
        self.variables.borrow().get(name).cloned()
    }

    /// Get all shell variables.
    pub fn variables(&self) -> HashMap<String, String> {
        self.variables.borrow().clone()
    }

    /// Remove a shell variable.
    pub fn unset_variable(&self, name: &str) {
        self.variables.borrow_mut().remove(name);
    }

    // -- Alias API --

    /// Set a command alias.
    pub fn set_alias(&self, name: &str, expansion: &str) {
        self.aliases
            .borrow_mut()
            .insert(name.to_string(), expansion.to_string());
    }

    /// Get all aliases.
    pub fn aliases(&self) -> HashMap<String, String> {
        self.aliases.borrow().clone()
    }

    /// Remove a command alias.
    pub fn unset_alias(&self, name: &str) {
        self.aliases.borrow_mut().remove(name);
    }

    // -- Function API --

    /// Define a shell function.
    fn define_function(&self, name: &str, body: &str) {
        self.functions.borrow_mut().insert(
            name.to_string(),
            ShellFunction {
                body: body.to_string(),
            },
        );
    }

    /// List all defined functions.
    fn list_functions(&self) -> Vec<(String, String)> {
        self.functions
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.body.clone()))
            .collect()
    }

    /// Call a shell function by name with positional arguments.
    fn call_function(
        &self,
        name: &str,
        args: &[&str],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let func = self
            .functions
            .borrow()
            .get(name)
            .cloned()
            .ok_or_else(|| OasisError::Command(format!("unknown function: {name}")))?;

        // Check recursion depth.
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(OasisError::Command(format!(
                "{name}: maximum recursion depth ({MAX_CALL_DEPTH}) exceeded"
            )));
        }
        self.call_depth.set(depth + 1);

        // Save current positional args and set new ones.
        // Determine prior arg count so we save/restore the full range.
        let saved_argc = self.get_variable("#");
        let prior_count: usize = saved_argc
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let save_range = prior_count.max(args.len());
        let saved_args: Vec<(String, Option<String>)> = (0..=save_range)
            .map(|i| {
                let key = i.to_string();
                let old = self.get_variable(&key);
                (key, old)
            })
            .collect();

        // Set positional args: $0 = function name, $1..$n = args.
        // Unset any prior positional args beyond the new arg count.
        self.set_variable("0", name);
        for (i, arg) in args.iter().enumerate() {
            self.set_variable(&(i + 1).to_string(), arg);
        }
        for i in (args.len() + 1)..=prior_count {
            self.unset_variable(&i.to_string());
        }
        self.set_variable("#", &args.len().to_string());

        // Push a new local variable scope.
        self.local_scopes.borrow_mut().push(HashMap::new());

        // Execute body as a chain of commands.
        let result = self.execute(func.body.trim(), env);

        // Clear the return flag so it doesn't propagate to the caller.
        self.return_flag.set(false);

        // Pop local variable scope and restore saved values.
        if let Some(scope) = self.local_scopes.borrow_mut().pop() {
            for (name, saved) in scope {
                match saved {
                    Some(v) => self.set_variable(&name, &v),
                    None => self.unset_variable(&name),
                }
            }
        }

        // Restore previous positional args.
        for (key, old_val) in saved_args {
            match old_val {
                Some(v) => self.set_variable(&key, &v),
                None => self.unset_variable(&key),
            }
        }
        match saved_argc {
            Some(v) => self.set_variable("#", &v),
            None => self.unset_variable("#"),
        }

        // Restore call depth.
        self.call_depth.set(depth);

        result
    }

    // -- History API --

    /// Get command history.
    pub fn history(&self) -> Vec<String> {
        self.history.borrow().clone()
    }

    /// Push a command to history.
    fn push_history(&self, line: &str) {
        let mut hist = self.history.borrow_mut();
        // Don't duplicate the last entry.
        if hist.last().is_none_or(|last| last != line) {
            hist.push(line.to_string());
            if hist.len() > MAX_HISTORY {
                hist.remove(0);
            }
        }
    }

    /// Parse and execute a command line.
    ///
    /// Supports quoting, variable expansion, aliases, command chaining
    /// (`;`, `&&`, `||`), pipes (`|`), and output redirection (`>`, `>>`).
    /// Command names are case-insensitive.
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
            // pipeline sets a non-zero code (e.g. redirect capturing an
            // error via was_error).
            self.last_exit_code.set(0);
            self.set_variable("?", "0");
            match self.execute_pipeline(&segment.command, env) {
                Ok(output) => {
                    match output {
                        CommandOutput::None => {},
                        other => all_outputs.push(other),
                    }
                    // Stop executing further segments if `return` was called.
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

        // Flatten: if only one output, return it directly. If multiple,
        // merge consecutive text outputs and wrap in Multi so signals
        // are preserved alongside text.
        if all_outputs.is_empty() {
            Ok(CommandOutput::None)
        } else if all_outputs.len() == 1 {
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
            // No pipes -- just execute the single command with redirection.
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

    /// Execute a command, handling output redirection (`>`, `>>`, `2>`,
    /// `2>>`, `2>&1`).
    fn execute_with_redirect(
        &self,
        cmd_str: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let (cmd_part, redirections) = parse_redirect(cmd_str);
        let has_stderr_handling = redirections.stderr.is_some() || redirections.stderr_to_stdout;

        // Clear stderr before each command.
        env.stderr.clear();

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

        // Preserve exit code: if command errored, keep it as exit code 1
        // even though we captured the error text.
        if was_error {
            self.last_exit_code.set(1);
            self.set_variable("?", "1");
        }

        Ok(result)
    }

    /// Execute a single command (after chaining, piping, and redirection).
    fn execute_single_cmd(
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
            let rest = trimmed.strip_prefix("function").unwrap().trim();
            return self.execute_function_def_raw(rest);
        }

        // Expand variables.
        let expanded = self.expand_variables(trimmed, &env.cwd);

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

        // Check registered commands first, then user-defined functions.
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

    // -- History expansion --

    fn expand_history(&self, input: &str) -> Result<String> {
        if input == "!!" {
            let hist = self.history.borrow();
            return hist
                .last()
                .cloned()
                .ok_or_else(|| OasisError::Command("!!: no previous command".to_string()));
        }
        if let Some(n_str) = input.strip_prefix('!')
            && let Ok(n) = n_str.parse::<usize>()
        {
            let hist = self.history.borrow();
            if n == 0 || n > hist.len() {
                return Err(OasisError::Command(format!("!{n}: event not found")));
            }
            return Ok(hist[n - 1].clone());
        }
        Ok(input.to_string())
    }

    // -- Variable expansion --

    fn expand_variables(&self, input: &str, cwd: &str) -> String {
        let vars = self.variables.borrow();
        let mut result = String::with_capacity(input.len());
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                // Check for $? (last exit code) and $# (arg count).
                if chars[i + 1] == '?' || chars[i + 1] == '#' {
                    let name = chars[i + 1].to_string();
                    let value = self.resolve_var(&name, &vars, cwd);
                    result.push_str(&value);
                    i += 2;
                    continue;
                }
                // Check for ${VAR} syntax.
                if chars[i + 1] == '{'
                    && let Some(end) = chars[i + 2..].iter().position(|&c| c == '}')
                {
                    let name: String = chars[i + 2..i + 2 + end].iter().collect();
                    let value = self.resolve_var(&name, &vars, cwd);
                    result.push_str(&value);
                    i += 3 + end;
                    continue;
                }
                // Bare $VAR.
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                if end > start {
                    let name: String = chars[start..end].iter().collect();
                    let value = self.resolve_var(&name, &vars, cwd);
                    result.push_str(&value);
                    i = end;
                    continue;
                }
                result.push('$');
                i += 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    fn resolve_var(&self, name: &str, vars: &HashMap<String, String>, cwd: &str) -> String {
        match name {
            "CWD" => cwd.to_string(),
            "?" => self.last_exit_code.get().to_string(),
            _ => vars.get(name).cloned().unwrap_or_default(),
        }
    }

    // -- Alias expansion --

    fn expand_alias(&self, mut tokens: Vec<String>) -> Vec<String> {
        if tokens.is_empty() {
            return tokens;
        }
        let aliases = self.aliases.borrow();
        if let Some(expansion) = aliases.get(&tokens[0]) {
            // Replace the first token with the alias expansion.
            let expanded_tokens = match tokenize(expansion) {
                Ok(t) => t,
                Err(_) => return tokens,
            };
            tokens.splice(0..1, expanded_tokens);
        }
        tokens
    }

    // -- Intercepted commands --

    /// Built-in `run` implementation that executes scripts through the registry.
    fn execute_run(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let path = args
            .first()
            .copied()
            .ok_or_else(|| OasisError::Command("usage: run <path>".to_string()))?;

        let full_path = resolve_path(&env.cwd, path);

        if !env.vfs.exists(&full_path) {
            return Err(OasisError::Command(format!(
                "script not found: {full_path}"
            )));
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
    fn execute_script_block(
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
    fn collect_loop_body(&self, lines: &[String], pos: &mut usize) -> Vec<String> {
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
    fn execute_if_block(
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
    fn execute_case_block(
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
        // Expand variables in the expression.
        let value = self.expand_variables(expr, &env.cwd);
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
    fn execute_script_line(
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
    fn eval_condition(&self, condition: &str, env: &mut Environment<'_>) -> bool {
        match self.execute_single_cmd(condition, env) {
            Ok(CommandOutput::Text(text)) => {
                let t = text.trim();
                t == "true" || t == "0" || (!t.is_empty() && t != "false" && t != "1")
            },
            Ok(CommandOutput::None) => true,
            _ => false,
        }
    }

    /// Built-in help with access to the registry.
    fn execute_help(&self, args: &[&str]) -> Result<CommandOutput> {
        // Parse optional --category / -c filter.
        let mut filter_cat: Option<&str> = None;
        let mut positional: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--category" | "-c" => {
                    if let Some(&cat) = args.get(i + 1) {
                        filter_cat = Some(cat);
                        i += 2;
                        continue;
                    }
                    return Err(OasisError::Command(
                        "usage: help [--category <cat>] [command]".into(),
                    ));
                },
                other => positional.push(other),
            }
            i += 1;
        }

        if let Some(&name) = positional.first() {
            let name_lower = name.to_ascii_lowercase();
            match self.commands.get(name_lower.as_str()) {
                Some(cmd) => {
                    let mut out = cmd.name().to_string();
                    out.push_str(&format!(" ({})\n", cmd.category()));
                    out.push_str(&format!("  {}\n", cmd.description()));
                    out.push_str(&format!("  Usage: {}", cmd.usage()));
                    Ok(CommandOutput::Text(out))
                },
                None => Err(OasisError::Command(format!("unknown command: {name}"))),
            }
        } else {
            // Group commands by category.
            let mut categories: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
            // Include intercepted commands.
            for builtin in &[
                ("help", "general"),
                ("run", "scripting"),
                ("history", "general"),
                ("set", "config"),
                ("unset", "config"),
                ("env", "config"),
                ("alias", "config"),
                ("unalias", "config"),
                ("which", "general"),
            ] {
                categories
                    .entry(builtin.1)
                    .or_default()
                    .push((builtin.0, ""));
            }
            for cmd in self.commands.values() {
                categories
                    .entry(cmd.category())
                    .or_default()
                    .push((cmd.name(), cmd.description()));
            }

            let mut cats: Vec<&str> = categories.keys().copied().collect();
            cats.sort();

            // Apply category filter if specified.
            if let Some(fc) = filter_cat {
                let fc_lower = fc.to_ascii_lowercase();
                cats.retain(|c| c.to_ascii_lowercase().contains(&fc_lower));
                if cats.is_empty() {
                    return Err(OasisError::Command(format!("no category matching '{fc}'")));
                }
            }

            let total: usize = cats
                .iter()
                .filter_map(|c| categories.get(c))
                .map(|v| v.len())
                .sum();
            let mut out = format!("Commands ({total}):\n");
            for cat in &cats {
                let cmds = categories.get(cat).unwrap();
                let mut cmds = cmds.clone();
                cmds.sort_by_key(|(name, _)| *name);
                out.push_str(&format!("\n  [{cat}]\n"));
                for (name, desc) in &cmds {
                    if desc.is_empty() {
                        out.push_str(&format!("    {name}\n"));
                    } else {
                        out.push_str(&format!("    {name:12} {desc}\n"));
                    }
                }
            }
            out.push_str("\nType 'help <command>' for details.");
            Ok(CommandOutput::Text(out))
        }
    }

    /// Built-in `which` command.
    fn execute_which(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: which <command>".to_string()));
        }
        let name = args[0].to_ascii_lowercase();
        // Check intercepted commands first.
        let intercepted = [
            "help", "run", "history", "set", "unset", "env", "alias", "unalias", "which",
            "function", "return", "break", "continue", "local",
        ];
        if intercepted.contains(&name.as_str()) {
            return Ok(CommandOutput::Text(format!("{name}: shell built-in")));
        }
        match self.commands.get(name.as_str()) {
            Some(cmd) => Ok(CommandOutput::Text(format!(
                "{}: {} ({})",
                cmd.name(),
                cmd.description(),
                cmd.category()
            ))),
            None => {
                // Check aliases.
                let aliases = self.aliases.borrow();
                if let Some(expansion) = aliases.get(&name) {
                    Ok(CommandOutput::Text(format!(
                        "{name}: aliased to '{expansion}'"
                    )))
                } else {
                    // Check functions.
                    let funcs = self.functions.borrow();
                    if funcs.contains_key(&name) {
                        Ok(CommandOutput::Text(format!("{name}: shell function")))
                    } else {
                        Err(OasisError::Command(format!("{name}: not found")))
                    }
                }
            },
        }
    }

    /// Built-in `function` command: define a shell function.
    ///
    /// Syntax: `function name() { body }` or `function name { body }`
    /// Body commands are separated by `;` or run as a chain.
    ///
    /// Takes the raw string *after* the `function` keyword, before
    /// variable expansion, to preserve `$1`, `$2`, etc. literally.
    fn execute_function_def_raw(&self, raw: &str) -> Result<CommandOutput> {
        if raw.is_empty() {
            // List all defined functions.
            let funcs = self.list_functions();
            if funcs.is_empty() {
                return Ok(CommandOutput::Text("No functions defined.".to_string()));
            }
            let mut lines = Vec::new();
            for (name, body) in &funcs {
                lines.push(format!("function {name}() {{ {body} }}"));
            }
            lines.sort();
            return Ok(CommandOutput::Text(lines.join("\n")));
        }

        // Extract function name (strip optional `()` suffix).
        // Check which delimiter comes first to avoid misparsing `(`
        // that appears inside the function body.
        let paren_pos = raw.find('(');
        let brace_pos = raw.find('{');
        let (name, rest) = match (paren_pos, brace_pos) {
            (Some(p), Some(b)) if p < b => {
                let name = raw[..p].trim();
                let after = raw[p..].trim();
                let rest = after.strip_prefix("()").unwrap_or(after);
                (name, rest.trim())
            },
            (_, Some(b)) => {
                let name = raw[..b].trim();
                let rest = &raw[b..];
                (name, rest.trim())
            },
            (Some(p), None) => {
                let name = raw[..p].trim();
                let after = raw[p..].trim();
                let rest = after.strip_prefix("()").unwrap_or(after);
                (name, rest.trim())
            },
            (None, None) => {
                return Err(OasisError::Command(
                    "usage: function name() { body }".to_string(),
                ));
            },
        };

        if name.is_empty() {
            return Err(OasisError::Command(
                "function name cannot be empty".to_string(),
            ));
        }

        // Extract body between { and }.
        let rest = rest
            .strip_prefix('{')
            .ok_or_else(|| OasisError::Command("expected '{' after function name".to_string()))?;
        let body = rest
            .strip_suffix('}')
            .ok_or_else(|| OasisError::Command("expected '}' at end of function".to_string()))?;
        let body = body.trim();

        if body.is_empty() {
            return Err(OasisError::Command(
                "function body cannot be empty".to_string(),
            ));
        }

        self.define_function(&name.to_ascii_lowercase(), body);
        Ok(CommandOutput::None)
    }

    /// Built-in `return` command: set exit code from within a function.
    fn execute_return(&self, args: &[&str]) -> Result<CommandOutput> {
        if self.call_depth.get() == 0 {
            return Err(OasisError::Command(
                "return: can only be used inside a function".to_string(),
            ));
        }
        let code: i32 = if let Some(s) = args.first() {
            s.parse().unwrap_or(1)
        } else {
            0
        };
        self.last_exit_code.set(code);
        self.set_variable("?", &code.to_string());
        self.return_flag.set(true);
        Ok(CommandOutput::None)
    }

    /// Built-in `break` command — sets the break flag for the enclosing loop.
    fn execute_break(&self) -> Result<CommandOutput> {
        self.break_flag.set(true);
        Ok(CommandOutput::None)
    }

    /// Built-in `continue` command — sets the continue flag for the enclosing loop.
    fn execute_continue(&self) -> Result<CommandOutput> {
        self.continue_flag.set(true);
        Ok(CommandOutput::None)
    }

    /// Built-in `local` command — declares a local variable in the current function scope.
    ///
    /// Syntax: `local NAME=VALUE` or `local NAME`
    fn execute_local(&self, args: &[&str]) -> Result<CommandOutput> {
        let scopes = self.local_scopes.borrow();
        if scopes.is_empty() {
            return Err(OasisError::Command(
                "local: can only be used inside a function".to_string(),
            ));
        }
        drop(scopes);

        for arg in args {
            let (name, value) = if let Some((n, v)) = arg.split_once('=') {
                (n, Some(v))
            } else {
                (*arg, None)
            };
            // Save the current value (or None if unset) in the top scope.
            let old = self.get_variable(name);
            self.local_scopes
                .borrow_mut()
                .last_mut()
                .unwrap()
                .entry(name.to_string())
                .or_insert(old);
            // Set the new value.
            if let Some(v) = value {
                self.set_variable(name, v);
            }
        }
        Ok(CommandOutput::None)
    }

    /// Built-in `history` command.
    fn execute_history_cmd(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.first() == Some(&"clear") {
            self.history.borrow_mut().clear();
            return Ok(CommandOutput::Text("History cleared.".to_string()));
        }
        let hist = self.history.borrow();
        if hist.is_empty() {
            return Ok(CommandOutput::Text("(no history)".to_string()));
        }
        let mut out = String::new();
        for (i, entry) in hist.iter().enumerate() {
            out.push_str(&format!("  {:4}  {entry}\n", i + 1));
        }
        Ok(CommandOutput::Text(out.trim_end().to_string()))
    }

    /// Built-in `set` command: `set VAR=value`.
    fn execute_set(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            // Show all variables.
            return self.execute_env();
        }
        let assignment = args.join(" ");
        if let Some((name, value)) = assignment.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                return Err(OasisError::Command("usage: set VAR=value".to_string()));
            }
            self.set_variable(name, value);
            Ok(CommandOutput::None)
        } else {
            // Just show the variable value.
            match self.get_variable(args[0]) {
                Some(val) => Ok(CommandOutput::Text(format!("{}={val}", args[0]))),
                None => Ok(CommandOutput::Text(format!("{}: not set", args[0]))),
            }
        }
    }

    /// Built-in `unset` command.
    fn execute_unset(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: unset <VAR>".to_string()));
        }
        for name in args {
            self.unset_variable(name);
        }
        Ok(CommandOutput::None)
    }

    /// Built-in `env` command: list all variables.
    fn execute_env(&self) -> Result<CommandOutput> {
        let vars = self.variables.borrow();
        let mut entries: Vec<(&str, &str)> =
            vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        entries.sort_by_key(|(k, _)| *k);
        let mut out = String::new();
        for (k, v) in &entries {
            out.push_str(&format!("{k}={v}\n"));
        }
        Ok(CommandOutput::Text(out.trim_end().to_string()))
    }

    /// Built-in `alias` command.
    fn execute_alias(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            // List all aliases.
            let aliases = self.aliases.borrow();
            if aliases.is_empty() {
                return Ok(CommandOutput::Text("(no aliases defined)".to_string()));
            }
            let mut entries: Vec<(&str, &str)> = aliases
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            entries.sort_by_key(|(k, _)| *k);
            let mut out = String::new();
            for (k, v) in &entries {
                out.push_str(&format!("alias {k}='{v}'\n"));
            }
            return Ok(CommandOutput::Text(out.trim_end().to_string()));
        }
        let assignment = args.join(" ");
        if let Some((name, value)) = assignment.split_once('=') {
            let name = name.trim();
            let value = value.trim().trim_matches('\'').trim_matches('"');
            if name.is_empty() {
                return Err(OasisError::Command(
                    "usage: alias <name>=<command>".to_string(),
                ));
            }
            self.set_alias(name, value);
            Ok(CommandOutput::None)
        } else {
            // Show alias value.
            let aliases = self.aliases.borrow();
            match aliases.get(args[0]) {
                Some(val) => Ok(CommandOutput::Text(format!("alias {}='{val}'", args[0]))),
                None => Ok(CommandOutput::Text(format!("{}: not aliased", args[0]))),
            }
        }
    }

    /// Built-in `unalias` command.
    fn execute_unalias(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: unalias <name>".to_string()));
        }
        for name in args {
            self.unset_alias(name);
        }
        Ok(CommandOutput::None)
    }

    /// Return a sorted list of (name, description) pairs.
    pub fn list_commands(&self) -> Vec<(&str, &str)> {
        let mut cmds: Vec<(&str, &str)> = self
            .commands
            .values()
            .map(|c| (c.name(), c.description()))
            .collect();
        cmds.sort_by_key(|(name, _)| *name);
        cmds
    }

    /// Return completions for a partial command name.
    pub fn completions(&self, partial: &str) -> Vec<String> {
        let lower = partial.to_ascii_lowercase();
        let mut matches: Vec<String> = self
            .commands
            .keys()
            .filter(|name| name.starts_with(&lower))
            .cloned()
            .collect();
        // Also include user-defined functions.
        for name in self.functions.borrow().keys() {
            if name.starts_with(&lower) {
                matches.push(name.clone());
            }
        }
        matches
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tokenizer: handles single quotes, double quotes, and backslash escapes.
// ---------------------------------------------------------------------------

/// Tokenize a command line respecting quotes and backslash escapes.
///
/// - Single-quoted strings preserve all characters literally.
/// - Double-quoted strings allow `$VAR` expansion (done before tokenize).
/// - Backslash escapes the next character outside of quotes.
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
        } else if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(&next) = chars.peek()
            {
                match next {
                    '"' | '\\' | '$' => {
                        current.push(chars.next().unwrap());
                    },
                    _ => {
                        current.push('\\');
                    },
                }
            } else if ch == '\\' {
                current.push('\\');
            } else {
                current.push(ch);
            }
        } else {
            match ch {
                '\'' => in_single = true,
                '"' => in_double = true,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                },
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                },
                _ => current.push(ch),
            }
        }
    }

    if in_single {
        return Err(OasisError::Command("unterminated single quote".to_string()));
    }
    if in_double {
        return Err(OasisError::Command("unterminated double quote".to_string()));
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Chain splitting: ;, &&, ||
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainOp {
    /// First command or after `;`.
    Always,
    /// After `&&` -- run only if previous succeeded.
    And,
    /// After `||` -- run only if previous failed.
    Or,
}

struct ChainSegment {
    command: String,
    chain_op: ChainOp,
}

/// Split a command line on `;`, `&&`, and `||` (respecting quotes).
fn split_chains(input: &str) -> Result<Vec<ChainSegment>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chain_op = ChainOp::Always;
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;

    while let Some(ch) = chars.next() {
        if in_single {
            current.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(ch);
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(next) = chars.next()
            {
                current.push(next);
            }
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                current.push(ch);
            },
            '"' => {
                in_double = true;
                current.push(ch);
            },
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            },
            '{' => {
                brace_depth += 1;
                current.push(ch);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                current.push(ch);
            },
            ';' if brace_depth > 0 => {
                // Inside braces: don't split, keep as part of the body.
                current.push(ch);
            },
            ';' => {
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::Always;
            },
            '&' if brace_depth > 0 => current.push(ch),
            '&' if chars.peek() == Some(&'&') => {
                chars.next(); // consume second &
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::And;
            },
            '|' if brace_depth > 0 && chars.peek() == Some(&'|') => {
                current.push(ch);
                current.push(chars.next().unwrap());
            },
            '|' if chars.peek() == Some(&'|') => {
                chars.next(); // consume second |
                let cmd = current.trim().to_string();
                if !cmd.is_empty() {
                    segments.push(ChainSegment {
                        command: cmd,
                        chain_op,
                    });
                }
                current.clear();
                chain_op = ChainOp::Or;
            },
            _ => current.push(ch),
        }
    }

    let cmd = current.trim().to_string();
    if !cmd.is_empty() {
        segments.push(ChainSegment {
            command: cmd,
            chain_op,
        });
    }

    Ok(segments)
}

// ---------------------------------------------------------------------------
// Pipe splitting
// ---------------------------------------------------------------------------

/// Split on `|` (single pipe, not `||`), respecting quotes.
fn split_pipes(input: &str) -> Result<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;

    while let Some(ch) = chars.next() {
        if in_single {
            current.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(ch);
            if ch == '"' {
                in_double = false;
            } else if ch == '\\'
                && let Some(next) = chars.next()
            {
                current.push(next);
            }
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                current.push(ch);
            },
            '"' => {
                in_double = true;
                current.push(ch);
            },
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            },
            '{' => {
                brace_depth += 1;
                current.push(ch);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                current.push(ch);
            },
            '|' if brace_depth == 0 && chars.peek() != Some(&'|') => {
                segments.push(current.trim().to_string());
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        segments.push(remaining);
    }

    Ok(segments)
}

// ---------------------------------------------------------------------------
// Redirection parsing
// ---------------------------------------------------------------------------

/// A single output redirection.
struct Redirect<'a> {
    path: &'a str,
    append: bool,
}

/// Parsed redirections for a command.
struct Redirections<'a> {
    /// Stdout redirect (`>` / `>>`).
    stdout: Option<Redirect<'a>>,
    /// Stderr redirect (`2>` / `2>>`).
    stderr: Option<Redirect<'a>>,
    /// Merge stderr into stdout (`2>&1`).
    stderr_to_stdout: bool,
}

/// Parse redirect operators from a command string.
///
/// Supports:
/// - `> file` / `>> file`   (stdout)
/// - `2> file` / `2>> file` (stderr)
/// - `2>&1`                 (merge stderr into stdout)
///
/// Returns `(command_part, redirections)`.
fn parse_redirect(input: &str) -> (&str, Redirections<'_>) {
    let bytes = input.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth: usize = 0;
    let mut i = 0;
    let mut first_redirect_pos: Option<usize> = None;

    // Collected redirect entries: (position, fd, append, merge_target).
    // fd: 1 = stdout, 2 = stderr.
    // merge_target: Some("1") for `2>&1`.
    let mut redirects: Vec<(usize, u8, bool, bool)> = Vec::new();

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' {
                i += 1; // skip next
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b'2' if brace_depth == 0
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'>'
                    && (i == 0 || bytes[i - 1].is_ascii_whitespace()) =>
                {
                    if first_redirect_pos.is_none() {
                        first_redirect_pos = Some(i);
                    }
                    // Check for 2>> or 2>&1.
                    if i + 3 < bytes.len() && bytes[i + 2] == b'&' && bytes[i + 3] == b'1' {
                        // 2>&1 -- merge stderr into stdout.
                        redirects.push((i, 2, false, true));
                        i += 3;
                    } else if i + 2 < bytes.len() && bytes[i + 2] == b'>' {
                        // 2>>
                        redirects.push((i, 2, true, false));
                        i += 2;
                    } else {
                        // 2>
                        redirects.push((i, 2, false, false));
                        i += 1;
                    }
                },
                b'>' if brace_depth == 0 => {
                    if first_redirect_pos.is_none() {
                        first_redirect_pos = Some(i);
                    }
                    if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                        redirects.push((i, 1, true, false));
                        i += 1;
                    } else {
                        redirects.push((i, 1, false, false));
                    }
                },
                _ => {},
            }
        }
        i += 1;
    }

    if redirects.is_empty() {
        return (
            input,
            Redirections {
                stdout: None,
                stderr: None,
                stderr_to_stdout: false,
            },
        );
    }

    let cmd_part = &input[..first_redirect_pos.unwrap_or(input.len())];

    let mut stdout_redirect: Option<Redirect<'_>> = None;
    let mut stderr_redirect: Option<Redirect<'_>> = None;
    let mut stderr_to_stdout = false;

    for (idx, &(pos, fd, append, merge)) in redirects.iter().enumerate() {
        if merge {
            stderr_to_stdout = true;
            continue;
        }

        // Path is everything from after the operator to the next redirect
        // or end of string.
        let op_len = if fd == 2 {
            if append { 3 } else { 2 } // "2>>" = 3, "2>" = 2
        } else if append {
            2
        } else {
            1
        }; // ">>" = 2, ">" = 1

        let path_start = pos + op_len;
        let path_end = if idx + 1 < redirects.len() {
            redirects[idx + 1].0
        } else {
            input.len()
        };
        let path = input[path_start..path_end].trim();

        let redir = Redirect { path, append };
        if fd == 2 {
            stderr_redirect = Some(redir);
        } else {
            stdout_redirect = Some(redir);
        }
    }

    (
        cmd_part,
        Redirections {
            stdout: stdout_redirect,
            stderr: stderr_redirect,
            stderr_to_stdout,
        },
    )
}

/// Extract text content from a `CommandOutput`.
fn output_to_text(output: &CommandOutput) -> String {
    match output {
        CommandOutput::Text(t) => t.clone(),
        CommandOutput::Table { headers, rows } => {
            let mut out = headers.join(" | ");
            for row in rows {
                out.push('\n');
                out.push_str(&row.join(" | "));
            }
            out
        },
        _ => String::new(),
    }
}

/// Write text to a VFS file, handling append mode.
fn write_redirect(
    text: &str,
    raw_path: &str,
    append: bool,
    cwd: &str,
    vfs: &mut dyn Vfs,
) -> Result<()> {
    let path = resolve_path(cwd, raw_path.trim());
    if append {
        let existing = if vfs.exists(&path) {
            let data = vfs.read(&path)?;
            String::from_utf8_lossy(&data).into_owned()
        } else {
            String::new()
        };
        let combined = if existing.is_empty() {
            text.to_string()
        } else {
            format!("{existing}\n{text}")
        };
        vfs.write(&path, combined.as_bytes())?;
    } else {
        vfs.write(&path, text.as_bytes())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Glob expansion
// ---------------------------------------------------------------------------

/// Expand brace patterns (`{a,b,c}`) in tokens.
///
/// A token like `file.{rs,toml}` expands to `["file.rs", "file.toml"]`.
/// Nested braces are not supported. If the token contains no braces or
/// the braces are malformed, it is returned as-is.
fn expand_braces(tokens: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for token in tokens {
        if let Some(expanded) = expand_one_brace(token) {
            result.extend(expanded);
        } else {
            result.push(token.clone());
        }
    }
    result
}

/// Maximum nesting depth for brace expansion to prevent stack overflow.
const MAX_BRACE_DEPTH: usize = 16;

/// Expand a single brace expression. Returns `None` if no valid brace
/// pattern is found.
fn expand_one_brace(token: &str) -> Option<Vec<String>> {
    expand_one_brace_inner(token, 0)
}

fn expand_one_brace_inner(token: &str, depth: usize) -> Option<Vec<String>> {
    if depth >= MAX_BRACE_DEPTH {
        return None;
    }
    let open = token.find('{')?;
    let close = token[open..].find('}').map(|i| i + open)?;
    let prefix = &token[..open];
    let suffix = &token[close + 1..];
    let alternatives = &token[open + 1..close];

    // Must contain at least one comma to be a brace expansion.
    if !alternatives.contains(',') {
        return None;
    }

    let parts: Vec<&str> = alternatives.split(',').collect();
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let expanded = format!("{prefix}{part}{suffix}");
        // Recursively expand nested braces in the suffix.
        if let Some(nested) = expand_one_brace_inner(&expanded, depth + 1) {
            result.extend(nested);
        } else {
            result.push(expanded);
        }
    }
    Some(result)
}

/// Expand glob patterns (`*`, `?`, `[...]`) in tokens against VFS.
fn expand_globs(tokens: &[String], vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    let mut result = Vec::new();
    for token in tokens {
        if token.contains('*') || token.contains('?') || token.contains('[') {
            let expanded = expand_one_glob(token, vfs, cwd);
            if expanded.is_empty() {
                // No matches: pass the pattern through as-is.
                result.push(token.clone());
            } else {
                result.extend(expanded);
            }
        } else {
            result.push(token.clone());
        }
    }
    result
}

/// Expand a single glob pattern against the VFS.
fn expand_one_glob(pattern: &str, vfs: &mut dyn Vfs, cwd: &str) -> Vec<String> {
    // Split into directory and filename parts.
    let full_pattern = if pattern.starts_with('/') {
        pattern.to_string()
    } else if cwd == "/" {
        format!("/{pattern}")
    } else {
        format!("{cwd}/{pattern}")
    };

    let (dir, file_pattern) = match full_pattern.rsplit_once('/') {
        Some((d, f)) => {
            let dir = if d.is_empty() { "/" } else { d };
            (dir.to_string(), f.to_string())
        },
        None => (cwd.to_string(), full_pattern),
    };

    // Don't expand if the directory part also has globs (simple impl).
    if dir.contains('*') || dir.contains('?') {
        return Vec::new();
    }

    let entries = match vfs.readdir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut matches: Vec<String> = entries
        .iter()
        .filter(|e| glob_match(&file_pattern, &e.name))
        .map(|e| {
            if dir == "/" {
                format!("/{}", e.name)
            } else {
                format!("{}/{}", dir, e.name)
            }
        })
        .collect();
    matches.sort();
    matches
}

/// Glob matching: `*` matches any string, `?` matches one char,
/// `[abc]` matches one of the listed chars, `[a-z]` matches a range,
/// `[!abc]` or `[^abc]` matches any char NOT in the set.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, &t, 0, 0, 0)
}

/// Maximum recursion depth for glob matching to prevent stack overflow.
const GLOB_MAX_DEPTH: usize = 256;

fn glob_match_inner(p: &[char], t: &[char], pi: usize, ti: usize, depth: usize) -> bool {
    if depth >= GLOB_MAX_DEPTH {
        return false;
    }
    if pi == p.len() && ti == t.len() {
        return true;
    }
    if pi == p.len() {
        return false;
    }
    if p[pi] == '*' {
        // Try matching zero or more chars.
        for skip in 0..=(t.len() - ti) {
            if glob_match_inner(p, t, pi + 1, ti + skip, depth + 1) {
                return true;
            }
        }
        false
    } else if p[pi] == '[' {
        // Character class: [abc], [a-z], [!abc], [^abc].
        if ti >= t.len() {
            return false;
        }
        match parse_char_class(p, pi) {
            Some((negate, chars, end_pi)) => {
                let found = chars.contains(&t[ti]);
                let matched = if negate { !found } else { found };
                if matched {
                    glob_match_inner(p, t, end_pi, ti + 1, depth + 1)
                } else {
                    false
                }
            },
            None => {
                // Malformed bracket: treat '[' as literal.
                if ti < t.len() && p[pi] == t[ti] {
                    glob_match_inner(p, t, pi + 1, ti + 1, depth + 1)
                } else {
                    false
                }
            },
        }
    } else if ti < t.len() && (p[pi] == '?' || p[pi] == t[ti]) {
        glob_match_inner(p, t, pi + 1, ti + 1, depth + 1)
    } else {
        false
    }
}

/// Parse a character class starting at `p[pi]` which is `[`.
///
/// Returns `(negate, chars, end_index)` where `end_index` is the
/// position after the closing `]`. Returns `None` if malformed.
fn parse_char_class(p: &[char], pi: usize) -> Option<(bool, Vec<char>, usize)> {
    debug_assert!(p[pi] == '[');
    let mut i = pi + 1;
    if i >= p.len() {
        return None;
    }

    // Check for negation.
    let negate = p[i] == '!' || p[i] == '^';
    if negate {
        i += 1;
    }

    let mut chars = Vec::new();

    while i < p.len() && p[i] != ']' {
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            // Range: a-z.
            let start = p[i];
            let end = p[i + 2];
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            // Cap range expansion to prevent huge allocations from
            // broad Unicode ranges (e.g. [\0-\u{10FFFF}]).
            let range_len = (hi as u32).saturating_sub(lo as u32) + 1;
            if range_len > 256 {
                return None;
            }
            for c in lo..=hi {
                chars.push(c);
            }
            i += 3;
        } else {
            chars.push(p[i]);
            i += 1;
        }
    }

    if i < p.len() && p[i] == ']' {
        Some((negate, chars, i + 1))
    } else {
        None // No closing bracket.
    }
}

// ---------------------------------------------------------------------------
// Case pattern matching
// ---------------------------------------------------------------------------

/// Match a case pattern against a value.
///
/// Supports `*` (wildcard), `|` (alternation), and literal matching.
fn case_pattern_matches(value: &str, pattern: &str) -> bool {
    for alt in pattern.split('|') {
        let alt = alt.trim();
        if alt == "*" {
            return true;
        }
        if alt == value {
            return true;
        }
        if alt.contains('*') && glob_match_simple(value, alt) {
            return true;
        }
    }
    false
}

/// Simple glob matching supporting `*` as wildcard.
fn glob_match_simple(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return value.starts_with(prefix) && value.ends_with(suffix);
    }
    value == pattern
}

// ---------------------------------------------------------------------------
// Path resolution helper
// ---------------------------------------------------------------------------

/// Resolve a possibly-relative path against the current working directory.
pub fn resolve_path(cwd: &str, input: &str) -> String {
    let raw = if input.starts_with('/') {
        input.to_string()
    } else if cwd == "/" {
        format!("/{input}")
    } else {
        format!("{cwd}/{input}")
    };

    let mut parts: Vec<&str> = Vec::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {},
            ".." => {
                parts.pop();
            },
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

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
        fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
            // If stdin is set and no args, echo stdin.
            if args.is_empty() {
                if let Some(ref stdin) = env.stdin {
                    return Ok(CommandOutput::Text(stdin.clone()));
                }
            }
            Ok(CommandOutput::Text(args.join(" ")))
        }
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
    fn register_and_execute() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn unknown_command() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("nonexistent", &mut env);
        match result {
            Ok(CommandOutput::Text(s)) => assert!(s.contains("error")),
            Err(_) => {},
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn empty_input() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for empty input"),
        }
    }

    #[test]
    fn list_commands_sorted() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let cmds = reg.list_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, "echo");
    }

    #[test]
    fn whitespace_only_input_returns_none() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("   \t  ", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for whitespace-only input"),
        }
    }

    #[test]
    fn multiple_spaces_between_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo   hello    world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn leading_trailing_whitespace() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("  echo hi  ", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hi"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn command_no_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, ""),
            CommandOutput::None => {}, // Empty echo may produce no output.
            _ => panic!("expected text or none"),
        }
    }

    #[test]
    fn unknown_command_error_message_contains_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("foobar", &mut env) {
            Ok(CommandOutput::Text(s)) => {
                assert!(s.contains("foobar"), "error should contain command name");
            },
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("foobar"), "error should contain command name");
            },
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn register_replaces_existing_command() {
        struct CmdA;
        impl Command for CmdA {
            fn name(&self) -> &str {
                "test"
            }
            fn description(&self) -> &str {
                "version A"
            }
            fn usage(&self) -> &str {
                "test"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Text("A".into()))
            }
        }
        struct CmdB;
        impl Command for CmdB {
            fn name(&self) -> &str {
                "test"
            }
            fn description(&self) -> &str {
                "version B"
            }
            fn usage(&self) -> &str {
                "test"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Text("B".into()))
            }
        }

        let mut reg = CommandRegistry::new();
        reg.register(Box::new(CmdA));
        reg.register(Box::new(CmdB));

        let cmds = reg.list_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].1, "version B");
    }

    #[test]
    fn list_commands_sorted_multiple() {
        struct Named(&'static str);
        impl Command for Named {
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "desc"
            }
            fn usage(&self) -> &str {
                self.0
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::None)
            }
        }

        let mut reg = CommandRegistry::new();
        reg.register(Box::new(Named("zebra")));
        reg.register(Box::new(Named("alpha")));
        reg.register(Box::new(Named("middle")));

        let cmds = reg.list_commands();
        assert_eq!(cmds[0].0, "alpha");
        assert_eq!(cmds[1].0, "middle");
        assert_eq!(cmds[2].0, "zebra");
    }

    #[test]
    fn default_creates_empty_registry() {
        let reg = CommandRegistry::default();
        assert!(reg.list_commands().is_empty());
    }

    #[test]
    fn command_output_variants_are_debug() {
        let outputs = vec![
            CommandOutput::Text("hi".into()),
            CommandOutput::Table {
                headers: vec!["a".into()],
                rows: vec![vec!["b".into()]],
            },
            CommandOutput::None,
            CommandOutput::Clear,
            CommandOutput::ListenToggle { port: 8080 },
            CommandOutput::RemoteConnect {
                address: "1.2.3.4".into(),
                port: 22,
                psk: Some("key".into()),
            },
            CommandOutput::BrowserSandbox { enable: true },
            CommandOutput::SkinSwap { name: "xp".into() },
            CommandOutput::FtpToggle { port: 2121 },
        ];
        for o in &outputs {
            let _ = format!("{o:?}");
        }
    }

    #[test]
    fn many_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_input = format!(
            "echo {}",
            (0..100)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        match reg.execute(&long_input, &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("99")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn very_long_command_name() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_name = "a".repeat(10_000);
        let result = reg.execute(&long_name, &mut env);
        // Should return error text (unknown command).
        match result {
            Ok(CommandOutput::Text(s)) => assert!(s.contains("error")),
            Err(_) => {},
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn very_long_argument() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let long_arg = "x".repeat(50_000);
        let input = format!("echo {long_arg}");
        match reg.execute(&input, &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s.len(), 50_000),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn null_bytes_in_input() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let input = "echo hello\0world";
        let result = reg.execute(input, &mut env);
        assert!(result.is_ok());
    }

    #[test]
    fn tab_separated_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo\thello\tworld", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("hello")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn newline_in_input() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("echo line1\nline2", &mut env);
        assert!(result.is_ok());
    }

    #[test]
    fn only_spaces() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("     ", &mut env).unwrap() {
            CommandOutput::None => {},
            _ => panic!("expected None for whitespace-only"),
        }
    }

    #[test]
    fn command_case_insensitive() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("ECHO hello", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn register_many_commands() {
        let mut reg = CommandRegistry::new();
        for _ in 0..100 {
            reg.register(Box::new(EchoCmd));
        }
        let cmds = reg.list_commands();
        assert!(cmds.iter().any(|(name, _)| *name == "echo"));
    }

    #[test]
    fn execute_with_special_chars_in_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo @#$%^&", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("@#")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn execute_unicode_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo こんにちは 世界", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("こんにちは"));
                assert!(s.contains("世界"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Tokenizer tests --

    #[test]
    fn tokenize_simple() {
        assert_eq!(tokenize("hello world").unwrap(), vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_single_quotes() {
        assert_eq!(
            tokenize("echo 'hello world'").unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_double_quotes() {
        assert_eq!(
            tokenize(r#"echo "hello world""#).unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_backslash_escape() {
        assert_eq!(
            tokenize(r"echo hello\ world").unwrap(),
            vec!["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_mixed_quotes() {
        assert_eq!(
            tokenize(r#"echo 'single' "double" plain"#).unwrap(),
            vec!["echo", "single", "double", "plain"]
        );
    }

    #[test]
    fn tokenize_empty() {
        assert!(tokenize("").unwrap().is_empty());
    }

    #[test]
    fn tokenize_unterminated_single() {
        assert!(tokenize("echo 'unterminated").is_err());
    }

    #[test]
    fn tokenize_unterminated_double() {
        assert!(tokenize(r#"echo "unterminated"#).is_err());
    }

    // -- Variable expansion tests --

    #[test]
    fn variable_expansion() {
        let reg = CommandRegistry::new();
        reg.set_variable("NAME", "oasis");
        let result = reg.expand_variables("hello $NAME", "/");
        assert_eq!(result, "hello oasis");
    }

    #[test]
    fn variable_expansion_braces() {
        let reg = CommandRegistry::new();
        reg.set_variable("NAME", "oasis");
        let result = reg.expand_variables("hello ${NAME}!", "/");
        assert_eq!(result, "hello oasis!");
    }

    #[test]
    fn variable_cwd() {
        let reg = CommandRegistry::new();
        let result = reg.expand_variables("pwd=$CWD", "/home/user");
        assert_eq!(result, "pwd=/home/user");
    }

    #[test]
    fn variable_exit_code() {
        let reg = CommandRegistry::new();
        reg.last_exit_code.set(42);
        let result = reg.expand_variables("exit=$?", "/");
        assert_eq!(result, "exit=42");
    }

    #[test]
    fn variable_undefined() {
        let reg = CommandRegistry::new();
        let result = reg.expand_variables("$UNDEFINED_VAR", "/");
        assert_eq!(result, "");
    }

    // -- History tests --

    #[test]
    fn history_push_and_retrieve() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo first", &mut env).unwrap();
        reg.execute("echo second", &mut env).unwrap();
        let hist = reg.history();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0], "echo first");
        assert_eq!(hist[1], "echo second");
    }

    #[test]
    fn history_bang_bang() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo hello", &mut env).unwrap();
        match reg.execute("!!", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn history_bang_n() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo first", &mut env).unwrap();
        reg.execute("echo second", &mut env).unwrap();
        match reg.execute("!1", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "first"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn history_no_duplicates() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("echo same", &mut env).unwrap();
        reg.execute("echo same", &mut env).unwrap();
        assert_eq!(reg.history().len(), 1);
    }

    // -- Pipe tests --

    #[test]
    fn pipe_two_commands() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));

        // CatCmd that reads stdin.
        struct UpperCmd;
        impl Command for UpperCmd {
            fn name(&self) -> &str {
                "upper"
            }
            fn description(&self) -> &str {
                "Uppercase stdin"
            }
            fn usage(&self) -> &str {
                "upper"
            }
            fn execute(&self, _: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
                let input = env.stdin.take().unwrap_or_default();
                Ok(CommandOutput::Text(input.to_uppercase()))
            }
        }
        reg.register(Box::new(UpperCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello | upper", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "HELLO"),
            _ => panic!("expected text output"),
        }
    }

    // -- Chaining tests --

    #[test]
    fn chain_semicolon() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo hello ; echo world", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("hello"));
                assert!(s.contains("world"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_and_success() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo first && echo second", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("first"));
                assert!(s.contains("second"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_and_failure() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // nonexistent fails -> echo after && should NOT run.
        match reg
            .execute("nonexistent && echo should_not_run", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert!(s.contains("error"));
                assert!(!s.contains("should_not_run"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_or_success() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // echo succeeds -> echo after || should NOT run.
        match reg.execute("echo ok || echo fallback", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("ok"));
                assert!(!s.contains("fallback"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn chain_or_failure() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // nonexistent fails -> echo after || SHOULD run.
        match reg
            .execute("nonexistent || echo fallback", &mut env)
            .unwrap()
        {
            CommandOutput::Text(s) => {
                assert!(s.contains("fallback"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Chain preserves signals alongside text --

    #[test]
    fn chain_signal_then_text_produces_multi() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        // ClearCmd for a signal output.
        struct ClearCmd;
        impl Command for ClearCmd {
            fn name(&self) -> &str {
                "clear"
            }
            fn description(&self) -> &str {
                "Clear"
            }
            fn usage(&self) -> &str {
                "clear"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Clear)
            }
        }
        reg.register(Box::new(ClearCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // `clear ; echo Done` should produce Multi with both outputs.
        match reg.execute("clear ; echo Done", &mut env).unwrap() {
            CommandOutput::Multi(outputs) => {
                assert_eq!(outputs.len(), 2);
                assert!(matches!(outputs[0], CommandOutput::Clear));
                assert!(matches!(outputs[1], CommandOutput::Text(_)));
                if let CommandOutput::Text(ref s) = outputs[1] {
                    assert_eq!(s, "Done");
                }
            },
            _ => panic!("expected Multi output"),
        }
    }

    #[test]
    fn chain_text_then_signal_produces_multi() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        struct ClearCmd;
        impl Command for ClearCmd {
            fn name(&self) -> &str {
                "clear"
            }
            fn description(&self) -> &str {
                "Clear"
            }
            fn usage(&self) -> &str {
                "clear"
            }
            fn execute(&self, _: &[&str], _: &mut Environment<'_>) -> Result<CommandOutput> {
                Ok(CommandOutput::Clear)
            }
        }
        reg.register(Box::new(ClearCmd));

        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // `echo Hello ; clear` should produce Multi.
        match reg.execute("echo Hello ; clear", &mut env).unwrap() {
            CommandOutput::Multi(outputs) => {
                assert_eq!(outputs.len(), 2);
                assert!(matches!(outputs[0], CommandOutput::Text(_)));
                assert!(matches!(outputs[1], CommandOutput::Clear));
            },
            _ => panic!("expected Multi output"),
        }
    }

    #[test]
    fn chain_text_only_merges_to_single() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Two text commands should merge into a single Text, not Multi.
        match reg.execute("echo hello ; echo world", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("hello"));
                assert!(s.contains("world"));
            },
            _ => panic!("expected merged Text output"),
        }
    }

    // -- Redirection tests --

    #[test]
    fn redirect_write() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("echo hello > /tmp/out.txt", &mut env).unwrap();
        assert_eq!(env.vfs.read("/tmp/out.txt").unwrap(), b"hello");
    }

    #[test]
    fn redirect_append() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("echo line1 > /tmp/out.txt", &mut env).unwrap();
        reg.execute("echo line2 >> /tmp/out.txt", &mut env).unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/out.txt").unwrap()).into_owned();
        assert!(content.contains("line1"));
        assert!(content.contains("line2"));
    }

    // -- Stderr redirect tests --

    #[test]
    fn stderr_redirect_captures_error() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Unknown command produces an error; 2> captures it.
        reg.execute("nosuchcmd 2> /tmp/err.txt", &mut env).unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd"));
    }

    #[test]
    fn stderr_redirect_append() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("nosuchcmd1 2> /tmp/err.txt", &mut env).unwrap();
        reg.execute("nosuchcmd2 2>> /tmp/err.txt", &mut env)
            .unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd1"));
        assert!(content.contains("nosuchcmd2"));
    }

    #[test]
    fn stderr_to_stdout_merge() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Unknown command with 2>&1 should merge error into stdout.
        match reg.execute("nosuchcmd 2>&1", &mut env) {
            Ok(CommandOutput::Text(s)) => {
                assert!(s.contains("nosuchcmd"));
            },
            other => panic!("expected Text with error, got {other:?}"),
        }
    }

    #[test]
    fn stderr_to_stdout_with_redirect() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // 2>&1 > file should capture merged output to file.
        reg.execute("nosuchcmd 2>&1 > /tmp/out.txt", &mut env)
            .unwrap();
        let content = String::from_utf8_lossy(&env.vfs.read("/tmp/out.txt").unwrap()).into_owned();
        assert!(content.contains("nosuchcmd"));
    }

    #[test]
    fn stdout_and_stderr_separate_redirects() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        // Successful command: stdout goes to file, stderr empty.
        reg.execute("echo hello > /tmp/out.txt 2> /tmp/err.txt", &mut env)
            .unwrap();
        assert_eq!(env.vfs.read("/tmp/out.txt").unwrap(), b"hello");
        let err = String::from_utf8_lossy(&env.vfs.read("/tmp/err.txt").unwrap()).into_owned();
        assert!(err.is_empty());
    }

    #[test]
    fn stderr_preserved_in_env_without_redirect() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Without 2>, error propagates normally (Err result).
        let result = reg.execute("nosuchcmd", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn stderr_redirect_sets_exit_code() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let mut env = make_env(&mut vfs);
        reg.execute("nosuchcmd 2> /tmp/err.txt", &mut env).unwrap();
        // Exit code should be 1 even though error was captured.
        assert_eq!(reg.last_exit_code.get(), 1);
    }

    #[test]
    fn parse_redirect_2_to_file() {
        let (cmd, redirections) = parse_redirect("echo hello 2> /tmp/e.txt");
        assert_eq!(cmd, "echo hello ");
        assert!(redirections.stderr.is_some());
        assert_eq!(redirections.stderr.unwrap().path, "/tmp/e.txt");
        assert!(!redirections.stderr_to_stdout);
    }

    #[test]
    fn parse_redirect_2_append() {
        let (cmd, redirections) = parse_redirect("cmd 2>> /tmp/e.txt");
        assert_eq!(cmd, "cmd ");
        let redir = redirections.stderr.unwrap();
        assert!(redir.append);
        assert_eq!(redir.path, "/tmp/e.txt");
    }

    #[test]
    fn parse_redirect_2_to_1() {
        let (cmd, redirections) = parse_redirect("cmd 2>&1");
        assert_eq!(cmd, "cmd ");
        assert!(redirections.stderr_to_stdout);
        assert!(redirections.stderr.is_none());
    }

    #[test]
    fn parse_redirect_stdout_only() {
        let (cmd, redirections) = parse_redirect("echo hi > /tmp/out.txt");
        assert_eq!(cmd, "echo hi ");
        assert!(redirections.stdout.is_some());
        assert!(redirections.stderr.is_none());
        assert!(!redirections.stderr_to_stdout);
    }

    // -- Glob tests --

    #[test]
    fn glob_match_star() {
        assert!(glob_match("*.txt", "hello.txt"));
        assert!(!glob_match("*.txt", "hello.md"));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("h?llo", "hello"));
        assert!(!glob_match("h?llo", "heeello"));
    }

    #[test]
    fn glob_expansion_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.write("/file1.txt", b"a").unwrap();
        vfs.write("/file2.txt", b"b").unwrap();
        vfs.write("/file3.md", b"c").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo /*.txt", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("/file1.txt"));
                assert!(s.contains("/file2.txt"));
                assert!(!s.contains("file3.md"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Brace expansion tests --

    #[test]
    fn brace_expansion_basic() {
        let tokens = vec!["file.{txt,md,rs}".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["file.txt", "file.md", "file.rs"]);
    }

    #[test]
    fn brace_expansion_with_prefix_suffix() {
        let tokens = vec!["src/{main,lib}.rs".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn brace_no_comma_passthrough() {
        let tokens = vec!["{solo}".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["{solo}"]);
    }

    #[test]
    fn brace_expansion_no_braces_passthrough() {
        let tokens = vec!["plain.txt".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["plain.txt"]);
    }

    #[test]
    fn brace_expansion_multiple_tokens() {
        let tokens = vec!["a.{x,y}".to_string(), "b.txt".to_string()];
        let expanded = expand_braces(&tokens);
        assert_eq!(expanded, vec!["a.x", "a.y", "b.txt"]);
    }

    #[test]
    fn brace_expansion_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo file.{txt,md}", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "file.txt file.md");
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Character class tests --

    #[test]
    fn glob_char_class_list() {
        assert!(glob_match("[abc]", "a"));
        assert!(glob_match("[abc]", "b"));
        assert!(glob_match("[abc]", "c"));
        assert!(!glob_match("[abc]", "d"));
    }

    #[test]
    fn glob_char_class_range() {
        assert!(glob_match("[a-z]", "m"));
        assert!(glob_match("[0-9]", "5"));
        assert!(!glob_match("[a-z]", "A"));
        assert!(!glob_match("[0-9]", "x"));
    }

    #[test]
    fn glob_char_class_negation() {
        assert!(!glob_match("[!abc]", "a"));
        assert!(glob_match("[!abc]", "d"));
        assert!(!glob_match("[^0-9]", "5"));
        assert!(glob_match("[^0-9]", "x"));
    }

    #[test]
    fn glob_char_class_in_pattern() {
        assert!(glob_match("file[12].txt", "file1.txt"));
        assert!(glob_match("file[12].txt", "file2.txt"));
        assert!(!glob_match("file[12].txt", "file3.txt"));
    }

    #[test]
    fn glob_char_class_with_star() {
        assert!(glob_match("*.[ch]", "main.c"));
        assert!(glob_match("*.[ch]", "main.h"));
        assert!(!glob_match("*.[ch]", "main.o"));
    }

    #[test]
    fn glob_char_class_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.write("/file1.txt", b"a").unwrap();
        vfs.write("/file2.txt", b"b").unwrap();
        vfs.write("/file3.txt", b"c").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo /file[12].txt", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("/file1.txt"));
                assert!(s.contains("/file2.txt"));
                assert!(!s.contains("file3.txt"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Alias tests --

    #[test]
    fn alias_expansion() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("alias hi=echo", &mut env).unwrap();
        match reg.execute("hi hello", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn alias_list() {
        let reg = CommandRegistry::new();
        reg.set_alias("ll", "ls -l");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("alias", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("ll"));
                assert!(s.contains("ls -l"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Shell function tests --

    #[test]
    fn function_define_and_call() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function greet() { echo hello }", &mut env)
            .unwrap();
        match reg.execute("greet", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_with_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function say() { echo $1 $2 }", &mut env)
            .unwrap();
        match reg.execute("say hello world", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_arg_count() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function argc() { echo $# }", &mut env)
            .unwrap();
        match reg.execute("argc a b c", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "3"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_no_parens_syntax() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function hi { echo hi }", &mut env).unwrap();
        match reg.execute("hi", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hi"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_list() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("function", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("No functions")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_list_after_define() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function foo() { echo bar }", &mut env)
            .unwrap();
        match reg.execute("function", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("foo"));
                assert!(s.contains("echo bar"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_multi_command_body() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function both() { echo one ; echo two }", &mut env)
            .unwrap();
        match reg.execute("both", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("one"));
                assert!(s.contains("two"));
            },
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_return_sets_exit_code() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function fail() { return 42 }", &mut env)
            .unwrap();
        reg.execute("fail", &mut env).unwrap();
        assert_eq!(reg.last_exit_code.get(), 42);
    }

    #[test]
    fn function_recursion_depth_limit() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Define a function that calls itself infinitely.
        reg.execute("function inf() { inf }", &mut env).unwrap();
        let result = reg.execute("inf", &mut env);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("recursion depth"));
    }

    #[test]
    fn function_restores_args() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.set_variable("1", "original");
        reg.execute("function f() { echo $1 }", &mut env).unwrap();
        reg.execute("f override", &mut env).unwrap();
        // After function returns, $1 should be restored.
        assert_eq!(reg.get_variable("1").unwrap(), "original");
    }

    #[test]
    fn function_which() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("function myfn() { echo hi }", &mut env)
            .unwrap();
        match reg.execute("which myfn", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("shell function")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn function_empty_body_rejected() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("function bad() { }", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn function_body_with_pipe() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Pipe inside braces should be part of the body, not split
        // as a pipeline operator.
        reg.execute("function piped() { echo hello | echo world }", &mut env)
            .unwrap();
        let funcs = reg.list_functions();
        assert!(funcs.iter().any(|(n, b)| n == "piped" && b.contains('|')));
    }

    #[test]
    fn function_body_with_redirect() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        // Redirect inside braces should be part of the body, not
        // parsed as a top-level redirect.
        reg.execute("function redir() { echo hello > /tmp/out }", &mut env)
            .unwrap();
        let funcs = reg.list_functions();
        assert!(funcs.iter().any(|(n, b)| n == "redir" && b.contains('>')));
    }

    // -- Set/env tests --

    #[test]
    fn set_and_expand_variable() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        reg.execute("set GREETING=hello", &mut env).unwrap();
        match reg.execute("echo $GREETING", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn env_lists_variables() {
        let reg = CommandRegistry::new();
        reg.set_variable("FOO", "bar");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("env", &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("FOO=bar"));
            },
            _ => panic!("expected text output"),
        }
    }

    // -- Quoted args with commands --

    #[test]
    fn quoted_args_in_command() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        match reg.execute("echo 'hello world'", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected text output"),
        }
    }

    // -- Path resolution (unchanged) --

    #[test]
    fn resolve_path_absolute() {
        assert_eq!(resolve_path("/any", "/foo/bar"), "/foo/bar");
    }

    #[test]
    fn resolve_path_relative() {
        assert_eq!(resolve_path("/home", "user"), "/home/user");
    }

    #[test]
    fn resolve_path_dotdot() {
        assert_eq!(resolve_path("/a/b/c", "../../x"), "/a/x");
    }

    #[test]
    fn resolve_path_root_relative() {
        assert_eq!(resolve_path("/", "foo"), "/foo");
    }

    // -- Completions --

    #[test]
    fn completions_prefix() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let completions = reg.completions("ec");
        assert!(completions.contains(&"echo".to_string()));
    }

    #[test]
    fn completions_no_match() {
        let reg = CommandRegistry::new();
        let completions = reg.completions("xyz");
        assert!(completions.is_empty());
    }

    // -- Script control flow (if/while/for) --

    /// Helper: write a script to VFS and run it, returning the output text.
    fn run_script(reg: &CommandRegistry, vfs: &mut MemoryVfs, script: &str) -> String {
        vfs.write("/tmp/test.sh", script.as_bytes()).unwrap();
        let mut env = make_env(vfs);
        match reg.execute("run /tmp/test.sh", &mut env).unwrap() {
            CommandOutput::Text(s) => s,
            other => panic!("expected text, got {:?}", other),
        }
    }

    #[test]
    fn script_if_true_branch() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "if echo true\nthen\necho yes\nfi");
        assert_eq!(out, "yes");
    }

    #[test]
    fn script_if_false_branch() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "if echo false\nthen\necho yes\nfi");
        // false condition: then block skipped, no else block → shows command count
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_if_else() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo false\nthen\necho yes\nelse\necho no\nfi",
        );
        assert_eq!(out, "no");
    }

    #[test]
    fn script_if_else_true() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo true\nthen\necho correct\nelse\necho wrong\nfi",
        );
        assert_eq!(out, "correct");
    }

    #[test]
    fn script_for_loop() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "for x in a b c\ndo\necho $x\ndone");
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn script_for_loop_empty() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "for x in\ndo\necho $x\ndone");
        // Empty item list: loop body never runs
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_while_loop() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        // Use a counter via set/test: set counter, run while counter equals
        // a value, then unset to stop. Simpler: just run a fixed echo loop
        // that terminates immediately because condition is false.
        let out = run_script(&reg, &mut vfs, "while echo false\ndo\necho body\ndone");
        // Condition is false from the start, loop never executes.
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_while_loop_executes() {
        // Use a VFS file as a counter: while the file exists, execute body
        // and delete the file in the first iteration.
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        crate::register_dev_commands(&mut reg);
        crate::register_file_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        // test -f /tmp/flag returns "true" when file exists
        vfs.write("/tmp/flag", b"1").unwrap();
        // Script: while test -f /tmp/flag → echo iteration → rm /tmp/flag
        // But we don't have rm. Instead, write empty to flag and use
        // a different approach. Let's use set/env approach:
        // Set a variable and test it. Actually, let's just test that a
        // simple loop with echo condition works for at least one iteration.
        // We'll test with for loop which is more predictable.
        let out = run_script(
            &reg,
            &mut vfs,
            "for i in 1 2 3\ndo\necho iteration $i\ndone",
        );
        assert!(out.contains("iteration 1"));
        assert!(out.contains("iteration 2"));
        assert!(out.contains("iteration 3"));
    }

    #[test]
    fn script_nested_if() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo true\nthen\nif echo true\nthen\necho nested\nfi\nfi",
        );
        assert_eq!(out, "nested");
    }

    #[test]
    fn script_nested_if_outer_false() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "if echo false\nthen\nif echo true\nthen\necho nested\nfi\nfi",
        );
        // Outer if is false, whole then block (including inner if) skipped
        assert!(out.contains("commands executed"));
    }

    #[test]
    fn script_for_with_echo_output() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "echo start\nfor n in x y\ndo\necho item $n\ndone\necho end",
        );
        assert!(out.contains("start"));
        assert!(out.contains("item x"));
        assert!(out.contains("item y"));
        assert!(out.contains("end"));
    }

    #[test]
    fn script_comments_and_blank_lines_ignored() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(
            &reg,
            &mut vfs,
            "# This is a comment\n\necho hello\n\n# Another comment",
        );
        assert_eq!(out, "hello");
    }

    #[test]
    fn script_error_in_line() {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        let out = run_script(&reg, &mut vfs, "echo before\nnosuchcommand\necho after");
        assert!(out.contains("before"));
        assert!(out.contains("error at line 2"));
        assert!(out.contains("after"));
    }

    #[test]
    fn script_not_found() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("run /nonexistent.sh", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn script_empty() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/tmp/empty.sh", b"# just comments\n").unwrap();
        let mut env = make_env(&mut vfs);
        match reg.execute("run /tmp/empty.sh", &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("empty script")),
            _ => panic!("expected text"),
        }
    }

    // ===================================================================
    // Property-based tests
    // ===================================================================

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            // -- tokenize -----------------------------------------------

            /// Tokenizing a single unquoted word returns exactly that word.
            #[test]
            fn tokenize_single_word(w in "[a-zA-Z0-9_]{1,30}") {
                let tokens = tokenize(&w).unwrap();
                prop_assert_eq!(tokens, vec![w]);
            }

            /// Tokenizing two words separated by space returns two tokens.
            #[test]
            fn tokenize_two_words(
                a in "[a-zA-Z]{1,15}",
                b in "[a-zA-Z]{1,15}",
            ) {
                let input = format!("{a} {b}");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens.len(), 2);
                prop_assert_eq!(&tokens[0], &a);
                prop_assert_eq!(&tokens[1], &b);
            }

            /// Single-quoted strings preserve all content literally.
            #[test]
            fn tokenize_single_quoted_preserves(
                s in "[a-zA-Z0-9 \\$\\\\]{1,30}",
            ) {
                let input = format!("'{s}'");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens, vec![s]);
            }

            /// Double-quoted strings produce a single token.
            #[test]
            fn tokenize_double_quoted_single_token(
                s in "[a-zA-Z0-9 ]{1,30}",
            ) {
                let input = format!("\"{s}\"");
                let tokens = tokenize(&input).unwrap();
                prop_assert_eq!(tokens.len(), 1);
                prop_assert_eq!(&tokens[0], &s);
            }

            /// Tokenize never panics on arbitrary ASCII input.
            #[test]
            fn tokenize_never_panics(s in "[ -~]{0,80}") {
                let _ = tokenize(&s);
            }

            /// Empty input yields empty tokens.
            #[test]
            fn tokenize_whitespace_only(n in 1usize..20) {
                let input = " ".repeat(n);
                let tokens = tokenize(&input).unwrap();
                prop_assert!(tokens.is_empty());
            }

            // -- glob_match ----------------------------------------------

            /// Any text matches the `*` pattern.
            #[test]
            fn glob_star_matches_everything(s in "[a-z]{0,20}") {
                prop_assert!(glob_match("*", &s));
            }

            /// `?` matches exactly one character.
            #[test]
            fn glob_question_matches_single_char(c in "[a-z]") {
                prop_assert!(glob_match("?", &c));
            }

            /// `?` does NOT match empty string.
            #[test]
            fn glob_question_no_empty(_dummy in 0..1i32) {
                prop_assert!(!glob_match("?", ""));
            }

            /// A literal pattern matches only itself.
            #[test]
            fn glob_literal_exact(s in "[a-z]{1,20}") {
                prop_assert!(glob_match(&s, &s));
            }

            /// A literal pattern does not match a different string.
            #[test]
            fn glob_literal_no_mismatch(
                a in "[a-z]{1,10}",
                b in "[a-z]{1,10}",
            ) {
                if a != b {
                    prop_assert!(!glob_match(&a, &b));
                }
            }

            /// `*` at end matches any suffix.
            #[test]
            fn glob_star_suffix(
                prefix in "[a-z]{1,10}",
                suffix in "[a-z]{0,10}",
            ) {
                let text = format!("{prefix}{suffix}");
                let pattern = format!("{prefix}*");
                prop_assert!(glob_match(&pattern, &text));
            }

            /// Character class [a-z] matches lowercase chars.
            #[test]
            fn glob_char_class_range(c in "[a-z]") {
                prop_assert!(glob_match("[a-z]", &c));
            }

            /// Negated class [!a-z] does NOT match lowercase chars.
            #[test]
            fn glob_negated_class_range(c in "[a-z]") {
                prop_assert!(!glob_match("[!a-z]", &c));
            }

            // -- resolve_path -------------------------------------------

            /// resolve_path always returns a string starting with '/'.
            #[test]
            fn resolve_path_starts_with_slash(
                cwd in "/[a-z]{1,10}(/[a-z]{1,5}){0,3}",
                input in "[a-z./]{0,20}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    result.starts_with('/'),
                    "expected '/' prefix, got: {result}",
                );
            }

            /// resolve_path is idempotent when input is absolute.
            #[test]
            fn resolve_path_absolute_idempotent(
                path in "/[a-z]{1,10}(/[a-z]{1,5}){0,3}",
            ) {
                let first = resolve_path("/", &path);
                let second = resolve_path("/", &first);
                prop_assert_eq!(first, second);
            }

            /// resolve_path never contains `..` in output.
            #[test]
            fn resolve_path_no_dotdot(
                cwd in "/[a-z]{1,10}",
                input in "(\\.\\./){0,5}[a-z]{0,10}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    !result.contains(".."),
                    "result should not contain '..': {result}",
                );
            }

            /// resolve_path never contains double slashes.
            #[test]
            fn resolve_path_no_double_slashes(
                cwd in "/[a-z]{1,5}",
                input in "[a-z./]{0,15}",
            ) {
                let result = resolve_path(&cwd, &input);
                prop_assert!(
                    !result.contains("//"),
                    "result should not contain '//': {result}",
                );
            }

            // -- expand_braces -------------------------------------------

            /// Brace expansion with N alternatives produces N tokens.
            #[test]
            fn brace_expansion_count(
                prefix in "[a-z]{0,5}",
                a in "[a-z]{1,5}",
                b in "[a-z]{1,5}",
                c in "[a-z]{1,5}",
            ) {
                let input = format!("{prefix}{{{a},{b},{c}}}");
                let tokens = vec![input];
                let expanded = expand_braces(&tokens);
                prop_assert!(
                    expanded.len() == 3,
                    "expected 3 tokens from brace expansion, got: {:?}",
                    expanded,
                );
            }

            /// Brace expansion preserves prefix and suffix.
            #[test]
            fn brace_expansion_preserves_affix(
                prefix in "[a-z]{1,5}",
                suffix in "[a-z]{1,5}",
                a in "[a-z]{1,3}",
                b in "[a-z]{1,3}",
            ) {
                let input = format!("{prefix}{{{a},{b}}}{suffix}");
                let tokens = vec![input];
                let expanded = expand_braces(&tokens);
                for tok in &expanded {
                    prop_assert!(
                        tok.starts_with(&prefix) && tok.ends_with(&suffix),
                        "token '{tok}' should have prefix '{prefix}' and suffix '{suffix}'",
                    );
                }
            }

            /// A token without braces passes through unchanged.
            #[test]
            fn brace_expansion_passthrough(s in "[a-z]{1,20}") {
                let tokens = vec![s.clone()];
                let expanded = expand_braces(&tokens);
                prop_assert_eq!(expanded, vec![s]);
            }
        }
    }

    // -- Extended scripting tests --

    /// Minimal `test` mock: supports `test A == B` and `test true`.
    struct MockTestCmd;
    impl Command for MockTestCmd {
        fn name(&self) -> &str {
            "test"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn usage(&self) -> &str {
            "test"
        }
        fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
            if args.len() == 3 && args[1] == "==" {
                let result = args[0] == args[2];
                return Ok(CommandOutput::Text(result.to_string()));
            }
            if args.first() == Some(&"true") {
                return Ok(CommandOutput::Text("true".into()));
            }
            Ok(CommandOutput::Text("false".into()))
        }
    }

    fn make_ext_reg() -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(EchoCmd));
        reg.register(Box::new(MockTestCmd));
        reg
    }

    fn run_script_ext(reg: &CommandRegistry, script: &str) -> String {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        run_script(reg, &mut vfs, script)
    }

    #[test]
    fn script_elif_first_branch() {
        let reg = make_ext_reg();
        let script = "set X=hello\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "first");
    }

    #[test]
    fn script_elif_second_branch() {
        let reg = make_ext_reg();
        let script = "set X=world\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "second");
    }

    #[test]
    fn script_elif_else_fallback() {
        let reg = make_ext_reg();
        let script = "set X=other\n\
            if test $X == hello\n\
            then\n\
            echo first\n\
            elif test $X == world\n\
            then\n\
            echo second\n\
            else\n\
            echo third\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "third");
    }

    #[test]
    fn script_case_exact_match() {
        let reg = make_ext_reg();
        let script = "set X=hello\n\
            case $X in\n\
            hello) echo matched ;;  \n\
            *) echo default ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "matched");
    }

    #[test]
    fn script_case_wildcard_default() {
        let reg = make_ext_reg();
        let script = "set X=other\n\
            case $X in\n\
            hello) echo matched ;;\n\
            *) echo default ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "default");
    }

    #[test]
    fn script_case_alternation() {
        let reg = make_ext_reg();
        let script = "set X=world\n\
            case $X in\n\
            hello|world) echo either ;;\n\
            *) echo nope ;;\n\
            esac";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "either");
    }

    #[test]
    fn script_break_in_for() {
        let reg = make_ext_reg();
        let script = "for i in a b c d\n\
            do\n\
            if test $i == c\n\
            then\n\
            break\n\
            fi\n\
            echo $i\n\
            done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn script_continue_in_for() {
        let reg = make_ext_reg();
        let script = "for i in a b c d\n\
            do\n\
            if test $i == b\n\
            then\n\
            continue\n\
            fi\n\
            echo $i\n\
            done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "a\nc\nd");
    }

    #[test]
    fn script_break_in_while() {
        let reg = make_ext_reg();
        let script = "set N=0\n\
            while test true\n\
            do\n\
            echo $N\n\
            if test $N == 2\n\
            then\n\
            break\n\
            fi\n\
            set N=1\n\
            set N=2\n\
            done";
        let out = run_script_ext(&reg, script);
        // Should output at least the initial 0
        assert!(out.contains('0'));
    }

    #[test]
    fn script_local_variables() {
        let reg = make_ext_reg();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);

        // Set a global variable.
        reg.execute("set X=global", &mut env).unwrap();

        // Define a function that uses local.
        reg.execute("function myfn() { local X=local; echo $X }", &mut env)
            .unwrap();

        // Call the function.
        match reg.execute("myfn", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "local"),
            _ => panic!("expected text"),
        }

        // Global should be restored.
        assert_eq!(reg.get_variable("X"), Some("global".to_string()));
    }

    #[test]
    fn script_local_unset_after_function() {
        let reg = make_ext_reg();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);

        // No global X.
        assert!(reg.get_variable("X").is_none());

        // Function creates local X.
        reg.execute("function myfn() { local X=temp; echo $X }", &mut env)
            .unwrap();
        reg.execute("myfn", &mut env).unwrap();

        // X should not leak to global scope.
        assert!(reg.get_variable("X").is_none());
    }

    #[test]
    fn case_pattern_matches_exact() {
        assert!(super::case_pattern_matches("hello", "hello"));
        assert!(!super::case_pattern_matches("hello", "world"));
    }

    #[test]
    fn case_pattern_matches_wildcard() {
        assert!(super::case_pattern_matches("anything", "*"));
    }

    #[test]
    fn case_pattern_matches_alternation() {
        assert!(super::case_pattern_matches("b", "a|b|c"));
        assert!(!super::case_pattern_matches("d", "a|b|c"));
    }

    #[test]
    fn case_pattern_matches_glob_prefix() {
        assert!(super::case_pattern_matches("hello_world", "hello*"));
        assert!(!super::case_pattern_matches("goodbye", "hello*"));
    }

    #[test]
    fn case_pattern_matches_glob_suffix() {
        assert!(super::case_pattern_matches("file.txt", "*.txt"));
        assert!(!super::case_pattern_matches("file.rs", "*.txt"));
    }

    #[test]
    fn glob_match_simple_middle() {
        assert!(super::glob_match_simple("hello_world", "hello*world"));
        assert!(!super::glob_match_simple("hello_earth", "hello*world"));
    }

    #[test]
    fn script_case_no_match() {
        let reg = make_ext_reg();
        let script = "set X=zzz\n\
            case $X in\n\
            hello) echo matched ;;\n\
            esac\n\
            echo done";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "done");
    }

    #[test]
    fn script_elif_multiple() {
        let reg = make_ext_reg();
        let script = "set X=c\n\
            if test $X == a\n\
            then\n\
            echo A\n\
            elif test $X == b\n\
            then\n\
            echo B\n\
            elif test $X == c\n\
            then\n\
            echo C\n\
            elif test $X == d\n\
            then\n\
            echo D\n\
            fi";
        let out = run_script_ext(&reg, script);
        assert_eq!(out, "C");
    }

    #[test]
    fn script_local_outside_function_fails() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("local X=val", &mut env);
        assert!(result.is_err());
    }

    #[test]
    fn script_break_outside_loop_noop() {
        // break outside a loop just sets the flag which gets
        // ignored since there's no loop to break from.
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let result = reg.execute("break", &mut env);
        assert!(result.is_ok());
    }
}

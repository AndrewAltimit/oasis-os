//! [`CommandRegistry`] struct and shell state management.
//!
//! Contains the registry definition, variable/alias/function APIs,
//! history management, and expansion helpers (variables, aliases,
//! history).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use oasis_types::error::{OasisError, Result};

use crate::jobs::JobManager;
use crate::types::{
    Command, CommandOutput, Environment, MAX_CALL_DEPTH, MAX_COMMANDS_PER_LINE, MAX_HISTORY,
    MAX_VAR_LEN, MAX_VARIABLES, ShellFunction,
};

/// Whether `name` can be assigned by the user: a letter or `_`, then
/// letters, digits and `_` (not `?`, `1` or `a b`).
pub(crate) fn is_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Registry of available commands with dispatch.
///
/// Also holds persistent shell state: variables, aliases, and history.
pub struct CommandRegistry {
    pub(crate) commands: HashMap<String, Box<dyn Command>>,
    pub(crate) variables: RefCell<HashMap<String, String>>,
    pub(crate) aliases: RefCell<HashMap<String, String>>,
    pub(crate) history: RefCell<Vec<String>>,
    pub(crate) last_exit_code: Cell<i32>,
    /// User-defined shell functions.
    pub(crate) functions: RefCell<HashMap<String, ShellFunction>>,
    /// Current function call depth (for recursion limiting).
    pub(crate) call_depth: Cell<usize>,
    /// Set by `return` to signal early exit from a function body.
    pub(crate) return_flag: Cell<bool>,
    /// Set by `break` inside a loop body.
    pub(crate) break_flag: Cell<bool>,
    /// Set by `continue` inside a loop body.
    pub(crate) continue_flag: Cell<bool>,
    /// Stack of local variable scopes for function calls.
    /// Each scope maps variable names to their saved values
    /// (None = was unset).
    pub(crate) local_scopes: RefCell<Vec<HashMap<String, Option<String>>>>,
    /// Nesting depth of [`CommandRegistry::execute`] calls (0 = idle).
    /// Only depth-0 (interactive) lines are recorded in the history.
    pub(crate) exec_depth: Cell<usize>,
    /// Background jobs queued with a trailing `&`.
    pub(crate) jobs: RefCell<JobManager>,
    /// Nesting depth of `run <script>` calls.
    pub(crate) script_depth: Cell<usize>,
    /// Simple commands run since the current top-level line started (see
    /// [`MAX_COMMANDS_PER_LINE`]).
    pub(crate) commands_run: Cell<u64>,
    /// Aliases being expanded by [`CommandRegistry::operator_alias`] (an
    /// alias does not expand again inside its own text).
    pub(crate) alias_stack: RefCell<Vec<String>>,
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
            exec_depth: Cell::new(0),
            jobs: RefCell::new(JobManager::new()),
            script_depth: Cell::new(0),
            commands_run: Cell::new(0),
            alias_stack: RefCell::new(Vec::new()),
        }
    }

    /// Register a command. Replaces any existing command with the same
    /// name.
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        self.commands.insert(cmd.name().to_string(), cmd);
    }

    /// Remove the command `name` (a plugin being unloaded takes its
    /// commands with it). Returns it, or `None` if it was not registered.
    pub fn unregister(&mut self, name: &str) -> Option<Box<dyn Command>> {
        self.commands.remove(name)
    }

    /// Names of the registered commands (unsorted; builtins, aliases and
    /// functions excluded).
    pub fn command_names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(String::as_str)
    }

    // -- Shell variable API --

    /// Set a shell variable.
    pub fn set_variable(&self, name: &str, value: &str) {
        self.variables
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
    }

    /// Set a variable on behalf of the user (`set`, `local`, `for`):
    /// like [`set_variable`](Self::set_variable) but refuses values over
    /// 64 KiB and new names beyond 4096 variables, so a script cannot grow
    /// the shell's memory without bound.
    pub(crate) fn assign_variable(&self, name: &str, value: &str) -> Result<()> {
        if !is_var_name(name) {
            return Err(OasisError::Command(
                format!("invalid variable name: '{name}'").into(),
            ));
        }
        if value.len() > MAX_VAR_LEN {
            return Err(OasisError::Command(
                format!(
                    "{name}: value too long ({} bytes, max {MAX_VAR_LEN})",
                    value.len()
                )
                .into(),
            ));
        }
        let vars = self.variables.borrow();
        if vars.len() >= MAX_VARIABLES && !vars.contains_key(name) {
            return Err(OasisError::Command(
                format!("{name}: too many variables (max {MAX_VARIABLES})").into(),
            ));
        }
        drop(vars);
        self.set_variable(name, value);
        Ok(())
    }

    /// Start a fresh per-line command budget if no command is running
    /// (every host entry point -- `execute`, scripts, jobs, inline control
    /// flow -- calls this, so the budget never carries over between lines).
    pub(crate) fn begin_top_level(&self) {
        if self.exec_depth.get() == 0 {
            self.commands_run.set(0);
        }
    }

    /// Count one simple command against the per-line budget; errors once
    /// [`MAX_COMMANDS_PER_LINE`] is exceeded.
    pub(crate) fn charge_command(&self) -> Result<()> {
        let n = self.commands_run.get().saturating_add(1);
        self.commands_run.set(n);
        if n > MAX_COMMANDS_PER_LINE {
            return Err(OasisError::Command(
                format!("command limit exceeded ({MAX_COMMANDS_PER_LINE} commands in one line)")
                    .into(),
            ));
        }
        Ok(())
    }

    /// Whether the current line has used up its command budget (loops stop
    /// iterating once it has).
    pub(crate) fn over_budget(&self) -> bool {
        self.commands_run.get() > MAX_COMMANDS_PER_LINE
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

    // -- Control flow flag accessors --

    /// Get the last command exit code.
    pub fn last_exit_code(&self) -> i32 {
        self.last_exit_code.get()
    }

    /// Whether the break flag is set.
    pub fn break_flag(&self) -> bool {
        self.break_flag.get()
    }

    /// Clear the break flag.
    pub fn clear_break(&self) {
        self.break_flag.set(false);
    }

    /// Whether the continue flag is set.
    pub fn continue_flag(&self) -> bool {
        self.continue_flag.get()
    }

    /// Clear the continue flag.
    pub fn clear_continue(&self) {
        self.continue_flag.set(false);
    }

    /// Whether the return flag is set.
    pub fn return_flag(&self) -> bool {
        self.return_flag.get()
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
    pub(crate) fn define_function(&self, name: &str, body: &str) {
        self.functions.borrow_mut().insert(
            name.to_string(),
            ShellFunction {
                body: body.to_string(),
            },
        );
    }

    /// List all defined functions.
    pub(crate) fn list_functions(&self) -> Vec<(String, String)> {
        self.functions
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.body.clone()))
            .collect()
    }

    /// Call a shell function by name with positional arguments.
    pub(crate) fn call_function(
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
            .ok_or_else(|| OasisError::Command(format!("unknown function: {name}").into()))?;

        // Check recursion depth.
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(OasisError::Command(
                format!(
                    "{name}: maximum recursion depth ({MAX_CALL_DEPTH}) \
                 exceeded"
                )
                .into(),
            ));
        }
        self.call_depth.set(depth + 1);

        let result = self.with_positional_args(name, args, || {
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
            result
        });

        // Restore call depth.
        self.call_depth.set(depth);

        result
    }

    /// Run `f` with the positional parameters `$0` = `name`, `$1`..`$n` =
    /// `args` and `$#` = n, restoring the caller's parameters afterwards
    /// (functions and `run <script> [args...]`).
    pub(crate) fn with_positional_args<R>(
        &self,
        name: &str,
        args: &[&str],
        f: impl FnOnce() -> R,
    ) -> R {
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

        // Unset any prior positional args beyond the new arg count.
        self.set_variable("0", name);
        for (i, arg) in args.iter().enumerate() {
            self.set_variable(&(i + 1).to_string(), arg);
        }
        for i in (args.len() + 1)..=prior_count {
            self.unset_variable(&i.to_string());
        }
        self.set_variable("#", &args.len().to_string());

        let result = f();

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
        result
    }

    /// Run `f` detached from the interactive session, as a background job
    /// (`cmd &`) or a host-scheduled script (cron) runs: like a POSIX
    /// subshell, its `$?`, `cd`s (`env.cwd`, `$OLDPWD`, `$CWD`) do not
    /// reach the session, whose values are restored afterwards. Other
    /// variables it sets stay set (the shell has no forked environment).
    pub fn run_detached<R>(
        &self,
        env: &mut Environment<'_>,
        f: impl FnOnce(&Self, &mut Environment<'_>) -> R,
    ) -> R {
        const SESSION_VARS: [&str; 3] = ["?", "OLDPWD", "CWD"];
        let code = self.last_exit_code.get();
        let vars = SESSION_VARS.map(|name| self.get_variable(name));
        let cwd = env.cwd.clone();
        let result = f(self, env);
        self.last_exit_code.set(code);
        for (name, saved) in SESSION_VARS.into_iter().zip(vars) {
            match saved {
                Some(v) => self.set_variable(name, &v),
                None => self.unset_variable(name),
            }
        }
        env.cwd = cwd;
        result
    }

    // -- History API --

    /// Get command history.
    pub fn history(&self) -> Vec<String> {
        self.history.borrow().clone()
    }

    /// Replace the command history (e.g. with entries loaded from disk).
    ///
    /// Blank entries are dropped and at most the newest 500 (`MAX_HISTORY`)
    /// entries are kept.
    pub fn set_history(&self, entries: Vec<String>) {
        let mut entries: Vec<String> = entries
            .into_iter()
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .collect();
        if entries.len() > MAX_HISTORY {
            entries.drain(..entries.len() - MAX_HISTORY);
        }
        *self.history.borrow_mut() = entries;
    }

    /// Visit the history (oldest first) without cloning it.
    pub fn with_history<R>(&self, f: impl FnOnce(&[String]) -> R) -> R {
        f(&self.history.borrow())
    }

    /// Names of all registered commands, shell builtins, aliases and
    /// functions (unsorted; used for tab completion).
    pub fn completion_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.commands.keys().cloned().collect();
        names.extend(crate::builtins::BUILTINS.iter().map(|b| b.name.to_string()));
        names.extend(self.aliases.borrow().keys().cloned());
        names.extend(self.functions.borrow().keys().cloned());
        names
    }

    /// Push a command to history.
    pub(crate) fn push_history(&self, line: &str) {
        let mut hist = self.history.borrow_mut();
        // Don't duplicate the last entry.
        if hist.last().is_none_or(|last| last != line) {
            hist.push(line.to_string());
            if hist.len() > MAX_HISTORY {
                hist.remove(0);
            }
        }
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

//! [`CommandRegistry`] struct and shell state management.
//!
//! Contains the registry definition, variable/alias/function APIs,
//! history management, and expansion helpers (variables, aliases,
//! history).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use oasis_types::error::{OasisError, Result};

use crate::types::{
    Command, CommandOutput, Environment, MAX_CALL_DEPTH, MAX_HISTORY, ShellFunction,
};

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

    /// Register a command. Replaces any existing command with the same
    /// name.
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
            .ok_or_else(|| OasisError::Command(format!("unknown function: {name}")))?;

        // Check recursion depth.
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(OasisError::Command(format!(
                "{name}: maximum recursion depth ({MAX_CALL_DEPTH}) \
                 exceeded"
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

    // -- History expansion --

    pub(crate) fn expand_history(&self, input: &str) -> Result<String> {
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

    pub(crate) fn expand_variables(&self, input: &str, cwd: &str) -> String {
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

    pub(crate) fn expand_alias(&self, mut tokens: Vec<String>) -> Vec<String> {
        if tokens.is_empty() {
            return tokens;
        }
        let aliases = self.aliases.borrow();
        if let Some(expansion) = aliases.get(&tokens[0]) {
            // Replace the first token with the alias expansion.
            let expanded_tokens = match crate::expander::tokenize(expansion) {
                Ok(t) => t,
                Err(_) => return tokens,
            };
            tokens.splice(0..1, expanded_tokens);
        }
        tokens
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

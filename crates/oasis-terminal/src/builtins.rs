//! Built-in shell commands implemented directly on [`CommandRegistry`].
//!
//! These are commands that need direct access to registry internals
//! (variables, aliases, history, functions, jobs) and so cannot be
//! implemented as standalone [`Command`](crate::Command) trait objects.
//!
//! [`BUILTINS`] is the single table the executor dispatches through and
//! that `help`, `which` and tab completion read, so a builtin is added in
//! exactly one place.

use std::collections::HashMap;

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

/// Signature of a builtin's implementation.
pub(crate) type BuiltinFn =
    fn(&CommandRegistry, &[&str], &mut Environment<'_>) -> Result<CommandOutput>;

/// One shell builtin: its `help` metadata and its implementation.
pub(crate) struct BuiltinSpec {
    /// What the user types.
    pub(crate) name: &'static str,
    /// `help` listing category.
    pub(crate) category: &'static str,
    /// Usage line for `help <name>`.
    pub(crate) usage: &'static str,
    /// One-line description.
    pub(crate) description: &'static str,
    /// Implementation.
    pub(crate) run: BuiltinFn,
}

/// Every shell builtin, intercepted by the executor before the command
/// map (so a registered command with the same name could never run).
pub(crate) static BUILTINS: &[BuiltinSpec] = &[
    BuiltinSpec {
        name: "help",
        category: "general",
        usage: "help [--category <cat>] [command]",
        description: "List commands or describe one",
        run: |reg, args, _| reg.execute_help(args),
    },
    BuiltinSpec {
        name: "which",
        category: "general",
        usage: "which <command>",
        description: "Say whether a name is a builtin, command, alias or function",
        run: |reg, args, _| reg.execute_which(args),
    },
    BuiltinSpec {
        name: "history",
        category: "general",
        usage: "history [clear]",
        description: "Show or clear the command history",
        run: |reg, args, _| reg.execute_history_cmd(args),
    },
    BuiltinSpec {
        name: "set",
        category: "config",
        usage: "set [VAR=value | VAR]",
        description: "Set a shell variable, or show one / all",
        run: |reg, args, _| reg.execute_set(args),
    },
    BuiltinSpec {
        name: "unset",
        category: "config",
        usage: "unset <VAR>...",
        description: "Remove shell variables",
        run: |reg, args, _| reg.execute_unset(args),
    },
    BuiltinSpec {
        name: "env",
        category: "config",
        usage: "env",
        description: "List all shell variables",
        run: |reg, _, _| reg.execute_env(),
    },
    BuiltinSpec {
        name: "alias",
        category: "config",
        usage: "alias [name=command | name]",
        description: "Define, show or list aliases",
        run: |reg, args, _| reg.execute_alias(args),
    },
    BuiltinSpec {
        name: "unalias",
        category: "config",
        usage: "unalias <name>...",
        description: "Remove aliases",
        run: |reg, args, _| reg.execute_unalias(args),
    },
    BuiltinSpec {
        name: "run",
        category: "scripting",
        usage: "run <script> [args...]",
        description: "Execute a script file (control flow, $1..$9)",
        run: |reg, args, env| reg.execute_run(args, env),
    },
    BuiltinSpec {
        name: "xargs",
        category: "scripting",
        usage: "xargs [-n N] [-I REPL] [command [args...]]",
        description: "Run a command with arguments read from stdin",
        run: |reg, args, env| reg.execute_xargs(args, env),
    },
    BuiltinSpec {
        name: "function",
        category: "scripting",
        usage: "function name() { body }",
        description: "Define a shell function, or list them",
        // Normally intercepted before expansion so the body is stored
        // literally; this path only runs when reached through an alias.
        run: |reg, args, _| reg.execute_function_def_raw(&args.join(" ")),
    },
    BuiltinSpec {
        name: "return",
        category: "scripting",
        usage: "return [code]",
        description: "Return from a shell function",
        run: |reg, args, _| reg.execute_return(args),
    },
    BuiltinSpec {
        name: "local",
        category: "scripting",
        usage: "local NAME[=VALUE]...",
        description: "Declare function-local variables",
        run: |reg, args, _| reg.execute_local(args),
    },
    BuiltinSpec {
        name: "break",
        category: "scripting",
        usage: "break",
        description: "Leave the enclosing loop",
        run: |reg, _, _| reg.execute_break(),
    },
    BuiltinSpec {
        name: "continue",
        category: "scripting",
        usage: "continue",
        description: "Skip to the next loop iteration",
        run: |reg, _, _| reg.execute_continue(),
    },
    BuiltinSpec {
        name: "true",
        category: "scripting",
        usage: "true",
        description: "Do nothing, successfully",
        run: |_, _, _| Ok(CommandOutput::None),
    },
    BuiltinSpec {
        name: "false",
        category: "scripting",
        usage: "false",
        description: "Do nothing, with exit status 1",
        run: |_, _, _| Ok(CommandOutput::status(1, CommandOutput::None)),
    },
    BuiltinSpec {
        name: "jobs",
        category: "jobs",
        usage: "jobs",
        description: "List background jobs",
        run: |reg, _, _| reg.execute_jobs(),
    },
    BuiltinSpec {
        name: "fg",
        category: "jobs",
        usage: "fg [%job]",
        description: "Run a job in the foreground",
        run: |reg, args, env| reg.execute_fg(args, env),
    },
    BuiltinSpec {
        name: "bg",
        category: "jobs",
        usage: "bg [%job]",
        description: "Resume a stopped job in the background",
        run: |reg, args, _| reg.execute_bg(args),
    },
    BuiltinSpec {
        name: "kill",
        category: "jobs",
        usage: "kill [-STOP|-CONT|-TERM] <%job|job-number>...",
        description: "Signal background jobs",
        run: |reg, args, _| reg.execute_kill(args),
    },
];

/// Look up a builtin by (lower-case) name.
pub(crate) fn builtin(name: &str) -> Option<&'static BuiltinSpec> {
    BUILTINS.iter().find(|b| b.name == name)
}

impl CommandRegistry {
    /// Built-in help with access to the registry.
    pub(crate) fn execute_help(&self, args: &[&str]) -> Result<CommandOutput> {
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
                None => match builtin(&name_lower) {
                    Some(b) => Ok(CommandOutput::Text(format!(
                        "{} ({}, shell built-in)\n  {}\n  Usage: {}",
                        b.name, b.category, b.description, b.usage
                    ))),
                    None => Err(OasisError::Command(
                        format!("unknown command: {name}").into(),
                    )),
                },
            }
        } else {
            // Group commands by category.
            let mut categories: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
            // Include intercepted commands.
            for b in BUILTINS {
                categories
                    .entry(b.category)
                    .or_default()
                    .push((b.name, b.description));
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
                    return Err(OasisError::Command(
                        format!("no category matching '{fc}'").into(),
                    ));
                }
            }

            let total: usize = cats
                .iter()
                .filter_map(|c| categories.get(c))
                .map(|v| v.len())
                .sum();
            let mut out = format!("Commands ({total}):\n");
            for cat in &cats {
                let Some(cmds) = categories.get(cat) else {
                    continue;
                };
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
    pub(crate) fn execute_which(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: which <command>".into()));
        }
        let name = args[0].to_ascii_lowercase();
        // Check intercepted commands first.
        if builtin(&name).is_some() {
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
                        Err(OasisError::Command(format!("{name}: not found").into()))
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
    pub(crate) fn execute_function_def_raw(&self, raw: &str) -> Result<CommandOutput> {
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
                    "usage: function name() { body }".into(),
                ));
            },
        };

        if name.is_empty() {
            return Err(OasisError::Command("function name cannot be empty".into()));
        }

        // Extract body between { and }.
        let rest = rest
            .strip_prefix('{')
            .ok_or_else(|| OasisError::Command("expected '{' after function name".into()))?;
        let body = rest
            .strip_suffix('}')
            .ok_or_else(|| OasisError::Command("expected '}' at end of function".into()))?;
        let body = body.trim();

        if body.is_empty() {
            return Err(OasisError::Command("function body cannot be empty".into()));
        }

        self.define_function(&name.to_ascii_lowercase(), body);
        Ok(CommandOutput::None)
    }

    /// Built-in `xargs`: run `command args... ITEMS` with the items read
    /// from stdin (whitespace-separated; one per line with `-I`).
    ///
    /// `-n N` runs the command once per N items; `-I REPL` runs it once
    /// per input line with every `REPL` in the arguments replaced by the
    /// line. The command defaults to `echo`. Items are passed as words,
    /// never re-parsed as shell syntax. Exits 123 if any run failed.
    pub(crate) fn execute_xargs(
        &self,
        args: &[&str],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let usage = || OasisError::Command("usage: xargs [-n N] [-I REPL] [command...]".into());
        let (mut per_run, mut replace) = (usize::MAX, None);
        let mut rest = args;
        loop {
            match rest {
                ["-n", n, tail @ ..] => {
                    per_run = n.parse().ok().filter(|&n| n > 0).ok_or_else(usage)?;
                    rest = tail;
                },
                ["-I", r, tail @ ..] => {
                    replace = Some(*r);
                    rest = tail;
                },
                [flag, ..] if flag.starts_with('-') && *flag != "-" => return Err(usage()),
                _ => break,
            }
        }
        let command: Vec<&str> = if rest.is_empty() {
            vec!["echo"]
        } else {
            rest.to_vec()
        };

        let input = env.stdin.take().unwrap_or_default();
        let runs: Vec<Vec<String>> = match replace {
            Some(r) => input
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|line| command.iter().map(|a| a.replace(r, line)).collect())
                .collect(),
            None => {
                let items: Vec<&str> = input.split_whitespace().collect();
                items
                    .chunks(per_run.min(items.len().max(1)))
                    .map(|chunk| {
                        let mut words: Vec<String> =
                            command.iter().map(|s| (*s).to_string()).collect();
                        words.extend(chunk.iter().map(|s| (*s).to_string()));
                        words
                    })
                    .collect()
            },
        };

        let mut outputs = Vec::new();
        let mut failed = false;
        for words in &runs {
            self.charge_command()?;
            self.last_exit_code.set(0);
            match self.run_words(words, env) {
                Ok(out) => {
                    failed |= self.last_exit_code.get() != 0;
                    outputs.push(out);
                },
                Err(e) => {
                    failed = true;
                    outputs.push(CommandOutput::Text(format!("xargs: {}: {e}", words[0])));
                },
            }
            env.stdin = None;
        }
        let output = crate::executor::merge_outputs(outputs);
        Ok(CommandOutput::status(if failed { 123 } else { 0 }, output))
    }

    /// Built-in `return` command: set exit code from within a function.
    pub(crate) fn execute_return(&self, args: &[&str]) -> Result<CommandOutput> {
        if self.call_depth.get() == 0 {
            return Err(OasisError::Command(
                "return: can only be used inside a function".into(),
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
    pub(crate) fn execute_break(&self) -> Result<CommandOutput> {
        self.break_flag.set(true);
        Ok(CommandOutput::None)
    }

    /// Built-in `continue` command — sets the continue flag for the enclosing loop.
    pub(crate) fn execute_continue(&self) -> Result<CommandOutput> {
        self.continue_flag.set(true);
        Ok(CommandOutput::None)
    }

    /// Built-in `local` command — declares a local variable in the current function scope.
    ///
    /// Syntax: `local NAME=VALUE` or `local NAME`
    pub(crate) fn execute_local(&self, args: &[&str]) -> Result<CommandOutput> {
        let scopes = self.local_scopes.borrow();
        if scopes.is_empty() {
            return Err(OasisError::Command(
                "local: can only be used inside a function".into(),
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
            let mut scopes_mut = self.local_scopes.borrow_mut();
            let Some(top) = scopes_mut.last_mut() else {
                return Err(OasisError::Command(
                    "local: scope unexpectedly empty".into(),
                ));
            };
            top.entry(name.to_string()).or_insert(old);
            // Set the new value.
            if let Some(v) = value {
                self.assign_variable(name, v)?;
            }
        }
        Ok(CommandOutput::None)
    }

    /// Built-in `history` command.
    pub(crate) fn execute_history_cmd(&self, args: &[&str]) -> Result<CommandOutput> {
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
    pub(crate) fn execute_set(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            // Show all variables.
            return self.execute_env();
        }
        let assignment = args.join(" ");
        if let Some((name, value)) = assignment.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                return Err(OasisError::Command("usage: set VAR=value".into()));
            }
            self.assign_variable(name, value)?;
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
    pub(crate) fn execute_unset(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: unset <VAR>".into()));
        }
        for name in args {
            self.unset_variable(name);
        }
        Ok(CommandOutput::None)
    }

    /// Built-in `env` command: list all variables.
    pub(crate) fn execute_env(&self) -> Result<CommandOutput> {
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
    pub(crate) fn execute_alias(&self, args: &[&str]) -> Result<CommandOutput> {
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
            // Strip one pair of enclosing quotes only: `alias g='echo "hi"'`
            // reaches here as `g=echo "hi"`, whose inner quotes must stay.
            let value = value.trim();
            let value = ['\'', '"']
                .iter()
                .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
                .unwrap_or(value);
            if name.is_empty() {
                return Err(OasisError::Command("usage: alias <name>=<command>".into()));
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
    pub(crate) fn execute_unalias(&self, args: &[&str]) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: unalias <name>".into()));
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

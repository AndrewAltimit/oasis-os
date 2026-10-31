//! The script executor: `run`, [`CommandRegistry::run_script_source`],
//! compound one-liners, and the tree walker behind them.

use oasis_types::error::{OasisError, Result};

use super::{Node, parse_script, split_statements};
use crate::expander::{case_glob, expand_words, glob_match, resolve_path, split_case_alternatives};
use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

/// Maximum iterations for a single `while` / `until` loop.
const MAX_LOOP_ITERATIONS: usize = 1000;

/// Maximum text a single script / compound line collects before further
/// output is dropped (with one note).
const MAX_SCRIPT_OUTPUT: usize = 4 * 1024 * 1024;

/// Per-run execution state: collected outputs plus error formatting mode.
struct ExecCtx {
    outputs: Vec<CommandOutput>,
    /// Script-file mode reports errors as `error at line N: ...`; inline
    /// mode as `error: ...`.
    script: bool,
    /// Text bytes collected so far (see [`MAX_SCRIPT_OUTPUT`]).
    output_bytes: usize,
    /// The command-budget error has been reported.
    budget_reported: bool,
}

impl ExecCtx {
    fn new(script: bool) -> Self {
        Self {
            outputs: Vec::new(),
            script,
            output_bytes: 0,
            budget_reported: false,
        }
    }

    /// Collect one statement's output, up to [`MAX_SCRIPT_OUTPUT`].
    fn push(&mut self, output: CommandOutput) {
        if self.output_bytes > MAX_SCRIPT_OUTPUT {
            return;
        }
        self.output_bytes += crate::pipeline::output_to_text(&output).len();
        if self.output_bytes > MAX_SCRIPT_OUTPUT {
            self.outputs.push(CommandOutput::Text(format!(
                "warning: output truncated (over {} MiB)",
                MAX_SCRIPT_OUTPUT >> 20
            )));
            return;
        }
        self.outputs.push(output);
    }
}

/// Flatten command outputs into display lines (script-file mode).
fn outputs_to_lines(outputs: Vec<CommandOutput>, lines: &mut Vec<String>) {
    for output in outputs {
        match output {
            CommandOutput::Text(text) => lines.extend(text.lines().map(str::to_string)),
            CommandOutput::Table { headers, rows } => {
                lines.push(headers.join(" | "));
                lines.extend(rows.iter().map(|r| r.join(" | ")));
            },
            CommandOutput::Clear => lines.push("(clear)".to_string()),
            CommandOutput::None => {},
            CommandOutput::Multi(inner) => outputs_to_lines(inner, lines),
            CommandOutput::Status { output, .. } => outputs_to_lines(vec![*output], lines),
            CommandOutput::Signal(_) => {
                lines.push("(signal command skipped in script)".to_string());
            },
        }
    }
}

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

        let script_depth = self.script_depth.get();
        if script_depth >= crate::types::MAX_SCRIPT_DEPTH {
            return Err(OasisError::Command(
                format!(
                    "run {full_path}: maximum script nesting depth ({}) exceeded",
                    crate::types::MAX_SCRIPT_DEPTH
                )
                .into(),
            ));
        }

        if !env.vfs.exists(&full_path) {
            return Err(OasisError::Command(
                format!("script not found: {full_path}").into(),
            ));
        }

        let data = env.vfs.read(&full_path)?;
        let source = String::from_utf8_lossy(&data);
        let count = split_statements(&source).len();
        if count == 0 {
            return Ok(CommandOutput::Text("(empty script)".to_string()));
        }
        self.script_depth.set(script_depth + 1);
        // `$1..$n` are the arguments after the script path (as advertised
        // by `run <script> [args...]`; they used to be dropped).
        let lines = self.with_positional_args(&full_path, &args[1..], || {
            self.run_script_source(&source, env)
        });
        self.script_depth.set(script_depth);
        let lines = lines?;
        if lines.is_empty() {
            Ok(CommandOutput::Text(format!(
                "Script {full_path}: {count} commands executed."
            )))
        } else {
            Ok(CommandOutput::Text(lines.join("\n")))
        }
    }

    /// Run shell script source and return its output as display lines.
    ///
    /// Supports the full control-flow syntax. A syntax error aborts the
    /// whole script (`Err`); a failing command is reported as
    /// `error at line N: ...` and execution continues. Signals (skin swap,
    /// network, ...) are skipped with a note.
    pub fn run_script_source(
        &self,
        source: &str,
        env: &mut Environment<'_>,
    ) -> Result<Vec<String>> {
        let nodes = parse_script(split_statements(source))?;
        let mut ctx = ExecCtx::new(true);
        self.begin_top_level();
        let depth = self.exec_depth.get();
        self.exec_depth.set(depth + 1);
        self.run_nodes(&nodes, env, &mut ctx);
        self.exec_depth.set(depth);
        self.clear_loop_flags();
        self.return_flag.set(false);

        let mut lines = Vec::new();
        outputs_to_lines(ctx.outputs, &mut lines);
        Ok(lines)
    }

    /// Execute an interactive line containing compound commands.
    ///
    /// Syntax errors are returned as `Err`; runtime errors of individual
    /// commands are reported inline (`error: ...`) and execution continues,
    /// matching the chain executor's behaviour for multi-command lines.
    pub(crate) fn execute_compound(
        &self,
        line: &str,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let nodes = parse_script(split_statements(line))?;
        let mut ctx = ExecCtx::new(false);
        self.run_nodes(&nodes, env, &mut ctx);
        Ok(crate::executor::merge_outputs(ctx.outputs))
    }

    fn clear_loop_flags(&self) {
        self.break_flag.set(false);
        self.continue_flag.set(false);
    }

    fn run_nodes(&self, nodes: &[Node], env: &mut Environment<'_>, ctx: &mut ExecCtx) {
        for node in nodes {
            if self.return_flag.get() || self.break_flag.get() || self.continue_flag.get() {
                return;
            }
            if self.over_budget() {
                // Report the exhausted budget once, then unwind.
                if !ctx.budget_reported {
                    ctx.budget_reported = true;
                    let e = self.charge_command().err();
                    if let Some(e) = e {
                        self.report_error(&e, 0, ctx);
                    }
                }
                return;
            }
            self.run_node(node, env, ctx);
        }
    }

    /// Record a failed statement: exit status 1 and an inline error line
    /// (`error at line N: ...` in script files, `error: ...` inline).
    fn report_error(&self, e: &OasisError, line: usize, ctx: &mut ExecCtx) {
        self.last_exit_code.set(1);
        self.set_variable("?", "1");
        let msg = if ctx.script && line > 0 {
            format!("error at line {line}: {e}")
        } else {
            format!("error: {e}")
        };
        ctx.push(CommandOutput::Text(msg));
    }

    fn run_node(&self, node: &Node, env: &mut Environment<'_>, ctx: &mut ExecCtx) {
        if let Node::Simple { text, line } = node {
            match self.execute_chain(text, env) {
                Ok(CommandOutput::None) => {},
                Ok(output) => ctx.push(output),
                Err(e) => self.report_error(&e, *line, ctx),
            }
            return;
        }
        // A block recurses on the host stack like a nested `execute`, so it
        // counts against the same nesting limit.
        let depth = self.exec_depth.get();
        if depth >= crate::types::MAX_EXEC_DEPTH {
            let e = OasisError::Command(
                format!(
                    "maximum nesting depth ({}) exceeded",
                    crate::types::MAX_EXEC_DEPTH
                )
                .into(),
            );
            return self.report_error(&e, 0, ctx);
        }
        self.exec_depth.set(depth + 1);
        self.run_block(node, env, ctx);
        self.exec_depth.set(depth);
    }

    /// Run an `if` / loop / `case` node (see [`run_node`](Self::run_node)).
    fn run_block(&self, node: &Node, env: &mut Environment<'_>, ctx: &mut ExecCtx) {
        match node {
            Node::Simple { .. } => {},
            Node::If {
                branches,
                else_body,
            } => {
                for (cond, body) in branches {
                    if self.eval_condition(cond, env, ctx) {
                        self.run_nodes(body, env, ctx);
                        return;
                    }
                }
                match else_body {
                    Some(body) => self.run_nodes(body, env, ctx),
                    // No branch ran: the `if` itself succeeds.
                    None => self.set_status(0),
                }
            },
            Node::Loop { cond, until, body } => {
                let mut iterations = 0;
                // A loop whose body never runs exits 0, not with the
                // status of its final (failed) condition.
                let mut body_status = 0;
                loop {
                    if self.over_budget() {
                        break;
                    }
                    if iterations >= MAX_LOOP_ITERATIONS {
                        ctx.push(CommandOutput::Text(format!(
                            "warning: loop terminated after {MAX_LOOP_ITERATIONS} \
                             iterations (limit reached)"
                        )));
                        break;
                    }
                    iterations += 1;
                    if self.eval_condition(cond, env, ctx) == *until {
                        break;
                    }
                    let stop = self.run_loop_body(body, env, ctx);
                    body_status = self.last_exit_code.get();
                    if stop {
                        break;
                    }
                }
                if !self.return_flag.get() {
                    self.set_status(body_status);
                }
            },
            Node::For { var, words, body } => {
                let words = match self.expand_words(words, env) {
                    Ok(words) => words,
                    Err(e) => return self.report_error(&e, 0, ctx),
                };
                // Zero iterations exit 0; otherwise the last body command's.
                self.set_status(0);
                for word in words {
                    if let Err(e) = self.assign_variable(var, &word) {
                        return self.report_error(&e, 0, ctx);
                    }
                    if self.over_budget() || self.run_loop_body(body, env, ctx) {
                        break;
                    }
                }
            },
            Node::Case { word, arms } => {
                let value = match self.expand_case_word(word, env) {
                    Ok(value) => value,
                    Err(e) => return self.report_error(&e, 0, ctx),
                };
                // No matching arm (or an empty one) exits 0.
                self.set_status(0);
                for (pattern, body) in arms {
                    match self.case_arm_matches(&value, pattern, env) {
                        Ok(true) => {
                            self.run_nodes(body, env, ctx);
                            break;
                        },
                        Ok(false) => {},
                        Err(e) => return self.report_error(&e, 0, ctx),
                    }
                }
            },
            Node::Redirected { node, suffix, line } => {
                self.run_redirected(node, suffix, *line, env, ctx);
            },
        }
    }

    /// Whether `value` matches one of the `|` alternatives of a `case` arm
    /// pattern. Each alternative is expanded like a word (quotes removed,
    /// `$VAR` / `$(...)` substituted, no globbing); its quoted parts match
    /// literally, unquoted `*` / `?` / `[...]` as globs.
    fn case_arm_matches(
        &self,
        value: &str,
        pattern: &str,
        env: &mut Environment<'_>,
    ) -> Result<bool> {
        for alt in split_case_alternatives(pattern) {
            let words = self.expand_words_of(alt, env)?;
            if glob_match(&case_glob(&words), value) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Run a compound command followed by `| cmd`, `> file`, `>> file`
    /// and/or `&& cmd` / `|| cmd` (`for ...; done | sort > out`): its
    /// output goes to the pipe or file, then the and-or list runs on its
    /// exit status.
    fn run_redirected(
        &self,
        node: &Node,
        suffix: &str,
        line: usize,
        env: &mut Environment<'_>,
        ctx: &mut ExecCtx,
    ) {
        let mut inner = ExecCtx::new(ctx.script);
        self.run_node(node, env, &mut inner);
        let output = crate::executor::merge_outputs(inner.outputs);
        if let Err(e) = self.finish_redirected(output, suffix, env, ctx) {
            self.report_error(&e, line, ctx);
        }
    }

    fn finish_redirected(
        &self,
        output: CommandOutput,
        suffix: &str,
        env: &mut Environment<'_>,
        ctx: &mut ExecCtx,
    ) -> Result<()> {
        let suffix = suffix.trim();
        // `| rest`: the rest of the line is a pipeline (and-or list) whose
        // first stage reads the compound's output.
        if let Some(rest) = suffix.strip_prefix('|').filter(|r| !r.starts_with('|')) {
            env.stdin = Some(crate::pipeline::capture_text(&output));
            let result = self.execute_chain(rest, env);
            env.stdin = None;
            match result? {
                CommandOutput::None => {},
                out => ctx.push(out),
            }
            return Ok(());
        }
        // Leading redirections, up to the first `&&` / `||`.
        let (redirs, chain) = match crate::pipeline::find_and_or(suffix) {
            Some(i) => (&suffix[..i], &suffix[i..]),
            None => (suffix, ""),
        };
        let (cmd, redirections) = crate::pipeline::parse_redirect(redirs);
        if !cmd.trim().is_empty()
            || redirections.stdin.is_some()
            || redirections.unsupported.is_some()
            || redirections.stdout == crate::pipeline::StreamDest::Stderr
            || (redirections.stderr_file().is_some() && redirections.stderr != redirections.stdout)
        {
            return Err(OasisError::Command(
                format!("syntax error: unsupported text after compound command: {redirs}").into(),
            ));
        }
        match redirections.stdout_file() {
            Some(r) => {
                let path = self.redirect_target(r.path, env)?;
                let text = crate::pipeline::capture_text(&output);
                crate::pipeline::write_redirect(&text, &path, r.append, &env.cwd, env.vfs)?;
            },
            None => ctx.push(output),
        }
        let chain = chain.trim();
        if chain.is_empty() {
            return Ok(());
        }
        let (run, rest) = match (chain.strip_prefix("&&"), chain.strip_prefix("||")) {
            (Some(rest), _) => (self.last_exit_code.get() == 0, rest),
            (None, Some(rest)) => (self.last_exit_code.get() != 0, rest),
            (None, None) => return Ok(()),
        };
        if run {
            match self.execute_chain(rest, env)? {
                CommandOutput::None => {},
                out => ctx.push(out),
            }
        }
        Ok(())
    }

    /// Run one loop iteration. Returns `true` when the loop must stop
    /// (`break` or `return`).
    fn run_loop_body(&self, body: &[Node], env: &mut Environment<'_>, ctx: &mut ExecCtx) -> bool {
        self.run_nodes(body, env, ctx);
        self.continue_flag.set(false);
        if self.break_flag.get() {
            self.break_flag.set(false);
            return true;
        }
        self.return_flag.get()
    }

    /// Expand a word list the way the shell expands command arguments:
    /// `$(...)`, variables, quote removal, braces and globs.
    fn expand_words(&self, words: &str, env: &mut Environment<'_>) -> Result<Vec<String>> {
        let words = self.expand_words_of(words, env)?;
        Ok(expand_words(words, env.vfs, &env.cwd))
    }

    /// Evaluate a condition list for `if` / `elif` / `while` / `until`.
    ///
    /// The statements run like any others (their output is shown); the
    /// exit status of the last one decides: 0 is true. `! cmd` negates,
    /// `a && b` / `a || b` combine, as everywhere else.
    fn eval_condition(&self, cond: &[Node], env: &mut Environment<'_>, ctx: &mut ExecCtx) -> bool {
        self.run_nodes(cond, env, ctx);
        self.last_exit_code.get() == 0
    }
}

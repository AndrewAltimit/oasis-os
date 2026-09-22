//! Shell script parser and executor for [`CommandRegistry`].
//!
//! This is the single control-flow implementation used by both script
//! files (`run <path>`) and interactive one-liners such as
//! `if test -f a; then echo yes; elif test -d a; then echo dir; fi`.
//!
//! Pipeline:
//!
//! 1. [`split_statements`] turns source text into logical statements. It
//!    splits on newlines and unquoted `;` (emitting `;;` as its own
//!    statement for `case`), and is aware of quotes, backslash escapes,
//!    `{ ... }` function bodies, `$( ... )` substitutions and `#` comments.
//! 2. [`parse_script`] builds a [`Node`] tree with arbitrarily nested
//!    `if`/`elif`/`else`/`fi`, `while`/`until`/`do`/`done`,
//!    `for`/`in`/`do`/`done` and `case`/`esac` blocks. Keywords may share a
//!    statement with the following command (`then echo yes`, `do body`,
//!    `else if ...`, `case $x in a) ...`).
//! 3. The executor walks the tree. Simple statements run through the full
//!    chain/pipeline executor, so `&&`, `||`, pipes and redirection work
//!    inside blocks.

use oasis_types::error::{OasisError, Result};

use crate::expander::{case_pattern_matches, expand_braces, expand_globs, resolve_path, tokenize};
use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

/// Maximum iterations for a single `while` / `until` loop.
const MAX_LOOP_ITERATIONS: usize = 1000;

// ---------------------------------------------------------------------------
// Statement splitting
// ---------------------------------------------------------------------------

/// One logical statement produced by [`split_statements`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stmt {
    /// Statement text (trimmed, without the separator).
    pub(crate) text: String,
    /// 1-based source line the statement starts on.
    pub(crate) line: usize,
}

/// Split shell source into logical statements.
///
/// Separators are newlines and unquoted `;`. A `;;` (case-arm terminator)
/// is emitted as its own `";;"` statement. Separators inside quotes,
/// `{ ... }` bodies and `$( ... )` substitutions are kept verbatim. A `#`
/// at the start of a word begins a comment that runs to end of line, and
/// a backslash-newline joins two physical lines.
pub(crate) fn split_statements(src: &str) -> Vec<Stmt> {
    fn flush(cur: &mut String, out: &mut Vec<Stmt>, line: usize) {
        let t = cur.trim();
        if !t.is_empty() {
            out.push(Stmt {
                text: t.to_string(),
                line,
            });
        }
        cur.clear();
    }

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut line = 1usize;
    let mut start_line = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut chars = src.chars().peekable();

    while let Some(c) = chars.next() {
        if cur.is_empty() {
            if c == ' ' || c == '\t' || c == '\r' {
                continue;
            }
            start_line = line;
        }
        if in_single {
            cur.push(c);
            match c {
                '\'' => in_single = false,
                '\n' => line += 1,
                _ => {},
            }
            continue;
        }
        if in_double {
            cur.push(c);
            match c {
                '"' => in_double = false,
                '\\' => {
                    if let Some(n) = chars.next() {
                        if n == '\n' {
                            line += 1;
                        }
                        cur.push(n);
                    }
                },
                '\n' => line += 1,
                _ => {},
            }
            continue;
        }
        let nested = brace_depth > 0 || paren_depth > 0;
        match c {
            '\'' => {
                in_single = true;
                cur.push(c);
            },
            '"' => {
                in_double = true;
                cur.push(c);
            },
            '\\' => match chars.next() {
                // Line continuation.
                Some('\n') => line += 1,
                Some(n) => {
                    cur.push(c);
                    cur.push(n);
                },
                None => cur.push(c),
            },
            '#' if !nested && cur.chars().last().is_none_or(char::is_whitespace) => {
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            },
            '{' => {
                brace_depth += 1;
                cur.push(c);
            },
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                cur.push(c);
            },
            '(' => {
                if paren_depth > 0 || cur.ends_with('$') {
                    paren_depth += 1;
                }
                cur.push(c);
            },
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                cur.push(c);
            },
            '\n' => {
                line += 1;
                if nested {
                    cur.push(c);
                } else {
                    flush(&mut cur, &mut out, start_line);
                }
            },
            ';' if !nested => {
                flush(&mut cur, &mut out, start_line);
                if chars.peek() == Some(&';') {
                    chars.next();
                    out.push(Stmt {
                        text: ";;".to_string(),
                        line,
                    });
                }
            },
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut out, start_line);
    out
}

/// First whitespace-delimited word of `s` (empty for blank input).
fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

/// Keywords that open a compound command.
const COMPOUND_KEYWORDS: [&str; 5] = ["if", "while", "until", "for", "case"];

/// Whether `line` contains a compound command (`if`, `while`, `until`,
/// `for`, `case`) at statement level and must go through the script
/// engine rather than the plain chain executor.
pub(crate) fn is_compound(line: &str) -> bool {
    // Cheap pre-filter: every compound keyword appears as a substring.
    if !COMPOUND_KEYWORDS.iter().any(|kw| line.contains(kw)) {
        return false;
    }
    split_statements(line)
        .iter()
        .any(|s| COMPOUND_KEYWORDS.contains(&first_word(&s.text)))
}

/// Find a keyword in a string at a word boundary, ignoring quoted text
/// and `{ ... }` groups. Returns the byte offset of the keyword.
pub(crate) fn find_keyword(input: &str, keyword: &str) -> Option<usize> {
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

/// Byte offset of the first `)` outside quotes (case-arm pattern end).
fn find_pattern_end(s: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ')' if !in_single && !in_double => return Some(i),
            _ => {},
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// A parsed shell construct.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    /// A plain command line (may contain `&&`, `||`, pipes, redirects).
    Simple { text: String, line: usize },
    /// `if C; then B; [elif C; then B;]... [else B;] fi`.
    If {
        branches: Vec<(Vec<Node>, Vec<Node>)>,
        else_body: Option<Vec<Node>>,
    },
    /// `while C; do B; done` (`until` = true inverts the condition).
    Loop {
        cond: Vec<Node>,
        until: bool,
        body: Vec<Node>,
    },
    /// `for VAR [in WORDS]; do B; done` (`words` is expanded at run time).
    For {
        var: String,
        words: String,
        body: Vec<Node>,
    },
    /// `case WORD in PAT) B ;; ... esac`.
    Case {
        word: String,
        arms: Vec<(String, Vec<Node>)>,
    },
}

fn syntax_error(msg: impl std::fmt::Display) -> OasisError {
    OasisError::Command(format!("syntax error: {msg}").into())
}

struct Parser {
    stmts: Vec<Stmt>,
    pos: usize,
}

impl Parser {
    fn peek_word(&self) -> Option<&str> {
        self.stmts.get(self.pos).map(|s| first_word(&s.text))
    }

    fn line(&self) -> usize {
        self.stmts
            .get(self.pos)
            .or(self.stmts.last())
            .map_or(0, |s| s.line)
    }

    /// Consume keyword `kw` at the start of the current statement. Any
    /// text following it on the same statement stays in place as the next
    /// statement (`then echo hi` -> `then` + `echo hi`).
    fn take_keyword(&mut self, kw: &str) -> bool {
        let Some(stmt) = self.stmts.get_mut(self.pos) else {
            return false;
        };
        if first_word(&stmt.text) != kw {
            return false;
        }
        let rest = stmt.text.trim_start()[kw.len()..].trim().to_string();
        if rest.is_empty() {
            self.pos += 1;
        } else {
            stmt.text = rest;
        }
        true
    }

    fn expect_keyword(&mut self, kw: &str, construct: &str) -> Result<()> {
        if self.take_keyword(kw) {
            Ok(())
        } else {
            Err(syntax_error(format_args!(
                "{construct}: missing '{kw}' (line {})",
                self.line()
            )))
        }
    }

    /// Parse statements until one whose first word is in `terms` (not
    /// consumed) or end of input.
    fn parse_block(&mut self, terms: &[&str]) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        while let Some(word) = self.peek_word() {
            if terms.contains(&word) {
                break;
            }
            let node = match word {
                "if" => self.parse_if()?,
                "while" | "until" => self.parse_loop()?,
                "for" => self.parse_for()?,
                "case" => self.parse_case()?,
                "then" | "do" | "done" | "fi" | "else" | "elif" | "esac" | ";;" => {
                    return Err(syntax_error(format_args!(
                        "unexpected '{word}' (line {})",
                        self.line()
                    )));
                },
                _ => {
                    let stmt = &self.stmts[self.pos];
                    let node = Node::Simple {
                        text: stmt.text.clone(),
                        line: stmt.line,
                    };
                    self.pos += 1;
                    node
                },
            };
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Parse a condition list terminated by `kw` (consumed).
    fn parse_condition(&mut self, kw: &str, construct: &str) -> Result<Vec<Node>> {
        let cond = self.parse_block(&[kw])?;
        if cond.is_empty() {
            return Err(syntax_error(format_args!(
                "{construct}: empty condition (line {})",
                self.line()
            )));
        }
        self.expect_keyword(kw, construct)?;
        Ok(cond)
    }

    fn parse_if(&mut self) -> Result<Node> {
        self.take_keyword("if");
        let mut branches = Vec::new();
        let mut else_body = None;
        let cond = self.parse_condition("then", "if")?;
        let body = self.parse_block(&["elif", "else", "fi"])?;
        branches.push((cond, body));
        loop {
            if self.take_keyword("elif") {
                let cond = self.parse_condition("then", "elif")?;
                let body = self.parse_block(&["elif", "else", "fi"])?;
                branches.push((cond, body));
            } else if self.take_keyword("else") {
                else_body = Some(self.parse_block(&["fi"])?);
                self.expect_keyword("fi", "if")?;
                break;
            } else {
                self.expect_keyword("fi", "if")?;
                break;
            }
        }
        Ok(Node::If {
            branches,
            else_body,
        })
    }

    fn parse_loop(&mut self) -> Result<Node> {
        let until = self.peek_word() == Some("until");
        let kw = if until { "until" } else { "while" };
        self.take_keyword(kw);
        let cond = self.parse_condition("do", kw)?;
        let body = self.parse_block(&["done"])?;
        self.expect_keyword("done", kw)?;
        Ok(Node::Loop { cond, until, body })
    }

    fn parse_for(&mut self) -> Result<Node> {
        let stmt = &self.stmts[self.pos];
        let header = stmt.text.trim_start()["for".len()..].trim().to_string();
        self.pos += 1;
        let (var, rest) = header
            .split_once(char::is_whitespace)
            .map_or((header.as_str(), ""), |(v, r)| (v, r.trim()));
        if var.is_empty() || !var.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(syntax_error(format_args!(
                "for: bad variable name '{var}' (line {})",
                stmt_line(&self.stmts, self.pos - 1)
            )));
        }
        let words = if first_word(rest) == "in" {
            rest["in".len()..].trim().to_string()
        } else if rest.is_empty() {
            String::new()
        } else {
            return Err(syntax_error(format_args!(
                "for: expected 'in' (line {})",
                stmt_line(&self.stmts, self.pos - 1)
            )));
        };
        self.expect_keyword("do", "for")?;
        let body = self.parse_block(&["done"])?;
        self.expect_keyword("done", "for")?;
        Ok(Node::For {
            var: var.to_string(),
            words,
            body,
        })
    }

    fn parse_case(&mut self) -> Result<Node> {
        let line = self.line();
        let header = self.stmts[self.pos].text.trim_start()["case".len()..]
            .trim()
            .to_string();
        let in_pos = find_keyword(&header, "in")
            .ok_or_else(|| syntax_error(format_args!("case: missing 'in' (line {line})")))?;
        let word = header[..in_pos].trim().to_string();
        let after = header[in_pos + 2..].trim().to_string();
        if after.is_empty() {
            self.pos += 1;
        } else {
            self.stmts[self.pos].text = after;
        }

        let mut arms = Vec::new();
        loop {
            let Some(stmt) = self.stmts.get(self.pos) else {
                return Err(syntax_error(format_args!(
                    "case: missing 'esac' (line {line})"
                )));
            };
            if first_word(&stmt.text) == "esac" {
                self.take_keyword("esac");
                break;
            }
            if stmt.text == ";;" {
                self.pos += 1;
                continue;
            }
            let end = find_pattern_end(&stmt.text).ok_or_else(|| {
                syntax_error(format_args!(
                    "case: expected 'PATTERN)' (line {})",
                    stmt.line
                ))
            })?;
            let pattern = stmt.text[..end]
                .trim()
                .trim_start_matches('(')
                .trim()
                .to_string();
            let rest = stmt.text[end + 1..].trim().to_string();
            if rest.is_empty() {
                self.pos += 1;
            } else {
                self.stmts[self.pos].text = rest;
            }
            let body = self.parse_block(&[";;", "esac"])?;
            if self.stmts.get(self.pos).is_some_and(|s| s.text == ";;") {
                self.pos += 1;
            }
            arms.push((pattern, body));
        }
        Ok(Node::Case { word, arms })
    }
}

fn stmt_line(stmts: &[Stmt], idx: usize) -> usize {
    stmts.get(idx).map_or(0, |s| s.line)
}

/// Parse statements into a node tree.
pub(crate) fn parse_script(stmts: Vec<Stmt>) -> Result<Vec<Node>> {
    let mut parser = Parser { stmts, pos: 0 };
    let nodes = parser.parse_block(&[])?;
    if let Some(word) = parser.peek_word() {
        return Err(syntax_error(format_args!(
            "unexpected '{word}' (line {})",
            parser.line()
        )));
    }
    Ok(nodes)
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Per-run execution state: collected outputs plus error formatting mode.
struct ExecCtx {
    outputs: Vec<CommandOutput>,
    /// Script-file mode reports errors as `error at line N: ...`; inline
    /// mode as `error: ...`.
    script: bool,
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
        let lines = self.run_script_source(&source, env)?;
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
        let mut ctx = ExecCtx {
            outputs: Vec::new(),
            script: true,
        };
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
        let mut ctx = ExecCtx {
            outputs: Vec::new(),
            script: false,
        };
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
            self.run_node(node, env, ctx);
        }
    }

    fn run_node(&self, node: &Node, env: &mut Environment<'_>, ctx: &mut ExecCtx) {
        match node {
            Node::Simple { text, line } => match self.execute_chain(text, env) {
                Ok(CommandOutput::None) => {},
                Ok(output) => ctx.outputs.push(output),
                Err(e) => {
                    self.last_exit_code.set(1);
                    self.set_variable("?", "1");
                    let msg = if ctx.script {
                        format!("error at line {line}: {e}")
                    } else {
                        format!("error: {e}")
                    };
                    ctx.outputs.push(CommandOutput::Text(msg));
                },
            },
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
                if let Some(body) = else_body {
                    self.run_nodes(body, env, ctx);
                }
            },
            Node::Loop { cond, until, body } => {
                let mut iterations = 0;
                loop {
                    if iterations >= MAX_LOOP_ITERATIONS {
                        ctx.outputs.push(CommandOutput::Text(format!(
                            "warning: loop terminated after {MAX_LOOP_ITERATIONS} \
                             iterations (limit reached)"
                        )));
                        break;
                    }
                    iterations += 1;
                    if self.eval_condition(cond, env, ctx) == *until {
                        break;
                    }
                    if self.run_loop_body(body, env, ctx) {
                        break;
                    }
                }
            },
            Node::For { var, words, body } => {
                for word in self.expand_words(words, env) {
                    self.set_variable(var, &word);
                    if self.run_loop_body(body, env, ctx) {
                        break;
                    }
                }
            },
            Node::Case { word, arms } => {
                let value = self.expand_words(word, env).join(" ");
                for (pattern, body) in arms {
                    if case_pattern_matches(&value, pattern) {
                        self.run_nodes(body, env, ctx);
                        break;
                    }
                }
            },
        }
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
    fn expand_words(&self, words: &str, env: &mut Environment<'_>) -> Vec<String> {
        let subst = self.expand_substitutions(words, env);
        let vars = self.expand_variables(&subst, &env.cwd);
        let tokens = tokenize(&vars)
            .unwrap_or_else(|_| vars.split_whitespace().map(str::to_string).collect());
        let tokens = expand_braces(&tokens);
        expand_globs(&tokens, env.vfs, &env.cwd)
    }

    /// Evaluate a condition list for `if` / `elif` / `while` / `until`.
    ///
    /// All statements run (their output is suppressed); the last one
    /// decides. A simple command is true when it succeeds with exit code 0
    /// and does not print `false` / `1` / nothing (the built-in `test`
    /// prints `true` / `false`). A leading `!` negates the result.
    fn eval_condition(&self, cond: &[Node], env: &mut Environment<'_>, ctx: &ExecCtx) -> bool {
        let Some((last, init)) = cond.split_last() else {
            return true;
        };
        let mut scratch = ExecCtx {
            outputs: Vec::new(),
            script: ctx.script,
        };
        self.run_nodes(init, env, &mut scratch);
        match last {
            Node::Simple { text, .. } => {
                let (negate, text) = match text.strip_prefix('!') {
                    Some(rest) if rest.starts_with(char::is_whitespace) => (true, rest.trim()),
                    _ => (false, text.as_str()),
                };
                let result = self.execute_chain(text, env);
                let truth = match &result {
                    Err(_) => false,
                    Ok(_) if self.last_exit_code.get() != 0 => false,
                    Ok(CommandOutput::Text(t)) => {
                        let t = t.trim();
                        !(t.is_empty() || t == "false" || t == "1")
                    },
                    Ok(_) => true,
                };
                truth != negate
            },
            compound => {
                self.run_node(compound, env, &mut scratch);
                self.last_exit_code.get() == 0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        split_statements(src).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn split_on_newlines_and_semicolons() {
        assert_eq!(
            texts("echo a; echo b\necho c"),
            ["echo a", "echo b", "echo c"]
        );
    }

    #[test]
    fn split_respects_quotes() {
        assert_eq!(
            texts("echo 'a;b'; echo \"c;d\""),
            ["echo 'a;b'", "echo \"c;d\""]
        );
    }

    #[test]
    fn split_keeps_braces_and_substitutions() {
        assert_eq!(
            texts("function f() { echo a; echo b }; echo $(echo x; echo y)"),
            ["function f() { echo a; echo b }", "echo $(echo x; echo y)"]
        );
    }

    #[test]
    fn split_emits_double_semicolon() {
        assert_eq!(
            texts("a) echo x ;; b) echo y;;"),
            ["a) echo x", ";;", "b) echo y", ";;"]
        );
    }

    #[test]
    fn split_strips_comments_but_not_dollar_hash() {
        assert_eq!(texts("# comment\necho $# # trailing"), ["echo $#"]);
        assert_eq!(texts("echo a#b"), ["echo a#b"]);
    }

    #[test]
    fn split_tracks_line_numbers() {
        let stmts = split_statements("echo a\n\n# c\necho b; echo c");
        let lines: Vec<usize> = stmts.iter().map(|s| s.line).collect();
        assert_eq!(lines, [1, 4, 4]);
    }

    #[test]
    fn split_line_continuation() {
        assert_eq!(texts("echo a \\\nb"), ["echo a b"]);
    }

    #[test]
    fn is_compound_detects_keywords_only_at_statement_start() {
        assert!(is_compound("if true; then echo x; fi"));
        assert!(is_compound("echo a; for x in 1; do echo $x; done"));
        assert!(!is_compound("echo if then fi"));
        assert!(!is_compound("echo 'a; if b'"));
    }

    #[test]
    fn parse_nested_if_with_elif() {
        let nodes = parse_script(split_statements(
            "if a; then if b; then c; fi; elif d; then e; else f; fi",
        ))
        .unwrap();
        assert_eq!(nodes.len(), 1);
        let Node::If {
            branches,
            else_body,
        } = &nodes[0]
        else {
            panic!("expected if");
        };
        assert_eq!(branches.len(), 2);
        assert!(matches!(branches[0].1[0], Node::If { .. }));
        assert!(else_body.is_some());
    }

    #[test]
    fn parse_errors_on_missing_terminators() {
        assert!(parse_script(split_statements("if a; then b")).is_err());
        assert!(parse_script(split_statements("while a; do b")).is_err());
        assert!(parse_script(split_statements("case x in a) b ;;")).is_err());
        assert!(parse_script(split_statements("echo a; fi")).is_err());
    }

    #[test]
    fn find_keyword_basic() {
        assert_eq!(find_keyword("foo then bar", "then"), Some(4));
        assert_eq!(find_keyword("undone", "done"), None);
        assert_eq!(find_keyword("echo 'then' fi", "then"), None);
    }
}

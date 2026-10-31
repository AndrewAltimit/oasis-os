//! The script parser: [`parse_script`] turns statements into a [`Node`]
//! tree of nested `if` / `while` / `until` / `for` / `case` blocks.

use oasis_types::error::{OasisError, Result};

use super::split::{find_pattern_end, first_word};
use super::{Stmt, find_keyword};

/// Maximum nesting of `if` / loop / `case` blocks (the parser recurses
/// once per level).
const MAX_BLOCK_NESTING: usize = 64;

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
    /// A compound command followed on its closing line by a pipe,
    /// redirection or and-or list: `done | sort`, `fi > out`,
    /// `esac && echo ok` (`suffix` = the text after the keyword).
    Redirected {
        node: Box<Node>,
        suffix: String,
        line: usize,
    },
}

/// Whether `rest` (text after a closing `done` / `fi` / `esac`) continues
/// the compound command: a pipe, redirection or `&&` / `||`.
fn is_compound_suffix(rest: &str) -> bool {
    rest.starts_with(['|', '>', '<'])
        || rest.starts_with("&&")
        || rest.starts_with("&>")
        || rest.starts_with("2>")
        || rest.starts_with("1>")
}

fn syntax_error(msg: impl std::fmt::Display) -> OasisError {
    OasisError::Command(format!("syntax error: {msg}").into())
}

struct Parser {
    stmts: Vec<Stmt>,
    pos: usize,
    /// Current block nesting (see `MAX_BLOCK_NESTING`).
    depth: usize,
    /// Suffix (and its line) found after the closing keyword of the
    /// compound command just parsed (see [`Node::Redirected`]).
    suffix: Option<(String, usize)>,
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

    /// Consume the closing keyword `kw` (`done`, `fi`, `esac`) of a
    /// compound command, recording a pipe / redirection / and-or suffix on
    /// the same statement (`done | sort`) in `self.suffix`.
    fn expect_closer(&mut self, kw: &str, construct: &str) -> Result<()> {
        if let Some(stmt) = self.stmts.get(self.pos) {
            let text = stmt.text.trim_start();
            if let Some(rest) = text.strip_prefix(kw)
                && rest.starts_with(char::is_whitespace)
                && is_compound_suffix(rest.trim_start())
            {
                self.suffix = Some((rest.trim().to_string(), stmt.line));
                self.pos += 1;
                return Ok(());
            }
        }
        self.expect_keyword(kw, construct)
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
        if self.depth >= MAX_BLOCK_NESTING {
            return Err(syntax_error(format_args!(
                "blocks nested deeper than {MAX_BLOCK_NESTING} (line {})",
                self.line()
            )));
        }
        self.depth += 1;
        let nodes = self.parse_block_inner(terms);
        self.depth -= 1;
        nodes
    }

    fn parse_block_inner(&mut self, terms: &[&str]) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        while let Some(word) = self.peek_word().map(str::to_string) {
            let word = word.as_str();
            if terms.contains(&word) {
                break;
            }
            let compound = match word {
                "if" => Some(self.parse_if()?),
                "while" | "until" => Some(self.parse_loop()?),
                "for" => Some(self.parse_for()?),
                "case" => Some(self.parse_case()?),
                _ => None,
            };
            if let Some(node) = compound {
                nodes.push(match self.suffix.take() {
                    Some((suffix, line)) => Node::Redirected {
                        node: Box::new(node),
                        suffix,
                        line,
                    },
                    None => node,
                });
                continue;
            }
            let node = match word {
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
                self.expect_closer("fi", "if")?;
                break;
            } else {
                self.expect_closer("fi", "if")?;
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
        self.expect_closer("done", kw)?;
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
            // `for VAR; do` iterates over the positional parameters.
            "\"$@\"".to_string()
        } else {
            return Err(syntax_error(format_args!(
                "for: expected 'in' (line {})",
                stmt_line(&self.stmts, self.pos - 1)
            )));
        };
        self.expect_keyword("do", "for")?;
        let body = self.parse_block(&["done"])?;
        self.expect_closer("done", "for")?;
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
                self.expect_closer("esac", "case")?;
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
    let mut parser = Parser {
        stmts,
        pos: 0,
        depth: 0,
        suffix: None,
    };
    let nodes = parser.parse_block(&[])?;
    if let Some(word) = parser.peek_word() {
        return Err(syntax_error(format_args!(
            "unexpected '{word}' (line {})",
            parser.line()
        )));
    }
    Ok(nodes)
}

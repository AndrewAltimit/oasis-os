//! Calculator application with expression evaluation.
//!
//! Provides a full-featured calculator with operator precedence, parentheses,
//! memory registers, and calculation history. The expression evaluator is a
//! pure recursive-descent parser with no external dependencies.
//!
//! Input: type `0-9 . + - * / ^ % ( )`, `=` / Enter evaluates, Backspace
//! deletes, Escape clears (and closes an already-clear calculator), Up /
//! Down recall history. Windowed mode draws a themed display panel and a
//! clickable key grid ([`keypad`]); on a gamepad the d-pad moves a keypad
//! cursor and Confirm presses the key under it. History persists to
//! [`history::HISTORY_PATH`].

use std::cell::Cell;

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

pub mod history;
pub mod keypad;
mod render;

use keypad::{CalcKey, CalcLayout, Dir, KEYS};
pub use render::CalcColors;

// ---------------------------------------------------------------
// CalcError
// ---------------------------------------------------------------

/// Errors that can occur during expression evaluation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CalcError {
    /// Division or modulo by zero.
    #[error("Division by zero")]
    DivisionByZero,
    /// Expression could not be parsed.
    #[error("Invalid expression: {0}")]
    InvalidExpression(String),
    /// Mismatched parentheses.
    #[error("Unmatched parentheses")]
    UnmatchedParen,
    /// Input was empty or whitespace-only.
    #[error("Empty expression")]
    EmptyExpression,
    /// Expression nesting exceeds the maximum depth.
    #[error("Expression too deeply nested (max {MAX_DEPTH} levels)")]
    TooDeep,
}

/// Maximum recursion depth for the expression parser.
const MAX_DEPTH: usize = 100;

// ---------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------

/// Tokens produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    LeftParen,
    RightParen,
}

/// Tokenize an expression string into a sequence of `Token`s.
fn tokenize(expr: &str) -> Result<Vec<Token>, CalcError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        match ch {
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            },
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            },
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            },
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            },
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            },
            '^' => {
                tokens.push(Token::Power);
                i += 1;
            },
            '(' => {
                tokens.push(Token::LeftParen);
                i += 1;
            },
            ')' => {
                tokens.push(Token::RightParen);
                i += 1;
            },
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                let mut has_dot = c == '.';
                i += 1;
                while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    if chars[i] == '.' {
                        if has_dot {
                            break;
                        }
                        has_dot = true;
                    }
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                let value = num_str.parse::<f64>().map_err(|_| {
                    CalcError::InvalidExpression(format!("invalid number: {num_str}"))
                })?;
                tokens.push(Token::Number(value));
            },
            other => {
                return Err(CalcError::InvalidExpression(format!(
                    "unexpected character: '{other}'"
                )));
            },
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------

/// Recursive-descent parser state.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    /// Increment depth and check the limit.
    fn enter(&mut self) -> Result<(), CalcError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(CalcError::TooDeep);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    /// Top-level: parse additive expression.
    fn parse_expr(&mut self) -> Result<f64, CalcError> {
        let mut left = self.parse_term()?;

        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_term()?;
                    left += right;
                },
                Token::Minus => {
                    self.advance();
                    let right = self.parse_term()?;
                    left -= right;
                },
                _ => break,
            }
        }

        Ok(left)
    }

    /// Multiplicative: *, /, %
    fn parse_term(&mut self) -> Result<f64, CalcError> {
        let mut left = self.parse_power()?;

        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.advance();
                    let right = self.parse_power()?;
                    left *= right;
                },
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    left /= right;
                },
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    left %= right;
                },
                _ => break,
            }
        }

        Ok(left)
    }

    /// Power: ^ (right-associative)
    fn parse_power(&mut self) -> Result<f64, CalcError> {
        let base = self.parse_unary()?;

        if let Some(Token::Power) = self.peek() {
            self.advance();
            // Right-associative: recurse into parse_power.
            let exp = self.parse_power()?;
            Ok(base.powf(exp))
        } else {
            Ok(base)
        }
    }

    /// Unary minus.
    fn parse_unary(&mut self) -> Result<f64, CalcError> {
        if let Some(Token::Minus) = self.peek() {
            self.advance();
            let val = self.parse_unary()?;
            Ok(-val)
        } else {
            self.parse_primary()
        }
    }

    /// Primary: number or parenthesised expression.
    fn parse_primary(&mut self) -> Result<f64, CalcError> {
        match self.advance() {
            Some(Token::Number(n)) => Ok(n),
            Some(Token::LeftParen) => {
                self.enter()?;
                let val = self.parse_expr()?;
                self.leave();
                match self.advance() {
                    Some(Token::RightParen) => Ok(val),
                    _ => Err(CalcError::UnmatchedParen),
                }
            },
            Some(tok) => Err(CalcError::InvalidExpression(format!(
                "unexpected token: {tok:?}"
            ))),
            None => Err(CalcError::InvalidExpression(
                "unexpected end of expression".to_string(),
            )),
        }
    }
}

// ---------------------------------------------------------------
// Public evaluate function
// ---------------------------------------------------------------

/// Evaluate a mathematical expression string and return the result.
///
/// Supports: `+`, `-`, `*`, `/`, `%` (modulo), `^` (power),
/// parentheses, unary minus, integers, and decimals.
///
/// Operator precedence (lowest to highest):
/// 1. Addition / subtraction
/// 2. Multiplication / division / modulo
/// 3. Exponentiation (right-associative)
/// 4. Unary minus
/// 5. Parentheses
pub fn evaluate(expr: &str) -> Result<f64, CalcError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(CalcError::EmptyExpression);
    }

    let tokens = tokenize(trimmed)?;
    if tokens.is_empty() {
        return Err(CalcError::EmptyExpression);
    }

    let mut parser = Parser::new(tokens);
    let result = parser.parse_expr()?;

    // Ensure all tokens were consumed.
    if parser.pos < parser.tokens.len() {
        return Err(CalcError::InvalidExpression(
            "trailing tokens after expression".to_string(),
        ));
    }

    Ok(result)
}

// ---------------------------------------------------------------
// CalcHistoryEntry
// ---------------------------------------------------------------

/// A single calculation stored in history.
#[derive(Debug, Clone)]
pub struct CalcHistoryEntry {
    /// The expression that was evaluated.
    pub expression: String,
    /// The computed result.
    pub result: f64,
}

// ---------------------------------------------------------------
// CalculatorApp
// ---------------------------------------------------------------

/// Frames a pressed key stays drawn in its pressed state.
const FLASH_FRAMES: u8 = 6;

/// Calculator application state.
#[derive(Debug)]
pub struct CalculatorApp {
    content: ContentState,
    /// Current display value (formatted result or input echo).
    display: String,
    /// Expression being typed by the user.
    input_buffer: String,
    /// Previous calculations (oldest first).
    history: Vec<CalcHistoryEntry>,
    /// Memory register (M+, MS, MR, MC).
    memory: f64,
    /// Result of the last successful evaluation.
    last_result: Option<f64>,
    /// Current error message (cleared on next input).
    error_message: Option<String>,
    /// Keypad cursor: index into [`keypad::KEYS`] (d-pad navigation).
    cursor: usize,
    /// Whether the keypad cursor is drawn (d-pad in use, not mouse/typing).
    cursor_visible: bool,
    /// History entry currently recalled with Up/Down (index into `history`).
    recall_index: Option<usize>,
    /// Key drawn pressed, with the frames left before it pops back up.
    flash: Cell<Option<(usize, u8)>>,
    /// Whether history has been loaded from the VFS yet.
    history_loaded: bool,
    /// Whether history changed since it was last written to the VFS.
    history_dirty: bool,
}

impl CalculatorApp {
    /// Create a new calculator app at the given VFS path.
    ///
    /// Persisted history is loaded from [`history::HISTORY_PATH`] on the
    /// first [`App::refresh`] / [`App::apply_vfs_ops`] call.
    pub fn new(path: &str) -> Self {
        let content = ContentState::new("Calculator", path);
        let mut app = Self {
            content,
            display: "0".to_string(),
            input_buffer: String::new(),
            history: Vec::new(),
            memory: 0.0,
            last_result: None,
            error_message: None,
            cursor: keypad::index_of(CalcKey::Digit('5')).unwrap_or(0),
            cursor_visible: false,
            recall_index: None,
            flash: Cell::new(None),
            history_loaded: false,
            history_dirty: false,
        };
        app.refresh_lines();
        app
    }

    /// Append a digit or decimal point to the input buffer.
    pub fn push_digit(&mut self, d: char) {
        self.error_message = None;
        self.recall_index = None;

        // Prevent multiple leading zeros (allow "0." but not "00").
        if d == '0' && self.input_buffer == "0" {
            return;
        }

        // Prevent multiple decimal points in the current number.
        if d == '.' {
            // Find the last number segment (after last operator/paren).
            let last_num_start = self
                .input_buffer
                .rfind(|c: char| "+-*/%^()".contains(c))
                .map_or(0, |p| p + 1);
            let current_num = &self.input_buffer[last_num_start..];
            if current_num.contains('.') {
                return;
            }
        }

        // Replace lone "0" with the digit (unless it's "0.").
        if self.input_buffer == "0" && d != '.' {
            self.input_buffer.clear();
        }

        self.input_buffer.push(d);
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Append an operator (+, -, *, /, ^, %) to the input buffer.
    pub fn push_operator(&mut self, op: char) {
        self.error_message = None;
        self.recall_index = None;

        // If buffer is empty but we have a last result, start from it.
        if self.input_buffer.is_empty() {
            if let Some(result) = self.last_result {
                self.input_buffer = format_number(result);
            } else {
                self.input_buffer.push('0');
            }
        }

        // Replace trailing operator with the new one.
        if let Some(last) = self.input_buffer.chars().last()
            && "+-*/%^".contains(last)
        {
            self.input_buffer.pop();
        }

        self.input_buffer.push(op);
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Type a `-`: a unary minus at the start of the entry or after an
    /// operator / `(`, otherwise the subtraction operator.
    pub fn push_minus(&mut self) {
        let unary = match self.input_buffer.chars().last() {
            None => self.last_result.is_none(),
            Some(c) => "*/^%(".contains(c),
        };
        if unary {
            self.error_message = None;
            self.recall_index = None;
            self.input_buffer.push('-');
            self.display = self.input_buffer.clone();
            self.refresh_lines();
        } else {
            self.push_operator('-');
        }
    }

    /// Append an opening or closing parenthesis.
    pub fn push_paren(&mut self, open: bool) {
        self.error_message = None;
        self.recall_index = None;

        if open {
            self.input_buffer.push('(');
        } else {
            self.input_buffer.push(')');
        }
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Evaluate the current input expression.
    pub fn evaluate_input(&mut self) {
        self.recall_index = None;
        if self.input_buffer.is_empty() {
            return;
        }

        match evaluate(&self.input_buffer) {
            Ok(result) => {
                let entry = CalcHistoryEntry {
                    expression: self.input_buffer.clone(),
                    result,
                };
                self.history.push(entry);
                let excess = self.history.len().saturating_sub(history::MAX_HISTORY);
                self.history.drain(..excess);
                self.history_dirty = true;
                self.display = format_number(result);
                self.last_result = Some(result);
                self.error_message = None;
                self.input_buffer.clear();
            },
            Err(e) => {
                self.error_message = Some(e.to_string());
                self.display = e.to_string();
            },
        }
        self.refresh_lines();
    }

    /// Clear the current input (C).
    pub fn clear(&mut self) {
        self.input_buffer.clear();
        self.display = "0".to_string();
        self.error_message = None;
        self.recall_index = None;
        self.refresh_lines();
    }

    /// Clear input, last result and history (AC). The cleared history is
    /// persisted on the next [`App::apply_vfs_ops`].
    pub fn clear_all(&mut self) {
        self.input_buffer.clear();
        self.display = "0".to_string();
        if !self.history.is_empty() {
            self.history.clear();
            self.history_dirty = true;
        }
        self.last_result = None;
        self.error_message = None;
        self.recall_index = None;
        self.refresh_lines();
    }

    /// Delete the last character from the input buffer.
    pub fn backspace(&mut self) {
        self.error_message = None;
        self.recall_index = None;
        self.input_buffer.pop();
        if self.input_buffer.is_empty() {
            self.display = "0".to_string();
        } else {
            self.display = self.input_buffer.clone();
        }
        self.refresh_lines();
    }

    /// Value MS / M+ act on: the typed number if the entry is a plain
    /// number, otherwise the last result.
    fn memory_operand(&self) -> Option<f64> {
        if !self.input_buffer.is_empty() {
            return self.input_buffer.parse::<f64>().ok().or(self.last_result);
        }
        self.last_result
    }

    /// Store the current value to memory (MS).
    pub fn memory_store(&mut self) {
        if let Some(v) = self.memory_operand() {
            self.memory = v;
        }
        self.refresh_lines();
    }

    /// Recall memory value into the input buffer (MR).
    pub fn memory_recall(&mut self) {
        self.insert_value(self.memory);
    }

    /// Insert the last result into the input buffer (Ans).
    pub fn insert_ans(&mut self) {
        if let Some(r) = self.last_result {
            self.insert_value(r);
        }
    }

    /// Append `value` to the entry (a fresh entry replaces a lone "0").
    fn insert_value(&mut self, value: f64) {
        self.error_message = None;
        self.recall_index = None;
        if self.input_buffer == "0" {
            self.input_buffer.clear();
        }
        self.input_buffer.push_str(&format_number(value));
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Add the current value to memory (M+).
    pub fn memory_add(&mut self) {
        if let Some(v) = self.memory_operand() {
            self.memory += v;
        }
        self.refresh_lines();
    }

    /// Clear the memory register (MC).
    pub fn memory_clear(&mut self) {
        self.memory = 0.0;
        self.refresh_lines();
    }

    /// Toggle the sign of the current input.
    pub fn negate(&mut self) {
        self.error_message = None;
        self.recall_index = None;

        if self.input_buffer.is_empty() {
            if let Some(result) = self.last_result {
                self.last_result = Some(-result);
                self.display = format_number(-result);
                self.refresh_lines();
            }
            return;
        }

        // Toggle leading minus on the whole buffer.
        if self.input_buffer.starts_with('-') {
            self.input_buffer.remove(0);
        } else {
            self.input_buffer.insert(0, '-');
        }

        if self.input_buffer.is_empty() {
            self.display = "0".to_string();
        } else {
            self.display = self.input_buffer.clone();
        }
        self.refresh_lines();
    }

    /// Recall an older (`older == true`) or newer history expression into
    /// the entry. Stepping newer past the newest entry clears the entry.
    pub fn recall_history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.recall_index, older) {
            (None, true) => Some(self.history.len() - 1),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) => (i + 1 < self.history.len()).then_some(i + 1),
        };
        self.error_message = None;
        match next {
            Some(i) => {
                self.input_buffer = self.history[i].expression.clone();
                self.display = self.input_buffer.clone();
            },
            None => {
                self.input_buffer.clear();
                self.display = "0".to_string();
            },
        }
        self.recall_index = next;
        self.refresh_lines();
    }

    /// Press a keypad key (click, d-pad Confirm or a typed character).
    pub fn press_key(&mut self, key: CalcKey) {
        if let Some(i) = keypad::index_of(key) {
            self.flash.set(Some((i, FLASH_FRAMES)));
        }
        match key {
            CalcKey::Digit(d) => self.push_digit(d),
            CalcKey::Op('-') => self.push_minus(),
            CalcKey::Op(op) => self.push_operator(op),
            CalcKey::OpenParen => self.push_paren(true),
            CalcKey::CloseParen => self.push_paren(false),
            CalcKey::Equals => self.evaluate_input(),
            CalcKey::Clear => self.clear(),
            CalcKey::AllClear => self.clear_all(),
            CalcKey::Backspace => self.backspace(),
            CalcKey::Negate => self.negate(),
            CalcKey::Ans => self.insert_ans(),
            CalcKey::MemClear => self.memory_clear(),
            CalcKey::MemRecall => self.memory_recall(),
            CalcKey::MemStore => self.memory_store(),
            CalcKey::MemAdd => self.memory_add(),
        }
    }

    /// Key a typed character stands for, if any.
    fn key_for_char(ch: char) -> Option<CalcKey> {
        Some(match ch {
            '0'..='9' | '.' => CalcKey::Digit(ch),
            ',' => CalcKey::Digit('.'),
            '+' | '-' | '*' | '/' | '^' | '%' => CalcKey::Op(ch),
            'x' | 'X' => CalcKey::Op('*'),
            '(' => CalcKey::OpenParen,
            ')' => CalcKey::CloseParen,
            '=' | '\n' | '\r' => CalcKey::Equals,
            _ => return None,
        })
    }

    /// Whether Escape has anything to clear (otherwise it closes the app).
    fn has_entry(&self) -> bool {
        !self.input_buffer.is_empty() || self.error_message.is_some() || self.display != "0"
    }

    /// Load persisted history once, keeping entries made before the load.
    fn ensure_history_loaded(&mut self, vfs: &dyn Vfs) -> bool {
        if self.history_loaded {
            return false;
        }
        self.history_loaded = true;
        let mut loaded = history::load(vfs);
        if loaded.is_empty() {
            return false;
        }
        loaded.append(&mut self.history);
        let excess = loaded.len().saturating_sub(history::MAX_HISTORY);
        loaded.drain(..excess);
        self.history = loaded;
        self.recall_index = None;
        self.refresh_lines();
        true
    }

    /// Format the calculator state into display lines (full-screen text
    /// mode and the generic line renderer).
    pub fn format_display_lines(&self) -> Vec<String> {
        let separator = "\u{2500}".repeat(30); // box-drawing horizontal line

        let mut lines = Vec::new();
        // No heading line — the app title already shows in the WM /
        // app-chrome title bar.

        // Input / result area.
        if self.input_buffer.is_empty() {
            lines.push(format!("  Result: {}", self.display));
        } else {
            lines.push(format!("  Input: {}", self.input_buffer));
            if let Some(ref err) = self.error_message {
                lines.push(format!("  Error: {err}"));
            } else if let Some(result) = self.last_result {
                lines.push(format!("  Last: {}", format_number(result)));
            }
        }

        // Keypad cursor (d-pad selects, Confirm presses).
        let key = KEYS.get(self.cursor).map_or("", |d| d.key.label());
        lines.push(format!(
            "  Key: [{key}]    Memory: {}",
            format_number(self.memory)
        ));

        lines.push(separator.clone());

        // History section.
        if self.history.is_empty() {
            lines.push("  History: (empty)".to_string());
        } else {
            lines.push("  History:".to_string());
            // Show most recent entries last (up to 10).
            let start = self.history.len().saturating_sub(10);
            for entry in &self.history[start..] {
                lines.push(format!(
                    "    {} = {}",
                    entry.expression,
                    format_number(entry.result)
                ));
            }
        }

        lines.push(separator);

        // Controls help.
        lines.push("  [D-pad]=Move  [Confirm]=Press  [Square]=DEL".to_string());
        lines.push("  [Triangle]=C  [Start]=AC  [Select]=Recall".to_string());

        lines
    }

    /// Rebuild the `content.lines` from the current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.format_display_lines();
    }
}

/// Format a number for display, removing trailing zeros from decimals.
fn format_number(n: f64) -> String {
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if n.is_nan() {
        return "NaN".to_string();
    }
    // If the number is effectively an integer, display without decimals.
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        // Up to 10 decimal places, strip trailing zeros.
        let s = format!("{n:.10}");
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}

// ---------------------------------------------------------------
// App trait implementation
// ---------------------------------------------------------------

impl App for CalculatorApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        self.ensure_history_loaded(vfs);
        let dir = match button {
            Button::Cancel => return AppAction::Exit,
            Button::Up => Some(Dir::Up),
            Button::Down => Some(Dir::Down),
            Button::Left => Some(Dir::Left),
            Button::Right => Some(Dir::Right),
            // Confirm presses the key under the keypad cursor.
            Button::Confirm => {
                if let Some(def) = KEYS.get(self.cursor) {
                    self.press_key(def.key);
                }
                None
            },
            Button::Triangle => {
                self.press_key(CalcKey::Clear);
                None
            },
            Button::Square => {
                self.press_key(CalcKey::Backspace);
                None
            },
            Button::Start => {
                self.press_key(CalcKey::AllClear);
                None
            },
            // Select steps back through history (gamepad Up/Down move
            // the keypad cursor).
            Button::Select => {
                self.recall_history(true);
                None
            },
        };
        if let Some(dir) = dir {
            self.cursor = keypad::step(self.cursor, dir);
            self.cursor_visible = true;
            self.refresh_lines();
        }
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, vfs: &dyn Vfs) -> Option<AppAction> {
        if mods.has_command() {
            return None;
        }
        self.ensure_history_loaded(vfs);
        match key {
            Key::Enter => self.press_key(CalcKey::Equals),
            Key::Backspace => self.press_key(CalcKey::Backspace),
            Key::Delete => self.press_key(CalcKey::Clear),
            // Escape clears; on an already-clear calculator it falls
            // through to Cancel and closes the app.
            Key::Escape if self.has_entry() => self.press_key(CalcKey::Clear),
            Key::Up => self.recall_history(true),
            Key::Down => self.recall_history(false),
            _ => return None,
        }
        self.cursor_visible = false;
        Some(AppAction::None)
    }

    fn accepts_text(&self) -> bool {
        true
    }

    fn handle_text_input(&mut self, ch: char) {
        if let Some(key) = Self::key_for_char(ch) {
            self.cursor_visible = false;
            self.press_key(key);
        }
    }

    fn handle_backspace(&mut self) {
        self.press_key(CalcKey::Backspace);
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
        let l = CalcLayout::compute(0, 0, cw, ch);
        if let Some(i) = l.key_at(lx, ly) {
            self.cursor = i;
            self.cursor_visible = false;
            self.press_key(KEYS[i].key);
        } else if let Some(row) = l.history_row_at(lx, ly)
            && let Some(idx) = self.history.len().checked_sub(row + 1)
        {
            // History rows are newest first; clicking one recalls it.
            self.recall_index = Some(idx);
            self.input_buffer = self.history[idx].expression.clone();
            self.display = self.input_buffer.clone();
            self.error_message = None;
            self.refresh_lines();
        }
        AppAction::None
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
        self.ensure_history_loaded(vfs);
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        let mut changed = self.ensure_history_loaded(vfs);
        if self.history_dirty {
            self.history_dirty = false;
            if let Err(e) = history::save(vfs, &self.history) {
                self.error_message = Some(format!("History not saved: {e}"));
                self.refresh_lines();
            }
            changed = true;
        }
        changed
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        self.draw_calculator(cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    // -- Expression evaluator: basic operations --

    #[test]
    fn eval_addition() {
        assert_eq!(evaluate("2 + 3").ok(), Some(5.0));
    }

    #[test]
    fn eval_subtraction() {
        assert_eq!(evaluate("10 - 4").ok(), Some(6.0));
    }

    #[test]
    fn eval_multiplication() {
        assert_eq!(evaluate("3 * 7").ok(), Some(21.0));
    }

    #[test]
    fn eval_division() {
        assert_eq!(evaluate("20 / 4").ok(), Some(5.0));
    }

    #[test]
    fn eval_modulo() {
        assert_eq!(evaluate("10 % 3").ok(), Some(1.0));
    }

    #[test]
    fn eval_power() {
        assert_eq!(evaluate("2 ^ 10").ok(), Some(1024.0));
    }

    // -- Precedence --

    #[test]
    fn eval_precedence_mul_before_add() {
        assert_eq!(evaluate("2 + 3 * 4").ok(), Some(14.0));
    }

    #[test]
    fn eval_precedence_complex() {
        // 2 + 3 * (4 - 1) = 2 + 9 = 11
        assert_eq!(evaluate("2 + 3 * (4 - 1)").ok(), Some(11.0));
    }

    #[test]
    fn eval_precedence_power_before_mul() {
        // 2 * 3 ^ 2 = 2 * 9 = 18
        assert_eq!(evaluate("2 * 3 ^ 2").ok(), Some(18.0));
    }

    #[test]
    fn eval_power_right_associative() {
        // 2 ^ 3 ^ 2 = 2 ^ 9 = 512 (not (2^3)^2 = 64)
        assert_eq!(evaluate("2 ^ 3 ^ 2").ok(), Some(512.0));
    }

    // -- Parentheses --

    #[test]
    fn eval_simple_parens() {
        assert_eq!(evaluate("(2 + 3) * 4").ok(), Some(20.0));
    }

    #[test]
    fn eval_nested_parens() {
        // ((2 + 3) * (4 - 1)) = 5 * 3 = 15
        assert_eq!(evaluate("((2 + 3) * (4 - 1))").ok(), Some(15.0));
    }

    #[test]
    fn eval_deeply_nested() {
        assert_eq!(evaluate("(((1 + 2)))").ok(), Some(3.0));
    }

    // -- Unary minus --

    #[test]
    fn eval_unary_minus() {
        assert_eq!(evaluate("-5").ok(), Some(-5.0));
    }

    #[test]
    fn eval_unary_minus_in_expr() {
        assert_eq!(evaluate("3 + -2").ok(), Some(1.0));
    }

    #[test]
    fn eval_double_unary_minus() {
        assert_eq!(evaluate("--5").ok(), Some(5.0));
    }

    #[test]
    fn eval_unary_minus_with_parens() {
        assert_eq!(evaluate("-(3 + 2)").ok(), Some(-5.0));
    }

    // -- Decimal numbers --

    #[test]
    fn eval_decimal() {
        let result = evaluate("1.5 + 2.5").ok();
        assert_eq!(result, Some(4.0));
    }

    #[test]
    fn eval_decimal_mul() {
        let result = evaluate("0.1 * 10").ok();
        assert!((result.unwrap_or(0.0) - 1.0).abs() < 1e-9);
    }

    // -- Error cases --

    #[test]
    fn eval_empty_expression() {
        assert_eq!(evaluate(""), Err(CalcError::EmptyExpression));
    }

    #[test]
    fn eval_whitespace_only() {
        assert_eq!(evaluate("   "), Err(CalcError::EmptyExpression));
    }

    #[test]
    fn eval_division_by_zero() {
        assert_eq!(evaluate("1 / 0"), Err(CalcError::DivisionByZero));
    }

    #[test]
    fn eval_modulo_by_zero() {
        assert_eq!(evaluate("5 % 0"), Err(CalcError::DivisionByZero));
    }

    #[test]
    fn eval_unmatched_left_paren() {
        assert_eq!(evaluate("(2 + 3"), Err(CalcError::UnmatchedParen));
    }

    #[test]
    fn eval_unmatched_right_paren() {
        assert!(evaluate("2 + 3)").is_err());
    }

    #[test]
    fn eval_invalid_char() {
        assert!(matches!(
            evaluate("2 & 3"),
            Err(CalcError::InvalidExpression(_))
        ));
    }

    #[test]
    fn eval_trailing_operator() {
        assert!(evaluate("2 +").is_err());
    }

    // -- Chained operations --

    #[test]
    fn eval_chained_add_sub() {
        assert_eq!(evaluate("1 + 2 - 3 + 4").ok(), Some(4.0));
    }

    #[test]
    fn eval_chained_mul_div() {
        assert_eq!(evaluate("12 / 3 * 2").ok(), Some(8.0));
    }

    // -- format_number --

    #[test]
    fn format_integer() {
        assert_eq!(format_number(42.0), "42");
    }

    #[test]
    fn format_decimal() {
        assert_eq!(format_number(2.75), "2.75");
    }

    #[test]
    fn format_negative() {
        assert_eq!(format_number(-7.0), "-7");
    }

    // -- CalculatorApp state tests --

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    #[test]
    fn app_new_display() {
        let app = CalculatorApp::new("/apps/calc");
        assert_eq!(app.display, "0");
        assert!(app.input_buffer.is_empty());
        assert!(app.history.is_empty());
    }

    #[test]
    fn app_title_and_path() {
        let app = CalculatorApp::new("/apps/calc");
        assert_eq!(app.title(), "Calculator");
        assert_eq!(app.path(), "/apps/calc");
    }

    #[test]
    fn app_push_digits() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_digit('2');
        app.push_digit('3');
        assert_eq!(app.input_buffer, "123");
        assert_eq!(app.display, "123");
    }

    #[test]
    fn app_push_operator() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('5');
        app.push_operator('+');
        assert_eq!(app.input_buffer, "5+");
    }

    #[test]
    fn app_evaluate() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('2');
        app.push_operator('+');
        app.push_digit('3');
        app.evaluate_input();
        assert_eq!(app.last_result, Some(5.0));
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.history[0].expression, "2+3");
        assert_eq!(app.history[0].result, 5.0);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn app_history_tracks_multiple() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_operator('+');
        app.push_digit('1');
        app.evaluate_input();
        app.push_digit('2');
        app.push_operator('*');
        app.push_digit('3');
        app.evaluate_input();
        assert_eq!(app.history.len(), 2);
        assert_eq!(app.history[0].result, 2.0);
        assert_eq!(app.history[1].result, 6.0);
    }

    #[test]
    fn app_clear() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('5');
        app.push_operator('+');
        app.push_digit('3');
        app.clear();
        assert!(app.input_buffer.is_empty());
        assert_eq!(app.display, "0");
    }

    #[test]
    fn app_clear_all() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_operator('+');
        app.push_digit('1');
        app.evaluate_input();
        app.clear_all();
        assert!(app.input_buffer.is_empty());
        assert!(app.history.is_empty());
        assert!(app.last_result.is_none());
        assert_eq!(app.display, "0");
    }

    #[test]
    fn app_backspace() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_digit('2');
        app.push_digit('3');
        app.backspace();
        assert_eq!(app.input_buffer, "12");
        assert_eq!(app.display, "12");
    }

    #[test]
    fn app_backspace_to_empty() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('5');
        app.backspace();
        assert!(app.input_buffer.is_empty());
        assert_eq!(app.display, "0");
    }

    // -- Memory operations --

    #[test]
    fn app_memory_store_and_recall() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('4');
        app.push_digit('2');
        app.evaluate_input();
        app.memory_store();
        assert_eq!(app.memory, 42.0);
        app.clear();
        app.memory_recall();
        assert_eq!(app.input_buffer, "42");
    }

    #[test]
    fn app_memory_add() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_digit('0');
        app.evaluate_input();
        app.memory_store();
        app.push_digit('5');
        app.evaluate_input();
        app.memory_add();
        assert_eq!(app.memory, 15.0);
    }

    #[test]
    fn app_memory_clear() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('7');
        app.evaluate_input();
        app.memory_store();
        assert_eq!(app.memory, 7.0);
        app.memory_clear();
        assert_eq!(app.memory, 0.0);
    }

    // -- Display formatting --

    #[test]
    fn display_lines_have_no_title_heading() {
        // The app title shows in the WM / app-chrome title bar; the content
        // must not repeat it (it read as a double title bar in windows).
        let app = CalculatorApp::new("/apps/calc");
        let lines = app.format_display_lines();
        assert!(!lines.iter().any(|l| l.contains("Calculator")));
        assert!(lines.iter().any(|l| l.contains("Result: 0")));
    }

    #[test]
    fn display_lines_contain_memory() {
        let app = CalculatorApp::new("/apps/calc");
        let lines = app.format_display_lines();
        assert!(lines.iter().any(|l| l.contains("Memory: 0")));
    }

    #[test]
    fn display_lines_show_history() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_operator('+');
        app.push_digit('1');
        app.evaluate_input();
        let lines = app.format_display_lines();
        assert!(lines.iter().any(|l| l.contains("1+1 = 2")));
    }

    #[test]
    fn display_lines_show_input() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('3');
        app.push_operator('*');
        let lines = app.format_display_lines();
        assert!(lines.iter().any(|l| l.contains("Input: 3*")));
    }

    // -- Edge cases --

    #[test]
    fn no_multiple_leading_zeros() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('0');
        app.push_digit('0');
        app.push_digit('0');
        // Should remain "0" not "000".
        assert!(app.input_buffer == "0" || app.input_buffer.is_empty());
    }

    #[test]
    fn no_multiple_decimals_in_number() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_digit('.');
        app.push_digit('2');
        app.push_digit('.');
        app.push_digit('3');
        // Second dot should be rejected.
        assert_eq!(app.input_buffer, "1.23");
    }

    #[test]
    fn consecutive_operator_replacement() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('5');
        app.push_operator('+');
        app.push_operator('-');
        // Should replace + with -.
        assert_eq!(app.input_buffer, "5-");
    }

    #[test]
    fn negate_input() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('5');
        app.negate();
        assert_eq!(app.input_buffer, "-5");
        app.negate();
        assert_eq!(app.input_buffer, "5");
    }

    #[test]
    fn evaluate_empty_is_noop() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.evaluate_input();
        assert!(app.history.is_empty());
        assert!(app.last_result.is_none());
    }

    // -- App trait integration --

    #[test]
    fn cancel_exits() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn confirm_presses_key_under_cursor() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('7');
        app.cursor = keypad::index_of(CalcKey::Equals).expect("=");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.last_result, Some(7.0));
    }

    #[test]
    fn triangle_clears() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('9');
        app.handle_input(&Button::Triangle, &vfs);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn square_backspaces() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('4');
        app.push_digit('2');
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.input_buffer, "4");
    }

    #[test]
    fn start_clears_all() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_operator('+');
        app.push_digit('1');
        app.evaluate_input();
        app.handle_input(&Button::Start, &vfs);
        assert!(app.history.is_empty());
    }

    #[test]
    fn downcast_works() {
        let app = CalculatorApp::new("/apps/calc");
        let any = app.as_any();
        assert!(any.downcast_ref::<CalculatorApp>().is_some());
    }

    #[test]
    fn lines_returns_content() {
        let app = CalculatorApp::new("/apps/calc");
        assert!(!app.lines().is_empty());
    }

    #[test]
    fn eval_single_number() {
        assert_eq!(evaluate("42").ok(), Some(42.0));
    }

    #[test]
    fn eval_large_expression() {
        // 1 + 2 + 3 + ... + 10 = 55
        assert_eq!(evaluate("1+2+3+4+5+6+7+8+9+10").ok(), Some(55.0));
    }

    #[test]
    fn decimal_point_allowed_in_new_number_after_operator() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('1');
        app.push_digit('.');
        app.push_digit('5');
        app.push_operator('+');
        app.push_digit('2');
        app.push_digit('.');
        app.push_digit('5');
        assert_eq!(app.input_buffer, "1.5+2.5");
    }

    // -- Keypad cursor (gamepad) --

    #[test]
    fn dpad_moves_cursor_and_confirm_types_digits() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        // Cursor starts on 5. Right -> 6, Up -> 9, Left -> 8.
        app.handle_input(&Button::Confirm, &vfs);
        app.handle_input(&Button::Right, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        app.handle_input(&Button::Left, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.input_buffer, "5698");
        assert!(app.cursor_visible);
        // The d-pad no longer types digits by itself.
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.input_buffer, "5698");
    }

    #[test]
    fn every_key_reachable_with_dpad() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        let mut seen = vec![false; KEYS.len()];
        for _ in 0..keypad::ROWS {
            for _ in 0..keypad::COLS {
                seen[app.cursor] = true;
                app.handle_input(&Button::Right, &vfs);
            }
            app.handle_input(&Button::Down, &vfs);
        }
        assert!(seen.iter().all(|s| *s), "unreachable keys: {seen:?}");
    }

    #[test]
    fn memory_keys_via_keypad() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        let press = |app: &mut CalculatorApp, key: CalcKey| {
            app.cursor = keypad::index_of(key).expect("key on keypad");
            app.handle_input(&Button::Confirm, &vfs);
        };
        type_str(&mut app, "12");
        press(&mut app, CalcKey::MemStore);
        assert_eq!(app.memory, 12.0);
        press(&mut app, CalcKey::Clear);
        app.handle_text_input('3');
        press(&mut app, CalcKey::MemAdd);
        assert_eq!(app.memory, 15.0);
        press(&mut app, CalcKey::Clear);
        press(&mut app, CalcKey::MemRecall);
        assert_eq!(app.input_buffer, "15");
        press(&mut app, CalcKey::MemClear);
        assert_eq!(app.memory, 0.0);
        // Parentheses and negate are reachable too.
        press(&mut app, CalcKey::Clear);
        press(&mut app, CalcKey::OpenParen);
        app.handle_text_input('2');
        press(&mut app, CalcKey::CloseParen);
        press(&mut app, CalcKey::Negate);
        press(&mut app, CalcKey::Equals);
        assert_eq!(app.last_result, Some(-2.0));
    }

    // -- Keyboard typing --

    fn type_str(app: &mut CalculatorApp, s: &str) {
        for ch in s.chars() {
            app.handle_text_input(ch);
        }
    }

    #[test]
    fn typed_expression_evaluates() {
        let mut app = CalculatorApp::new("/apps/calc");
        assert!(app.accepts_text());
        type_str(&mut app, "(2+3)*4^2-10%4=");
        assert_eq!(app.last_result, Some(78.0));
        type_str(&mut app, "7/2");
        let vfs = make_vfs();
        assert_eq!(
            app.handle_key(&Key::Enter, Modifiers::NONE, &vfs),
            Some(AppAction::None)
        );
        assert_eq!(app.last_result, Some(3.5));
        assert_eq!(app.history.len(), 2);
    }

    #[test]
    fn typed_unary_minus() {
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "-3*-2=");
        assert_eq!(app.last_result, Some(6.0));
        // After a result, '-' subtracts from it.
        type_str(&mut app, "-1=");
        assert_eq!(app.last_result, Some(5.0));
    }

    #[test]
    fn typed_unknown_chars_ignored() {
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "1a+ b2");
        assert_eq!(app.input_buffer, "1+2");
    }

    #[test]
    fn backspace_and_escape_keys() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "123");
        app.handle_key(&Key::Backspace, Modifiers::NONE, &vfs);
        assert_eq!(app.input_buffer, "12");
        app.handle_backspace();
        assert_eq!(app.input_buffer, "1");
        // Escape clears the entry...
        assert_eq!(
            app.handle_key(&Key::Escape, Modifiers::NONE, &vfs),
            Some(AppAction::None)
        );
        assert!(app.input_buffer.is_empty());
        // ...and on a clear calculator falls through (Cancel -> Exit).
        assert_eq!(app.handle_key(&Key::Escape, Modifiers::NONE, &vfs), None);
        // Command shortcuts are left to the host.
        assert_eq!(app.handle_key(&Key::Char('c'), Modifiers::CTRL, &vfs), None);
    }

    #[test]
    fn up_down_recall_history() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "1+1=");
        type_str(&mut app, "2*3=");
        app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
        assert_eq!(app.input_buffer, "2*3");
        app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
        assert_eq!(app.input_buffer, "1+1");
        // Stays on the oldest entry.
        app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
        assert_eq!(app.input_buffer, "1+1");
        app.handle_key(&Key::Down, Modifiers::NONE, &vfs);
        assert_eq!(app.input_buffer, "2*3");
        app.handle_key(&Key::Down, Modifiers::NONE, &vfs);
        assert!(app.input_buffer.is_empty());
        // Gamepad Select recalls too.
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.input_buffer, "2*3");
        // Editing a recalled expression and evaluating adds a new entry.
        type_str(&mut app, "+1=");
        assert_eq!(app.last_result, Some(7.0));
        assert_eq!(app.history.len(), 3);
    }

    // -- Clicks --

    const CW: u32 = 300;
    const CH: u32 = 220;

    fn click_key(app: &mut CalculatorApp, key: CalcKey) {
        let l = CalcLayout::compute(0, 0, CW, CH);
        let r = l.key_rect(keypad::index_of(key).expect("key on keypad"));
        app.handle_click(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2, CW, CH, false);
    }

    #[test]
    fn clicking_keys_builds_and_evaluates() {
        let mut app = CalculatorApp::new("/apps/calc");
        for key in [
            CalcKey::Digit('9'),
            CalcKey::Op('*'),
            CalcKey::OpenParen,
            CalcKey::Digit('1'),
            CalcKey::Op('+'),
            CalcKey::Digit('2'),
            CalcKey::CloseParen,
        ] {
            click_key(&mut app, key);
        }
        assert_eq!(app.input_buffer, "9*(1+2)");
        click_key(&mut app, CalcKey::Equals);
        assert_eq!(app.last_result, Some(27.0));
        // The clicked key flashes pressed.
        let eq = keypad::index_of(CalcKey::Equals).expect("=");
        assert_eq!(app.flash.get().map(|(k, _)| k), Some(eq));
    }

    #[test]
    fn click_outside_keys_does_nothing() {
        let mut app = CalculatorApp::new("/apps/calc");
        let l = CalcLayout::compute(0, 0, CW, CH);
        app.handle_click(l.display.x + 5, l.display.y + 5, CW, CH, false);
        assert!(app.input_buffer.is_empty());
        assert!(app.flash.get().is_none());
    }

    #[test]
    fn click_history_row_recalls_entry() {
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "1+1=");
        type_str(&mut app, "2+2=");
        let (w, h) = (460, 240);
        let l = CalcLayout::compute(0, 0, w, h);
        let pane = l.history.expect("wide layout has history");
        // Rows are newest first: the second row is the older entry.
        let y = pane.y + (2 * keypad::HISTORY_ROW_H) as i32 + 2;
        app.handle_click(pane.x + 4, y, w, h, false);
        assert_eq!(app.input_buffer, "1+1");
    }

    // -- Rendering --

    #[test]
    fn windowed_draw_shows_keys_and_display() {
        use oasis_test_backend::{DrawCommand, RecordingBackend};
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "12+3");
        let mut backend = RecordingBackend::new(460, 240);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, 460, 240, &mut backend, &at)
            .expect("draw");
        let texts: Vec<&str> = backend
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        for label in ["MC", "MR", "MS", "M+", "7", "=", "DEL", "12+3", "History"] {
            assert!(texts.contains(&label), "missing {label:?} in {texts:?}");
        }
    }

    #[test]
    fn flash_expires_after_a_few_frames() {
        let mut app = CalculatorApp::new("/apps/calc");
        app.handle_text_input('4');
        assert!(app.flash.get().is_some());
        let mut backend = oasis_test_backend::RecordingBackend::new(300, 220);
        let at = ActiveTheme::default();
        for _ in 0..FLASH_FRAMES {
            app.draw_windowed(0, 0, 300, 220, &mut backend, &at)
                .expect("draw");
        }
        assert!(app.flash.get().is_none());
    }

    #[test]
    fn full_screen_lines_show_keypad_cursor() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        assert!(app.lines().iter().any(|l| l.contains("Key: [5]")));
        app.handle_input(&Button::Right, &vfs);
        assert!(app.lines().iter().any(|l| l.contains("Key: [6]")));
    }

    // -- History persistence --

    #[test]
    fn history_persists_across_instances() {
        let mut vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "6*7=");
        type_str(&mut app, "1/4=");
        assert!(app.apply_vfs_ops(&mut vfs));
        // Nothing left to write on the next frame.
        assert!(!app.apply_vfs_ops(&mut vfs));
        assert!(vfs.exists(history::HISTORY_PATH));

        let mut reopened = CalculatorApp::new("/apps/calc");
        reopened.refresh(&vfs);
        assert_eq!(reopened.history.len(), 2);
        assert_eq!(reopened.history[0].expression, "6*7");
        assert_eq!(reopened.history[1].result, 0.25);
        // Up recalls the persisted entries.
        reopened.handle_key(&Key::Up, Modifiers::NONE, &vfs);
        assert_eq!(reopened.input_buffer, "1/4");
    }

    #[test]
    fn history_loaded_after_typing_keeps_new_entries() {
        let mut vfs = make_vfs();
        let seed = CalcHistoryEntry {
            expression: "1+1".into(),
            result: 2.0,
        };
        history::save(&mut vfs, &[seed]).expect("seed history");
        let mut app = CalculatorApp::new("/apps/calc");
        // Typed before the host's first per-frame hook ran.
        type_str(&mut app, "3+3=");
        app.apply_vfs_ops(&mut vfs);
        let exprs: Vec<_> = app.history.iter().map(|e| e.expression.as_str()).collect();
        assert_eq!(exprs, ["1+1", "3+3"]);
        assert_eq!(history::load(&vfs).len(), 2);
    }

    #[test]
    fn all_clear_persists_empty_history() {
        let mut vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        type_str(&mut app, "2+2=");
        app.apply_vfs_ops(&mut vfs);
        app.clear_all();
        app.apply_vfs_ops(&mut vfs);
        assert!(history::load(&vfs).is_empty());
    }
}

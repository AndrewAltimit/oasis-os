//! Calculator application with expression evaluation.
//!
//! Provides a full-featured calculator with operator precedence, parentheses,
//! memory registers, and calculation history. The expression evaluator is a
//! pure recursive-descent parser with no external dependencies.

use std::any::Any;

use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

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

/// Calculator application state.
#[derive(Debug)]
pub struct CalculatorApp {
    content: ContentState,
    /// Current display value (formatted result or input echo).
    display: String,
    /// Expression being typed by the user.
    input_buffer: String,
    /// Previous calculations.
    history: Vec<CalcHistoryEntry>,
    /// Memory register (M+, MS, MR, MC).
    memory: f64,
    /// Result of the last successful evaluation.
    last_result: Option<f64>,
    /// Current error message (cleared on next input).
    error_message: Option<String>,
}

impl CalculatorApp {
    /// Create a new calculator app at the given VFS path.
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
        };
        app.refresh_lines();
        app
    }

    /// Append a digit or decimal point to the input buffer.
    pub fn push_digit(&mut self, d: char) {
        self.error_message = None;

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

    /// Append an opening or closing parenthesis.
    pub fn push_paren(&mut self, open: bool) {
        self.error_message = None;

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
        self.refresh_lines();
    }

    /// Clear input and history (AC).
    pub fn clear_all(&mut self) {
        self.input_buffer.clear();
        self.display = "0".to_string();
        self.history.clear();
        self.last_result = None;
        self.error_message = None;
        self.refresh_lines();
    }

    /// Delete the last character from the input buffer.
    pub fn backspace(&mut self) {
        self.error_message = None;
        self.input_buffer.pop();
        if self.input_buffer.is_empty() {
            self.display = "0".to_string();
        } else {
            self.display = self.input_buffer.clone();
        }
        self.refresh_lines();
    }

    /// Store the current display value to memory (MS).
    pub fn memory_store(&mut self) {
        if let Some(result) = self.last_result {
            self.memory = result;
        } else if let Ok(val) = self.input_buffer.parse::<f64>() {
            self.memory = val;
        }
        self.refresh_lines();
    }

    /// Recall memory value into the input buffer (MR).
    pub fn memory_recall(&mut self) {
        self.error_message = None;
        let mem_str = format_number(self.memory);
        self.input_buffer.push_str(&mem_str);
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Add the current display value to memory (M+).
    pub fn memory_add(&mut self) {
        if let Some(result) = self.last_result {
            self.memory += result;
        } else if let Ok(val) = self.input_buffer.parse::<f64>() {
            self.memory += val;
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

    /// Format the calculator state into display lines.
    pub fn format_display_lines(&self) -> Vec<String> {
        let separator = "\u{2500}".repeat(30); // box-drawing horizontal line

        let mut lines = Vec::new();
        lines.push("Calculator".to_string());
        lines.push(separator.clone());

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

        lines.push(separator.clone());

        // History section.
        if self.history.is_empty() {
            lines.push("  History: (empty)".to_string());
        } else {
            lines.push("  History:".to_string());
            // Show most recent entries last (up to 20).
            let start = self.history.len().saturating_sub(20);
            for entry in &self.history[start..] {
                lines.push(format!(
                    "    {} = {}",
                    entry.expression,
                    format_number(entry.result)
                ));
            }
        }

        lines.push(separator);

        // Memory.
        lines.push(format!("  Memory: {}", format_number(self.memory)));

        // Controls help.
        lines.push(String::new());
        lines.push("  [Confirm]=  [Triangle]=C  [Square]=BS".to_string());
        lines.push("  [Select]=Op  [Start]=AC  [D-pad]=Digits".to_string());

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

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => AppAction::Exit,

            // Confirm = evaluate (=)
            Button::Confirm => {
                self.evaluate_input();
                AppAction::None
            },

            // Triangle = clear (C)
            Button::Triangle => {
                self.clear();
                AppAction::None
            },

            // Square = backspace
            Button::Square => {
                self.backspace();
                AppAction::None
            },

            // Start = clear all (AC)
            Button::Start => {
                self.clear_all();
                AppAction::None
            },

            // D-pad: digit entry
            //   Up = 8, Down = 2, Left = 4, Right = 6
            Button::Up => {
                self.push_digit('8');
                AppAction::None
            },
            Button::Down => {
                self.push_digit('2');
                AppAction::None
            },
            Button::Left => {
                self.push_digit('4');
                AppAction::None
            },
            Button::Right => {
                self.push_digit('6');
                AppAction::None
            },

            // Select = cycle operators (+, -, *, /, ^, %)
            Button::Select => {
                let next_op = match self.input_buffer.chars().last() {
                    Some('+') => '-',
                    Some('-') if self.input_buffer.len() > 1 => '*',
                    Some('*') => '/',
                    Some('/') => '^',
                    Some('^') => '%',
                    Some('%') => '+',
                    _ => '+',
                };
                self.push_operator(next_op);
                AppAction::None
            },
        }
    }

    fn handle_click(
        &mut self,
        _lx: i32,
        _ly: i32,
        _cw: u32,
        _ch: u32,
        _fullscreen: bool,
    ) -> AppAction {
        AppAction::None
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
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
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
        assert_eq!(format_number(3.14), "3.14");
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
    fn display_lines_contain_calculator_title() {
        let app = CalculatorApp::new("/apps/calc");
        let lines = app.format_display_lines();
        assert!(lines.iter().any(|l| l.contains("Calculator")));
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
    fn confirm_evaluates() {
        let vfs = make_vfs();
        let mut app = CalculatorApp::new("/apps/calc");
        app.push_digit('7');
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
}

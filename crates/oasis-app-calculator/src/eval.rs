//! The expression evaluator: tokenizer + recursive-descent parser with
//! operator precedence, a recursion bound and finite-result checks.

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
    /// The result is infinite or not a number (`0^-1`, `9^999`, ...).
    #[error("Result is undefined or out of range")]
    NotFinite,
}

/// Maximum recursion depth for the expression parser.
const MAX_DEPTH: usize = 100;

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
            // Right-associative: recurse into parse_power. Each level
            // counts toward MAX_DEPTH so `2^2^2^...` can't overflow the
            // stack.
            self.enter()?;
            let exp = self.parse_power()?;
            self.leave();
            Ok(base.powf(exp))
        } else {
            Ok(base)
        }
    }

    /// Unary minus.
    fn parse_unary(&mut self) -> Result<f64, CalcError> {
        if let Some(Token::Minus) = self.peek() {
            self.advance();
            // Counted like a paren level so `-----...5` can't overflow
            // the stack.
            self.enter()?;
            let val = self.parse_unary()?;
            self.leave();
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

    evaluate_tokens(tokenize(trimmed)?)
}

/// Evaluate `rest` with the exact value `first` standing in front of it
/// (chaining from a previous result without re-parsing its rounded
/// display string).
pub(crate) fn evaluate_after(first: f64, rest: &str) -> Result<f64, CalcError> {
    let mut tokens = vec![Token::Number(first)];
    tokens.extend(tokenize(rest)?);
    evaluate_tokens(tokens)
}

/// Parse and evaluate a token list; non-finite results are errors (they
/// would display as `Infinity` / `NaN`, which can't be typed back in).
fn evaluate_tokens(tokens: Vec<Token>) -> Result<f64, CalcError> {
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

    if !result.is_finite() {
        return Err(CalcError::NotFinite);
    }
    Ok(result)
}

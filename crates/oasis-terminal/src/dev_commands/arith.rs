//! `seq` and `expr`, plus the arithmetic expression evaluator behind
//! `expr`.

use oasis_types::error::OasisError;

use crate::interpreter::CommandOutput;

// ---------------------------------------------------------------------------
// seq
// ---------------------------------------------------------------------------

define_command!(
    SeqCmd,
    "seq",
    "Print a sequence of numbers",
    "seq [start] <end> [step]",
    "dev",
    |args, _env| {
        let (start, end, step) = match args.len() {
            0 => {
                return Err(OasisError::Command(
                    "usage: seq [start] <end> [step]".into(),
                ));
            },
            1 => {
                let end: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                (1i64, end, 1i64)
            },
            2 => {
                let start: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                let end: i64 = args[1]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                (start, end, if start <= end { 1 } else { -1 })
            },
            _ => {
                let start: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                let end: i64 = args[1]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                let step: i64 = args[2]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".into()))?;
                if step == 0 {
                    return Err(OasisError::Command("step cannot be 0".into()));
                }
                (start, end, step)
            },
        };

        let mut result = Vec::new();
        let mut i = start;
        let limit = 10_000; // Safety limit.
        while (step > 0 && i <= end) || (step < 0 && i >= end) {
            result.push(i.to_string());
            i += step;
            if result.len() >= limit {
                break;
            }
        }
        Ok(CommandOutput::Text(result.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// expr
// ---------------------------------------------------------------------------

define_command!(
    ExprCmd,
    "expr",
    "Evaluate arithmetic expression",
    "expr <expression>",
    "dev",
    |args, _env| {
        if args.is_empty() {
            return Err(OasisError::Command("usage: expr <expression>".into()));
        }
        let expr_str = args.join(" ");
        match eval_expr(&expr_str) {
            Ok(val) => {
                if val == val.floor() && val.abs() < i64::MAX as f64 {
                    Ok(CommandOutput::Text(format!("{}", val as i64)))
                } else {
                    Ok(CommandOutput::Text(format!("{val}")))
                }
            },
            Err(e) => Err(OasisError::Command(format!("expr: {e}").into())),
        }
    }
);

/// Maximum nesting depth for parenthesised sub-expressions.
const EXPR_MAX_DEPTH: usize = 64;

/// Simple arithmetic expression evaluator supporting +, -, *, /, %, ().
fn eval_expr(input: &str) -> std::result::Result<f64, String> {
    let tokens = tokenize_expr(input)?;
    let mut pos = 0;
    let result = parse_add_sub(&tokens, &mut pos, 0)?;
    if pos < tokens.len() {
        return Err(format!("unexpected token: {}", tokens[pos]));
    }
    Ok(result)
}

fn tokenize_expr(input: &str) -> std::result::Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
        } else if ch.is_ascii_digit() || ch == '.' {
            let mut num = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    num.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(num);
        } else if "+-*/%()".contains(ch) {
            tokens.push(ch.to_string());
            chars.next();
        } else {
            return Err(format!("unexpected character: {ch}"));
        }
    }
    Ok(tokens)
}

fn parse_add_sub(
    tokens: &[String],
    pos: &mut usize,
    depth: usize,
) -> std::result::Result<f64, String> {
    let mut left = parse_mul_div(tokens, pos, depth)?;
    while *pos < tokens.len() && (tokens[*pos] == "+" || tokens[*pos] == "-") {
        let op = tokens[*pos].clone();
        *pos += 1;
        let right = parse_mul_div(tokens, pos, depth)?;
        if op == "+" {
            left += right;
        } else {
            left -= right;
        }
    }
    Ok(left)
}

fn parse_mul_div(
    tokens: &[String],
    pos: &mut usize,
    depth: usize,
) -> std::result::Result<f64, String> {
    let mut left = parse_unary(tokens, pos, depth)?;
    while *pos < tokens.len() && (tokens[*pos] == "*" || tokens[*pos] == "/" || tokens[*pos] == "%")
    {
        let op = tokens[*pos].clone();
        *pos += 1;
        let right = parse_unary(tokens, pos, depth)?;
        match op.as_str() {
            "*" => left *= right,
            "/" => {
                if right == 0.0 {
                    return Err("division by zero".to_string());
                }
                left /= right;
            },
            "%" => {
                if right == 0.0 {
                    return Err("division by zero".to_string());
                }
                left %= right;
            },
            _ => {},
        }
    }
    Ok(left)
}

fn parse_unary(
    tokens: &[String],
    pos: &mut usize,
    depth: usize,
) -> std::result::Result<f64, String> {
    if *pos < tokens.len() && tokens[*pos] == "-" {
        *pos += 1;
        let val = parse_primary(tokens, pos, depth)?;
        Ok(-val)
    } else {
        parse_primary(tokens, pos, depth)
    }
}

fn parse_primary(
    tokens: &[String],
    pos: &mut usize,
    depth: usize,
) -> std::result::Result<f64, String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of expression".to_string());
    }
    if tokens[*pos] == "(" {
        if depth >= EXPR_MAX_DEPTH {
            return Err("expression too deeply nested".to_string());
        }
        *pos += 1;
        let val = parse_add_sub(tokens, pos, depth + 1)?;
        if *pos >= tokens.len() || tokens[*pos] != ")" {
            return Err("missing closing parenthesis".to_string());
        }
        *pos += 1;
        return Ok(val);
    }
    let num: f64 = tokens[*pos]
        .parse()
        .map_err(|_| format!("expected number, got: {}", tokens[*pos]))?;
    *pos += 1;
    Ok(num)
}

register_commands!(register, [SeqCmd, ExprCmd]);

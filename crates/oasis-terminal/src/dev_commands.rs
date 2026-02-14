//! Developer tool commands: base64, json, uuid, seq, expr, test, xargs.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment};

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

struct Base64Cmd;
impl Command for Base64Cmd {
    fn name(&self) -> &str {
        "base64"
    }
    fn description(&self) -> &str {
        "Encode/decode base64"
    }
    fn usage(&self) -> &str {
        "base64 [-d] <text>"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let mut decode = false;
        let mut text_parts = Vec::new();
        for &arg in args {
            if arg == "-d" || arg == "--decode" {
                decode = true;
            } else {
                text_parts.push(arg);
            }
        }
        let input = if text_parts.is_empty() {
            env.stdin.clone().unwrap_or_default()
        } else {
            text_parts.join(" ")
        };

        if decode {
            match base64_decode(&input) {
                Ok(decoded) => Ok(CommandOutput::Text(decoded)),
                Err(e) => Err(OasisError::Command(format!("base64 decode error: {e}"))),
            }
        } else {
            Ok(CommandOutput::Text(base64_encode(input.as_bytes())))
        }
    }
}

const B64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(B64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(B64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(B64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(B64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(input: &str) -> std::result::Result<String, String> {
    let input = input.trim();
    let mut bytes = Vec::new();
    let chars: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r')
        .collect();

    for chunk in chars.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let vals: Vec<u32> = chunk
            .iter()
            .map(|&b| {
                if b == b'=' {
                    return Ok(0u32);
                }
                B64_CHARS
                    .iter()
                    .position(|&c| c == b)
                    .map(|p| p as u32)
                    .ok_or_else(|| format!("invalid base64 char: {}", b as char))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let triple = (vals[0] << 18)
            | (vals[1] << 12)
            | (vals.get(2).copied().unwrap_or(0) << 6)
            | vals.get(3).copied().unwrap_or(0);

        bytes.push(((triple >> 16) & 0xFF) as u8);
        if chunk.len() > 2 && chunk[2] != b'=' {
            bytes.push(((triple >> 8) & 0xFF) as u8);
        }
        if chunk.len() > 3 && chunk[3] != b'=' {
            bytes.push((triple & 0xFF) as u8);
        }
    }
    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8: {e}"))
}

// ---------------------------------------------------------------------------
// json
// ---------------------------------------------------------------------------

struct JsonCmd;
impl Command for JsonCmd {
    fn name(&self) -> &str {
        "json"
    }
    fn description(&self) -> &str {
        "Pretty-print or validate JSON"
    }
    fn usage(&self) -> &str {
        "json <text> | json validate <text>"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let input = if args.is_empty() {
            env.stdin.clone().unwrap_or_default()
        } else if args[0] == "validate" {
            args[1..].join(" ")
        } else {
            args.join(" ")
        };

        let validate_only = args.first() == Some(&"validate");

        // Simple JSON validation: check balanced braces/brackets and quotes.
        let trimmed = input.trim();
        let valid = is_valid_json(trimmed);

        if validate_only {
            if valid {
                Ok(CommandOutput::Text("Valid JSON".to_string()))
            } else {
                Err(OasisError::Command("Invalid JSON".to_string()))
            }
        } else if valid {
            // Simple pretty-print: add newlines and indentation.
            Ok(CommandOutput::Text(pretty_json(trimmed)))
        } else {
            Err(OasisError::Command("Invalid JSON".to_string()))
        }
    }
}

fn is_valid_json(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for ch in s.chars() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            },
            _ => {},
        }
    }
    depth == 0 && !in_string
}

fn pretty_json(s: &str) -> String {
    let mut result = String::new();
    let mut indent = 0usize;
    let mut in_string = false;
    let mut escape = false;

    for ch in s.chars() {
        if escape {
            result.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            result.push(ch);
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            result.push(ch);
            continue;
        }
        if in_string {
            result.push(ch);
            continue;
        }
        match ch {
            '{' | '[' => {
                indent += 1;
                result.push(ch);
                result.push('\n');
                push_indent(&mut result, indent);
            },
            '}' | ']' => {
                indent = indent.saturating_sub(1);
                result.push('\n');
                push_indent(&mut result, indent);
                result.push(ch);
            },
            ',' => {
                result.push(ch);
                result.push('\n');
                push_indent(&mut result, indent);
            },
            ':' => {
                result.push_str(": ");
            },
            c if c.is_whitespace() => {},
            _ => result.push(ch),
        }
    }
    result
}

fn push_indent(s: &mut String, level: usize) {
    for _ in 0..level {
        s.push_str("  ");
    }
}

// ---------------------------------------------------------------------------
// uuid
// ---------------------------------------------------------------------------

struct UuidCmd;
impl Command for UuidCmd {
    fn name(&self) -> &str {
        "uuid"
    }
    fn description(&self) -> &str {
        "Generate a pseudo-random UUID v4"
    }
    fn usage(&self) -> &str {
        "uuid"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        // Simple PRNG-based UUID v4 (not cryptographic).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let mut state = seed;
        let mut bytes = [0u8; 16];
        for b in &mut bytes {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (state >> 33) as u8;
        }
        // Set version (4) and variant (RFC 4122).
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        Ok(CommandOutput::Text(format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
            bytes[4],
            bytes[5],
            bytes[6],
            bytes[7],
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15],
        )))
    }
}

// ---------------------------------------------------------------------------
// seq
// ---------------------------------------------------------------------------

struct SeqCmd;
impl Command for SeqCmd {
    fn name(&self) -> &str {
        "seq"
    }
    fn description(&self) -> &str {
        "Print a sequence of numbers"
    }
    fn usage(&self) -> &str {
        "seq [start] <end> [step]"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let (start, end, step) = match args.len() {
            0 => {
                return Err(OasisError::Command(
                    "usage: seq [start] <end> [step]".to_string(),
                ));
            },
            1 => {
                let end: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                (1i64, end, 1i64)
            },
            2 => {
                let start: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                let end: i64 = args[1]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                (start, end, if start <= end { 1 } else { -1 })
            },
            _ => {
                let start: i64 = args[0]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                let end: i64 = args[1]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                let step: i64 = args[2]
                    .parse()
                    .map_err(|_| OasisError::Command("invalid number".to_string()))?;
                if step == 0 {
                    return Err(OasisError::Command("step cannot be 0".to_string()));
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
}

// ---------------------------------------------------------------------------
// expr
// ---------------------------------------------------------------------------

struct ExprCmd;
impl Command for ExprCmd {
    fn name(&self) -> &str {
        "expr"
    }
    fn description(&self) -> &str {
        "Evaluate arithmetic expression"
    }
    fn usage(&self) -> &str {
        "expr <expression>"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: expr <expression>".to_string()));
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
            Err(e) => Err(OasisError::Command(format!("expr: {e}"))),
        }
    }
}

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

// ---------------------------------------------------------------------------
// test
// ---------------------------------------------------------------------------

struct TestCmd;
impl Command for TestCmd {
    fn name(&self) -> &str {
        "test"
    }
    fn description(&self) -> &str {
        "Evaluate conditional expression"
    }
    fn usage(&self) -> &str {
        "test <expr> | test -f <file> | test -d <dir> | test <a> = <b>"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Ok(CommandOutput::Text("false".to_string()));
        }
        let result = match args[0] {
            "-f" => {
                let path = args
                    .get(1)
                    .ok_or_else(|| OasisError::Command("usage: test -f <file>".to_string()))?;
                let full = crate::interpreter::resolve_path(&env.cwd, path);
                env.vfs.exists(&full)
                    && env
                        .vfs
                        .stat(&full)
                        .map(|m| m.kind == oasis_vfs::EntryKind::File)
                        .unwrap_or(false)
            },
            "-d" => {
                let path = args
                    .get(1)
                    .ok_or_else(|| OasisError::Command("usage: test -d <dir>".to_string()))?;
                let full = crate::interpreter::resolve_path(&env.cwd, path);
                env.vfs.exists(&full)
                    && env
                        .vfs
                        .stat(&full)
                        .map(|m| m.kind == oasis_vfs::EntryKind::Directory)
                        .unwrap_or(false)
            },
            "-n" => args.get(1).is_some_and(|s| !s.is_empty()),
            "-z" => args.get(1).is_none_or(|s| s.is_empty()),
            _ => {
                if args.len() >= 3 {
                    match args[1] {
                        "=" | "==" => args[0] == args[2],
                        "!=" => args[0] != args[2],
                        "-eq" => {
                            let a: i64 = args[0].parse().unwrap_or(0);
                            let b: i64 = args[2].parse().unwrap_or(0);
                            a == b
                        },
                        "-ne" => {
                            let a: i64 = args[0].parse().unwrap_or(0);
                            let b: i64 = args[2].parse().unwrap_or(0);
                            a != b
                        },
                        "-lt" => {
                            let a: i64 = args[0].parse().unwrap_or(0);
                            let b: i64 = args[2].parse().unwrap_or(0);
                            a < b
                        },
                        "-gt" => {
                            let a: i64 = args[0].parse().unwrap_or(0);
                            let b: i64 = args[2].parse().unwrap_or(0);
                            a > b
                        },
                        _ => !args[0].is_empty(),
                    }
                } else {
                    !args[0].is_empty()
                }
            },
        };
        Ok(CommandOutput::Text(
            if result { "true" } else { "false" }.to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// xargs
// ---------------------------------------------------------------------------

struct XargsCmd;
impl Command for XargsCmd {
    fn name(&self) -> &str {
        "xargs"
    }
    fn description(&self) -> &str {
        "Build command from stdin lines"
    }
    fn usage(&self) -> &str {
        "xargs <command>"
    }
    fn category(&self) -> &str {
        "dev"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: xargs <command>".to_string()));
        }
        let base_cmd = args.join(" ");
        let input = env.stdin.clone().unwrap_or_default();
        let items: Vec<&str> = input
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        if items.is_empty() {
            return Ok(CommandOutput::Text("(no input for xargs)".to_string()));
        }

        // Build the expanded command line.
        let full_cmd = format!("{base_cmd} {}", items.join(" "));
        Ok(CommandOutput::Text(format!(
            "xargs: would execute: {full_cmd}"
        )))
    }
}

/// Register developer tool commands.
pub fn register_dev_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(Base64Cmd));
    reg.register(Box::new(JsonCmd));
    reg.register(Box::new(UuidCmd));
    reg.register(Box::new(SeqCmd));
    reg.register(Box::new(ExprCmd));
    reg.register(Box::new(TestCmd));
    reg.register(Box::new(XargsCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn base64_encode() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "base64 hello").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "aGVsbG8="),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn base64_decode() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "base64 -d aGVsbG8=").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn json_validate_valid() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, r#"json validate {"key":"value"}"#).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("Valid")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn json_validate_invalid() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "json validate {bad").is_err());
    }

    #[test]
    fn uuid_format() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "uuid").unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s.len(), 36);
                assert_eq!(s.matches('-').count(), 4);
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn seq_basic() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "seq 5").unwrap() {
            CommandOutput::Text(s) => {
                let lines: Vec<&str> = s.lines().collect();
                assert_eq!(lines, ["1", "2", "3", "4", "5"]);
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn seq_range() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "seq 3 6").unwrap() {
            CommandOutput::Text(s) => {
                let lines: Vec<&str> = s.lines().collect();
                assert_eq!(lines, ["3", "4", "5", "6"]);
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn expr_arithmetic() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "expr 2 + 3 * 4").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "14"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn expr_parens() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "expr (2 + 3) * 4").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "20"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_file_exists() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"data").unwrap();
        match exec(&reg, &mut vfs, "test -f /test.txt").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "true"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_file_missing() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "test -f /nope.txt").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "false"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_dir() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        match exec(&reg, &mut vfs, "test -d /tmp").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "true"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_string_eq() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "test foo = foo").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "true"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_numeric_lt() {
        let mut reg = CommandRegistry::new();
        register_dev_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "test 3 -lt 5").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "true"),
            _ => panic!("expected text"),
        }
    }
}

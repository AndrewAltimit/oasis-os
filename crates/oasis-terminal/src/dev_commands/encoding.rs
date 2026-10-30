//! `base64`, `json` and `uuid`.

use oasis_types::error::OasisError;
use oasis_types::util::base64;

use crate::cmd_helpers::time_seed;
use crate::interpreter::CommandOutput;

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

define_command!(
    Base64Cmd,
    "base64",
    "Encode/decode base64",
    "base64 [-d] <text>",
    "dev",
    |args, env| {
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
                Err(e) => Err(OasisError::Command(
                    format!("base64 decode error: {e}").into(),
                )),
            }
        } else {
            Ok(CommandOutput::Text(base64::encode(input.as_bytes())))
        }
    }
);

fn base64_decode(input: &str) -> std::result::Result<String, String> {
    let bytes = base64::decode(input).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8: {e}"))
}

// ---------------------------------------------------------------------------
// json
// ---------------------------------------------------------------------------

define_command!(
    JsonCmd,
    "json",
    "Pretty-print or validate JSON",
    "json <text> | json validate <text>",
    "dev",
    |args, env| {
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
                Err(OasisError::Command("Invalid JSON".into()))
            }
        } else if valid {
            // Simple pretty-print: add newlines and indentation.
            Ok(CommandOutput::Text(pretty_json(trimmed)))
        } else {
            Err(OasisError::Command("Invalid JSON".into()))
        }
    }
);

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

define_command!(
    UuidCmd,
    "uuid",
    "Generate a pseudo-random UUID v4",
    "uuid",
    "dev",
    |_args, env| {
        // Simple PRNG-based UUID v4 (not cryptographic).
        let seed = time_seed(env);

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
);

register_commands!(register, [Base64Cmd, JsonCmd, UuidCmd]);

//! `@font-face` and `@counter-style` descriptor value parsers.
//!
//! Includes the [`unquote`] helper used by both at-rule families to
//! strip surrounding quotes from string-typed descriptor values.

use super::types;

/// Strip surrounding `"…"` or `'…'` quotes from a single-token value.
pub(super) fn unquote(s: &str) -> String {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && let (Some(&first), Some(&last)) = (bytes.first(), bytes.last())
        && (first == b'"' || first == b'\'')
        && first == last
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Parse a `font-weight` descriptor value for `@font-face`.
///
/// Supports single values (`400`, `bold`) and ranges (`100 900`).
pub(super) fn parse_font_weight_descriptor(raw: &str) -> (u16, u16) {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let parse_one = |s: &str| -> u16 {
        match s.to_ascii_lowercase().as_str() {
            "normal" => 400,
            "bold" => 700,
            _ => s.parse::<u16>().unwrap_or(400).clamp(1, 1000),
        }
    };
    if parts.len() >= 2 {
        let lo = parse_one(parts[0]);
        let hi = parse_one(parts[1]);
        (lo.min(hi), lo.max(hi))
    } else if let Some(v) = parts.first() {
        let w = parse_one(v);
        (w, w)
    } else {
        (400, 400)
    }
}

/// Parse a comma-separated `unicode-range` descriptor value.
///
/// Supports `U+XXXX`, `U+XXXX-YYYY`, and `U+XX??` wildcard forms.
pub(super) fn parse_unicode_range_list(raw: &str) -> Vec<types::UnicodeRange> {
    let mut ranges = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if let Some(r) = parse_single_unicode_range(part) {
            ranges.push(r);
        }
    }
    ranges
}

/// Parse a single Unicode range like `U+0020-007F` or `U+4?`.
fn parse_single_unicode_range(s: &str) -> Option<types::UnicodeRange> {
    let s = s.trim();
    // Must start with U+ or u+
    let hex_part = s.strip_prefix("U+").or_else(|| s.strip_prefix("u+"))?;

    if let Some((start_s, end_s)) = hex_part.split_once('-') {
        // Range form: U+XXXX-YYYY
        let start = u32::from_str_radix(start_s.trim(), 16).ok()?;
        let end = u32::from_str_radix(end_s.trim(), 16).ok()?;
        Some(types::UnicodeRange { start, end })
    } else if hex_part.contains('?') {
        // Wildcard form: U+4?  →  U+40-4F, U+4?? → U+400-4FF
        let lo = hex_part.replace('?', "0");
        let hi = hex_part.replace('?', "F");
        let start = u32::from_str_radix(&lo, 16).ok()?;
        let end = u32::from_str_radix(&hi, 16).ok()?;
        Some(types::UnicodeRange { start, end })
    } else {
        // Single codepoint: U+XXXX
        let cp = u32::from_str_radix(hex_part.trim(), 16).ok()?;
        Some(types::UnicodeRange { start: cp, end: cp })
    }
}

/// Split a `symbols` descriptor value into individual entries, treating
/// quoted strings as atomic and unquoted runs as bare identifiers.
pub(super) fn split_counter_symbols(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str: Option<char> = None;
    for ch in raw.chars() {
        if let Some(q) = in_str {
            if ch == q {
                in_str = None;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
                in_str = Some(ch);
            },
            c if c.is_whitespace() => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                    cur.clear();
                }
            },
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Parse an `additive-symbols` descriptor: comma-separated `<weight> <symbol>`
/// pairs. Symbols can be quoted strings or bare idents.
pub(super) fn parse_additive_symbols(raw: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // First token is the weight; remainder is the symbol.
        let (weight_s, sym_s) = match part.split_once(char::is_whitespace) {
            Some(p) => p,
            None => continue,
        };
        let Ok(weight) = weight_s.trim().parse::<i32>() else {
            continue;
        };
        let symbols = split_counter_symbols(sym_s);
        if let Some(first) = symbols.into_iter().next() {
            out.push((weight, first));
        }
    }
    out
}

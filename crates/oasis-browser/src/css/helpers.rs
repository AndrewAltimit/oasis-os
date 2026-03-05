//! CSS helper functions for colour parsing, unit parsing, media-query
//! evaluation, and CSS text reconstruction.
//!
//! Extracted from [`super::parser`] to keep the main parser module focused
//! on token-stream consumption and AST construction.

use super::parser::{CssColor, CssValue, LengthUnit};
use super::tokenizer::CssToken;

// Re-import `parse_value_list` so `parse_font_weight` can call it.
use super::parser::parse_value_list;

// -------------------------------------------------------------------
// Font-weight helper
// -------------------------------------------------------------------

pub(crate) fn parse_font_weight(tokens: &[CssToken]) -> CssValue {
    let non_ws: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, CssToken::Whitespace))
        .collect();
    if non_ws.len() == 1 {
        match &non_ws[0] {
            CssToken::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                return match lower.as_str() {
                    "bold" => CssValue::Number(700.0),
                    "normal" => CssValue::Number(400.0),
                    "lighter" => CssValue::Number(100.0),
                    "bolder" => CssValue::Number(900.0),
                    _ => CssValue::Keyword(s.clone()),
                };
            },
            CssToken::Number(n) => return CssValue::Number(*n),
            _ => {},
        }
    }
    let values = parse_value_list(tokens);
    if values.len() == 1 {
        values.into_iter().next().expect("len checked")
    } else {
        CssValue::Multiple(values)
    }
}

// -------------------------------------------------------------------
// Property classification
// -------------------------------------------------------------------

pub(crate) fn is_color_property(prop: &str) -> bool {
    matches!(
        prop,
        "color"
            | "background-color"
            | "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "outline-color"
    )
}

// -------------------------------------------------------------------
// Unit parsing
// -------------------------------------------------------------------

pub(crate) fn parse_unit(unit: &str) -> Option<LengthUnit> {
    match unit.to_ascii_lowercase().as_str() {
        "px" => Some(LengthUnit::Px),
        "em" => Some(LengthUnit::Em),
        "rem" => Some(LengthUnit::Rem),
        "pt" => Some(LengthUnit::Pt),
        _ => None,
    }
}

// -------------------------------------------------------------------
// Colour parsing
// -------------------------------------------------------------------

pub(crate) fn try_parse_color(tokens: &[CssToken]) -> Option<CssColor> {
    let non_ws: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, CssToken::Whitespace))
        .collect();
    if non_ws.is_empty() {
        return None;
    }

    // Single hash: #rgb / #rrggbb / #rgba / #rrggbbaa.
    if non_ws.len() == 1 {
        if let CssToken::Hash(h) = non_ws[0] {
            return parse_hex_color(h);
        }
        if let CssToken::Ident(name) = non_ws[0] {
            return named_color(name);
        }
    }

    // rgb() / rgba().
    if let CssToken::Function(name) = non_ws[0] {
        let lower = name.to_ascii_lowercase();
        if lower == "rgb" || lower == "rgba" {
            return parse_rgb_function(&non_ws[1..]);
        }
    }

    None
}

pub(crate) fn parse_hex_color(hex: &str) -> Option<CssColor> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = hex_digit(hex.as_bytes()[0])?;
            let g = hex_digit(hex.as_bytes()[1])?;
            let b = hex_digit(hex.as_bytes()[2])?;
            Some(CssColor::new(r << 4 | r, g << 4 | g, b << 4 | b, 255))
        },
        4 => {
            let r = hex_digit(hex.as_bytes()[0])?;
            let g = hex_digit(hex.as_bytes()[1])?;
            let b = hex_digit(hex.as_bytes()[2])?;
            let a = hex_digit(hex.as_bytes()[3])?;
            Some(CssColor::new(
                r << 4 | r,
                g << 4 | g,
                b << 4 | b,
                a << 4 | a,
            ))
        },
        6 => {
            let r = hex_byte(&hex[0..2])?;
            let g = hex_byte(&hex[2..4])?;
            let b = hex_byte(&hex[4..6])?;
            Some(CssColor::new(r, g, b, 255))
        },
        8 => {
            let r = hex_byte(&hex[0..2])?;
            let g = hex_byte(&hex[2..4])?;
            let b = hex_byte(&hex[4..6])?;
            let a = hex_byte(&hex[6..8])?;
            Some(CssColor::new(r, g, b, a))
        },
        _ => None,
    }
}

pub(crate) fn hex_digit(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn hex_byte(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

pub(crate) fn parse_rgb_function(tokens: &[&CssToken]) -> Option<CssColor> {
    let numbers: Vec<f32> = tokens
        .iter()
        .filter_map(|t| match t {
            CssToken::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    if numbers.len() >= 3 {
        let r = numbers[0].clamp(0.0, 255.0) as u8;
        let g = numbers[1].clamp(0.0, 255.0) as u8;
        let b = numbers[2].clamp(0.0, 255.0) as u8;
        let a = if numbers.len() >= 4 {
            (numbers[3].clamp(0.0, 1.0) * 255.0) as u8
        } else {
            255
        };
        Some(CssColor::new(r, g, b, a))
    } else {
        None
    }
}

pub(crate) fn named_color(name: &str) -> Option<CssColor> {
    match name.to_ascii_lowercase().as_str() {
        "black" => Some(CssColor::new(0, 0, 0, 255)),
        "white" => Some(CssColor::new(255, 255, 255, 255)),
        "red" => Some(CssColor::new(255, 0, 0, 255)),
        "green" => Some(CssColor::new(0, 128, 0, 255)),
        "blue" => Some(CssColor::new(0, 0, 255, 255)),
        "yellow" => Some(CssColor::new(255, 255, 0, 255)),
        "cyan" | "aqua" => Some(CssColor::new(0, 255, 255, 255)),
        "magenta" | "fuchsia" => Some(CssColor::new(255, 0, 255, 255)),
        "orange" => Some(CssColor::new(255, 165, 0, 255)),
        "purple" => Some(CssColor::new(128, 0, 128, 255)),
        "gray" | "grey" => Some(CssColor::new(128, 128, 128, 255)),
        "lime" => Some(CssColor::new(0, 255, 0, 255)),
        "navy" => Some(CssColor::new(0, 0, 128, 255)),
        "teal" => Some(CssColor::new(0, 128, 128, 255)),
        "maroon" => Some(CssColor::new(128, 0, 0, 255)),
        "olive" => Some(CssColor::new(128, 128, 0, 255)),
        "silver" => Some(CssColor::new(192, 192, 192, 255)),
        "transparent" => Some(CssColor::new(0, 0, 0, 0)),
        "pink" => Some(CssColor::new(255, 192, 203, 255)),
        "brown" => Some(CssColor::new(165, 42, 42, 255)),
        "coral" => Some(CssColor::new(255, 127, 80, 255)),
        "gold" => Some(CssColor::new(255, 215, 0, 255)),
        _ => None,
    }
}

// -------------------------------------------------------------------
// Media query evaluation
// -------------------------------------------------------------------

/// Evaluate a simplified media query against the OASIS viewport.
///
/// Supports: `screen`, `all`, `not print`, `(max-width: Xpx)`,
/// `(min-width: Xpx)`, and comma-separated alternatives.
/// The viewport is hardcoded to 480x272 (PSP native resolution).
pub(crate) fn eval_media_query(query: &str) -> bool {
    const VIEWPORT_WIDTH: f32 = 480.0;

    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    // Comma-separated: any match means true.
    for part in query.split(',') {
        if eval_single_media_query(part.trim(), VIEWPORT_WIDTH) {
            return true;
        }
    }
    false
}

pub(crate) fn eval_single_media_query(query: &str, viewport_width: f32) -> bool {
    let query = query.trim();
    if query.is_empty() || query == "all" || query == "screen" {
        return true;
    }
    if query == "print" || query == "not screen" {
        return false;
    }
    if let Some(rest) = query.strip_prefix("not ") {
        return !eval_single_media_query(rest, viewport_width);
    }
    // Handle compound conditions like "screen and (max-width: 600px)".
    // Split on " and " and evaluate each part.
    let parts: Vec<&str> = query.split(" and ").collect();
    for part in &parts {
        let p = part.trim();
        // "only" is a CSS3 modifier for backwards compat; strip it.
        let p = p.strip_prefix("only ").unwrap_or(p);
        if p == "screen" || p == "all" || p.is_empty() {
            continue;
        }
        if p == "print" {
            return false;
        }
        // Parenthesized feature: (max-width: 600px), (min-width: 320px)
        let inner = p.trim_start_matches('(').trim_end_matches(')').trim();
        if let Some(rest) = inner.strip_prefix("max-width:") {
            let px = parse_px_value(rest.trim());
            if viewport_width > px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("min-width:") {
            let px = parse_px_value(rest.trim());
            if viewport_width < px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("prefers-color-scheme:") {
            // We always use light mode.
            if rest.trim() != "light" {
                return false;
            }
        } else {
            // Unknown features: treat as NOT matching (safe default).
            return false;
        }
    }
    true
}

/// Parse a pixel value like "600px" or "600" from a media query.
pub(crate) fn parse_px_value(s: &str) -> f32 {
    let s = s.trim().trim_end_matches("px");
    s.parse::<f32>().unwrap_or(0.0)
}

// -------------------------------------------------------------------
// CSS text reconstruction
// -------------------------------------------------------------------

/// Reconstruct CSS text from a token stream.
///
/// Used to store custom property values and `var()` fallback text as
/// raw strings that can be re-tokenized later during cascade resolution.
pub(crate) fn tokens_to_css_text(tokens: &[CssToken]) -> String {
    let mut out = String::new();
    for tok in tokens {
        match tok {
            CssToken::Ident(s) => out.push_str(s),
            CssToken::Hash(s) => {
                out.push('#');
                out.push_str(s);
            },
            CssToken::String(s) => {
                out.push('"');
                out.push_str(s);
                out.push('"');
            },
            CssToken::Number(n) => out.push_str(&format!("{n}")),
            CssToken::Percentage(n) => {
                out.push_str(&format!("{n}"));
                out.push('%');
            },
            CssToken::Dimension(n, u) => {
                out.push_str(&format!("{n}"));
                out.push_str(u);
            },
            CssToken::Colon => out.push(':'),
            CssToken::Semicolon => out.push(';'),
            CssToken::Comma => out.push(','),
            CssToken::OpenBrace => out.push('{'),
            CssToken::CloseBrace => out.push('}'),
            CssToken::OpenParen => out.push('('),
            CssToken::CloseParen => out.push(')'),
            CssToken::OpenBracket => out.push('['),
            CssToken::CloseBracket => out.push(']'),
            CssToken::Dot => out.push('.'),
            CssToken::Greater => out.push('>'),
            CssToken::Plus => out.push('+'),
            CssToken::Star => out.push('*'),
            CssToken::Slash => out.push('/'),
            CssToken::Delim(c) => out.push(*c),
            CssToken::Whitespace => out.push(' '),
            CssToken::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            },
            CssToken::Function(s) => {
                out.push_str(s);
                out.push('(');
            },
            CssToken::Eof => {},
        }
    }
    out.trim().to_string()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // hex_digit
    // ---------------------------------------------------------------

    #[test]
    fn hex_digit_decimal() {
        for (ch, expected) in (b'0'..=b'9').zip(0u8..=9) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_lowercase() {
        for (ch, expected) in (b'a'..=b'f').zip(10u8..=15) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_uppercase() {
        for (ch, expected) in (b'A'..=b'F').zip(10u8..=15) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_invalid() {
        assert_eq!(hex_digit(b'g'), None);
        assert_eq!(hex_digit(b'G'), None);
        assert_eq!(hex_digit(b' '), None);
        assert_eq!(hex_digit(b'z'), None);
    }

    // ---------------------------------------------------------------
    // hex_byte
    // ---------------------------------------------------------------

    #[test]
    fn hex_byte_valid() {
        assert_eq!(hex_byte("ff"), Some(255));
        assert_eq!(hex_byte("00"), Some(0));
        assert_eq!(hex_byte("7f"), Some(127));
        assert_eq!(hex_byte("FF"), Some(255));
        assert_eq!(hex_byte("a0"), Some(160));
    }

    #[test]
    fn hex_byte_invalid() {
        assert_eq!(hex_byte("zz"), None);
        assert_eq!(hex_byte(""), None);
        assert_eq!(hex_byte("gg"), None);
    }

    // ---------------------------------------------------------------
    // parse_hex_color
    // ---------------------------------------------------------------

    #[test]
    fn hex_color_3_digit() {
        let c = parse_hex_color("f00").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn hex_color_3_digit_with_hash() {
        let c = parse_hex_color("#abc").unwrap();
        assert_eq!(c, CssColor::new(0xaa, 0xbb, 0xcc, 255));
    }

    #[test]
    fn hex_color_4_digit_rgba() {
        let c = parse_hex_color("f008").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 0x88));
    }

    #[test]
    fn hex_color_6_digit() {
        let c = parse_hex_color("ff8000").unwrap();
        assert_eq!(c, CssColor::new(255, 128, 0, 255));
    }

    #[test]
    fn hex_color_6_digit_with_hash() {
        let c = parse_hex_color("#336699").unwrap();
        assert_eq!(c, CssColor::new(0x33, 0x66, 0x99, 255));
    }

    #[test]
    fn hex_color_8_digit_rgba() {
        let c = parse_hex_color("ff000080").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 128));
    }

    #[test]
    fn hex_color_invalid_length() {
        assert!(parse_hex_color("f").is_none());
        assert!(parse_hex_color("ff").is_none());
        assert!(parse_hex_color("fffff").is_none());
        assert!(parse_hex_color("fffffff").is_none());
        assert!(parse_hex_color("fffffffff").is_none());
    }

    #[test]
    fn hex_color_invalid_chars() {
        assert!(parse_hex_color("xyz").is_none());
        assert!(parse_hex_color("gggggg").is_none());
    }

    // ---------------------------------------------------------------
    // named_color
    // ---------------------------------------------------------------

    #[test]
    fn named_color_basic() {
        assert_eq!(named_color("black"), Some(CssColor::new(0, 0, 0, 255)));
        assert_eq!(
            named_color("white"),
            Some(CssColor::new(255, 255, 255, 255))
        );
        assert_eq!(named_color("red"), Some(CssColor::new(255, 0, 0, 255)));
    }

    #[test]
    fn named_color_case_insensitive() {
        assert_eq!(named_color("BLACK"), Some(CssColor::new(0, 0, 0, 255)));
        assert_eq!(named_color("Red"), Some(CssColor::new(255, 0, 0, 255)));
    }

    #[test]
    fn named_color_aliases() {
        // cyan == aqua
        assert_eq!(named_color("cyan"), named_color("aqua"));
        // magenta == fuchsia
        assert_eq!(named_color("magenta"), named_color("fuchsia"));
        // gray == grey
        assert_eq!(named_color("gray"), named_color("grey"));
    }

    #[test]
    fn named_color_transparent() {
        assert_eq!(named_color("transparent"), Some(CssColor::new(0, 0, 0, 0)));
    }

    #[test]
    fn named_color_unknown() {
        assert!(named_color("chartreuse").is_none());
        assert!(named_color("").is_none());
        assert!(named_color("notacolor").is_none());
    }

    // ---------------------------------------------------------------
    // parse_rgb_function
    // ---------------------------------------------------------------

    #[test]
    fn rgb_function_three_args() {
        let tokens = [
            CssToken::Number(100.0),
            CssToken::Comma,
            CssToken::Number(200.0),
            CssToken::Comma,
            CssToken::Number(50.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(100, 200, 50, 255));
    }

    #[test]
    fn rgba_function_four_args() {
        let tokens = [
            CssToken::Number(255.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(128.0),
            CssToken::Comma,
            CssToken::Number(0.5),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 128, 127));
    }

    #[test]
    fn rgb_function_clamped() {
        let tokens = [
            CssToken::Number(300.0),
            CssToken::Comma,
            CssToken::Number(-10.0),
            CssToken::Comma,
            CssToken::Number(128.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 128, 255));
    }

    #[test]
    fn rgb_function_too_few_args() {
        let tokens = [
            CssToken::Number(100.0),
            CssToken::Comma,
            CssToken::Number(200.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        assert!(parse_rgb_function(&refs).is_none());
    }

    #[test]
    fn rgb_function_empty() {
        let refs: Vec<&CssToken> = vec![];
        assert!(parse_rgb_function(&refs).is_none());
    }

    // ---------------------------------------------------------------
    // try_parse_color
    // ---------------------------------------------------------------

    #[test]
    fn try_parse_color_hex() {
        let tokens = [CssToken::Hash("ff0000".into())];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn try_parse_color_named() {
        let tokens = [CssToken::Ident("blue".into())];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 0, 255, 255));
    }

    #[test]
    fn try_parse_color_rgb_function() {
        let tokens = [
            CssToken::Function("rgb".into()),
            CssToken::Number(10.0),
            CssToken::Comma,
            CssToken::Number(20.0),
            CssToken::Comma,
            CssToken::Number(30.0),
            CssToken::CloseParen,
        ];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(10, 20, 30, 255));
    }

    #[test]
    fn try_parse_color_rgba_function() {
        let tokens = [
            CssToken::Function("RGBA".into()),
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 0, 0, 0));
    }

    #[test]
    fn try_parse_color_empty() {
        assert!(try_parse_color(&[]).is_none());
    }

    #[test]
    fn try_parse_color_whitespace_only() {
        let tokens = [CssToken::Whitespace, CssToken::Whitespace];
        assert!(try_parse_color(&tokens).is_none());
    }

    #[test]
    fn try_parse_color_unknown_function() {
        let tokens = [
            CssToken::Function("hsl".into()),
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        assert!(try_parse_color(&tokens).is_none());
    }

    // ---------------------------------------------------------------
    // parse_unit
    // ---------------------------------------------------------------

    #[test]
    fn parse_unit_known() {
        assert_eq!(parse_unit("px"), Some(LengthUnit::Px));
        assert_eq!(parse_unit("em"), Some(LengthUnit::Em));
        assert_eq!(parse_unit("rem"), Some(LengthUnit::Rem));
        assert_eq!(parse_unit("pt"), Some(LengthUnit::Pt));
    }

    #[test]
    fn parse_unit_case_insensitive() {
        assert_eq!(parse_unit("PX"), Some(LengthUnit::Px));
        assert_eq!(parse_unit("Em"), Some(LengthUnit::Em));
        assert_eq!(parse_unit("REM"), Some(LengthUnit::Rem));
    }

    #[test]
    fn parse_unit_unknown() {
        assert_eq!(parse_unit("vh"), None);
        assert_eq!(parse_unit("vw"), None);
        assert_eq!(parse_unit(""), None);
        assert_eq!(parse_unit("cm"), None);
    }

    // ---------------------------------------------------------------
    // is_color_property
    // ---------------------------------------------------------------

    #[test]
    fn is_color_property_true() {
        assert!(is_color_property("color"));
        assert!(is_color_property("background-color"));
        assert!(is_color_property("border-color"));
        assert!(is_color_property("border-top-color"));
        assert!(is_color_property("border-right-color"));
        assert!(is_color_property("border-bottom-color"));
        assert!(is_color_property("border-left-color"));
        assert!(is_color_property("outline-color"));
    }

    #[test]
    fn is_color_property_false() {
        assert!(!is_color_property("background"));
        assert!(!is_color_property("border"));
        assert!(!is_color_property("font-size"));
        assert!(!is_color_property(""));
        assert!(!is_color_property("Color"));
    }

    // ---------------------------------------------------------------
    // parse_font_weight
    // ---------------------------------------------------------------

    #[test]
    fn font_weight_bold() {
        let tokens = [CssToken::Ident("bold".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    #[test]
    fn font_weight_normal() {
        let tokens = [CssToken::Ident("normal".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(400.0));
    }

    #[test]
    fn font_weight_lighter() {
        let tokens = [CssToken::Ident("lighter".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(100.0));
    }

    #[test]
    fn font_weight_bolder() {
        let tokens = [CssToken::Ident("bolder".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(900.0));
    }

    #[test]
    fn font_weight_case_insensitive() {
        let tokens = [CssToken::Ident("BOLD".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    #[test]
    fn font_weight_numeric() {
        let tokens = [CssToken::Number(600.0)];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(600.0));
    }

    #[test]
    fn font_weight_unknown_keyword() {
        let tokens = [CssToken::Ident("fancy".into())];
        assert_eq!(
            parse_font_weight(&tokens),
            CssValue::Keyword("fancy".into())
        );
    }

    #[test]
    fn font_weight_with_whitespace() {
        let tokens = [
            CssToken::Whitespace,
            CssToken::Ident("bold".into()),
            CssToken::Whitespace,
        ];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    // ---------------------------------------------------------------
    // parse_px_value
    // ---------------------------------------------------------------

    #[test]
    fn parse_px_value_with_unit() {
        assert!((parse_px_value("600px") - 600.0).abs() < f32::EPSILON);
        assert!((parse_px_value("320px") - 320.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_without_unit() {
        assert!((parse_px_value("480") - 480.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_with_whitespace() {
        assert!((parse_px_value("  100px  ") - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_invalid() {
        assert!((parse_px_value("abc") - 0.0).abs() < f32::EPSILON);
        assert!((parse_px_value("") - 0.0).abs() < f32::EPSILON);
    }

    // ---------------------------------------------------------------
    // eval_media_query / eval_single_media_query
    // ---------------------------------------------------------------

    #[test]
    fn media_query_empty_is_true() {
        assert!(eval_media_query(""));
        assert!(eval_media_query("  "));
    }

    #[test]
    fn media_query_screen_and_all() {
        assert!(eval_media_query("screen"));
        assert!(eval_media_query("all"));
    }

    #[test]
    fn media_query_print_is_false() {
        assert!(!eval_media_query("print"));
    }

    #[test]
    fn media_query_not_print() {
        assert!(eval_media_query("not print"));
    }

    #[test]
    fn media_query_not_screen() {
        assert!(!eval_media_query("not screen"));
    }

    #[test]
    fn media_query_max_width_pass() {
        // viewport = 480, max-width 600 => true
        assert!(eval_media_query("(max-width: 600px)"));
    }

    #[test]
    fn media_query_max_width_fail() {
        // viewport = 480, max-width 200 => false
        assert!(!eval_media_query("(max-width: 200px)"));
    }

    #[test]
    fn media_query_min_width_pass() {
        // viewport = 480, min-width 320 => true
        assert!(eval_media_query("(min-width: 320px)"));
    }

    #[test]
    fn media_query_min_width_fail() {
        // viewport = 480, min-width 800 => false
        assert!(!eval_media_query("(min-width: 800px)"));
    }

    #[test]
    fn media_query_compound() {
        assert!(eval_media_query("screen and (max-width: 600px)"));
        assert!(!eval_media_query("print and (max-width: 600px)"));
    }

    #[test]
    fn media_query_comma_separated() {
        // "print, screen" => print=false OR screen=true => true
        assert!(eval_media_query("print, screen"));
        // "print, not screen" => both false
        assert!(!eval_media_query("print, not screen"));
    }

    #[test]
    fn media_query_only_modifier() {
        assert!(eval_media_query("only screen"));
    }

    #[test]
    fn media_query_prefers_color_scheme() {
        assert!(eval_media_query("(prefers-color-scheme: light)"));
        assert!(!eval_media_query("(prefers-color-scheme: dark)"));
    }

    #[test]
    fn media_query_unknown_feature() {
        assert!(!eval_media_query("(hover: hover)"));
    }

    #[test]
    fn eval_single_media_query_with_viewport() {
        assert!(eval_single_media_query("(max-width: 1024px)", 800.0));
        assert!(!eval_single_media_query("(max-width: 600px)", 800.0));
        assert!(eval_single_media_query("(min-width: 600px)", 800.0));
        assert!(!eval_single_media_query("(min-width: 1024px)", 800.0));
    }

    // ---------------------------------------------------------------
    // tokens_to_css_text
    // ---------------------------------------------------------------

    #[test]
    fn css_text_empty() {
        assert_eq!(tokens_to_css_text(&[]), "");
    }

    #[test]
    fn css_text_ident() {
        let tokens = [CssToken::Ident("auto".into())];
        assert_eq!(tokens_to_css_text(&tokens), "auto");
    }

    #[test]
    fn css_text_hash() {
        let tokens = [CssToken::Hash("ff0000".into())];
        assert_eq!(tokens_to_css_text(&tokens), "#ff0000");
    }

    #[test]
    fn css_text_string() {
        let tokens = [CssToken::String("hello".into())];
        assert_eq!(tokens_to_css_text(&tokens), "\"hello\"");
    }

    #[test]
    fn css_text_number() {
        let tokens = [CssToken::Number(42.0)];
        assert_eq!(tokens_to_css_text(&tokens), "42");
    }

    #[test]
    fn css_text_percentage() {
        let tokens = [CssToken::Percentage(50.0)];
        assert_eq!(tokens_to_css_text(&tokens), "50%");
    }

    #[test]
    fn css_text_dimension() {
        let tokens = [CssToken::Dimension(10.0, "px".into())];
        assert_eq!(tokens_to_css_text(&tokens), "10px");
    }

    #[test]
    fn css_text_punctuation() {
        let tokens = [
            CssToken::OpenBrace,
            CssToken::CloseBrace,
            CssToken::OpenParen,
            CssToken::CloseParen,
            CssToken::OpenBracket,
            CssToken::CloseBracket,
            CssToken::Colon,
            CssToken::Semicolon,
            CssToken::Comma,
            CssToken::Dot,
            CssToken::Greater,
            CssToken::Plus,
            CssToken::Star,
            CssToken::Slash,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "{}()[]:;,.>+*/");
    }

    #[test]
    fn css_text_delim() {
        let tokens = [CssToken::Delim('~')];
        assert_eq!(tokens_to_css_text(&tokens), "~");
    }

    #[test]
    fn css_text_at_keyword() {
        let tokens = [CssToken::AtKeyword("media".into())];
        assert_eq!(tokens_to_css_text(&tokens), "@media");
    }

    #[test]
    fn css_text_function() {
        let tokens = [
            CssToken::Function("rgb".into()),
            CssToken::Number(255.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "rgb(255,0,0)");
    }

    #[test]
    fn css_text_eof_ignored() {
        let tokens = [CssToken::Ident("x".into()), CssToken::Eof];
        assert_eq!(tokens_to_css_text(&tokens), "x");
    }

    #[test]
    fn css_text_whitespace_trimmed() {
        let tokens = [
            CssToken::Whitespace,
            CssToken::Ident("a".into()),
            CssToken::Whitespace,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "a");
    }

    #[test]
    fn css_text_complex_declaration() {
        let tokens = [
            CssToken::Ident("margin".into()),
            CssToken::Colon,
            CssToken::Whitespace,
            CssToken::Dimension(10.0, "px".into()),
            CssToken::Whitespace,
            CssToken::Dimension(20.0, "px".into()),
            CssToken::Semicolon,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "margin: 10px 20px;");
    }
}

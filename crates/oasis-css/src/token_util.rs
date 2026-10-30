//! Token-level value helpers shared by the declaration parser:
//! `font-weight` keywords, colour-property classification, length units,
//! and CSS text reconstruction from a token stream.

use crate::parser::{CssValue, LengthUnit, parse_value_list};
use crate::tokenizer::CssToken;

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
    match values.len() {
        1 => match values.into_iter().next() {
            Some(v) => v,
            None => CssValue::Keyword(String::new()),
        },
        _ => CssValue::Multiple(values),
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
        "ex" => Some(LengthUnit::Ex),
        "ch" => Some(LengthUnit::Ch),
        _ => None,
    }
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

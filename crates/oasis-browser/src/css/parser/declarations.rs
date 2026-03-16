//! Declaration and value parsing for the CSS parser.
//!
//! Implements declaration list parsing, value token collection,
//! `!important` handling, and the [`parse_value_list`] free function.

use super::super::helpers::{
    is_color_property, named_color, parse_font_weight, parse_hex_color, parse_unit,
    tokens_to_css_text, try_parse_color,
};
use super::super::shorthand::{
    parse_linear_gradient, parse_radial_gradient, parse_repeating_linear_gradient,
};
use super::super::tokenizer::CssToken;
use super::CssParser;
use super::types::{CssValue, Declaration};

impl CssParser {
    // -- declarations ------------------------------------------------

    pub(super) fn parse_declaration_list(&mut self) -> Vec<Declaration> {
        let mut decls = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                CssToken::CloseBrace | CssToken::Eof => break,
                _ => {},
            }
            if let Some(decl) = self.try_parse_declaration() {
                decls.push(decl);
            } else {
                // Recovery: skip to next `;` or `}`.
                self.skip_to_semicolon_or_brace();
            }
        }
        decls
    }

    fn skip_to_semicolon_or_brace(&mut self) {
        loop {
            match self.peek() {
                CssToken::Semicolon => {
                    self.advance();
                    break;
                },
                CssToken::CloseBrace | CssToken::Eof => break,
                _ => {
                    self.advance();
                },
            }
        }
    }

    fn try_parse_declaration(&mut self) -> Option<Declaration> {
        self.skip_whitespace();
        let property = match self.peek().clone() {
            CssToken::Ident(name) => {
                self.advance();
                name
            },
            _ => return None,
        };
        self.skip_whitespace();
        if !self.expect(&CssToken::Colon) {
            return None;
        }
        self.skip_whitespace();
        let raw_values = self.collect_value_tokens();
        let important = self.check_important(&raw_values);
        let values = if important {
            self.strip_important(raw_values)
        } else {
            raw_values
        };
        // Custom properties (--*) store value as raw CSS text.
        let value = if property.starts_with("--") {
            CssValue::String(tokens_to_css_text(&values))
        } else {
            self.parse_value(&property, &values)
        };
        // Consume trailing semicolon if present.
        self.skip_whitespace();
        if self.peek() == &CssToken::Semicolon {
            self.advance();
        }
        Some(Declaration {
            property: property.to_ascii_lowercase(),
            value,
            important,
        })
    }

    fn collect_value_tokens(&mut self) -> Vec<CssToken> {
        let mut toks = Vec::new();
        let mut paren_depth = 0u32;
        loop {
            match self.peek() {
                CssToken::Semicolon if paren_depth == 0 => break,
                CssToken::CloseBrace if paren_depth == 0 => break,
                CssToken::Eof => break,
                CssToken::OpenParen => {
                    paren_depth += 1;
                    toks.push(self.advance());
                },
                CssToken::CloseParen => {
                    paren_depth = paren_depth.saturating_sub(1);
                    toks.push(self.advance());
                },
                _ => {
                    toks.push(self.advance());
                },
            }
        }
        toks
    }

    fn check_important(&self, tokens: &[CssToken]) -> bool {
        // Look for `!` `important` at end (ignoring whitespace).
        let non_ws: Vec<_> = tokens
            .iter()
            .filter(|t| !matches!(t, CssToken::Whitespace))
            .collect();
        if non_ws.len() >= 2 {
            let last = non_ws[non_ws.len() - 1];
            let prev = non_ws[non_ws.len() - 2];
            if matches!(prev, CssToken::Delim('!'))
                && matches!(last, CssToken::Ident(s) if s.eq_ignore_ascii_case("important"))
            {
                return true;
            }
        }
        false
    }

    fn strip_important(&self, tokens: Vec<CssToken>) -> Vec<CssToken> {
        // Remove trailing `!important` (and any whitespace around it).
        let mut out = tokens;
        // Pop from end: ident("important"), whitespace?, delim('!'),
        // whitespace?.
        while matches!(out.last(), Some(CssToken::Whitespace)) {
            out.pop();
        }
        if matches!(
            out.last(),
            Some(CssToken::Ident(s)) if s.eq_ignore_ascii_case("important")
        ) {
            out.pop();
        }
        while matches!(out.last(), Some(CssToken::Whitespace)) {
            out.pop();
        }
        if matches!(out.last(), Some(CssToken::Delim('!'))) {
            out.pop();
        }
        while matches!(out.last(), Some(CssToken::Whitespace)) {
            out.pop();
        }
        out
    }

    fn parse_value(&self, property: &str, tokens: &[CssToken]) -> CssValue {
        let prop_lower = property.to_ascii_lowercase();

        // Try colour-valued properties first.
        if is_color_property(&prop_lower)
            && let Some(color) = try_parse_color(tokens)
        {
            return CssValue::Color(color);
        }

        // font-weight keyword normalisation.
        if prop_lower == "font-weight" {
            return parse_font_weight(tokens);
        }

        // Collect individual parsed values (skip whitespace separators).
        let values = parse_value_list(tokens);

        match values.len() {
            0 => CssValue::Keyword(String::new()),
            1 => match values.into_iter().next() {
                Some(v) => v,
                None => CssValue::Keyword(String::new()),
            },
            _ => CssValue::Multiple(values),
        }
    }
}

// -------------------------------------------------------------------
// Value parsing helpers
// -------------------------------------------------------------------

pub(crate) fn parse_value_list(tokens: &[CssToken]) -> Vec<CssValue> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            CssToken::Whitespace | CssToken::Comma => {
                i += 1;
            },
            CssToken::Dimension(n, u) => {
                if let Some(unit) = parse_unit(u) {
                    out.push(CssValue::Length(*n, unit));
                } else {
                    out.push(CssValue::Keyword(format!("{}{}", n, u)));
                }
                i += 1;
            },
            CssToken::Percentage(n) => {
                out.push(CssValue::Percentage(*n));
                i += 1;
            },
            CssToken::Number(n) => {
                // 0 is a valid length.
                out.push(CssValue::Number(*n));
                i += 1;
            },
            CssToken::Hash(h) => {
                if let Some(c) = parse_hex_color(h) {
                    out.push(CssValue::Color(c));
                } else {
                    out.push(CssValue::Keyword(format!("#{}", h)));
                }
                i += 1;
            },
            CssToken::Function(name) => {
                // Collect until matching `)`.
                let start = i;
                let mut depth = 1u32;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match &tokens[i] {
                        CssToken::OpenParen | CssToken::Function(_) => depth += 1,
                        CssToken::CloseParen => depth -= 1,
                        _ => {},
                    }
                    i += 1;
                }
                let inner = &tokens[start..i];
                if name.eq_ignore_ascii_case("var") {
                    // Parse var(--name) or var(--name, fallback).
                    let args = &inner[1..]; // skip Function token
                    let args = match args.last() {
                        Some(CssToken::CloseParen) => &args[..args.len() - 1],
                        _ => args,
                    };
                    let mut prop_name = None;
                    let mut comma_pos = None;
                    for (j, tok) in args.iter().enumerate() {
                        match tok {
                            CssToken::Ident(id) if prop_name.is_none() && id.starts_with("--") => {
                                prop_name = Some(id.to_ascii_lowercase());
                            },
                            CssToken::Comma if prop_name.is_some() && comma_pos.is_none() => {
                                comma_pos = Some(j);
                            },
                            _ => {},
                        }
                    }
                    if let Some(pname) = prop_name {
                        let fallback = comma_pos.map(|cp| tokens_to_css_text(&args[cp + 1..]));
                        out.push(CssValue::Var(pname, fallback));
                    } else {
                        out.push(CssValue::Keyword("var()".into()));
                    }
                } else if name.eq_ignore_ascii_case("url") {
                    // Parse url(...) function.
                    let args = &inner[1..]; // skip Function token
                    let args = match args.last() {
                        Some(CssToken::CloseParen) => &args[..args.len() - 1],
                        _ => args,
                    };
                    let url_str = args
                        .iter()
                        .filter_map(|t| match t {
                            CssToken::String(s) => Some(s.as_str()),
                            CssToken::Ident(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    out.push(CssValue::Url(url_str));
                } else if name.eq_ignore_ascii_case("calc") {
                    // Preserve calc() expression as raw text for later
                    // resolution when the containing block size is known.
                    let args = &inner[1..]; // skip Function token
                    let args = match args.last() {
                        Some(CssToken::CloseParen) => &args[..args.len() - 1],
                        _ => args,
                    };
                    out.push(CssValue::Calc(tokens_to_css_text(args)));
                } else if let Some(c) = try_parse_color(inner) {
                    out.push(CssValue::Color(c));
                } else if name.eq_ignore_ascii_case("linear-gradient") {
                    if let Some(grad) = parse_linear_gradient(inner) {
                        out.push(CssValue::Gradient(grad));
                    } else {
                        out.push(CssValue::Keyword(format!("{}()", name)));
                    }
                } else if name.eq_ignore_ascii_case("repeating-linear-gradient") {
                    if let Some(grad) = parse_repeating_linear_gradient(inner) {
                        out.push(CssValue::Gradient(grad));
                    } else {
                        out.push(CssValue::Keyword(format!("{}()", name)));
                    }
                } else if name.eq_ignore_ascii_case("radial-gradient") {
                    if let Some(grad) = parse_radial_gradient(inner) {
                        out.push(CssValue::RadialGradient(grad));
                    } else {
                        out.push(CssValue::Keyword(format!("{}()", name)));
                    }
                } else {
                    out.push(CssValue::Keyword(format!("{}()", name)));
                }
            },
            CssToken::Ident(name) => {
                if let Some(c) = named_color(name) {
                    out.push(CssValue::Color(c));
                } else {
                    out.push(CssValue::Keyword(name.clone()));
                }
                i += 1;
            },
            CssToken::String(s) => {
                out.push(CssValue::String(s.clone()));
                i += 1;
            },
            _ => {
                i += 1;
            },
        }
    }
    out
}

//! CSS parser.
//!
//! Consumes the token stream produced by [`super::tokenizer::CssTokenizer`]
//! and builds a typed stylesheet AST with selectors, declarations, specificity,
//! shorthand expansion, and color parsing.

mod declarations;
mod selectors;
mod types;

pub use super::helpers::MediaViewport;
pub(crate) use declarations::parse_value_list;
#[allow(unused_imports)]
pub use types::{
    AttrOp, Combinator, CompoundSelector, CssColor, CssValue, Declaration, KeyframeStop,
    KeyframesRule, LengthUnit, Rule, Selector, SimpleSelector, Specificity, Stylesheet,
};

// SelectorList is used by cascade tests and parser tests (cfg(test) only),
// so the re-export is only needed during test builds.
#[cfg(test)]
pub(crate) use types::SelectorList;

use super::helpers::eval_media_query_with_viewport;
use super::shorthand::expand_shorthands;
use super::tokenizer::{CssToken, CssTokenizer};

impl Stylesheet {
    /// Parse an entire CSS stylesheet.
    ///
    /// Uses the default 480x272 PSP viewport for `@media` evaluation.
    pub fn parse(input: &str) -> Self {
        Self::parse_with_viewport(input, MediaViewport::DEFAULT)
    }

    /// Parse a CSS stylesheet with a specific viewport for `@media` queries.
    ///
    /// `@media (min-width: ...)`, `@media (max-width: ...)`,
    /// `@media (min-height: ...)`, `@media (max-height: ...)`,
    /// `@media screen`, `@media all`, and `@media (prefers-color-scheme: ...)`
    /// are evaluated against the given viewport.
    pub fn parse_with_viewport(input: &str, viewport: MediaViewport) -> Self {
        let tokens = CssTokenizer::new(input).tokenize();
        let mut parser = CssParser::new(tokens, viewport);
        parser.parse_stylesheet()
    }
}

/// Parse an inline `style="..."` attribute into declarations.
pub fn parse_inline_style(input: &str) -> Vec<Declaration> {
    let tokens = CssTokenizer::new(input).tokenize();
    let mut parser = CssParser::new(tokens, MediaViewport::DEFAULT);
    parser.parse_declaration_list()
}

// -------------------------------------------------------------------
// Internal parser
// -------------------------------------------------------------------

struct CssParser {
    tokens: Vec<CssToken>,
    pos: usize,
    viewport: MediaViewport,
}

impl CssParser {
    fn new(tokens: Vec<CssToken>, viewport: MediaViewport) -> Self {
        Self {
            tokens,
            pos: 0,
            viewport,
        }
    }

    // -- helpers -----------------------------------------------------

    fn peek(&self) -> &CssToken {
        self.tokens.get(self.pos).unwrap_or(&CssToken::Eof)
    }

    fn advance(&mut self) -> CssToken {
        if self.pos < self.tokens.len() {
            let tok = std::mem::replace(&mut self.tokens[self.pos], CssToken::Eof);
            self.pos += 1;
            tok
        } else {
            CssToken::Eof
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek() == &CssToken::Whitespace {
            self.advance();
        }
    }

    fn expect(&mut self, expected: &CssToken) -> bool {
        self.skip_whitespace();
        if self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), CssToken::Eof)
    }

    /// Peek and check if the next token is an at-keyword, returning its
    /// lowercase name without cloning the full token.
    fn peek_at_keyword_lc(&self) -> Option<String> {
        match self.peek() {
            CssToken::AtKeyword(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        }
    }

    // -- stylesheet --------------------------------------------------

    fn parse_stylesheet(&mut self) -> Stylesheet {
        let mut rules = Vec::new();
        let mut keyframes = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() {
                break;
            }
            if let Some(lc) = self.peek_at_keyword_lc() {
                if lc == "media" {
                    // Parse @media rules and include matching ones.
                    let media_rules = self.parse_media_rule();
                    rules.extend(media_rules);
                } else if lc == "supports" {
                    let supports_rules = self.parse_supports_rule();
                    rules.extend(supports_rules);
                } else if lc == "keyframes" || lc == "-webkit-keyframes" {
                    if let Some(kf) = self.parse_keyframes_rule() {
                        keyframes.push(kf);
                    }
                } else {
                    // Other at-rules: skip.
                    self.skip_at_rule();
                }
                continue;
            }
            match self.try_parse_rule() {
                Some(rule) => rules.push(rule),
                None => {
                    // Recovery: skip one token and try again.
                    self.advance();
                },
            }
        }
        Stylesheet { rules, keyframes }
    }

    fn skip_at_rule(&mut self) {
        self.advance(); // consume @keyword
        let mut brace_depth = 0;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::Semicolon if brace_depth == 0 => {
                    self.advance();
                    break;
                },
                CssToken::OpenBrace => {
                    brace_depth += 1;
                    self.advance();
                },
                CssToken::CloseBrace => {
                    if brace_depth <= 1 {
                        self.advance();
                        break;
                    }
                    brace_depth -= 1;
                    self.advance();
                },
                _ => {
                    self.advance();
                },
            }
        }
    }

    /// Parse an `@media` rule. Evaluates the media query against the
    /// OASIS viewport (480x272, screen). If it matches, the inner rules
    /// are returned; otherwise they are discarded.
    fn parse_media_rule(&mut self) -> Vec<Rule> {
        self.advance(); // consume @media token
        self.skip_whitespace();

        // Collect the media query condition tokens up to the opening brace.
        let mut condition = String::new();
        loop {
            match self.peek() {
                CssToken::OpenBrace | CssToken::Eof => break,
                _ => {
                    let tok = self.peek().clone();
                    self.advance();
                    match tok {
                        CssToken::Ident(s) => condition.push_str(&s),
                        CssToken::Number(n) => condition.push_str(&format!("{n}")),
                        CssToken::Dimension(n, unit) => {
                            condition.push_str(&format!("{n}{unit}"));
                        },
                        CssToken::Whitespace => condition.push(' '),
                        CssToken::OpenParen => condition.push('('),
                        CssToken::CloseParen => condition.push(')'),
                        CssToken::Colon => condition.push(':'),
                        CssToken::Comma => condition.push(','),
                        CssToken::Delim(c) => condition.push(c),
                        _ => {},
                    }
                },
            }
        }

        // Consume the opening brace.
        if !self.expect(&CssToken::OpenBrace) {
            return Vec::new();
        }

        // Parse inner rules.
        let mut inner_rules = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            if matches!(self.peek(), CssToken::AtKeyword(_)) {
                self.skip_at_rule();
                continue;
            }
            if let Some(rule) = self.try_parse_rule() {
                inner_rules.push(rule);
            } else {
                self.advance();
            }
        }
        self.expect(&CssToken::CloseBrace);

        // Evaluate the media condition.
        if eval_media_query_with_viewport(&condition, self.viewport) {
            inner_rules
        } else {
            Vec::new()
        }
    }

    /// Parse an `@supports` rule. Evaluates the feature query by checking
    /// whether the property name is recognized by `apply_declaration`.
    /// If supported, the inner rules are returned; otherwise discarded.
    fn parse_supports_rule(&mut self) -> Vec<Rule> {
        self.advance(); // consume @supports token
        self.skip_whitespace();

        // Collect the condition tokens up to the opening brace.
        let mut condition = String::new();
        loop {
            match self.peek() {
                CssToken::OpenBrace | CssToken::Eof => break,
                _ => {
                    let tok = self.peek().clone();
                    self.advance();
                    match tok {
                        CssToken::Ident(s) => condition.push_str(&s),
                        CssToken::Number(n) => condition.push_str(&format!("{n}")),
                        CssToken::Dimension(n, unit) => {
                            condition.push_str(&format!("{n}{unit}"));
                        },
                        CssToken::Whitespace => condition.push(' '),
                        CssToken::OpenParen => condition.push('('),
                        CssToken::CloseParen => condition.push(')'),
                        CssToken::Colon => condition.push(':'),
                        CssToken::Comma => condition.push(','),
                        CssToken::Hash(h) => {
                            condition.push('#');
                            condition.push_str(&h);
                        },
                        CssToken::Delim(c) => condition.push(c),
                        _ => {},
                    }
                },
            }
        }

        // Consume the opening brace.
        if !self.expect(&CssToken::OpenBrace) {
            return Vec::new();
        }

        // Parse inner rules.
        let mut inner_rules = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            if matches!(self.peek(), CssToken::AtKeyword(_)) {
                self.skip_at_rule();
                continue;
            }
            if let Some(rule) = self.try_parse_rule() {
                inner_rules.push(rule);
            } else {
                self.advance();
            }
        }
        self.expect(&CssToken::CloseBrace);

        if eval_supports_condition(&condition) {
            inner_rules
        } else {
            Vec::new()
        }
    }

    fn try_parse_rule(&mut self) -> Option<Rule> {
        let selectors = self.parse_selector_list()?;
        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            // Recovery: skip to next `}` or EOF.
            self.skip_to_close_brace();
            return None;
        }
        let declarations = self.parse_declaration_list();
        self.expect(&CssToken::CloseBrace);
        let declarations = expand_shorthands(declarations);
        Some(Rule {
            selectors,
            declarations,
        })
    }

    /// Parse an `@keyframes name { ... }` rule.
    fn parse_keyframes_rule(&mut self) -> Option<types::KeyframesRule> {
        self.advance(); // consume @keyframes token
        self.skip_whitespace();

        // Read the animation name.
        let name = match self.peek().clone() {
            CssToken::Ident(s) => {
                self.advance();
                s
            },
            _ => {
                // Recovery: skip this at-rule.
                self.skip_to_close_brace();
                return None;
            },
        };

        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            return None;
        }

        let mut stops = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }

            // Parse keyframe selector: percentage, "from", or "to".
            let percentage = match self.peek().clone() {
                CssToken::Ident(ref kw) if kw.eq_ignore_ascii_case("from") => {
                    self.advance();
                    Some(0.0)
                },
                CssToken::Ident(ref kw) if kw.eq_ignore_ascii_case("to") => {
                    self.advance();
                    Some(100.0)
                },
                CssToken::Number(n) => {
                    self.advance();
                    // Expect '%' delimiter after the number.
                    self.skip_whitespace();
                    if let CssToken::Delim('%') = self.peek() {
                        self.advance();
                    }
                    Some(n)
                },
                CssToken::Percentage(n) => {
                    self.advance();
                    Some(n)
                },
                _ => {
                    // Recovery: skip to next `}`.
                    self.advance();
                    continue;
                },
            };

            if let Some(pct) = percentage {
                self.skip_whitespace();
                if !self.expect(&CssToken::OpenBrace) {
                    continue;
                }
                let declarations = self.parse_declaration_list();
                self.expect(&CssToken::CloseBrace);
                let declarations = expand_shorthands(declarations);
                stops.push(types::KeyframeStop {
                    percentage: pct,
                    declarations,
                });
            }
        }
        self.expect(&CssToken::CloseBrace);

        // Sort stops by percentage for interpolation.
        stops.sort_by(|a, b| {
            a.percentage
                .partial_cmp(&b.percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Some(types::KeyframesRule { name, stops })
    }

    fn skip_to_close_brace(&mut self) {
        let mut depth = 0;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::OpenBrace => {
                    depth += 1;
                    self.advance();
                },
                CssToken::CloseBrace => {
                    if depth == 0 {
                        self.advance();
                        break;
                    }
                    depth -= 1;
                    self.advance();
                },
                _ => {
                    self.advance();
                },
            }
        }
    }
}

// -------------------------------------------------------------------
// @supports condition evaluation
// -------------------------------------------------------------------

/// List of CSS property names recognized by `apply_declaration`.
const SUPPORTED_PROPERTIES: &[&str] = &[
    "display",
    "visibility",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-width",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-style",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "width",
    "height",
    "max-width",
    "min-width",
    "max-height",
    "min-height",
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-transform",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "background-color",
    "background",
    "list-style-type",
    "list-style-position",
    "border-collapse",
    "border-spacing",
    "float",
    "clear",
    "overflow",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "z-index",
    "flex-direction",
    "flex-wrap",
    "justify-content",
    "align-items",
    "align-content",
    "align-self",
    "order",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "gap",
    "grid-gap",
    "column-gap",
    "grid-column-gap",
    "row-gap",
    "grid-row-gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column-start",
    "grid-column-end",
    "grid-column",
    "grid-row-start",
    "grid-row-end",
    "grid-row",
    "border-radius",
    "opacity",
    "box-shadow",
    "text-shadow",
    "box-sizing",
    "vertical-align",
    "background-image",
    "word-break",
    "overflow-wrap",
    "word-wrap",
    "text-overflow",
    "content",
    "outline-width",
    "outline-color",
    "outline-style",
    "outline-offset",
    "outline",
    "transition",
    "direction",
    "animation",
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
    "transform",
    "transform-origin",
    "filter",
    "counter-reset",
    "counter-increment",
    "grid-auto-flow",
    "grid-template-areas",
    "grid-area",
    "table-layout",
    "will-change",
    "tab-size",
    "column-count",
    "column-width",
    "columns",
];

/// Evaluate an `@supports` condition string.
///
/// Supports simple `(property: value)` conditions and `not (...)`.
/// Unknown or unsupported conditions evaluate to `false`.
fn eval_supports_condition(condition: &str) -> bool {
    let condition = condition.trim();
    if condition.is_empty() {
        return false;
    }

    // Handle `not (...)`.
    if let Some(rest) = condition.strip_prefix("not ") {
        return !eval_supports_condition(rest.trim());
    }

    // Handle compound `(...) and (...)`
    if condition.contains(") and (") {
        let parts: Vec<&str> = condition.split(") and (").collect();
        return parts.iter().all(|p| {
            let trimmed = p.trim().trim_start_matches('(').trim_end_matches(')');
            eval_supports_single(&format!("({trimmed})"))
        });
    }

    // Handle compound `(...) or (...)`
    if condition.contains(") or (") {
        let parts: Vec<&str> = condition.split(") or (").collect();
        return parts.iter().any(|p| {
            let trimmed = p.trim().trim_start_matches('(').trim_end_matches(')');
            eval_supports_single(&format!("({trimmed})"))
        });
    }

    eval_supports_single(condition)
}

/// Evaluate a single `(property: value)` supports condition.
fn eval_supports_single(condition: &str) -> bool {
    let inner = condition
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();

    // Split on the first colon to get property name.
    if let Some(colon_pos) = inner.find(':') {
        let property = inner[..colon_pos].trim();
        // Check if property is supported (exists in apply_declaration).
        // Also allow custom properties (--*).
        if property.starts_with("--") {
            return true;
        }
        SUPPORTED_PROPERTIES.contains(&property)
    } else {
        false
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests;

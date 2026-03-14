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
pub use types::{
    AttrOp, Combinator, CompoundSelector, CssColor, CssValue, Declaration, LengthUnit, Rule,
    Selector, SimpleSelector, Specificity, Stylesheet,
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
        Stylesheet { rules }
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
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests;

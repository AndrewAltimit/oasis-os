//! CSS parser.
//!
//! Consumes the token stream produced by [`super::tokenizer::CssTokenizer`]
//! and builds a typed stylesheet AST with selectors, declarations, specificity,
//! shorthand expansion, and color parsing.

pub use super::helpers::MediaViewport;
use super::helpers::{
    eval_media_query_with_viewport, is_color_property, named_color, parse_font_weight,
    parse_hex_color, parse_unit, tokens_to_css_text, try_parse_color,
};
use super::shorthand::{expand_shorthands, parse_linear_gradient};
use super::tokenizer::{CssToken, CssTokenizer};

// -------------------------------------------------------------------
// Selector types
// -------------------------------------------------------------------

/// Attribute selector match operator.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrOp {
    /// `[attr]` -- attribute exists.
    Exists,
    /// `[attr=val]` -- exact match.
    Equals,
    /// `[attr~=val]` -- space-separated word match.
    Includes,
    /// `[attr|=val]` -- exact or prefix with hyphen.
    DashMatch,
    /// `[attr^=val]` -- starts with.
    Prefix,
    /// `[attr$=val]` -- ends with.
    Suffix,
    /// `[attr*=val]` -- substring match.
    Substring,
}

/// A single, atomic selector component.
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    /// Type selector: `div`, `p`, `h1`.
    Type(String),
    /// Class selector: `.classname`.
    Class(String),
    /// ID selector: `#idname`.
    Id(String),
    /// Universal selector: `*`.
    Universal,
    /// Pseudo-class: `:hover`, `:first-child`.
    PseudoClass(String),
    /// Functional pseudo-class with argument: `:nth-child(2n+1)`.
    PseudoClassFn(String, String),
    /// Pseudo-element: `::before`, `::after`.
    PseudoElement(String),
    /// Negation: `:not(selector)`.
    Not(Box<CompoundSelector>),
    /// Attribute selector: `[attr]`, `[attr=val]`, etc.
    Attribute {
        name: String,
        op: AttrOp,
        value: Option<String>,
    },
}

/// Combinator linking two compound selectors.
#[derive(Debug, Clone, PartialEq)]
pub enum Combinator {
    /// Descendant: `div p` (whitespace).
    Descendant,
    /// Child: `div > p`.
    Child,
    /// Adjacent sibling: `h1 + p`.
    AdjacentSibling,
    /// General sibling: `h1 ~ p`.
    GeneralSibling,
}

/// A compound selector is a sequence of simple selectors applied to the
/// same element (e.g. `div.class#id`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    /// Parts that must all match the same element.
    pub parts: Vec<SimpleSelector>,
}

/// A full selector is a chain of compound selectors separated by
/// combinators.  Each entry stores the compound selector and the
/// combinator that *preceded* it (`None` for the first in the chain).
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<(CompoundSelector, Option<Combinator>)>,
}

/// Comma-separated list of selectors.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList {
    pub selectors: Vec<Selector>,
}

// -------------------------------------------------------------------
// Specificity
// -------------------------------------------------------------------

/// CSS specificity in the standard (inline, id, class, type) tuple form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity {
    /// 1 if the style originates from an inline `style` attribute.
    pub inline: u8,
    /// Count of ID selectors.
    pub ids: u8,
    /// Count of class, pseudo-class, and attribute selectors.
    pub classes: u8,
    /// Count of type selectors and pseudo-elements.
    pub types: u8,
}

impl Selector {
    /// Compute the specificity of this selector.  Inline is always 0 here;
    /// the caller bumps it for inline styles.
    pub fn specificity(&self) -> Specificity {
        let mut ids: u8 = 0;
        let mut classes: u8 = 0;
        let mut types: u8 = 0;
        for (compound, _) in &self.parts {
            for simple in &compound.parts {
                match simple {
                    SimpleSelector::Id(_) => {
                        ids = ids.saturating_add(1);
                    },
                    SimpleSelector::Class(_)
                    | SimpleSelector::PseudoClass(_)
                    | SimpleSelector::PseudoClassFn(_, _)
                    | SimpleSelector::Attribute { .. } => {
                        classes = classes.saturating_add(1);
                    },
                    SimpleSelector::Not(inner) => {
                        // :not() itself doesn't count, but its argument does.
                        for inner_simple in &inner.parts {
                            match inner_simple {
                                SimpleSelector::Id(_) => {
                                    ids = ids.saturating_add(1);
                                },
                                SimpleSelector::Class(_)
                                | SimpleSelector::PseudoClass(_)
                                | SimpleSelector::PseudoClassFn(_, _)
                                | SimpleSelector::Attribute { .. } => {
                                    classes = classes.saturating_add(1);
                                },
                                SimpleSelector::Type(_) => {
                                    types = types.saturating_add(1);
                                },
                                _ => {},
                            }
                        }
                    },
                    SimpleSelector::Type(_) | SimpleSelector::PseudoElement(_) => {
                        types = types.saturating_add(1);
                    },
                    SimpleSelector::Universal => {},
                }
            }
        }
        Specificity {
            inline: 0,
            ids,
            classes,
            types,
        }
    }
}

// -------------------------------------------------------------------
// Declaration / value types
// -------------------------------------------------------------------

/// A single CSS property declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
}

/// A parsed CSS value.
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    /// An unresolved keyword (e.g. `auto`, `inherit`, `solid`).
    Keyword(String),
    /// A length with unit.
    Length(f32, LengthUnit),
    /// A percentage value.
    Percentage(f32),
    /// A resolved colour.
    Color(CssColor),
    /// A bare number.
    Number(f32),
    /// Multiple values (shorthand expansions, font stacks, etc.).
    Multiple(Vec<CssValue>),
    /// A quoted string value.
    String(String),
    /// A `var(--name)` or `var(--name, fallback)` reference.
    Var(String, Option<String>),
    /// A `url(...)` value.
    Url(String),
    /// A parsed `linear-gradient(...)` value.
    Gradient(crate::css::values::LinearGradient),
}

/// Supported CSS length units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Pt,
}

/// An RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    pub(crate) const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

// -------------------------------------------------------------------
// Rule / Stylesheet
// -------------------------------------------------------------------

/// A style rule (selector list + declarations).
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: SelectorList,
    pub declarations: Vec<Declaration>,
}

/// A complete parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

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

    // -- selectors ---------------------------------------------------

    fn parse_selector_list(&mut self) -> Option<SelectorList> {
        let mut selectors = Vec::new();
        if let Some(sel) = self.parse_selector() {
            selectors.push(sel);
        } else {
            return None;
        }
        loop {
            self.skip_whitespace();
            if self.peek() == &CssToken::Comma {
                self.advance();
                self.skip_whitespace();
                if let Some(sel) = self.parse_selector() {
                    selectors.push(sel);
                }
            } else {
                break;
            }
        }
        Some(SelectorList { selectors })
    }

    fn parse_selector(&mut self) -> Option<Selector> {
        self.skip_whitespace();
        let first = self.parse_compound_selector()?;
        let mut parts = vec![(first, None)];
        loop {
            // Check for combinator or whitespace (descendant).
            let has_ws = self.peek() == &CssToken::Whitespace;
            if has_ws {
                self.skip_whitespace();
            }

            // Explicit combinators.
            let combinator = match self.peek() {
                CssToken::Greater => {
                    self.advance();
                    self.skip_whitespace();
                    Some(Combinator::Child)
                },
                CssToken::Plus => {
                    self.advance();
                    self.skip_whitespace();
                    Some(Combinator::AdjacentSibling)
                },
                CssToken::Delim('~') => {
                    self.advance();
                    self.skip_whitespace();
                    Some(Combinator::GeneralSibling)
                },
                _ if has_ws => {
                    // Could be descendant combinator or end of selector.
                    if self.is_selector_start() {
                        Some(Combinator::Descendant)
                    } else {
                        break;
                    }
                },
                _ => break,
            };

            if let Some(compound) = self.parse_compound_selector() {
                parts.push((compound, combinator));
            } else {
                break;
            }
        }
        Some(Selector { parts })
    }

    fn is_selector_start(&self) -> bool {
        matches!(
            self.peek(),
            CssToken::Ident(_)
                | CssToken::Hash(_)
                | CssToken::Dot
                | CssToken::Star
                | CssToken::Colon
        )
    }

    fn parse_compound_selector(&mut self) -> Option<CompoundSelector> {
        let mut parts = Vec::new();
        loop {
            match self.peek().clone() {
                CssToken::Ident(name) => {
                    self.advance();
                    parts.push(SimpleSelector::Type(name));
                },
                CssToken::Hash(name) => {
                    self.advance();
                    parts.push(SimpleSelector::Id(name));
                },
                CssToken::Dot => {
                    self.advance();
                    if let CssToken::Ident(name) = self.peek().clone() {
                        self.advance();
                        parts.push(SimpleSelector::Class(name));
                    }
                },
                CssToken::Star => {
                    self.advance();
                    parts.push(SimpleSelector::Universal);
                },
                CssToken::Colon => {
                    self.advance();
                    // Check for double-colon `::` pseudo-element.
                    if self.peek() == &CssToken::Colon {
                        self.advance();
                        if let CssToken::Ident(name) = self.peek().clone() {
                            self.advance();
                            let lc = name.to_ascii_lowercase();
                            parts.push(SimpleSelector::PseudoElement(lc));
                        }
                        continue;
                    }
                    match self.peek().clone() {
                        CssToken::Ident(name) => {
                            self.advance();
                            // Legacy single-colon pseudo-elements.
                            let lc = name.to_ascii_lowercase();
                            if lc == "before" || lc == "after" {
                                parts.push(SimpleSelector::PseudoElement(lc));
                            } else {
                                parts.push(SimpleSelector::PseudoClass(name));
                            }
                        },
                        CssToken::Function(name) => {
                            self.advance();
                            let lc = name.to_ascii_lowercase();
                            if lc == "not" {
                                // Parse :not(compound)
                                self.skip_whitespace();
                                if let Some(inner) = self.parse_compound_selector() {
                                    parts.push(SimpleSelector::Not(Box::new(inner)));
                                }
                                self.skip_whitespace();
                                if self.peek() == &CssToken::CloseParen {
                                    self.advance();
                                }
                            } else {
                                // Functional pseudo-class like :nth-child(2n+1)
                                let arg = self.consume_until_close_paren();
                                parts.push(SimpleSelector::PseudoClassFn(lc, arg));
                            }
                        },
                        _ => {},
                    }
                },
                CssToken::OpenBracket => {
                    self.advance();
                    if let Some(attr_sel) = self.parse_attribute_selector() {
                        parts.push(attr_sel);
                    }
                },
                _ => break,
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(CompoundSelector { parts })
        }
    }

    // -- attribute selector / pseudo-class helpers ---------------------

    /// Consume tokens until `)`, collecting them as a trimmed string.
    fn consume_until_close_paren(&mut self) -> String {
        let mut arg = String::new();
        loop {
            match self.peek() {
                CssToken::CloseParen | CssToken::Eof => {
                    if self.peek() == &CssToken::CloseParen {
                        self.advance();
                    }
                    break;
                },
                _ => {
                    let tok = self.peek().clone();
                    self.advance();
                    match tok {
                        CssToken::Ident(s) => arg.push_str(&s),
                        CssToken::Number(n) => arg.push_str(&format!("{n}")),
                        CssToken::Plus => arg.push('+'),
                        CssToken::Whitespace => arg.push(' '),
                        CssToken::Delim(c) => arg.push(c),
                        _ => {},
                    }
                },
            }
        }
        arg.trim().to_string()
    }

    /// Parse an attribute selector after `[` has been consumed.
    /// Returns `SimpleSelector::Attribute { .. }`.
    fn parse_attribute_selector(&mut self) -> Option<SimpleSelector> {
        self.skip_whitespace();
        let name = match self.peek().clone() {
            CssToken::Ident(n) => {
                self.advance();
                n
            },
            _ => {
                // Skip to `]`.
                while self.peek() != &CssToken::CloseBracket && self.peek() != &CssToken::Eof {
                    self.advance();
                }
                if self.peek() == &CssToken::CloseBracket {
                    self.advance();
                }
                return None;
            },
        };

        self.skip_whitespace();

        // Check for operator or `]`.
        if self.peek() == &CssToken::CloseBracket {
            self.advance();
            return Some(SimpleSelector::Attribute {
                name,
                op: AttrOp::Exists,
                value: None,
            });
        }

        // Parse operator: =, ~=, |=, ^=, $=, *=
        let op = match self.peek().clone() {
            CssToken::Delim('=') => {
                self.advance();
                AttrOp::Equals
            },
            CssToken::Delim('~') => {
                self.advance();
                // Expect `=`.
                if self.peek() == &CssToken::Delim('=') {
                    self.advance();
                }
                AttrOp::Includes
            },
            CssToken::Delim('|') => {
                self.advance();
                if self.peek() == &CssToken::Delim('=') {
                    self.advance();
                }
                AttrOp::DashMatch
            },
            CssToken::Delim('^') => {
                self.advance();
                if self.peek() == &CssToken::Delim('=') {
                    self.advance();
                }
                AttrOp::Prefix
            },
            CssToken::Delim('$') => {
                self.advance();
                if self.peek() == &CssToken::Delim('=') {
                    self.advance();
                }
                AttrOp::Suffix
            },
            CssToken::Star => {
                self.advance();
                if self.peek() == &CssToken::Delim('=') {
                    self.advance();
                }
                AttrOp::Substring
            },
            _ => {
                // Unknown operator, skip to `]`.
                while self.peek() != &CssToken::CloseBracket && self.peek() != &CssToken::Eof {
                    self.advance();
                }
                if self.peek() == &CssToken::CloseBracket {
                    self.advance();
                }
                return Some(SimpleSelector::Attribute {
                    name,
                    op: AttrOp::Exists,
                    value: None,
                });
            },
        };

        self.skip_whitespace();

        // Parse value (ident or string).
        let value = match self.peek().clone() {
            CssToken::Ident(v) => {
                self.advance();
                Some(v)
            },
            CssToken::String(v) => {
                self.advance();
                Some(v)
            },
            _ => None,
        };

        self.skip_whitespace();
        if self.peek() == &CssToken::CloseBracket {
            self.advance();
        }

        Some(SimpleSelector::Attribute { name, op, value })
    }

    // -- declarations ------------------------------------------------

    fn parse_declaration_list(&mut self) -> Vec<Declaration> {
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
            1 => values.into_iter().next().expect("len checked"),
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
                            CssToken::Ident(id) if prop_name.is_none() => {
                                if id.starts_with("--") {
                                    prop_name = Some(id.to_ascii_lowercase());
                                }
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
                } else if let Some(c) = try_parse_color(inner) {
                    out.push(CssValue::Color(c));
                } else if name.eq_ignore_ascii_case("linear-gradient") {
                    if let Some(grad) = parse_linear_gradient(inner) {
                        out.push(CssValue::Gradient(grad));
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

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- helper -------------------------------------------------------

    fn parse(css: &str) -> Stylesheet {
        Stylesheet::parse(css)
    }

    fn first_decls(css: &str) -> Vec<Declaration> {
        let sheet = parse(css);
        assert!(!sheet.rules.is_empty(), "expected at least one rule");
        sheet.rules[0].declarations.clone()
    }

    fn first_selectors(css: &str) -> SelectorList {
        let sheet = parse(css);
        sheet.rules[0].selectors.clone()
    }

    // -- test 1: simple rule -----------------------------------------

    #[test]
    fn simple_rule() {
        let sheet = parse("p { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        let rule = &sheet.rules[0];
        let sel = &rule.selectors.selectors[0];
        assert_eq!(sel.parts[0].0.parts, vec![SimpleSelector::Type("p".into())]);
        assert_eq!(rule.declarations.len(), 1);
        assert_eq!(rule.declarations[0].property, "color");
        assert_eq!(
            rule.declarations[0].value,
            CssValue::Color(CssColor::new(255, 0, 0, 255))
        );
    }

    // -- test 2: class selector --------------------------------------

    #[test]
    fn class_selector() {
        let sheet = parse(".intro { font-size: 14px; }");
        let sel = &sheet.rules[0].selectors.selectors[0];
        assert_eq!(
            sel.parts[0].0.parts,
            vec![SimpleSelector::Class("intro".into())]
        );
        assert_eq!(
            sheet.rules[0].declarations[0].value,
            CssValue::Length(14.0, LengthUnit::Px)
        );
    }

    // -- test 3: id selector -----------------------------------------

    #[test]
    fn id_selector() {
        let sheet = parse("#header { background-color: #333; }");
        let sel = &sheet.rules[0].selectors.selectors[0];
        assert_eq!(
            sel.parts[0].0.parts,
            vec![SimpleSelector::Id("header".into())]
        );
        assert_eq!(
            sheet.rules[0].declarations[0].value,
            CssValue::Color(CssColor::new(0x33, 0x33, 0x33, 255))
        );
    }

    // -- test 4: descendant selector ---------------------------------

    #[test]
    fn descendant_selector() {
        let decls = first_decls("div p { margin: 10px; }");
        // Should expand margin shorthand.
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[0].property, "margin-top");
    }

    // -- test 5: child selector --------------------------------------

    #[test]
    fn child_selector() {
        let sels = first_selectors("div > p { color: blue; }");
        let sel = &sels.selectors[0];
        assert_eq!(sel.parts.len(), 2);
        assert_eq!(sel.parts[1].1, Some(Combinator::Child));
    }

    // -- test 6: grouped selectors -----------------------------------

    #[test]
    fn grouped_selectors() {
        let sheet = parse("h1, h2, h3 { font-weight: bold; }");
        assert_eq!(sheet.rules[0].selectors.selectors.len(), 3);
    }

    // -- test 7: compound selector -----------------------------------

    #[test]
    fn compound_selector() {
        let sels = first_selectors("p.intro#first { color: green; }");
        let parts = &sels.selectors[0].parts[0].0.parts;
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], SimpleSelector::Type("p".into()));
        assert_eq!(parts[1], SimpleSelector::Class("intro".into()));
        assert_eq!(parts[2], SimpleSelector::Id("first".into()));
    }

    // -- test 8: multiple declarations -------------------------------

    #[test]
    fn multiple_declarations() {
        let sheet = parse("p { color: red; font-size: 12px; display: block; }");
        assert_eq!(sheet.rules[0].declarations.len(), 3);
    }

    // -- test 9: shorthand expansion ---------------------------------

    #[test]
    fn shorthand_margin_two_values() {
        let decls = first_decls("div { margin: 10px 20px; }");
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[0].property, "margin-top");
        assert_eq!(decls[0].value, CssValue::Length(10.0, LengthUnit::Px));
        assert_eq!(decls[1].property, "margin-right");
        assert_eq!(decls[1].value, CssValue::Length(20.0, LengthUnit::Px));
        assert_eq!(decls[2].property, "margin-bottom");
        assert_eq!(decls[2].value, CssValue::Length(10.0, LengthUnit::Px));
        assert_eq!(decls[3].property, "margin-left");
        assert_eq!(decls[3].value, CssValue::Length(20.0, LengthUnit::Px));
    }

    #[test]
    fn shorthand_margin_three_values() {
        let decls = first_decls("div { margin: 10px 20px 30px; }");
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[0].value, CssValue::Length(10.0, LengthUnit::Px));
        assert_eq!(decls[1].value, CssValue::Length(20.0, LengthUnit::Px));
        assert_eq!(decls[2].value, CssValue::Length(30.0, LengthUnit::Px));
        assert_eq!(decls[3].value, CssValue::Length(20.0, LengthUnit::Px));
    }

    #[test]
    fn shorthand_margin_four_values() {
        let decls = first_decls("div { margin: 10px 20px 30px 40px; }");
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[3].value, CssValue::Length(40.0, LengthUnit::Px));
    }

    #[test]
    fn shorthand_padding() {
        let decls = first_decls("div { padding: 5px; }");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.property.starts_with("padding-"));
            assert_eq!(d.value, CssValue::Length(5.0, LengthUnit::Px));
        }
    }

    #[test]
    fn shorthand_border() {
        let decls = first_decls("div { border: 1px solid black; }");
        assert!(
            decls.iter().any(|d| d.property == "border-width"
                && d.value == CssValue::Length(1.0, LengthUnit::Px))
        );
        assert!(
            decls
                .iter()
                .any(|d| d.property == "border-style"
                    && d.value == CssValue::Keyword("solid".into()))
        );
        assert!(decls.iter().any(|d| d.property == "border-color"
            && d.value == CssValue::Color(CssColor::new(0, 0, 0, 255))));
    }

    #[test]
    fn shorthand_background_color() {
        let decls = first_decls("div { background: #fff; }");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].property, "background-color");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(255, 255, 255, 255))
        );
    }

    // -- test 10: colour parsing -------------------------------------

    #[test]
    fn color_named() {
        let decls = first_decls("p { color: red; }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(255, 0, 0, 255))
        );
    }

    #[test]
    fn color_hex_short() {
        let decls = first_decls("p { background-color: #abc; }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(0xaa, 0xbb, 0xcc, 255))
        );
    }

    #[test]
    fn color_hex_long() {
        let decls = first_decls("p { color: #11aa33; }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(0x11, 0xaa, 0x33, 255))
        );
    }

    #[test]
    fn color_hex_with_alpha() {
        let decls = first_decls("p { color: #11aa3380; }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(0x11, 0xaa, 0x33, 0x80))
        );
    }

    #[test]
    fn color_rgb_function() {
        let decls = first_decls("p { color: rgb(100, 200, 50); }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(100, 200, 50, 255))
        );
    }

    #[test]
    fn color_rgba_function() {
        let decls = first_decls("p { color: rgba(100, 200, 50, 0.5); }");
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(100, 200, 50, 127))
        );
    }

    #[test]
    fn color_transparent() {
        let decls = first_decls("p { color: transparent; }");
        assert_eq!(decls[0].value, CssValue::Color(CssColor::new(0, 0, 0, 0)));
    }

    // -- test 11: specificity ----------------------------------------

    #[test]
    fn specificity_type_only() {
        let sels = first_selectors("p { color: red; }");
        assert_eq!(
            sels.selectors[0].specificity(),
            Specificity {
                inline: 0,
                ids: 0,
                classes: 0,
                types: 1,
            }
        );
    }

    #[test]
    fn specificity_class() {
        let sels = first_selectors(".foo { color: red; }");
        assert_eq!(
            sels.selectors[0].specificity(),
            Specificity {
                inline: 0,
                ids: 0,
                classes: 1,
                types: 0,
            }
        );
    }

    #[test]
    fn specificity_id() {
        let sels = first_selectors("#bar { color: red; }");
        assert_eq!(
            sels.selectors[0].specificity(),
            Specificity {
                inline: 0,
                ids: 1,
                classes: 0,
                types: 0,
            }
        );
    }

    #[test]
    fn specificity_compound() {
        // p.intro#first => types=1, classes=1, ids=1
        let sels = first_selectors("p.intro#first { color: red; }");
        assert_eq!(
            sels.selectors[0].specificity(),
            Specificity {
                inline: 0,
                ids: 1,
                classes: 1,
                types: 1,
            }
        );
    }

    #[test]
    fn specificity_descendant() {
        // div p => types=2
        let sels = first_selectors("div p { color: red; }");
        assert_eq!(
            sels.selectors[0].specificity(),
            Specificity {
                inline: 0,
                ids: 0,
                classes: 0,
                types: 2,
            }
        );
    }

    #[test]
    fn specificity_ordering() {
        let a = Specificity {
            inline: 0,
            ids: 1,
            classes: 0,
            types: 0,
        };
        let b = Specificity {
            inline: 0,
            ids: 0,
            classes: 10,
            types: 10,
        };
        assert!(a > b, "ID selector should outrank classes + types");
    }

    // -- test 12: !important -----------------------------------------

    #[test]
    fn important_flag() {
        let decls = first_decls("p { color: red !important; }");
        assert!(decls[0].important);
        assert_eq!(
            decls[0].value,
            CssValue::Color(CssColor::new(255, 0, 0, 255))
        );
    }

    #[test]
    fn not_important() {
        let decls = first_decls("p { color: red; }");
        assert!(!decls[0].important);
    }

    // -- test 13: inline style parsing -------------------------------

    #[test]
    fn inline_style() {
        let decls = parse_inline_style("color: red; font-size: 16px;");
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].property, "color");
        assert_eq!(decls[1].property, "font-size");
    }

    // -- test 14: malformed input recovery ---------------------------

    #[test]
    fn malformed_recovery_bad_declaration() {
        // Missing colon -- the bad declaration should be skipped.
        let sheet = parse("p { color red; font-size: 12px; }");
        // At least font-size should survive.
        let decls = &sheet.rules[0].declarations;
        assert!(
            decls.iter().any(|d| d.property == "font-size"),
            "should recover and parse font-size"
        );
    }

    #[test]
    fn malformed_recovery_unclosed_brace() {
        // Unclosed rule should not panic.
        let sheet = parse("p { color: red; ");
        // May or may not produce a rule, but must not panic.
        let _ = sheet;
    }

    #[test]
    fn malformed_recovery_extra_close_brace() {
        let sheet = parse("} p { color: red; }");
        assert!(
            !sheet.rules.is_empty(),
            "should recover after stray close-brace"
        );
    }

    // -- font-weight normalisation -----------------------------------

    #[test]
    fn font_weight_bold() {
        let decls = first_decls("p { font-weight: bold; }");
        assert_eq!(decls[0].value, CssValue::Number(700.0));
    }

    #[test]
    fn font_weight_normal() {
        let decls = first_decls("p { font-weight: normal; }");
        assert_eq!(decls[0].value, CssValue::Number(400.0));
    }

    // -- multiple rules ---------------------------------------------

    #[test]
    fn multiple_rules() {
        let sheet = parse("p { color: red; } div { color: blue; }");
        assert_eq!(sheet.rules.len(), 2);
    }

    // -- at-rule skipping -------------------------------------------

    #[test]
    fn at_rule_skipped() {
        let sheet = parse("@import url('a.css'); p { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(
            sheet.rules[0].selectors.selectors[0].parts[0].0.parts[0],
            SimpleSelector::Type("p".into())
        );
    }

    #[test]
    fn at_media_screen_parsed() {
        let sheet = parse(
            "@media screen { body { color: red; } } \
             p { color: blue; }",
        );
        // @media screen matches, so body rule is included alongside p rule.
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn at_media_print_skipped() {
        let sheet = parse(
            "@media print { body { color: red; } } \
             p { color: blue; }",
        );
        // @media print does not match screen, so only p rule remains.
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_max_width_match() {
        // 480 <= 600, so this should match.
        let sheet = parse("@media (max-width: 600px) { p { color: red; } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_max_width_no_match() {
        // 480 > 320, so this should NOT match.
        let sheet = parse("@media (max-width: 320px) { p { color: red; } }");
        assert_eq!(sheet.rules.len(), 0);
    }

    // -- @media with custom viewport ----------------------------------

    #[test]
    fn at_media_min_height_match() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
        };
        let sheet =
            Stylesheet::parse_with_viewport("@media (min-height: 200px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_min_height_no_match() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
        };
        let sheet =
            Stylesheet::parse_with_viewport("@media (min-height: 600px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 0);
    }

    #[test]
    fn at_media_max_height_match() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
        };
        let sheet =
            Stylesheet::parse_with_viewport("@media (max-height: 400px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_max_height_no_match() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
        };
        let sheet =
            Stylesheet::parse_with_viewport("@media (max-height: 200px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 0);
    }

    #[test]
    fn at_media_custom_viewport_width() {
        let vp = MediaViewport {
            width: 1024.0,
            height: 768.0,
        };
        // With 1024px viewport, max-width: 320 should NOT match.
        let sheet =
            Stylesheet::parse_with_viewport("@media (max-width: 320px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 0);
        // But min-width: 800 SHOULD match.
        let sheet =
            Stylesheet::parse_with_viewport("@media (min-width: 800px) { p { color: red; } }", vp);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_screen_and_min_width() {
        let vp = MediaViewport {
            width: 800.0,
            height: 600.0,
        };
        let sheet = Stylesheet::parse_with_viewport(
            "@media screen and (min-width: 480px) { p { color: red; } }",
            vp,
        );
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_all_matches() {
        let sheet = parse("@media all { p { color: red; } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_prefers_color_scheme_dark() {
        let sheet = parse("@media (prefers-color-scheme: dark) { p { color: white; } }");
        // Dark mode is always false.
        assert_eq!(sheet.rules.len(), 0);
    }

    #[test]
    fn at_media_prefers_color_scheme_light() {
        let sheet = parse("@media (prefers-color-scheme: light) { p { color: black; } }");
        // Light mode is always true.
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn at_media_compound_width_and_height() {
        let vp = MediaViewport {
            width: 800.0,
            height: 600.0,
        };
        let sheet = Stylesheet::parse_with_viewport(
            "@media (min-width: 480px) and (min-height: 400px) { \
             p { color: red; } }",
            vp,
        );
        assert_eq!(sheet.rules.len(), 1);
        // Fail on height.
        let sheet = Stylesheet::parse_with_viewport(
            "@media (min-width: 480px) and (min-height: 800px) { \
             p { color: red; } }",
            vp,
        );
        assert_eq!(sheet.rules.len(), 0);
    }

    // -- pseudo-class ------------------------------------------------

    #[test]
    fn pseudo_class_selector() {
        let sels = first_selectors("a:hover { color: red; }");
        let parts = &sels.selectors[0].parts[0].0.parts;
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], SimpleSelector::Type("a".into()));
        assert_eq!(parts[1], SimpleSelector::PseudoClass("hover".into()));
    }

    // -- universal selector ------------------------------------------

    #[test]
    fn universal_selector() {
        let sels = first_selectors("* { margin: 0; }");
        assert_eq!(
            sels.selectors[0].parts[0].0.parts[0],
            SimpleSelector::Universal
        );
    }

    // -- empty stylesheet -------------------------------------------

    #[test]
    fn empty_stylesheet() {
        let sheet = parse("");
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn whitespace_only_stylesheet() {
        let sheet = parse("   \n\t  ");
        assert!(sheet.rules.is_empty());
    }

    // -- robustness / edge cases ----------------------------------------

    #[test]
    fn unclosed_rule_block() {
        let sheet = parse("p { color: red;");
        // Should not panic; may or may not produce a rule.
        let _ = sheet;
    }

    #[test]
    fn unclosed_value() {
        let sheet = parse("p { color: ");
        let _ = sheet;
    }

    #[test]
    fn missing_colon() {
        let sheet = parse("p { color red; }");
        // Malformed declaration -- parser should skip gracefully.
        let _ = sheet;
    }

    #[test]
    fn missing_semicolon_between_declarations() {
        let sheet = parse("p { color: red background: blue; }");
        let _ = sheet;
    }

    #[test]
    fn empty_selector() {
        let sheet = parse("{ color: red; }");
        let _ = sheet;
    }

    #[test]
    fn empty_declaration_block() {
        let sheet = parse("p { }");
        assert_eq!(sheet.rules.len(), 1);
        assert!(sheet.rules[0].declarations.is_empty());
    }

    #[test]
    fn very_long_property_value() {
        let val = "x".repeat(10_000);
        let css = format!("p {{ content: \"{val}\"; }}");
        let sheet = parse(&css);
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn very_long_selector_chain() {
        // div > div > div > ... (100 levels)
        let sel: String = (0..100).map(|_| "div").collect::<Vec<_>>().join(" > ");
        let css = format!("{sel} {{ color: red; }}");
        let sheet = parse(&css);
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn many_rules() {
        let css: String = (0..500)
            .map(|i| format!(".c{i} {{ color: red; }}"))
            .collect::<Vec<_>>()
            .join("\n");
        let sheet = parse(&css);
        assert_eq!(sheet.rules.len(), 500);
    }

    #[test]
    fn nested_braces() {
        // CSS doesn't normally nest, but parser should handle gracefully.
        let sheet = parse("p { color: red; { nested: bad; } }");
        let _ = sheet;
    }

    #[test]
    fn unmatched_closing_brace() {
        let sheet = parse("} p { color: red; }");
        let _ = sheet;
    }

    #[test]
    fn at_rule_unknown() {
        let sheet = parse("@unknown { p { color: red; } }");
        let _ = sheet;
    }

    #[test]
    fn at_media_rule() {
        let sheet = parse("@media screen { p { color: red; } }");
        // @media screen matches, inner rules are extracted.
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn comments_in_css() {
        let sheet = parse("/* comment */ p { color: red; /* inline */ }");
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn multiple_selectors_comma_separated() {
        let sheet = parse("h1, h2, h3 { color: blue; }");
        assert_eq!(sheet.rules.len(), 1);
        assert!(sheet.rules[0].selectors.selectors.len() >= 3);
    }

    #[test]
    fn selector_with_pseudo_class() {
        let sheet = parse("a:hover { color: red; }");
        let _ = sheet; // Should not panic.
    }

    #[test]
    fn selector_with_pseudo_element() {
        let sheet = parse("p::before { content: 'x'; }");
        let _ = sheet;
    }

    #[test]
    fn null_bytes_in_css() {
        let sheet = parse("p { color: re\0d; }");
        let _ = sheet;
    }

    #[test]
    fn extremely_specific_selector() {
        // #id.c1.c2.c3...c50
        let classes: String = (0..50).map(|i| format!(".c{i}")).collect();
        let css = format!("#id{classes} {{ color: red; }}");
        let sheet = parse(&css);
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn numeric_property_values() {
        let sheet = parse("p { width: 100px; height: 50%; margin: 0; }");
        let decls = &sheet.rules[0].declarations;
        // margin: 0 may be expanded into 4 longhand properties.
        assert!(decls.len() >= 3);
    }

    #[test]
    fn shorthand_property() {
        let sheet = parse("p { margin: 10px 20px 30px 40px; }");
        assert!(!sheet.rules.is_empty());
    }

    #[test]
    fn color_hex_values() {
        let sheet = parse("p { color: #fff; background: #aabbcc; border-color: #12345678; }");
        // border-color is expanded into 4 longhand properties.
        assert_eq!(sheet.rules[0].declarations.len(), 6);
    }

    #[test]
    fn trailing_garbage_after_rules() {
        let sheet = parse("p { color: red; } garbage here");
        // The first rule should still parse.
        assert!(!sheet.rules.is_empty());
    }

    // -- CSS custom properties / var() parsing tests ----------------------

    #[test]
    fn var_function_parsed() {
        let decls = first_decls("p { color: var(--my-color); }");
        assert_eq!(decls[0].property, "color");
        assert_eq!(decls[0].value, CssValue::Var("--my-color".into(), None));
    }

    #[test]
    fn var_function_with_fallback() {
        let decls = first_decls("p { color: var(--my-color, blue); }");
        assert_eq!(
            decls[0].value,
            CssValue::Var("--my-color".into(), Some("blue".into()))
        );
    }

    #[test]
    fn var_function_with_hex_fallback() {
        let decls = first_decls("p { color: var(--my-color, #202122); }");
        assert_eq!(
            decls[0].value,
            CssValue::Var("--my-color".into(), Some("#202122".into()))
        );
    }

    #[test]
    fn custom_property_stored_as_raw_text() {
        let decls = first_decls(":root { --color: #202122; }");
        assert_eq!(decls[0].property, "--color");
        assert_eq!(decls[0].value, CssValue::String("#202122".into()));
    }

    #[test]
    fn custom_property_complex_value() {
        let decls = first_decls(":root { --border: 1px solid red; }");
        assert_eq!(decls[0].property, "--border");
        assert_eq!(decls[0].value, CssValue::String("1px solid red".into()));
    }

    #[test]
    fn var_in_multiple_value_property() {
        let decls = first_decls("p { border: 1px solid var(--color); }");
        // The border shorthand should expand, and var() should end up
        // in border-color.
        let bc = decls.iter().find(|d| d.property == "border-color");
        assert!(bc.is_some(), "border-color should exist");
        assert!(
            matches!(&bc.unwrap().value, CssValue::Var(name, None) if name == "--color"),
            "border-color should be var(--color)"
        );
    }

    #[test]
    fn linear_gradient_to_right() {
        let css = "div { background: linear-gradient(to right, red, blue); }";
        let sheet = parse(css);
        let decls = &sheet.rules[0].declarations;
        let bg_image = decls
            .iter()
            .find(|d| d.property == "background-image")
            .expect("should have background-image");
        assert!(
            matches!(&bg_image.value, CssValue::Gradient(_)),
            "should parse as gradient"
        );
        if let CssValue::Gradient(ref g) = bg_image.value {
            assert_eq!(g.direction, crate::css::values::GradientDirection::ToRight);
            assert_eq!(g.stops.len(), 2);
        }
    }

    #[test]
    fn linear_gradient_default_direction() {
        let css = "div { background-image: linear-gradient(red, blue); }";
        let sheet = parse(css);
        let decls = &sheet.rules[0].declarations;
        let bg_image = decls
            .iter()
            .find(|d| d.property == "background-image")
            .expect("should have background-image");
        assert!(
            matches!(&bg_image.value, CssValue::Gradient(_)),
            "expected gradient"
        );
        let CssValue::Gradient(g) = &bg_image.value else {
            unreachable!()
        };
        assert_eq!(g.direction, crate::css::values::GradientDirection::ToBottom);
        assert_eq!(g.stops.len(), 2);
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Parsing arbitrary CSS never panics.
            #[test]
            fn parse_never_panics(input in "[ -~]{0,120}") {
                let _ = Stylesheet::parse(&input);
            }

            /// Parsing inline styles never panics.
            #[test]
            fn parse_inline_never_panics(input in "[ -~]{0,80}") {
                let _ = parse_inline_style(&input);
            }

            /// Valid 3-digit hex colors parse successfully.
            #[test]
            fn hex_color_3_digit(
                r in "[0-9a-fA-F]",
                g in "[0-9a-fA-F]",
                b in "[0-9a-fA-F]",
            ) {
                let hex = format!("#{r}{g}{b}");
                let color = parse_hex_color(&hex);
                prop_assert!(
                    color.is_some(),
                    "valid 3-digit hex '{hex}' should parse",
                );
            }

            /// Valid 6-digit hex colors parse successfully.
            #[test]
            fn hex_color_6_digit(
                r in "[0-9a-fA-F]{2}",
                g in "[0-9a-fA-F]{2}",
                b in "[0-9a-fA-F]{2}",
            ) {
                let hex = format!("#{r}{g}{b}");
                let color = parse_hex_color(&hex);
                prop_assert!(
                    color.is_some(),
                    "valid 6-digit hex '{hex}' should parse",
                );
            }

            /// Invalid hex strings (wrong length) return None.
            #[test]
            fn hex_color_bad_length(
                s in "[0-9a-f]{1,2}|[0-9a-f]{5}|[0-9a-f]{7}|[0-9a-f]{9,12}",
            ) {
                let hex = format!("#{s}");
                prop_assert!(
                    parse_hex_color(&hex).is_none(),
                    "invalid-length hex '{hex}' should not parse",
                );
            }

            /// Named color lookup is case-insensitive.
            #[test]
            fn named_color_case_insensitive(
                name in proptest::sample::select(vec![
                    "red".to_string(), "Red".to_string(), "RED".to_string(),
                    "blue".to_string(), "Blue".to_string(), "BLUE".to_string(),
                    "green".to_string(), "Green".to_string(), "GREEN".to_string(),
                    "white".to_string(), "White".to_string(),
                    "black".to_string(), "Black".to_string(), "BLACK".to_string(),
                ]),
            ) {
                prop_assert!(
                    named_color(&name).is_some(),
                    "named color '{}' should be recognized", name,
                );
            }

            /// A valid rule with random property name parses without panic.
            #[test]
            fn rule_with_random_property(
                prop_name in "[a-z\\-]{1,20}",
                value in "[a-z0-9]{1,10}",
            ) {
                let css = format!("p {{ {prop_name}: {value}; }}");
                let sheet = Stylesheet::parse(&css);
                // Should parse the rule (property may not be recognized,
                // but shouldn't panic).
                prop_assert!(!sheet.rules.is_empty());
            }
        }
    }
}

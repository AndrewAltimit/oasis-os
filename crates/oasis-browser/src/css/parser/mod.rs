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
    AttrOp, Combinator, CompoundSelector, ContainerCondition, ContainerFeature, CssColor, CssValue,
    Declaration, KeyframeStop, KeyframesRule, LengthUnit, PropertyId, Rule, Selector,
    SimpleSelector, Specificity, Stylesheet,
};

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

/// Parse a CSS selector string into a [`SelectorList`].
///
/// Used by `querySelector` / `querySelectorAll` to leverage the full
/// CSS selector engine (combinators, attribute selectors, pseudo-classes).
#[cfg_attr(not(feature = "javascript"), allow(dead_code))]
pub(crate) fn parse_selector_string(input: &str) -> Option<SelectorList> {
    use super::tokenizer::CssTokenizer;
    let tokens = CssTokenizer::new(input).tokenize();
    let mut parser = CssParser::new(tokens, MediaViewport::DEFAULT);
    parser.parse_selector_list()
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
    /// Declared cascade layers in order of first appearance.
    /// Synthetic `__anon_N` names are used for `@layer { ... }` blocks.
    layers: Vec<String>,
    /// If we're currently inside a `@layer name { ... }` block, this
    /// is the layer's index in `layers`; otherwise `None`.
    current_layer: Option<u16>,
}

impl CssParser {
    fn new(tokens: Vec<CssToken>, viewport: MediaViewport) -> Self {
        Self {
            tokens,
            pos: 0,
            viewport,
            layers: Vec::new(),
            current_layer: None,
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

    /// Maximum number of CSS rules to prevent pathological stylesheets
    /// from consuming unbounded memory during parsing.
    const MAX_CSS_RULES: usize = 50_000;

    fn parse_stylesheet(&mut self) -> Stylesheet {
        let mut rules = Vec::new();
        let mut keyframes = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() {
                break;
            }
            if rules.len() >= Self::MAX_CSS_RULES {
                log::warn!(
                    "CSS rule limit reached ({}), truncating stylesheet",
                    Self::MAX_CSS_RULES
                );
                break;
            }
            if let Some(lc) = self.peek_at_keyword_lc() {
                if lc == "media" {
                    // Parse @media rules and include matching ones.
                    for r in self.parse_media_rule() {
                        flatten_nested_rule(None, r, &mut rules);
                    }
                } else if lc == "supports" {
                    for r in self.parse_supports_rule() {
                        flatten_nested_rule(None, r, &mut rules);
                    }
                } else if lc == "keyframes" || lc == "-webkit-keyframes" {
                    if let Some(kf) = self.parse_keyframes_rule() {
                        keyframes.push(kf);
                    }
                } else if lc == "layer" {
                    for r in self.parse_layer_rule() {
                        flatten_nested_rule(None, r, &mut rules);
                    }
                } else if lc == "container" {
                    for r in self.parse_container_rule() {
                        flatten_nested_rule(None, r, &mut rules);
                    }
                } else {
                    // Other at-rules: skip.
                    self.skip_at_rule();
                }
                continue;
            }
            match self.try_parse_rule() {
                Some(rule) => flatten_nested_rule(None, rule, &mut rules),
                None => {
                    // Recovery: skip one token and try again.
                    self.advance();
                },
            }
        }
        Stylesheet {
            rules,
            keyframes,
            layers: std::mem::take(&mut self.layers),
        }
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
    fn parse_media_rule(&mut self) -> Vec<ParsedRule> {
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
    fn parse_supports_rule(&mut self) -> Vec<ParsedRule> {
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

    /// Parse an `@layer` rule. Supports three forms:
    ///
    /// 1. `@layer foo;` — statement form, registers `foo` so its
    ///    position in the layer order is fixed even before any rules
    ///    are declared inside it.
    /// 2. `@layer foo, bar, baz;` — multi-name statement form, same
    ///    as above but registers several layers at once.
    /// 3. `@layer foo { ... }` — block form, registers `foo` (if not
    ///    already registered) and tags every rule inside with it.
    /// 4. `@layer { ... }` — anonymous block form, generates a
    ///    synthetic name so the layer remains distinct.
    ///
    /// Nested layer names like `foo.bar` are parsed as a single name
    /// string (`"foo.bar"`); we don't currently model hierarchical
    /// layer nesting, so each dotted name is just an opaque identifier.
    fn parse_layer_rule(&mut self) -> Vec<ParsedRule> {
        self.advance(); // consume @layer token
        self.skip_whitespace();

        // Collect the first layer name (if any).
        let mut names: Vec<String> = Vec::new();
        if let Some(first) = self.parse_layer_name() {
            names.push(first);
            loop {
                self.skip_whitespace();
                if self.peek() == &CssToken::Comma {
                    self.advance();
                    self.skip_whitespace();
                    if let Some(n) = self.parse_layer_name() {
                        names.push(n);
                    }
                } else {
                    break;
                }
            }
        }

        self.skip_whitespace();
        match self.peek() {
            CssToken::Semicolon => {
                self.advance();
                // Statement form: just register the names in order.
                for n in &names {
                    self.register_layer(n);
                }
                Vec::new()
            },
            CssToken::OpenBrace => {
                self.advance();
                // Block form. Multi-name block (`@layer a, b { ... }`)
                // isn't allowed by the spec; treat it as using the
                // first name and register the rest as empty layers.
                let name = if names.is_empty() {
                    self.synthesise_anonymous_layer_name()
                } else {
                    names[0].clone()
                };
                let layer_idx = self.register_layer(&name);
                for extra in names.iter().skip(1) {
                    self.register_layer(extra);
                }

                let previous = self.current_layer;
                self.current_layer = Some(layer_idx);
                let inner_rules = self.parse_at_rule_block();
                self.current_layer = previous;
                inner_rules
            },
            _ => {
                // Malformed; skip until we reach `;` or a closing brace.
                self.skip_to_semicolon_or_close_brace();
                Vec::new()
            },
        }
    }

    /// Parse an `@container [name?] (condition) { ... }` rule.
    ///
    /// The optional name precedes the condition. Conditions are joined
    /// with `and` and may use `min-width` / `max-width` / `width` /
    /// `min-height` / `max-height` / `height`, plus the
    /// `inline-size` / `block-size` aliases (treated as their physical
    /// equivalents under our LTR-only assumption). Anything we can't
    /// parse evaluates as never-matching at cascade time.
    fn parse_container_rule(&mut self) -> Vec<ParsedRule> {
        self.advance(); // consume @container token
        self.skip_whitespace();

        // Optional container name comes before the condition. It's a
        // bare ident (not introduced by `(`).
        let name = if let CssToken::Ident(s) = self.peek().clone() {
            self.advance();
            self.skip_whitespace();
            Some(s)
        } else {
            None
        };

        // Collect everything up to the opening brace as the condition.
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

        if !self.expect(&CssToken::OpenBrace) {
            return Vec::new();
        }

        let inner_rules = self.parse_at_rule_block();

        let cond = parse_container_condition(name, &condition);
        // Tag every emitted child rule with this condition. Inner rules
        // that already carry a container condition (from a nested
        // `@container`) keep theirs — innermost wins. A future
        // refinement could AND the conditions instead.
        let mut tagged = Vec::with_capacity(inner_rules.len());
        for mut r in inner_rules {
            if r.container.is_none() {
                r.container = Some(cond.clone());
            }
            tagged.push(r);
        }
        tagged
    }

    /// Parse a single layer name: an identifier optionally followed by
    /// `.ident` segments (e.g. `framework.theme`). Returns `None` if
    /// no identifier is present.
    fn parse_layer_name(&mut self) -> Option<String> {
        let mut out = String::new();
        let CssToken::Ident(first) = self.peek().clone() else {
            return None;
        };
        self.advance();
        out.push_str(&first);
        loop {
            if self.peek() == &CssToken::Delim('.') {
                self.advance();
                if let CssToken::Ident(next) = self.peek().clone() {
                    self.advance();
                    out.push('.');
                    out.push_str(&next);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Some(out)
    }

    /// Register a layer by name if it isn't already registered.
    /// Returns the layer's index in `self.layers`.
    fn register_layer(&mut self, name: &str) -> u16 {
        if let Some(idx) = self.layers.iter().position(|n| n == name) {
            return idx as u16;
        }
        let idx = self.layers.len();
        self.layers.push(name.to_string());
        idx as u16
    }

    /// Generate a unique synthetic name for an anonymous `@layer { ... }`.
    fn synthesise_anonymous_layer_name(&self) -> String {
        format!("__anon_{}", self.layers.len())
    }

    /// Parse the body of an at-rule that wraps style rules (like
    /// `@layer foo { ... }`). Consumes the closing `}`.
    fn parse_at_rule_block(&mut self) -> Vec<ParsedRule> {
        let mut inner_rules = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            if let Some(lc) = self.peek_at_keyword_lc() {
                if lc == "layer" {
                    // Nested @layer (e.g. @layer framework { @layer
                    // base { ... } }). Recursively parsed and registered.
                    inner_rules.extend(self.parse_layer_rule());
                    continue;
                }
                if lc == "media" {
                    inner_rules.extend(self.parse_media_rule());
                    continue;
                }
                if lc == "supports" {
                    inner_rules.extend(self.parse_supports_rule());
                    continue;
                }
                if lc == "container" {
                    inner_rules.extend(self.parse_container_rule());
                    continue;
                }
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
        inner_rules
    }

    /// Recovery helper for malformed `@layer` statements.
    fn skip_to_semicolon_or_close_brace(&mut self) {
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

    fn try_parse_rule(&mut self) -> Option<ParsedRule> {
        let selectors = self.parse_selector_list()?;
        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            // Recovery: skip to next `}` or EOF.
            self.skip_to_close_brace();
            return None;
        }
        let (declarations, nested) = self.parse_rule_body();
        self.expect(&CssToken::CloseBrace);
        let declarations = expand_shorthands(declarations);
        Some(ParsedRule {
            selectors,
            declarations,
            nested,
            layer: self.current_layer,
            container: None,
        })
    }

    /// Parse the body of a style rule — a mixture of declarations and
    /// nested style rules (CSS Nesting). Uses lookahead to distinguish
    /// declarations (`prop: value;`) from nested rules (`selector { ... }`).
    fn parse_rule_body(&mut self) -> (Vec<types::Declaration>, Vec<ParsedRule>) {
        let mut decls = Vec::new();
        let mut nested = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                CssToken::CloseBrace | CssToken::Eof => break,
                CssToken::AtKeyword(_) => {
                    // Nested at-rules (@media inside a rule). We only
                    // support @media here; other at-rules are skipped.
                    let lc = match self.peek_at_keyword_lc() {
                        Some(s) => s,
                        None => {
                            self.advance();
                            continue;
                        },
                    };
                    if lc == "media" {
                        nested.extend(self.parse_nested_media_rule());
                    } else {
                        self.skip_at_rule();
                    }
                    continue;
                },
                _ => {},
            }
            if self.lookahead_is_nested_rule() {
                if let Some(rule) = self.try_parse_rule() {
                    nested.push(rule);
                } else {
                    self.advance();
                }
            } else if let Some(decl) = self.try_parse_declaration() {
                decls.push(decl);
            } else {
                self.skip_to_semicolon_or_brace_in_body();
            }
        }
        (decls, nested)
    }

    /// Lookahead: scan forward from `self.pos`, tracking paren/bracket
    /// depth. Return `true` if the next block-terminating token at depth 0
    /// is `{` (qualified rule) rather than `;`, `}`, or EOF (declaration).
    fn lookahead_is_nested_rule(&self) -> bool {
        let mut i = self.pos;
        let mut paren = 0i32;
        let mut bracket = 0i32;
        while i < self.tokens.len() {
            match &self.tokens[i] {
                CssToken::Eof => return false,
                CssToken::OpenParen => paren += 1,
                CssToken::CloseParen => paren -= 1,
                CssToken::OpenBracket => bracket += 1,
                CssToken::CloseBracket => bracket -= 1,
                CssToken::OpenBrace if paren == 0 && bracket == 0 => return true,
                CssToken::Semicolon if paren == 0 && bracket == 0 => return false,
                CssToken::CloseBrace if paren == 0 && bracket == 0 => return false,
                _ => {},
            }
            i += 1;
        }
        false
    }

    fn skip_to_semicolon_or_brace_in_body(&mut self) {
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

    /// Parse a nested `@media (...) { ... }` rule: returns the inner
    /// parsed rules (with the enclosing rule's selectors still un-applied)
    /// if the media query matches; empty otherwise.
    fn parse_nested_media_rule(&mut self) -> Vec<ParsedRule> {
        self.advance(); // @media
        self.skip_whitespace();
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
        if !self.expect(&CssToken::OpenBrace) {
            return Vec::new();
        }
        let (_decls, inner) = self.parse_rule_body();
        self.expect(&CssToken::CloseBrace);
        if eval_media_query_with_viewport(&condition, self.viewport) {
            inner
        } else {
            Vec::new()
        }
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
// CSS Nesting: parsed rule tree and flattening
// -------------------------------------------------------------------

/// A rule as it comes out of the parser, possibly carrying nested child
/// rules from CSS Nesting. Flattened into concrete [`Rule`]s by
/// [`flatten_nested_rule`] before the stylesheet is finalised.
struct ParsedRule {
    selectors: SelectorList,
    declarations: Vec<types::Declaration>,
    nested: Vec<ParsedRule>,
    /// Cascade layer the rule belongs to, or `None` if unlayered.
    layer: Option<u16>,
    /// `@container` condition the rule was nested inside, if any.
    container: Option<ContainerCondition>,
}

/// Flatten a (possibly-nested) parsed rule into concrete [`Rule`]s,
/// desugaring the `&` nesting selector using the CSS Nesting semantics.
///
/// Parent and child selector lists are combined via a Cartesian product:
/// for each (parent, child) pair, we substitute `&` with the parent's
/// compound chain. Child rules without any `&` are treated as
/// `& <child>` (descendant combinator), matching the spec.
fn flatten_nested_rule(parent: Option<&SelectorList>, rule: ParsedRule, out: &mut Vec<Rule>) {
    let ParsedRule {
        selectors,
        declarations,
        nested,
        layer,
        container,
    } = rule;

    let effective = match parent {
        None => selectors,
        Some(parent_list) => combine_selector_lists(parent_list, &selectors),
    };

    // Skip emitting an empty-body parent rule when it exists purely as a
    // container for nested children. A bare `p {}` with no nested
    // children is still emitted so that existing behaviour is preserved.
    if !declarations.is_empty() || nested.is_empty() {
        out.push(Rule {
            selectors: effective.clone(),
            declarations,
            layer,
            container: container.clone(),
        });
    }

    for child in nested {
        flatten_nested_rule(Some(&effective), child, out);
    }
}

/// Combine a parent selector list with a child selector list using the
/// CSS Nesting desugaring rules. Produces the Cartesian product of
/// (parent × child) with `&` substituted for each parent selector.
fn combine_selector_lists(parent: &SelectorList, child: &SelectorList) -> SelectorList {
    let mut out = Vec::with_capacity(parent.selectors.len() * child.selectors.len());
    for child_sel in &child.selectors {
        for parent_sel in &parent.selectors {
            out.push(combine_parent_child(parent_sel, child_sel));
        }
    }
    SelectorList { selectors: out }
}

/// Combine one parent selector with one child selector. Replaces each
/// occurrence of the nesting marker `&` in `child` with `parent`'s
/// compound chain. If `child` does not contain `&`, the result is
/// `parent <descendant> child`.
fn combine_parent_child(parent: &Selector, child: &Selector) -> Selector {
    let child_has_nest = child.parts.iter().any(|(c, _)| {
        c.parts
            .iter()
            .any(|s| matches!(s, types::SimpleSelector::Nest))
    });

    if !child_has_nest {
        // Implicit `& descendant`: prepend parent chain before child,
        // with a descendant combinator linking them.
        let mut parts = parent.parts.clone();
        for (i, (compound, comb)) in child.parts.iter().enumerate() {
            let effective_comb = if i == 0 {
                Some(Combinator::Descendant)
            } else {
                comb.clone()
            };
            parts.push((compound.clone(), effective_comb));
        }
        return Selector { parts };
    }

    // Child contains `&` — splice parent into each occurrence.
    let mut out: Vec<(CompoundSelector, Option<Combinator>)> = Vec::new();
    for (compound, combinator) in &child.parts {
        let has_nest_here = compound
            .parts
            .iter()
            .any(|s| matches!(s, types::SimpleSelector::Nest));
        if !has_nest_here {
            let effective_comb = if out.is_empty() {
                None
            } else {
                combinator.clone()
            };
            out.push((compound.clone(), effective_comb));
            continue;
        }

        // Extract the non-`&` parts of this compound.
        let extras: Vec<types::SimpleSelector> = compound
            .parts
            .iter()
            .filter(|s| !matches!(s, types::SimpleSelector::Nest))
            .cloned()
            .collect();

        if parent.parts.is_empty() {
            // Shouldn't happen — a parent selector always has parts.
            continue;
        }
        let parent_last_idx = parent.parts.len() - 1;
        for (pi, (pcomp, pcomb)) in parent.parts.iter().enumerate() {
            let effective_comb = if pi == 0 {
                if out.is_empty() {
                    None
                } else {
                    combinator.clone()
                }
            } else {
                pcomb.clone()
            };
            if pi == parent_last_idx && !extras.is_empty() {
                let mut merged = pcomp.parts.clone();
                merged.extend(extras.iter().cloned());
                out.push((CompoundSelector { parts: merged }, effective_comb));
            } else {
                out.push((pcomp.clone(), effective_comb));
            }
        }
    }

    Selector { parts: out }
}

// -------------------------------------------------------------------
// @container condition parsing
// -------------------------------------------------------------------

/// Parse a `@container` condition string into a [`ContainerCondition`].
///
/// Recognises `(min-width: Npx)`, `(max-width: Npx)`, `(width: Npx)`,
/// the `height` variants, and the `inline-size` / `block-size` aliases
/// (treated as their physical equivalents under our LTR-only horizontal
/// writing-mode assumption). Multiple features are joined with ` and `.
/// Any feature we can't parse causes that predicate to be dropped — the
/// remaining predicates still apply, and an empty feature list always
/// evaluates true.
fn parse_container_condition(name: Option<String>, raw: &str) -> ContainerCondition {
    let mut features = Vec::new();
    let raw = raw.trim();
    if !raw.is_empty() {
        for part in raw.split(" and ") {
            let inner = part
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            if let Some(f) = parse_container_feature(inner) {
                features.push(f);
            }
        }
    }
    ContainerCondition { name, features }
}

fn parse_container_feature(inner: &str) -> Option<ContainerFeature> {
    let (key, value) = inner.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    let px = parse_px_in_condition(value.trim())?;
    Some(match key.as_str() {
        "min-width" | "min-inline-size" => ContainerFeature::MinWidth(px),
        "max-width" | "max-inline-size" => ContainerFeature::MaxWidth(px),
        "width" | "inline-size" => ContainerFeature::Width(px),
        "min-height" | "min-block-size" => ContainerFeature::MinHeight(px),
        "max-height" | "max-block-size" => ContainerFeature::MaxHeight(px),
        "height" | "block-size" => ContainerFeature::Height(px),
        _ => return None,
    })
}

fn parse_px_in_condition(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    s.trim().parse::<f32>().ok()
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
    "background-size",
    "background-position",
    "background-repeat",
    "text-decoration-line",
    "text-decoration-color",
    "text-decoration-style",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
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
    "overflow-x",
    "overflow-y",
    "cursor",
    "pointer-events",
    "user-select",
    "aspect-ratio",
    "text-underline-offset",
    "object-position",
    "place-items",
    "place-content",
    "appearance",
    "-webkit-appearance",
    "-moz-appearance",
    "-webkit-line-clamp",
    "line-clamp",
    "-webkit-box-orient",
    "accent-color",
    "caret-color",
    "color-scheme",
    "isolation",
    "resize",
    "touch-action",
    "grid-template",
    // Mask longhands (compositor overhaul PR6).
    "mask-image",
    "mask-mode",
    "mask-composite",
    "mask-clip",
    "mask-origin",
    "mask-position",
    "mask-size",
    "mask-repeat",
    // Container queries.
    "container-type",
    "container-name",
    "container",
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

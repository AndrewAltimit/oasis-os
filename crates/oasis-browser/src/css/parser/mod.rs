//! CSS parser.
//!
//! Consumes the token stream produced by [`super::tokenizer::CssTokenizer`]
//! and builds a typed stylesheet AST with selectors, declarations, specificity,
//! shorthand expansion, and color parsing.

mod containers;
mod declarations;
mod font_face;
mod nested;
mod selectors;
mod supports;
mod types;

pub use super::helpers::MediaViewport;
pub(crate) use declarations::parse_value_list;
#[allow(unused_imports)]
pub use types::{
    AttrOp, Combinator, CompoundSelector, ContainerCondition, ContainerFeature, CounterStyleRule,
    CssColor, CssValue, Declaration, FontDisplay, FontFaceRule, FontFaceSrc, FontFaceStyle,
    KeyframeStop, KeyframesRule, LengthUnit, PropertyId, PropertyRule, Rule, ScopeCondition,
    Selector, SimpleSelector, Specificity, Stylesheet, UnicodeRange,
};

pub(crate) use types::SelectorList;

use containers::parse_container_condition;
use font_face::{
    parse_additive_symbols, parse_font_weight_descriptor, parse_unicode_range_list,
    split_counter_symbols, unquote,
};
use nested::{ParsedRule, flatten_nested_rule};
use supports::eval_supports_condition;

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
        let mut counter_styles: Vec<CounterStyleRule> = Vec::new();
        let mut properties: Vec<PropertyRule> = Vec::new();
        let mut font_faces: Vec<types::FontFaceRule> = Vec::new();
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
                } else if lc == "scope" {
                    for r in self.parse_scope_rule() {
                        flatten_nested_rule(None, r, &mut rules);
                    }
                } else if lc == "counter-style" {
                    if let Some(cs) = self.parse_counter_style_rule() {
                        counter_styles.push(cs);
                    }
                } else if lc == "property" {
                    if let Some(pr) = self.parse_property_rule() {
                        properties.push(pr);
                    }
                } else if lc == "font-face" {
                    if let Some(ff) = self.parse_font_face_rule() {
                        font_faces.push(ff);
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
            counter_styles,
            properties,
            font_faces,
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
                        // The CSS tokenizer collapses `<ident>(` into a
                        // single Function token, which would otherwise
                        // be silently dropped from the condition string
                        // (breaking e.g. zero-space `@container (a)and(b)`
                        // where `and(` tokenizes as Function("and")).
                        CssToken::Function(ref s) => {
                            condition.push_str(s);
                            condition.push('(');
                        },
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
                        // The CSS tokenizer collapses `<ident>(` into a
                        // single Function token, which would otherwise
                        // be silently dropped from the condition string
                        // (breaking e.g. zero-space `@container (a)and(b)`
                        // where `and(` tokenizes as Function("and")).
                        CssToken::Function(ref s) => {
                            condition.push_str(s);
                            condition.push('(');
                        },
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
                        // The CSS tokenizer collapses `<ident>(` into a
                        // single Function token, which would otherwise
                        // be silently dropped from the condition string
                        // (breaking e.g. zero-space `@container (a)and(b)`
                        // where `and(` tokenizes as Function("and")).
                        CssToken::Function(ref s) => {
                            condition.push_str(s);
                            condition.push('(');
                        },
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
        // Tag every emitted child rule with this condition. For nested
        // `@container` rules, AND-combine by merging features so both
        // the outer and inner conditions must hold.
        let mut tagged = Vec::with_capacity(inner_rules.len());
        for mut r in inner_rules {
            if let Some(ref mut inner) = r.container {
                // AND-combine: append outer features to the inner
                // condition. The container name, if any, comes from
                // the inner (more specific) condition; fall back to
                // the outer name if the inner doesn't specify one.
                inner.features.extend(cond.features.iter().cloned());
                if inner.name.is_none() {
                    inner.name.clone_from(&cond.name);
                }
            } else {
                r.container = Some(cond.clone());
            }
            tagged.push(r);
        }
        tagged
    }

    /// Parse an `@scope (root) [to (limit)]? { ... }` rule.
    ///
    /// Both root and limit selectors are kept as raw strings — the
    /// cascade re-parses them via `parse_selector_string` so the full
    /// selector engine handles them. A bare `@scope { ... }` (no root)
    /// is allowed and means "no root constraint" in our model; a
    /// real browser would scope it to the stylesheet's owner element,
    /// but for `<style>` blocks that's effectively `<html>`.
    fn parse_scope_rule(&mut self) -> Vec<ParsedRule> {
        self.advance(); // consume @scope
        self.skip_whitespace();

        let root = self.parse_optional_parenthesised_selector();
        self.skip_whitespace();

        // Optional `to (limit)` clause.
        let mut limit = None;
        if let CssToken::Ident(s) = self.peek().clone()
            && s.eq_ignore_ascii_case("to")
        {
            self.advance();
            self.skip_whitespace();
            limit = self.parse_optional_parenthesised_selector();
            self.skip_whitespace();
        }

        if !self.expect(&CssToken::OpenBrace) {
            return Vec::new();
        }

        let inner_rules = self.parse_at_rule_block();
        let cond = ScopeCondition { root, limit };
        let mut tagged = Vec::with_capacity(inner_rules.len());
        for mut r in inner_rules {
            if r.scope.is_none() {
                r.scope = Some(cond.clone());
            }
            tagged.push(r);
        }
        tagged
    }

    /// Read a `(...)` group as a raw selector string. Returns `None`
    /// if the next token isn't an open paren. The contents are
    /// reassembled with single-space whitespace collapsing so the
    /// downstream selector parser can handle them.
    fn parse_optional_parenthesised_selector(&mut self) -> Option<String> {
        if self.peek() != &CssToken::OpenParen {
            return None;
        }
        self.advance(); // consume (
        let mut depth: i32 = 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::OpenParen => {
                    depth += 1;
                    out.push('(');
                    self.advance();
                },
                CssToken::CloseParen => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        break;
                    }
                    out.push(')');
                },
                _ => {
                    let tok = self.peek().clone();
                    self.advance();
                    match tok {
                        CssToken::Ident(s) => out.push_str(&s),
                        CssToken::Hash(s) => {
                            out.push('#');
                            out.push_str(&s);
                        },
                        CssToken::Number(n) => out.push_str(&format!("{n}")),
                        CssToken::Whitespace => out.push(' '),
                        CssToken::Colon => out.push(':'),
                        CssToken::Comma => out.push(','),
                        CssToken::Delim(c) => out.push(c),
                        CssToken::Dot => out.push('.'),
                        _ => {},
                    }
                },
            }
        }
        let trimmed = out.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Parse a `@counter-style name { descriptor: value; ... }` rule.
    /// Currently parse-only — descriptors are stored as raw strings or
    /// lightly typed. List-item rendering still uses the built-in
    /// styles only.
    fn parse_counter_style_rule(&mut self) -> Option<CounterStyleRule> {
        self.advance(); // consume @counter-style
        self.skip_whitespace();
        let name = match self.peek().clone() {
            CssToken::Ident(s) => {
                self.advance();
                s
            },
            CssToken::String(s) => {
                self.advance();
                s
            },
            _ => {
                self.skip_to_close_brace();
                return None;
            },
        };
        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            return None;
        }

        let mut rule = CounterStyleRule {
            name,
            system: None,
            symbols: Vec::new(),
            additive_symbols: Vec::new(),
            range: None,
            prefix: None,
            suffix: None,
            pad: None,
            negative: None,
            fallback: None,
            speak_as: None,
        };

        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            let key = match self.peek().clone() {
                CssToken::Ident(s) => {
                    self.advance();
                    s.to_ascii_lowercase()
                },
                _ => {
                    self.advance();
                    continue;
                },
            };
            self.skip_whitespace();
            if !self.expect(&CssToken::Colon) {
                self.skip_to_semicolon_or_close_brace();
                continue;
            }
            self.skip_whitespace();
            let raw = self.collect_descriptor_value();
            match key.as_str() {
                "system" => rule.system = Some(raw.trim().to_ascii_lowercase()),
                "symbols" => {
                    rule.symbols = split_counter_symbols(&raw);
                },
                "additive-symbols" => {
                    rule.additive_symbols = parse_additive_symbols(&raw);
                },
                "range" => rule.range = Some(raw.trim().to_string()),
                "prefix" => rule.prefix = Some(unquote(raw.trim())),
                "suffix" => rule.suffix = Some(unquote(raw.trim())),
                "pad" => rule.pad = Some(raw.trim().to_string()),
                "negative" => rule.negative = Some(raw.trim().to_string()),
                "fallback" => rule.fallback = Some(raw.trim().to_string()),
                "speak-as" => rule.speak_as = Some(raw.trim().to_string()),
                _ => {},
            }
            // Consume optional terminating `;`.
            if self.peek() == &CssToken::Semicolon {
                self.advance();
            }
        }
        self.expect(&CssToken::CloseBrace);
        Some(rule)
    }

    /// Parse an `@property --name { ... }` registration. Reads the
    /// `syntax`, `inherits`, and `initial-value` descriptors. Other
    /// descriptors are tolerated and ignored.
    fn parse_property_rule(&mut self) -> Option<PropertyRule> {
        self.advance(); // consume @property
        self.skip_whitespace();
        // Property name is a `<custom-property-name>` token, which our
        // tokenizer surfaces as `Ident("--foo")` (the leading `--`
        // makes it a normal ident from a tokenizer's point of view).
        let name = match self.peek().clone() {
            CssToken::Ident(s) if s.starts_with("--") => {
                self.advance();
                s
            },
            _ => {
                self.skip_to_close_brace();
                return None;
            },
        };
        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            return None;
        }

        let mut rule = PropertyRule {
            name,
            syntax: None,
            inherits: false,
            initial_value: None,
        };

        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            let key = match self.peek().clone() {
                CssToken::Ident(s) => {
                    self.advance();
                    s.to_ascii_lowercase()
                },
                _ => {
                    self.advance();
                    continue;
                },
            };
            self.skip_whitespace();
            if !self.expect(&CssToken::Colon) {
                self.skip_to_semicolon_or_close_brace();
                continue;
            }
            self.skip_whitespace();
            let raw = self.collect_descriptor_value();
            match key.as_str() {
                "syntax" => rule.syntax = Some(unquote(raw.trim())),
                "inherits" => rule.inherits = raw.trim().eq_ignore_ascii_case("true"),
                "initial-value" => rule.initial_value = Some(raw.trim().to_string()),
                _ => {},
            }
            if self.peek() == &CssToken::Semicolon {
                self.advance();
            }
        }
        self.expect(&CssToken::CloseBrace);
        Some(rule)
    }

    /// Parse an `@font-face { ... }` rule into a [`FontFaceRule`].
    ///
    /// CSS `@font-face` has no prelude — it goes straight to a
    /// declaration block of descriptors:
    ///
    /// ```css
    /// @font-face {
    ///   font-family: "Open Sans";
    ///   src: url("open-sans.woff2") format("woff2"),
    ///        url("open-sans.woff") format("woff");
    ///   font-weight: 400;
    ///   font-style: normal;
    ///   font-display: swap;
    ///   unicode-range: U+0020-007F;
    /// }
    /// ```
    fn parse_font_face_rule(&mut self) -> Option<types::FontFaceRule> {
        self.advance(); // consume @font-face
        self.skip_whitespace();
        if !self.expect(&CssToken::OpenBrace) {
            return None;
        }

        let mut family: Option<String> = None;
        let mut src: Vec<types::FontFaceSrc> = Vec::new();
        let mut weight_lo: u16 = 400;
        let mut weight_hi: u16 = 400;
        let mut style = types::FontFaceStyle::Normal;
        let mut display = types::FontDisplay::Auto;
        let mut unicode_range: Vec<types::UnicodeRange> = Vec::new();

        loop {
            self.skip_whitespace();
            if self.at_eof() || self.peek() == &CssToken::CloseBrace {
                break;
            }
            let key = match self.peek().clone() {
                CssToken::Ident(s) => {
                    self.advance();
                    s.to_ascii_lowercase()
                },
                _ => {
                    self.advance();
                    continue;
                },
            };
            self.skip_whitespace();
            if !self.expect(&CssToken::Colon) {
                self.skip_to_semicolon_or_close_brace();
                continue;
            }
            self.skip_whitespace();

            match key.as_str() {
                "font-family" => {
                    let raw = self.collect_descriptor_value();
                    family = Some(unquote(raw.trim()));
                },
                "src" => {
                    src = self.parse_font_face_src_list();
                },
                "font-weight" => {
                    let raw = self.collect_descriptor_value();
                    let (lo, hi) = parse_font_weight_descriptor(raw.trim());
                    weight_lo = lo;
                    weight_hi = hi;
                },
                "font-style" => {
                    let raw = self.collect_descriptor_value();
                    style = match raw.trim().to_ascii_lowercase().as_str() {
                        "italic" => types::FontFaceStyle::Italic,
                        "oblique" => types::FontFaceStyle::Oblique,
                        _ => types::FontFaceStyle::Normal,
                    };
                },
                "font-display" => {
                    let raw = self.collect_descriptor_value();
                    display = match raw.trim().to_ascii_lowercase().as_str() {
                        "block" => types::FontDisplay::Block,
                        "swap" => types::FontDisplay::Swap,
                        "fallback" => types::FontDisplay::Fallback,
                        "optional" => types::FontDisplay::Optional,
                        _ => types::FontDisplay::Auto,
                    };
                },
                "unicode-range" => {
                    // Unicode ranges like U+0020-007F use hex notation
                    // that the CSS tokenizer doesn't preserve. Collect
                    // the raw token text manually.
                    unicode_range = self.parse_unicode_range_descriptor();
                },
                _ => {
                    // Unrecognized descriptor — skip.
                    let _raw = self.collect_descriptor_value();
                },
            }
            if self.peek() == &CssToken::Semicolon {
                self.advance();
            }
        }
        self.expect(&CssToken::CloseBrace);

        // `font-family` and `src` are both required.
        let family = family?;
        if src.is_empty() {
            return None;
        }

        Some(types::FontFaceRule {
            family,
            src,
            weight: (weight_lo, weight_hi),
            style,
            display,
            unicode_range,
        })
    }

    /// Parse the value of a `src:` descriptor inside `@font-face`.
    ///
    /// The grammar is a comma-separated list of `url()` or `local()`
    /// entries, each optionally followed by `format()` / `tech()`.
    fn parse_font_face_src_list(&mut self) -> Vec<types::FontFaceSrc> {
        let mut sources = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                CssToken::Eof | CssToken::CloseBrace | CssToken::Semicolon => break,
                CssToken::Function(name) => {
                    let name_lc = name.to_ascii_lowercase();
                    match name_lc.as_str() {
                        "url" => {
                            self.advance(); // consume url(
                            self.skip_whitespace();
                            let url = match self.peek().clone() {
                                CssToken::String(s) => {
                                    self.advance();
                                    s
                                },
                                CssToken::Ident(s) => {
                                    self.advance();
                                    s
                                },
                                _ => {
                                    // Malformed url() — collect to closing paren.
                                    self.skip_to_close_paren();
                                    self.skip_past_comma_or_semi();
                                    continue;
                                },
                            };
                            self.skip_whitespace();
                            if self.peek() == &CssToken::CloseParen {
                                self.advance();
                            }
                            // Check for format() hint after the url().
                            self.skip_whitespace();
                            let format = self.try_parse_format_hint();
                            sources.push(types::FontFaceSrc::Url { url, format });
                        },
                        "local" => {
                            self.advance(); // consume local(
                            self.skip_whitespace();
                            let name = match self.peek().clone() {
                                CssToken::String(s) => {
                                    self.advance();
                                    s
                                },
                                CssToken::Ident(s) => {
                                    // Collect multi-word local names.
                                    self.advance();
                                    let mut full = s;
                                    while let CssToken::Ident(next) = self.peek() {
                                        full.push(' ');
                                        full.push_str(next);
                                        self.advance();
                                    }
                                    full
                                },
                                _ => {
                                    self.skip_to_close_paren();
                                    self.skip_past_comma_or_semi();
                                    continue;
                                },
                            };
                            self.skip_whitespace();
                            if self.peek() == &CssToken::CloseParen {
                                self.advance();
                            }
                            sources.push(types::FontFaceSrc::Local(name));
                        },
                        _ => {
                            // Unknown function — skip past it.
                            self.advance();
                            self.skip_to_close_paren();
                        },
                    }
                },
                CssToken::Comma => {
                    self.advance();
                },
                _ => {
                    // Recovery: skip unexpected token.
                    self.advance();
                },
            }
        }
        // Don't consume the semicolon/closebrace — the caller handles it.
        sources
    }

    /// Try to parse a `format("woff2", ...)` hint after a `url()` in
    /// `@font-face src:`. Returns the list of format strings (may be empty).
    fn try_parse_format_hint(&mut self) -> Vec<String> {
        let mut formats = Vec::new();
        if let CssToken::Function(f) = self.peek() {
            if f.eq_ignore_ascii_case("format") {
                self.advance(); // consume format(
                loop {
                    self.skip_whitespace();
                    match self.peek().clone() {
                        CssToken::String(s) => {
                            self.advance();
                            formats.push(s);
                        },
                        CssToken::Ident(s) => {
                            self.advance();
                            formats.push(s);
                        },
                        CssToken::Comma => {
                            self.advance();
                        },
                        CssToken::CloseParen => {
                            self.advance();
                            break;
                        },
                        _ => {
                            self.skip_to_close_paren();
                            break;
                        },
                    }
                }
            } else if f.eq_ignore_ascii_case("tech") {
                // Skip tech() hints — not actionable.
                self.advance();
                self.skip_to_close_paren();
            }
        }
        formats
    }

    /// Parse the `unicode-range` descriptor value by collecting raw
    /// token text. Unicode range notation (U+XXXX) uses hex which the
    /// CSS tokenizer emits as separate tokens (Ident "U", Plus, Number).
    /// `collect_descriptor_value` reassembles the text representation;
    /// `from_str_radix` handles any leading-zero differences.
    fn parse_unicode_range_descriptor(&mut self) -> Vec<types::UnicodeRange> {
        let raw = self.collect_descriptor_value();
        parse_unicode_range_list(raw.trim())
    }

    /// Skip tokens until (and including) the next `)`.
    fn skip_to_close_paren(&mut self) {
        let mut depth = 1;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::OpenParen => {
                    depth += 1;
                    self.advance();
                },
                CssToken::CloseParen => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        break;
                    }
                },
                _ => {
                    self.advance();
                },
            }
        }
    }

    /// Skip past the next comma or semicolon (for error recovery in
    /// `@font-face src:` lists).
    fn skip_past_comma_or_semi(&mut self) {
        loop {
            match self.peek() {
                CssToken::Comma => {
                    self.advance();
                    break;
                },
                CssToken::Semicolon | CssToken::CloseBrace | CssToken::Eof => break,
                _ => {
                    self.advance();
                },
            }
        }
    }

    /// Read tokens up to (but not consuming) the next semicolon /
    /// close-brace at brace-depth 0, reassembling them as raw text.
    /// Used by `@counter-style` / `@property` descriptor bodies.
    fn collect_descriptor_value(&mut self) -> String {
        let mut out = String::new();
        let mut paren_depth: i32 = 0;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::Semicolon if paren_depth == 0 => break,
                CssToken::CloseBrace if paren_depth == 0 => break,
                _ => {
                    let tok = self.peek().clone();
                    self.advance();
                    match tok {
                        CssToken::Ident(s) => out.push_str(&s),
                        CssToken::String(s) => {
                            out.push('"');
                            out.push_str(&s);
                            out.push('"');
                        },
                        CssToken::Hash(s) => {
                            out.push('#');
                            out.push_str(&s);
                        },
                        CssToken::Number(n) => out.push_str(&format!("{n}")),
                        CssToken::Percentage(n) => {
                            out.push_str(&format!("{n}%"));
                        },
                        CssToken::Dimension(n, u) => {
                            out.push_str(&format!("{n}{u}"));
                        },
                        CssToken::Whitespace => out.push(' '),
                        CssToken::OpenParen => {
                            paren_depth += 1;
                            out.push('(');
                        },
                        CssToken::CloseParen => {
                            // Clamp at 0 so a stray `)` in malformed
                            // input can't drive the depth negative —
                            // that would silently break the
                            // `paren_depth == 0` stop conditions for
                            // `;` / `}` and let the loop run away.
                            paren_depth = (paren_depth - 1).max(0);
                            out.push(')');
                        },
                        CssToken::Colon => out.push(':'),
                        CssToken::Comma => out.push(','),
                        CssToken::Plus => out.push('+'),
                        CssToken::Delim(c) => out.push(c),
                        _ => {},
                    }
                },
            }
        }
        out
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
                if lc == "scope" {
                    inner_rules.extend(self.parse_scope_rule());
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
            scope: None,
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
                        // The CSS tokenizer collapses `<ident>(` into a
                        // single Function token, which would otherwise
                        // be silently dropped from the condition string
                        // (breaking e.g. zero-space `@container (a)and(b)`
                        // where `and(` tokenizes as Function("and")).
                        CssToken::Function(ref s) => {
                            condition.push_str(s);
                            condition.push('(');
                        },
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
        // Consume tokens until we've skipped past one balanced `{...}`
        // block. If no opening brace is ever seen (caller advanced past
        // the body already, or stream is malformed), stop at the next
        // `}` at top level so we don't gobble the remainder of the
        // stylesheet. Previously this only broke on a `}` at depth 0,
        // which — starting at pre-`{` position — can only happen when
        // two `}` tokens appear back-to-back, so a single bad rule at
        // top level used to eat every subsequent rule.
        let mut depth = 0i32;
        let mut saw_open = false;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::OpenBrace => {
                    depth += 1;
                    saw_open = true;
                    self.advance();
                },
                CssToken::CloseBrace => {
                    self.advance();
                    if saw_open {
                        depth -= 1;
                        if depth <= 0 {
                            break;
                        }
                    } else {
                        // Unmatched closer — probably the outer block.
                        break;
                    }
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

//! CSS parser.
//!
//! Consumes the token stream produced by [`super::tokenizer::CssTokenizer`]
//! and builds a typed stylesheet AST with selectors, declarations, specificity,
//! shorthand expansion, and color parsing.

use super::tokenizer::{CssToken, CssTokenizer};

// -------------------------------------------------------------------
// Selector types
// -------------------------------------------------------------------

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
}

/// Combinator linking two compound selectors.
#[derive(Debug, Clone, PartialEq)]
pub enum Combinator {
    /// Descendant: `div p` (whitespace).
    Descendant,
    /// Child: `div > p`.
    Child,
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
                    SimpleSelector::Class(_) | SimpleSelector::PseudoClass(_) => {
                        classes = classes.saturating_add(1);
                    },
                    SimpleSelector::Type(_) => {
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
    const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
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
    pub fn parse(input: &str) -> Self {
        let tokens = CssTokenizer::new(input).tokenize();
        let mut parser = CssParser::new(tokens);
        parser.parse_stylesheet()
    }
}

/// Parse an inline `style="..."` attribute into declarations.
pub fn parse_inline_style(input: &str) -> Vec<Declaration> {
    let tokens = CssTokenizer::new(input).tokenize();
    let mut parser = CssParser::new(tokens);
    parser.parse_declaration_list()
}

// -------------------------------------------------------------------
// Internal parser
// -------------------------------------------------------------------

struct CssParser {
    tokens: Vec<CssToken>,
    pos: usize,
}

impl CssParser {
    fn new(tokens: Vec<CssToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    // -- helpers -----------------------------------------------------

    fn peek(&self) -> &CssToken {
        self.tokens.get(self.pos).unwrap_or(&CssToken::Eof)
    }

    fn advance(&mut self) -> CssToken {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(CssToken::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
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

    // -- stylesheet --------------------------------------------------

    fn parse_stylesheet(&mut self) -> Stylesheet {
        let mut rules = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_eof() {
                break;
            }
            // At-rules: consume and discard.
            if matches!(self.peek(), CssToken::AtKeyword(_)) {
                self.skip_at_rule();
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
                    // Consume but treat as descendant for our simple model.
                    self.advance();
                    self.skip_whitespace();
                    Some(Combinator::Descendant)
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
                    if let CssToken::Ident(name) = self.peek().clone() {
                        self.advance();
                        parts.push(SimpleSelector::PseudoClass(name));
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
        let value = self.parse_value(&property, &values);
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

fn parse_value_list(tokens: &[CssToken]) -> Vec<CssValue> {
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
                        CssToken::OpenParen => depth += 1,
                        CssToken::CloseParen => depth -= 1,
                        _ => {},
                    }
                    i += 1;
                }
                let inner = &tokens[start..i];
                if let Some(c) = try_parse_color(inner) {
                    out.push(CssValue::Color(c));
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

fn parse_font_weight(tokens: &[CssToken]) -> CssValue {
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

fn is_color_property(prop: &str) -> bool {
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

fn parse_unit(unit: &str) -> Option<LengthUnit> {
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

fn try_parse_color(tokens: &[CssToken]) -> Option<CssColor> {
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

fn parse_hex_color(hex: &str) -> Option<CssColor> {
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

fn hex_digit(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

fn hex_byte(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

fn parse_rgb_function(tokens: &[&CssToken]) -> Option<CssColor> {
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

fn named_color(name: &str) -> Option<CssColor> {
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
// Shorthand expansion
// -------------------------------------------------------------------

fn expand_shorthands(decls: Vec<Declaration>) -> Vec<Declaration> {
    let mut out = Vec::new();
    for decl in decls {
        match decl.property.as_str() {
            "margin" => {
                out.extend(expand_box_shorthand("margin", &decl.value, decl.important));
            },
            "padding" => {
                out.extend(expand_box_shorthand("padding", &decl.value, decl.important));
            },
            "border" => {
                out.extend(expand_border(&decl.value, decl.important));
            },
            "background" => {
                out.extend(expand_background(&decl.value, decl.important));
            },
            _ => out.push(decl),
        }
    }
    out
}

fn expand_box_shorthand(prefix: &str, value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let (top, right, bottom, left) = match values.len() {
        1 => {
            let v = &values[0];
            (v.clone(), v.clone(), v.clone(), v.clone())
        },
        2 => {
            let tb = &values[0];
            let lr = &values[1];
            (tb.clone(), lr.clone(), tb.clone(), lr.clone())
        },
        3 => (
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[1].clone(),
        ),
        _ => (
            values[0].clone(),
            values.get(1).cloned().unwrap_or_else(|| values[0].clone()),
            values.get(2).cloned().unwrap_or_else(|| values[0].clone()),
            values.get(3).cloned().unwrap_or_else(|| values[0].clone()),
        ),
    };

    vec![
        Declaration {
            property: format!("{}-top", prefix),
            value: top,
            important,
        },
        Declaration {
            property: format!("{}-right", prefix),
            value: right,
            important,
        },
        Declaration {
            property: format!("{}-bottom", prefix),
            value: bottom,
            important,
        },
        Declaration {
            property: format!("{}-left", prefix),
            value: left,
            important,
        },
    ]
}

fn expand_border(value: &CssValue, important: bool) -> Vec<Declaration> {
    let values = match value {
        CssValue::Multiple(vs) => vs.clone(),
        other => vec![other.clone()],
    };

    let mut width = CssValue::Keyword("medium".into());
    let mut style = CssValue::Keyword("none".into());
    let mut color = CssValue::Keyword("currentcolor".into());

    for v in &values {
        match v {
            CssValue::Length(..) | CssValue::Number(_) => {
                width = v.clone();
            },
            CssValue::Color(_) => {
                color = v.clone();
            },
            CssValue::Keyword(kw) => {
                let lower = kw.to_ascii_lowercase();
                if is_border_style(&lower) {
                    style = v.clone();
                } else if let Some(c) = named_color(&lower) {
                    color = CssValue::Color(c);
                } else {
                    // Fallback: treat as style.
                    style = v.clone();
                }
            },
            _ => {},
        }
    }

    vec![
        Declaration {
            property: "border-width".into(),
            value: width,
            important,
        },
        Declaration {
            property: "border-style".into(),
            value: style,
            important,
        },
        Declaration {
            property: "border-color".into(),
            value: color,
            important,
        },
    ]
}

fn is_border_style(s: &str) -> bool {
    matches!(
        s,
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

fn expand_background(value: &CssValue, important: bool) -> Vec<Declaration> {
    // Simple heuristic: if the value is a color, set background-color.
    match value {
        CssValue::Color(_) => {
            vec![Declaration {
                property: "background-color".into(),
                value: value.clone(),
                important,
            }]
        },
        CssValue::Multiple(vs) => {
            // If any value is a colour, use it.
            for v in vs {
                if matches!(v, CssValue::Color(_)) {
                    return vec![Declaration {
                        property: "background-color".into(),
                        value: v.clone(),
                        important,
                    }];
                }
            }
            vec![Declaration {
                property: "background".into(),
                value: value.clone(),
                important,
            }]
        },
        CssValue::Keyword(name) => {
            if let Some(c) = named_color(name) {
                vec![Declaration {
                    property: "background-color".into(),
                    value: CssValue::Color(c),
                    important,
                }]
            } else {
                vec![Declaration {
                    property: "background".into(),
                    value: value.clone(),
                    important,
                }]
            }
        },
        _ => {
            vec![Declaration {
                property: "background".into(),
                value: value.clone(),
                important,
            }]
        },
    }
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
    fn at_media_skipped() {
        let sheet = parse(
            "@media screen { body { color: red; } } \
             p { color: blue; }",
        );
        assert_eq!(sheet.rules.len(), 1);
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
        // We don't support @media but it should not crash.
        let _ = sheet;
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
        let sheet = parse(
            "p { color: #fff; background: #aabbcc; border-color: #12345678; }",
        );
        assert_eq!(sheet.rules[0].declarations.len(), 3);
    }

    #[test]
    fn trailing_garbage_after_rules() {
        let sheet = parse("p { color: red; } garbage here");
        // The first rule should still parse.
        assert!(!sheet.rules.is_empty());
    }
}

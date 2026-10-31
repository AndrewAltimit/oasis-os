//! Selector parsing for the CSS parser.
//!
//! Implements selector list, compound selector, combinator, attribute
//! selector, and pseudo-class/element parsing on [`super::CssParser`].

use super::super::tokenizer::CssToken;
use super::CssParser;
use super::types::{AttrOp, Combinator, CompoundSelector, Selector, SelectorList, SimpleSelector};

impl CssParser {
    // -- selectors ---------------------------------------------------

    pub(super) fn parse_selector_list(&mut self) -> Option<SelectorList> {
        let mut selectors = Vec::new();
        selectors.push(self.parse_selector()?);
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
                | CssToken::OpenBracket
                | CssToken::Delim('&')
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
                CssToken::Delim('&') => {
                    self.advance();
                    parts.push(SimpleSelector::Nest);
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
                            let nests = matches!(lc.as_str(), "not" | "is" | "where" | "has");
                            if nests && self.nesting >= super::MAX_CSS_NESTING {
                                // Too deep: drop the argument and match
                                // nothing (`:is()` with an empty list).
                                self.skip_to_matching_close_paren();
                                parts.push(SimpleSelector::Is(Vec::new()));
                                continue;
                            }
                            if nests {
                                self.nesting += 1;
                            }
                            self.parse_selector_function(&lc, &mut parts);
                            if nests {
                                self.nesting -= 1;
                            }
                        },
                        _ => {},
                    }
                },
                CssToken::OpenBracket => {
                    self.advance();
                    // A malformed attribute selector matches nothing
                    // (dropping it would widen the selector).
                    let attr_sel = self.parse_attribute_selector();
                    parts.push(attr_sel.unwrap_or(SimpleSelector::Is(Vec::new())));
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

    /// Skip past the `)` closing the function whose name token was just
    /// consumed, stepping over nested parentheses / functions.
    fn skip_to_matching_close_paren(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::OpenParen | CssToken::Function(_) => depth += 1,
                CssToken::CloseParen => {
                    if depth == 0 {
                        self.advance();
                        break;
                    }
                    depth -= 1;
                },
                _ => {},
            }
            self.advance();
        }
    }

    /// Parse the argument of a functional pseudo-class (after its name
    /// token) and push the resulting simple selector.
    fn parse_selector_function(&mut self, lc: &str, parts: &mut Vec<SimpleSelector>) {
        if lc == "not" || lc == "is" || lc == "where" {
            // Selectors Level 4: a comma-separated list of complex
            // selectors. `:is()` / `:where()` take a forgiving list
            // (invalid entries are dropped); an invalid entry makes
            // `:not()` match nothing rather than widen the selector.
            let (inner_list, any_invalid) = self.parse_complex_selector_list();
            if self.peek() == &CssToken::CloseParen {
                self.advance();
            }
            parts.push(match lc {
                "not" if any_invalid => SimpleSelector::Is(Vec::new()),
                "not" => SimpleSelector::Not(inner_list),
                "is" => SimpleSelector::Is(inner_list),
                _ => SimpleSelector::Where(inner_list),
            });
        } else if lc == "has" {
            // Parse :has(relative-selector-list).
            let inner_list = self.parse_relative_selector_list();
            self.skip_whitespace();
            if self.peek() == &CssToken::CloseParen {
                self.advance();
            }
            parts.push(SimpleSelector::Has(inner_list));
        } else {
            // Functional pseudo-class like :nth-child(2n+1)
            let arg = self.consume_until_close_paren();
            parts.push(SimpleSelector::PseudoClassFn(lc.to_string(), arg));
        }
    }

    /// Parse a comma-separated list of relative selectors inside `:has()`.
    ///
    /// Each relative selector may begin with an explicit combinator
    /// (`>`, `+`, `~`); otherwise the descendant combinator is implied.
    /// Stops at `)` (does not consume it).
    fn parse_relative_selector_list(&mut self) -> Vec<(Combinator, Selector)> {
        let mut list = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == &CssToken::CloseParen || self.peek() == &CssToken::Eof {
                break;
            }
            let leading = match self.peek() {
                CssToken::Greater => {
                    self.advance();
                    self.skip_whitespace();
                    Combinator::Child
                },
                CssToken::Plus => {
                    self.advance();
                    self.skip_whitespace();
                    Combinator::AdjacentSibling
                },
                CssToken::Delim('~') => {
                    self.advance();
                    self.skip_whitespace();
                    Combinator::GeneralSibling
                },
                _ => Combinator::Descendant,
            };
            if let Some(sel) = self.parse_selector() {
                list.push((leading, sel));
            } else {
                break;
            }
            self.skip_whitespace();
            if self.peek() == &CssToken::Comma {
                self.advance();
            } else {
                break;
            }
        }
        list
    }

    /// Parse the comma-separated complex selector list inside `:is()` /
    /// `:where()` / `:not()`, and whether any entry was invalid (and
    /// skipped). Stops at the closing `)` (does not consume it).
    fn parse_complex_selector_list(&mut self) -> (Vec<Selector>, bool) {
        let mut list = Vec::new();
        let mut any_invalid = false;
        loop {
            self.skip_whitespace();
            if matches!(self.peek(), CssToken::CloseParen | CssToken::Eof) {
                break;
            }
            let sel = self.parse_selector();
            self.skip_whitespace();
            match (sel, self.peek()) {
                (Some(sel), CssToken::Comma | CssToken::CloseParen | CssToken::Eof) => {
                    list.push(sel);
                },
                _ => {
                    any_invalid = true;
                    self.skip_to_list_separator();
                },
            }
            if self.peek() == &CssToken::Comma {
                self.advance();
            } else {
                break;
            }
        }
        (list, any_invalid)
    }

    /// Skip to the next `,` or `)` at this nesting level (not consumed).
    fn skip_to_list_separator(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                CssToken::Eof => break,
                CssToken::Comma | CssToken::CloseParen if depth == 0 => break,
                CssToken::OpenParen | CssToken::Function(_) => depth += 1,
                CssToken::CloseParen => depth -= 1,
                _ => {},
            }
            self.advance();
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
                        // `2n`, `-3n`, `2n-1`: an `an+b` with a step is a
                        // dimension token (unit `n` / `n-1`).
                        CssToken::Dimension(n, unit) => arg.push_str(&format!("{n}{unit}")),
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

    /// Skip to (and consume) the `]` closing an attribute selector.
    fn skip_past_close_bracket(&mut self) {
        while self.peek() != &CssToken::CloseBracket && self.peek() != &CssToken::Eof {
            self.advance();
        }
        if self.peek() == &CssToken::CloseBracket {
            self.advance();
        }
    }

    /// Parse an attribute selector after `[` has been consumed:
    /// `[name]`, `[name op value]` or `[name op value i|s]`.
    ///
    /// A malformed selector yields `None`; the caller turns it into a
    /// never-matching part rather than silently widening the selector.
    fn parse_attribute_selector(&mut self) -> Option<SimpleSelector> {
        self.skip_whitespace();
        let CssToken::Ident(name) = self.peek().clone() else {
            self.skip_past_close_bracket();
            return None;
        };
        self.advance();
        self.skip_whitespace();

        if self.peek() == &CssToken::CloseBracket {
            self.advance();
            return Some(SimpleSelector::Attribute {
                name,
                op: AttrOp::Exists,
                value: None,
                case_insensitive: false,
            });
        }

        // Operator: =, ~=, |=, ^=, $=, *=
        let op = match self.peek() {
            CssToken::Delim('=') => AttrOp::Equals,
            CssToken::Delim('~') => AttrOp::Includes,
            CssToken::Delim('|') => AttrOp::DashMatch,
            CssToken::Delim('^') => AttrOp::Prefix,
            CssToken::Delim('$') => AttrOp::Suffix,
            CssToken::Star => AttrOp::Substring,
            _ => {
                self.skip_past_close_bracket();
                return None;
            },
        };
        self.advance();
        if op != AttrOp::Equals {
            if self.peek() != &CssToken::Delim('=') {
                self.skip_past_close_bracket();
                return None;
            }
            self.advance();
        }
        self.skip_whitespace();

        // Value: ident or string.
        let value = match self.peek().clone() {
            CssToken::Ident(v) | CssToken::String(v) => {
                self.advance();
                v
            },
            _ => {
                self.skip_past_close_bracket();
                return None;
            },
        };
        self.skip_whitespace();

        // Optional case-sensitivity flag (Selectors 4 §6.3).
        let flag = match self.peek().clone() {
            CssToken::Ident(f) => {
                self.advance();
                self.skip_whitespace();
                match f.to_ascii_lowercase().as_str() {
                    "i" => Some(true),
                    "s" => Some(false),
                    _ => {
                        self.skip_past_close_bracket();
                        return None;
                    },
                }
            },
            _ => None,
        };
        if self.peek() != &CssToken::CloseBracket {
            self.skip_past_close_bracket();
            return None;
        }
        self.advance();

        let case_insensitive =
            flag.unwrap_or_else(|| html_case_insensitive_attribute(&name.to_ascii_lowercase()));
        let value = if case_insensitive {
            value.to_ascii_lowercase()
        } else {
            value
        };
        Some(SimpleSelector::Attribute {
            name,
            op,
            value: Some(value),
            case_insensitive,
        })
    }
}

/// HTML attributes whose values attribute selectors compare
/// ASCII-case-insensitively unless the selector carries an `s` flag
/// (HTML §4.16.2 "case-sensitivity of selectors").
fn html_case_insensitive_attribute(name: &str) -> bool {
    matches!(
        name,
        "accept"
            | "accept-charset"
            | "align"
            | "alink"
            | "axis"
            | "bgcolor"
            | "charset"
            | "checked"
            | "clear"
            | "codetype"
            | "color"
            | "compact"
            | "declare"
            | "defer"
            | "dir"
            | "direction"
            | "disabled"
            | "enctype"
            | "face"
            | "frame"
            | "hreflang"
            | "http-equiv"
            | "lang"
            | "language"
            | "link"
            | "media"
            | "method"
            | "multiple"
            | "nohref"
            | "noresize"
            | "noshade"
            | "nowrap"
            | "readonly"
            | "rel"
            | "rev"
            | "rules"
            | "scope"
            | "scrolling"
            | "selected"
            | "shape"
            | "target"
            | "text"
            | "type"
            | "valign"
            | "valuetype"
            | "vlink"
    )
}

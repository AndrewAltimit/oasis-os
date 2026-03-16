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
                            } else if lc == "is" || lc == "where" {
                                // Parse :is(selector-list) / :where(selector-list)
                                let inner_list = self.parse_compound_selector_list();
                                self.skip_whitespace();
                                if self.peek() == &CssToken::CloseParen {
                                    self.advance();
                                }
                                if lc == "is" {
                                    parts.push(SimpleSelector::Is(inner_list));
                                } else {
                                    parts.push(SimpleSelector::Where(inner_list));
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

    /// Parse a comma-separated list of compound selectors inside `:is()` / `:where()`.
    ///
    /// Stops at `)` (does not consume it).
    fn parse_compound_selector_list(&mut self) -> Vec<CompoundSelector> {
        let mut list = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == &CssToken::CloseParen || self.peek() == &CssToken::Eof {
                break;
            }
            if let Some(compound) = self.parse_compound_selector() {
                list.push(compound);
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
}

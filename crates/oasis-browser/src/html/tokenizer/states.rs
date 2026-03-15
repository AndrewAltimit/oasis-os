//! State machine implementations for the HTML tokenizer.
//!
//! Each method corresponds to one state in the WHATWG tokenization
//! specification and is called from the main dispatch loop in `mod.rs`.

use super::Tokenizer;
use super::char_ref::resolve_named_ref;
use super::types::{State, TagBuilder, Token};

// -----------------------------------------------------------------------
// Tag and data state implementations
// -----------------------------------------------------------------------

impl Tokenizer {
    /// **Data** -- default entry point.
    pub(crate) fn state_data(&mut self) -> Option<Token> {
        match self.consume() {
            Some('<') => {
                self.state = State::TagOpen;
                None
            },
            Some('&') => {
                self.return_state = State::Data;
                self.state = State::CharacterReference;
                None
            },
            Some(ch) => Some(Token::Character(ch.to_string())),
            None => Some(Token::Eof),
        }
    }

    /// **TagOpen** -- after `<`.
    pub(crate) fn state_tag_open(&mut self) -> Option<Token> {
        match self.peek() {
            Some('!') => {
                self.consume();
                self.state = State::MarkupDeclarationOpen;
                None
            },
            Some('/') => {
                self.consume();
                self.state = State::EndTagOpen;
                None
            },
            Some(ch) if ch.is_ascii_alphabetic() => {
                self.current_tag = Some(TagBuilder::new(false));
                self.state = State::TagName;
                None
            },
            Some('?') => {
                self.current_comment.clear();
                self.state = State::BogusComment;
                None
            },
            _ => {
                self.state = State::Data;
                Some(Token::Character("<".into()))
            },
        }
    }

    /// **EndTagOpen** -- after `</`.
    pub(crate) fn state_end_tag_open(&mut self) -> Option<Token> {
        match self.peek() {
            Some(ch) if ch.is_ascii_alphabetic() => {
                self.current_tag = Some(TagBuilder::new(true));
                self.state = State::TagName;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Character("</".into()))
            },
            _ => {
                self.current_comment.clear();
                self.state = State::BogusComment;
                None
            },
        }
    }

    /// **TagName** -- accumulate tag name characters.
    pub(crate) fn state_tag_name(&mut self) -> Option<Token> {
        match self.consume() {
            Some(ch) if ch.is_ascii_whitespace() => {
                self.state = State::BeforeAttributeName;
                None
            },
            Some('/') => {
                self.state = State::SelfClosingStartTag;
                None
            },
            Some('>') => {
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            Some(ch) => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.name.push(ch.to_ascii_lowercase());
                }
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **SelfClosingStartTag** -- after `/` inside a tag.
    pub(crate) fn state_self_closing(&mut self) -> Option<Token> {
        match self.consume() {
            Some('>') => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.self_closing = true;
                }
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
            _ => {
                self.reconsume();
                self.state = State::BeforeAttributeName;
                None
            },
        }
    }

    // -- attribute states ---------------------------------------------------

    /// **BeforeAttributeName** -- after tag name, before attribute.
    pub(crate) fn state_before_attr_name(&mut self) -> Option<Token> {
        // Skip whitespace.
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.consume();
        }
        match self.peek() {
            Some('/') | Some('>') | None => {
                self.state = State::AfterAttributeName;
                None
            },
            Some('=') => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.finish_attribute();
                    tag.current_attr_name.push('=');
                }
                self.consume();
                self.state = State::AttributeName;
                None
            },
            _ => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.finish_attribute();
                }
                self.state = State::AttributeName;
                None
            },
        }
    }

    /// **AttributeName** -- accumulate attribute name.
    pub(crate) fn state_attr_name(&mut self) -> Option<Token> {
        match self.consume() {
            Some(ch) if ch.is_ascii_whitespace() || ch == '/' || ch == '>' => {
                self.reconsume();
                self.state = State::AfterAttributeName;
                None
            },
            Some('=') => {
                self.state = State::BeforeAttributeValue;
                None
            },
            Some(ch) => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.current_attr_name.push(ch.to_ascii_lowercase());
                }
                None
            },
            None => {
                self.state = State::AfterAttributeName;
                None
            },
        }
    }

    /// **AfterAttributeName** -- after attribute name.
    pub(crate) fn state_after_attr_name(&mut self) -> Option<Token> {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.consume();
        }
        match self.peek() {
            Some('/') => {
                self.consume();
                self.state = State::SelfClosingStartTag;
                None
            },
            Some('=') => {
                self.consume();
                self.state = State::BeforeAttributeValue;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
            _ => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.finish_attribute();
                }
                self.state = State::AttributeName;
                None
            },
        }
    }

    /// **BeforeAttributeValue** -- before `=` value.
    pub(crate) fn state_before_attr_value(&mut self) -> Option<Token> {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.consume();
        }
        match self.peek() {
            Some('"') => {
                self.consume();
                self.state = State::AttributeValueDoubleQuoted;
                None
            },
            Some('\'') => {
                self.consume();
                self.state = State::AttributeValueSingleQuoted;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            _ => {
                self.state = State::AttributeValueUnquoted;
                None
            },
        }
    }

    /// **AttributeValueDoubleQuoted** -- inside `"..."`.
    pub(crate) fn state_attr_val_dq(&mut self) -> Option<Token> {
        match self.consume() {
            Some('"') => {
                self.state = State::AfterAttributeValueQuoted;
                None
            },
            Some('&') => {
                self.return_state = State::AttributeValueDoubleQuoted;
                self.state = State::CharacterReference;
                None
            },
            Some(ch) => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.current_attr_value.push(ch);
                }
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AttributeValueSingleQuoted** -- inside `'...'`.
    pub(crate) fn state_attr_val_sq(&mut self) -> Option<Token> {
        match self.consume() {
            Some('\'') => {
                self.state = State::AfterAttributeValueQuoted;
                None
            },
            Some('&') => {
                self.return_state = State::AttributeValueSingleQuoted;
                self.state = State::CharacterReference;
                None
            },
            Some(ch) => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.current_attr_value.push(ch);
                }
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AttributeValueUnquoted** -- bare attribute value.
    pub(crate) fn state_attr_val_unquoted(&mut self) -> Option<Token> {
        match self.consume() {
            Some(ch) if ch.is_ascii_whitespace() => {
                self.state = State::BeforeAttributeName;
                None
            },
            Some('&') => {
                self.return_state = State::AttributeValueUnquoted;
                self.state = State::CharacterReference;
                None
            },
            Some('>') => {
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            Some(ch) => {
                if let Some(tag) = self.current_tag.as_mut() {
                    tag.current_attr_value.push(ch);
                }
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AfterAttributeValueQuoted** -- after closing quote.
    pub(crate) fn state_after_attr_val_q(&mut self) -> Option<Token> {
        match self.peek() {
            Some(ch) if ch.is_ascii_whitespace() => {
                self.consume();
                self.state = State::BeforeAttributeName;
                None
            },
            Some('/') => {
                self.consume();
                self.state = State::SelfClosingStartTag;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(self.emit_current_tag())
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
            _ => {
                self.state = State::BeforeAttributeName;
                None
            },
        }
    }

    // -- markup declaration / comment / doctype -----------------------------

    /// **MarkupDeclarationOpen** -- after `<!`.
    pub(crate) fn state_markup_decl_open(&mut self) -> Option<Token> {
        if self.starts_with_ci("--") {
            self.consume();
            self.consume();
            self.current_comment.clear();
            self.state = State::CommentStart;
            None
        } else if self.starts_with_ci("DOCTYPE") {
            for _ in 0..7 {
                self.consume();
            }
            self.state = State::Doctype;
            None
        } else {
            self.current_comment.clear();
            self.state = State::BogusComment;
            None
        }
    }

    /// **CommentStart**.
    pub(crate) fn state_comment_start(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::CommentStartDash;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            _ => {
                self.state = State::Comment;
                None
            },
        }
    }

    /// **CommentStartDash**.
    pub(crate) fn state_comment_start_dash(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::CommentEnd;
                None
            },
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            None => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            _ => {
                self.current_comment.push('-');
                self.state = State::Comment;
                None
            },
        }
    }

    /// **Comment** -- inside comment body.
    pub(crate) fn state_comment(&mut self) -> Option<Token> {
        match self.consume() {
            Some('-') => {
                self.state = State::CommentEndDash;
                None
            },
            Some(ch) => {
                self.current_comment.push(ch);
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
        }
    }

    /// **CommentEndDash** -- saw one `-` inside comment.
    pub(crate) fn state_comment_end_dash(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::CommentEnd;
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            _ => {
                self.current_comment.push('-');
                self.state = State::Comment;
                None
            },
        }
    }

    /// **CommentEnd** -- saw `--` inside comment.
    pub(crate) fn state_comment_end(&mut self) -> Option<Token> {
        match self.peek() {
            Some('>') => {
                self.consume();
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            Some('-') => {
                self.consume();
                self.current_comment.push('-');
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            _ => {
                self.current_comment.push_str("--");
                self.state = State::Comment;
                None
            },
        }
    }

    /// **Doctype** -- after `DOCTYPE` keyword.
    pub(crate) fn state_doctype(&mut self) -> Option<Token> {
        match self.peek() {
            Some(ch) if ch.is_ascii_whitespace() => {
                self.consume();
                self.state = State::BeforeDoctypeName;
                None
            },
            Some('>') => {
                self.state = State::BeforeDoctypeName;
                None
            },
            None => {
                self.current_doctype.force_quirks = true;
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            _ => {
                self.state = State::BeforeDoctypeName;
                None
            },
        }
    }

    /// **BeforeDoctypeName**.
    pub(crate) fn state_before_doctype_name(&mut self) -> Option<Token> {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.consume();
        }
        match self.peek() {
            Some('>') => {
                self.consume();
                self.current_doctype.force_quirks = true;
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            None => {
                self.current_doctype.force_quirks = true;
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            Some(ch) => {
                self.consume();
                self.current_doctype.name = Some(ch.to_ascii_lowercase().to_string());
                self.state = State::DoctypeName;
                None
            },
        }
    }

    /// **DoctypeName**.
    pub(crate) fn state_doctype_name(&mut self) -> Option<Token> {
        match self.consume() {
            Some(ch) if ch.is_ascii_whitespace() => {
                self.state = State::AfterDoctypeName;
                None
            },
            Some('>') => {
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            Some(ch) => {
                if let Some(ref mut name) = self.current_doctype.name {
                    name.push(ch.to_ascii_lowercase());
                }
                None
            },
            None => {
                self.current_doctype.force_quirks = true;
                self.state = State::Data;
                Some(self.emit_doctype())
            },
        }
    }

    /// **AfterDoctypeName** -- skip remaining tokens until `>`.
    pub(crate) fn state_after_doctype_name(&mut self) -> Option<Token> {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.consume();
        }
        match self.consume() {
            Some('>') => {
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            None => {
                self.current_doctype.force_quirks = true;
                self.state = State::Data;
                Some(self.emit_doctype())
            },
            Some(_) => None, // skip SYSTEM/PUBLIC etc.
        }
    }

    /// Helper: take the current doctype builder and emit a token.
    pub(crate) fn emit_doctype(&mut self) -> Token {
        std::mem::replace(
            &mut self.current_doctype,
            super::types::DoctypeBuilder::new(),
        )
        .into_token()
    }

    /// **BogusComment**.
    pub(crate) fn state_bogus_comment(&mut self) -> Option<Token> {
        match self.consume() {
            Some('>') => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
            Some(ch) => {
                self.current_comment.push(ch);
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.current_comment)))
            },
        }
    }

    // -- RAWTEXT / RCDATA ---------------------------------------------------

    /// Match end tag at current position (case-insensitive).
    fn check_end_tag_at_pos(&self, end_tag: &str) -> bool {
        let chars: Vec<char> = end_tag.chars().collect();
        if self.pos + chars.len() > self.input.len() {
            return false;
        }
        chars
            .iter()
            .enumerate()
            .all(|(i, &expected)| self.input[self.pos + i].eq_ignore_ascii_case(&expected))
    }

    /// Returns `true` when the input at the current position looks like
    /// a valid end tag for the current RAWTEXT/RCDATA element.
    fn at_content_end_tag(&self, end_tag: &str) -> bool {
        if !self.check_end_tag_at_pos(end_tag) {
            return false;
        }
        let after = self.pos + end_tag.len();
        matches!(
            self.input.get(after).copied(),
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('/') | None
        )
    }

    /// Consume the end tag for a RAWTEXT/RCDATA element and return it
    /// as a token.
    fn consume_content_end_tag(&mut self, end_tag: &str) -> Token {
        self.pos += end_tag.len();
        let tag_name = self.last_start_tag.clone().unwrap_or_default();
        let mut builder = TagBuilder::new(true);
        builder.name = tag_name;
        self.current_tag = Some(builder);
        // Skip to `>`.
        loop {
            match self.consume() {
                Some('>') | None => break,
                _ => {},
            }
        }
        match self.current_tag.take() {
            Some(tag) => tag.into_token(),
            None => Token::Eof,
        }
    }

    /// **RawText** -- for `<script>`, `<style>`, etc.
    pub(crate) fn state_rawtext(&mut self) -> Option<Token> {
        let end_tag = match self.last_start_tag {
            Some(ref s) => format!("</{s}"),
            None => {
                self.state = State::Data;
                return None;
            },
        };

        let mut text = String::new();
        loop {
            if self.pos >= self.input.len() {
                self.state = State::Data;
                return if text.is_empty() {
                    Some(Token::Eof)
                } else {
                    Some(Token::Character(text))
                };
            }

            if self.at_content_end_tag(&end_tag) {
                self.state = State::Data;
                if !text.is_empty() {
                    return Some(Token::Character(text));
                }
                return Some(self.consume_content_end_tag(&end_tag));
            }

            text.push(self.input[self.pos]);
            self.pos += 1;
        }
    }

    /// **RcData** -- for `<title>`, `<textarea>`.
    pub(crate) fn state_rcdata(&mut self) -> Option<Token> {
        let end_tag = match self.last_start_tag {
            Some(ref s) => format!("</{s}"),
            None => {
                self.state = State::Data;
                return None;
            },
        };

        let mut text = String::new();
        loop {
            if self.pos >= self.input.len() {
                self.state = State::Data;
                return if text.is_empty() {
                    Some(Token::Eof)
                } else {
                    Some(Token::Character(text))
                };
            }

            if self.at_content_end_tag(&end_tag) {
                self.state = State::Data;
                if !text.is_empty() {
                    return Some(Token::Character(text));
                }
                return Some(self.consume_content_end_tag(&end_tag));
            }

            let ch = self.input[self.pos];
            if ch == '&' {
                self.pos += 1;
                text.push_str(&self.resolve_inline_char_ref());
            } else {
                self.pos += 1;
                text.push(ch);
            }
        }
    }

    /// Resolve a character reference inline (used by RCDATA).
    /// Assumes the `&` has already been consumed.
    fn resolve_inline_char_ref(&mut self) -> String {
        match self.peek() {
            Some('#') => {
                self.consume();
                self.resolve_inline_numeric_ref()
            },
            Some(ch) if ch.is_ascii_alphanumeric() => {
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() {
                        self.consume();
                        name.push(c);
                    } else if c == ';' {
                        self.consume();
                        name.push(';');
                        break;
                    } else {
                        break;
                    }
                }
                resolve_named_ref(&name)
                    .map(String::from)
                    .unwrap_or_else(|| format!("&{name}"))
            },
            _ => "&".into(),
        }
    }

    /// Resolve `&#...;` or `&#x...;` inline.
    fn resolve_inline_numeric_ref(&mut self) -> String {
        let is_hex = matches!(self.peek(), Some('x' | 'X'));
        if is_hex {
            self.consume();
        }
        let mut code: u32 = 0;
        let mut any_digit = false;
        loop {
            match self.peek() {
                Some(ch) if is_hex && ch.is_ascii_hexdigit() => {
                    self.consume();
                    any_digit = true;
                    code = code
                        .saturating_mul(16)
                        .saturating_add(ch.to_digit(16).unwrap_or(0));
                },
                Some(ch) if !is_hex && ch.is_ascii_digit() => {
                    self.consume();
                    any_digit = true;
                    code = code
                        .saturating_mul(10)
                        .saturating_add(ch.to_digit(10).unwrap_or(0));
                },
                Some(';') => {
                    self.consume();
                    break;
                },
                _ => break,
            }
        }
        if !any_digit {
            return if is_hex { "&#x" } else { "&#" }.into();
        }
        let ch = match code {
            0 | 0xD800..=0xDFFF => '\u{FFFD}',
            c if c > 0x10FFFF => '\u{FFFD}',
            _ => char::from_u32(code).unwrap_or('\u{FFFD}'),
        };
        ch.to_string()
    }
}

//! WHATWG HTML tokenizer.
//!
//! Implements the tokenization state machine from the
//! [WHATWG HTML parsing specification][spec]. Consumes a UTF-8 `&str`
//! and emits a flat `Vec<Token>`.
//!
//! This is a practical subset covering the states needed for real-world
//! HTML: tags, attributes, comments, DOCTYPE, character references,
//! RAWTEXT (`<script>`, `<style>`), and RCDATA (`<title>`, `<textarea>`).
//! Malformed input is always handled gracefully -- the tokenizer never
//! panics.
//!
//! ## State machine overview
//!
//! ```text
//!   Data ──'<'──> TagOpen ──'/'──> EndTagOpen ──> TagName
//!     │              │                               │
//!     │              ├──'!'──> MarkupDeclarationOpen  ├──ws──> BeforeAttributeName
//!     │              │            ├──"--"──> Comment  │            │
//!     │              │            └──"DOCTYPE"──>...  │       AttributeName
//!     │              └──alpha──> TagName              │            │
//!     │                                              '>'     BeforeAttributeValue
//!     │                                            (emit)     │     │     │
//!     ├── CharacterReference (&#...; / &name;)               "     '   unquoted
//!     ├── RawText  (<script>, <style> -- no tag parsing)
//!     └── RcData   (<title>, <textarea> -- refs only)
//! ```
//!
//! Each character is consumed once, advancing through states until a
//! token is emitted. The `RawText` and `RcData` states suppress normal
//! tag recognition inside their respective elements.
//!
//! [spec]: https://html.spec.whatwg.org/multipage/parsing.html#tokenization

mod char_ref;
mod states;
mod types;

#[cfg(test)]
mod tests;

// Re-export public types so the module API is unchanged.
// These types are part of Token's public API (e.g. Token::Doctype(DoctypeToken),
// StartTagToken { attributes: Vec<Attribute> }) and are used by tree_builder tests.
#[allow(unused_imports)]
pub use types::{Attribute, DoctypeToken, EndTagToken, StartTagToken, Token};

use types::{DoctypeBuilder, State, TagBuilder};

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// WHATWG HTML tokenizer.
///
/// Construct with [`Tokenizer::new`], then call [`Tokenizer::tokenize`] to
/// consume the input and produce a `Vec<Token>`.
pub struct Tokenizer {
    input: Vec<char>,
    pub(crate) pos: usize,
    pub(crate) state: State,
    pub(crate) return_state: State,
    pub(crate) current_tag: Option<TagBuilder>,
    pub(crate) current_comment: String,
    pub(crate) current_doctype: DoctypeBuilder,
    pub(crate) temp_buffer: String,
    pub(crate) last_start_tag: Option<String>,
    pub(crate) char_ref_code: u32,
}

impl Tokenizer {
    /// Create a new tokenizer over the given UTF-8 input.
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            state: State::Data,
            return_state: State::Data,
            current_tag: None,
            current_comment: String::new(),
            current_doctype: DoctypeBuilder::new(),
            temp_buffer: String::new(),
            last_start_tag: None,
            char_ref_code: 0,
        }
    }

    /// Consume the input and return the token stream.
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();
        loop {
            let token = self.next_token();
            let is_eof = token == Token::Eof;
            Self::push_coalesced(&mut tokens, token);
            if is_eof {
                break;
            }
        }
        tokens
    }

    /// Coalesce consecutive `Character` tokens.
    fn push_coalesced(tokens: &mut Vec<Token>, token: Token) {
        if let Token::Character(ref new_text) = token
            && let Some(Token::Character(prev)) = tokens.last_mut()
        {
            prev.push_str(new_text);
            return;
        }
        tokens.push(token);
    }

    // -- helpers ------------------------------------------------------------

    pub(crate) fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    pub(crate) fn consume(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    pub(crate) fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// Case-insensitive look-ahead.
    pub(crate) fn starts_with_ci(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.pos + chars.len() > self.input.len() {
            return false;
        }
        chars
            .iter()
            .enumerate()
            .all(|(i, &expected)| self.input[self.pos + i].eq_ignore_ascii_case(&expected))
    }

    fn is_rawtext_element(name: &str) -> bool {
        matches!(
            name,
            "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript"
        )
    }

    fn is_rcdata_element(name: &str) -> bool {
        matches!(name, "title" | "textarea")
    }

    /// After emitting a start tag, transition to the appropriate
    /// content state if the tag is RAWTEXT or RCDATA.
    fn maybe_switch_content_state(&mut self, name: &str) {
        if Self::is_rawtext_element(name) {
            self.state = State::RawText;
        } else if Self::is_rcdata_element(name) {
            self.state = State::RcData;
        }
    }

    /// Emit a start/end tag from the current tag builder, updating
    /// `last_start_tag` and the state machine as needed.
    pub(crate) fn emit_current_tag(&mut self) -> Token {
        let Some(tag) = self.current_tag.take() else {
            // State machine invariant: current_tag is always Some here.
            // Return a harmless empty comment if the invariant is violated
            // (defensive: avoids panicking on malformed input).
            return Token::Comment(String::new());
        };
        let name = tag.name.clone();
        let is_start = !tag.is_end_tag;
        let tok = tag.into_token();
        if is_start {
            self.last_start_tag = Some(name.clone());
            self.maybe_switch_content_state(&name);
        }
        tok
    }

    // -- main dispatch ------------------------------------------------------

    /// Produce the next token. This drains `temp_buffer` when the
    /// character-reference sub-machine has finished, then resumes the
    /// normal state loop.
    fn next_token(&mut self) -> Token {
        loop {
            // Drain temp_buffer left over from a character reference
            // that returned to a data-like state. Must be checked
            // every iteration so the resolved text is emitted before
            // the next data character is consumed.
            if !self.temp_buffer.is_empty() && !self.state.is_char_ref() {
                let text = std::mem::take(&mut self.temp_buffer);
                return Token::Character(text);
            }

            match self.state {
                State::Data => {
                    if let Some(t) = self.state_data() {
                        return t;
                    }
                },
                State::TagOpen => {
                    if let Some(t) = self.state_tag_open() {
                        return t;
                    }
                },
                State::EndTagOpen => {
                    if let Some(t) = self.state_end_tag_open() {
                        return t;
                    }
                },
                State::TagName => {
                    if let Some(t) = self.state_tag_name() {
                        return t;
                    }
                },
                State::SelfClosingStartTag => {
                    if let Some(t) = self.state_self_closing() {
                        return t;
                    }
                },
                State::BeforeAttributeName => {
                    if let Some(t) = self.state_before_attr_name() {
                        return t;
                    }
                },
                State::AttributeName => {
                    if let Some(t) = self.state_attr_name() {
                        return t;
                    }
                },
                State::AfterAttributeName => {
                    if let Some(t) = self.state_after_attr_name() {
                        return t;
                    }
                },
                State::BeforeAttributeValue => {
                    if let Some(t) = self.state_before_attr_value() {
                        return t;
                    }
                },
                State::AttributeValueDoubleQuoted => {
                    if let Some(t) = self.state_attr_val_dq() {
                        return t;
                    }
                },
                State::AttributeValueSingleQuoted => {
                    if let Some(t) = self.state_attr_val_sq() {
                        return t;
                    }
                },
                State::AttributeValueUnquoted => {
                    if let Some(t) = self.state_attr_val_unquoted() {
                        return t;
                    }
                },
                State::AfterAttributeValueQuoted => {
                    if let Some(t) = self.state_after_attr_val_q() {
                        return t;
                    }
                },
                State::MarkupDeclarationOpen => {
                    if let Some(t) = self.state_markup_decl_open() {
                        return t;
                    }
                },
                State::CommentStart => {
                    if let Some(t) = self.state_comment_start() {
                        return t;
                    }
                },
                State::CommentStartDash => {
                    if let Some(t) = self.state_comment_start_dash() {
                        return t;
                    }
                },
                State::Comment => {
                    if let Some(t) = self.state_comment() {
                        return t;
                    }
                },
                State::CommentEndDash => {
                    if let Some(t) = self.state_comment_end_dash() {
                        return t;
                    }
                },
                State::CommentEnd => {
                    if let Some(t) = self.state_comment_end() {
                        return t;
                    }
                },
                State::Doctype => {
                    if let Some(t) = self.state_doctype() {
                        return t;
                    }
                },
                State::BeforeDoctypeName => {
                    if let Some(t) = self.state_before_doctype_name() {
                        return t;
                    }
                },
                State::DoctypeName => {
                    if let Some(t) = self.state_doctype_name() {
                        return t;
                    }
                },
                State::AfterDoctypeName => {
                    if let Some(t) = self.state_after_doctype_name() {
                        return t;
                    }
                },
                State::BogusComment => {
                    if let Some(t) = self.state_bogus_comment() {
                        return t;
                    }
                },
                State::CharacterReference => {
                    if let Some(t) = self.state_char_ref() {
                        return t;
                    }
                },
                State::NumericCharacterReference => {
                    if let Some(t) = self.state_numeric_char_ref() {
                        return t;
                    }
                },
                State::HexCharacterReferenceStart => {
                    if let Some(t) = self.state_hex_char_ref_start() {
                        return t;
                    }
                },
                State::HexCharacterReference => {
                    if let Some(t) = self.state_hex_char_ref() {
                        return t;
                    }
                },
                State::DecimalCharacterReference => {
                    if let Some(t) = self.state_dec_char_ref() {
                        return t;
                    }
                },
                State::NamedCharacterReference => {
                    if let Some(t) = self.state_named_char_ref() {
                        return t;
                    }
                },
                State::RawText => {
                    if let Some(t) = self.state_rawtext() {
                        return t;
                    }
                },
                State::RcData => {
                    if let Some(t) = self.state_rcdata() {
                        return t;
                    }
                },
            }
        }
    }
}

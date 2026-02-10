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
//! [spec]: https://html.spec.whatwg.org/multipage/parsing.html#tokenization

// ---------------------------------------------------------------------------
// Token types
// ---------------------------------------------------------------------------

/// A single token emitted by the tokenizer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    StartTag(StartTagToken),
    EndTag(EndTagToken),
    Character(String),
    Comment(String),
    Doctype(DoctypeToken),
    Eof,
}

/// An opening tag with optional attributes and self-closing flag.
#[derive(Debug, Clone, PartialEq)]
pub struct StartTagToken {
    pub name: String,
    pub attributes: Vec<Attribute>,
    pub self_closing: bool,
}

/// A closing tag (attributes are discarded per the spec).
#[derive(Debug, Clone, PartialEq)]
pub struct EndTagToken {
    pub name: String,
}

/// A `<!DOCTYPE ...>` token.
#[derive(Debug, Clone, PartialEq)]
pub struct DoctypeToken {
    pub name: Option<String>,
    pub force_quirks: bool,
}

/// A single `name="value"` attribute pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Tokenizer state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    SelfClosingStartTag,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentEndDash,
    CommentEnd,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    BogusComment,
    CharacterReference,
    NumericCharacterReference,
    HexCharacterReferenceStart,
    HexCharacterReference,
    DecimalCharacterReference,
    NamedCharacterReference,
    RawText,
    RcData,
}

impl State {
    /// True when the state belongs to the character-reference sub-machine.
    fn is_char_ref(self) -> bool {
        matches!(
            self,
            Self::CharacterReference
                | Self::NumericCharacterReference
                | Self::HexCharacterReferenceStart
                | Self::HexCharacterReference
                | Self::DecimalCharacterReference
                | Self::NamedCharacterReference
        )
    }
}

/// Builder used while assembling a tag token.
#[derive(Debug, Clone)]
struct TagBuilder {
    name: String,
    attributes: Vec<Attribute>,
    self_closing: bool,
    is_end_tag: bool,
    current_attr_name: String,
    current_attr_value: String,
}

impl TagBuilder {
    fn new(is_end_tag: bool) -> Self {
        Self {
            name: String::new(),
            attributes: Vec::new(),
            self_closing: false,
            is_end_tag,
            current_attr_name: String::new(),
            current_attr_value: String::new(),
        }
    }

    /// Finish the current attribute (if any) and push it.
    fn finish_attribute(&mut self) {
        if !self.current_attr_name.is_empty() {
            self.attributes.push(Attribute {
                name: std::mem::take(&mut self.current_attr_name),
                value: std::mem::take(&mut self.current_attr_value),
            });
        } else {
            self.current_attr_name.clear();
            self.current_attr_value.clear();
        }
    }

    /// Convert into a `Token`.
    fn into_token(mut self) -> Token {
        self.finish_attribute();
        if self.is_end_tag {
            Token::EndTag(EndTagToken { name: self.name })
        } else {
            Token::StartTag(StartTagToken {
                name: self.name,
                attributes: self.attributes,
                self_closing: self.self_closing,
            })
        }
    }
}

/// Builder used while assembling a DOCTYPE token.
#[derive(Debug, Clone)]
struct DoctypeBuilder {
    name: Option<String>,
    force_quirks: bool,
}

impl DoctypeBuilder {
    fn new() -> Self {
        Self {
            name: None,
            force_quirks: false,
        }
    }

    fn into_token(self) -> Token {
        Token::Doctype(DoctypeToken {
            name: self.name,
            force_quirks: self.force_quirks,
        })
    }
}

// ---------------------------------------------------------------------------
// Named character reference table
// ---------------------------------------------------------------------------

/// Resolve a named character reference (without the leading `&`).
/// `name` may include a trailing semicolon.
fn resolve_named_ref(name: &str) -> Option<&'static str> {
    let key = name.strip_suffix(';').unwrap_or(name);
    // Delegate to the central entity table first.
    if let Some(s) = super::entities::lookup_entity(key) {
        return Some(s);
    }
    // Extra entities used in the tokenizer but not in the main table.
    match key {
        "zwnj" => Some("\u{200C}"),
        "zwj" => Some("\u{200D}"),
        "lrm" => Some("\u{200E}"),
        "rlm" => Some("\u{200F}"),
        "iexcl" => Some("\u{00A1}"),
        "iquest" => Some("\u{00BF}"),
        "dagger" => Some("\u{2020}"),
        "Dagger" => Some("\u{2021}"),
        "permil" => Some("\u{2030}"),
        "prime" => Some("\u{2032}"),
        "Prime" => Some("\u{2033}"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// WHATWG HTML tokenizer.
///
/// Construct with [`Tokenizer::new`], then call [`Tokenizer::tokenize`] to
/// consume the input and produce a `Vec<Token>`.
pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    state: State,
    return_state: State,
    current_tag: Option<TagBuilder>,
    current_comment: String,
    current_doctype: DoctypeBuilder,
    temp_buffer: String,
    last_start_tag: Option<String>,
    char_ref_code: u32,
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

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn consume(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// Case-insensitive look-ahead.
    fn starts_with_ci(&self, s: &str) -> bool {
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
    fn emit_current_tag(&mut self) -> Token {
        let tag = self.current_tag.take().unwrap();
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

    // -----------------------------------------------------------------------
    // State implementations
    // -----------------------------------------------------------------------

    /// **Data** -- default entry point.
    fn state_data(&mut self) -> Option<Token> {
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
    fn state_tag_open(&mut self) -> Option<Token> {
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
    fn state_end_tag_open(&mut self) -> Option<Token> {
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
    fn state_tag_name(&mut self) -> Option<Token> {
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
                self.current_tag
                    .as_mut()
                    .unwrap()
                    .name
                    .push(ch.to_ascii_lowercase());
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **SelfClosingStartTag** -- after `/` inside a tag.
    fn state_self_closing(&mut self) -> Option<Token> {
        match self.consume() {
            Some('>') => {
                self.current_tag.as_mut().unwrap().self_closing = true;
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

    /// **BeforeAttributeName** -- after tag name, before attribute.
    fn state_before_attr_name(&mut self) -> Option<Token> {
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
                let tag = self.current_tag.as_mut().unwrap();
                tag.finish_attribute();
                tag.current_attr_name.push('=');
                self.consume();
                self.state = State::AttributeName;
                None
            },
            _ => {
                self.current_tag.as_mut().unwrap().finish_attribute();
                self.state = State::AttributeName;
                None
            },
        }
    }

    /// **AttributeName** -- accumulate attribute name.
    fn state_attr_name(&mut self) -> Option<Token> {
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
                self.current_tag
                    .as_mut()
                    .unwrap()
                    .current_attr_name
                    .push(ch.to_ascii_lowercase());
                None
            },
            None => {
                self.state = State::AfterAttributeName;
                None
            },
        }
    }

    /// **AfterAttributeName** -- after attribute name.
    fn state_after_attr_name(&mut self) -> Option<Token> {
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
                self.current_tag.as_mut().unwrap().finish_attribute();
                self.state = State::AttributeName;
                None
            },
        }
    }

    /// **BeforeAttributeValue** -- before `=` value.
    fn state_before_attr_value(&mut self) -> Option<Token> {
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
    fn state_attr_val_dq(&mut self) -> Option<Token> {
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
                self.current_tag
                    .as_mut()
                    .unwrap()
                    .current_attr_value
                    .push(ch);
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AttributeValueSingleQuoted** -- inside `'...'`.
    fn state_attr_val_sq(&mut self) -> Option<Token> {
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
                self.current_tag
                    .as_mut()
                    .unwrap()
                    .current_attr_value
                    .push(ch);
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AttributeValueUnquoted** -- bare attribute value.
    fn state_attr_val_unquoted(&mut self) -> Option<Token> {
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
                self.current_tag
                    .as_mut()
                    .unwrap()
                    .current_attr_value
                    .push(ch);
                None
            },
            None => {
                self.state = State::Data;
                Some(Token::Eof)
            },
        }
    }

    /// **AfterAttributeValueQuoted** -- after closing quote.
    fn state_after_attr_val_q(&mut self) -> Option<Token> {
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
    fn state_markup_decl_open(&mut self) -> Option<Token> {
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
    fn state_comment_start(&mut self) -> Option<Token> {
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
    fn state_comment_start_dash(&mut self) -> Option<Token> {
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
    fn state_comment(&mut self) -> Option<Token> {
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
    fn state_comment_end_dash(&mut self) -> Option<Token> {
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
    fn state_comment_end(&mut self) -> Option<Token> {
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
    fn state_doctype(&mut self) -> Option<Token> {
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
    fn state_before_doctype_name(&mut self) -> Option<Token> {
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
    fn state_doctype_name(&mut self) -> Option<Token> {
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
    fn state_after_doctype_name(&mut self) -> Option<Token> {
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
    fn emit_doctype(&mut self) -> Token {
        std::mem::replace(&mut self.current_doctype, DoctypeBuilder::new()).into_token()
    }

    /// **BogusComment**.
    fn state_bogus_comment(&mut self) -> Option<Token> {
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

    // -- character references -----------------------------------------------

    /// **CharacterReference** -- after `&`.
    fn state_char_ref(&mut self) -> Option<Token> {
        self.temp_buffer.clear();
        self.temp_buffer.push('&');

        match self.peek() {
            Some('#') => {
                self.consume();
                self.temp_buffer.push('#');
                self.state = State::NumericCharacterReference;
                None
            },
            Some(ch) if ch.is_ascii_alphanumeric() => {
                self.state = State::NamedCharacterReference;
                None
            },
            _ => {
                self.flush_temp_buffer();
                self.state = self.return_state;
                None
            },
        }
    }

    /// **NumericCharacterReference** -- after `&#`.
    fn state_numeric_char_ref(&mut self) -> Option<Token> {
        self.char_ref_code = 0;
        match self.peek() {
            Some('x' | 'X') => {
                self.consume();
                self.temp_buffer.push('x');
                self.state = State::HexCharacterReferenceStart;
                None
            },
            _ => {
                self.state = State::DecimalCharacterReference;
                None
            },
        }
    }

    /// **HexCharacterReferenceStart**.
    fn state_hex_char_ref_start(&mut self) -> Option<Token> {
        if matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
            self.state = State::HexCharacterReference;
        } else {
            self.flush_temp_buffer();
            self.state = self.return_state;
        }
        None
    }

    /// **HexCharacterReference**.
    fn state_hex_char_ref(&mut self) -> Option<Token> {
        match self.peek() {
            Some(ch) if ch.is_ascii_hexdigit() => {
                self.consume();
                self.char_ref_code = self
                    .char_ref_code
                    .saturating_mul(16)
                    .saturating_add(ch.to_digit(16).unwrap_or(0));
                None
            },
            Some(';') => {
                self.consume();
                self.finish_numeric_char_ref();
                self.state = self.return_state;
                None
            },
            _ => {
                self.finish_numeric_char_ref();
                self.state = self.return_state;
                None
            },
        }
    }

    /// **DecimalCharacterReference**.
    fn state_dec_char_ref(&mut self) -> Option<Token> {
        match self.peek() {
            Some(ch) if ch.is_ascii_digit() => {
                self.consume();
                self.char_ref_code = self
                    .char_ref_code
                    .saturating_mul(10)
                    .saturating_add(ch.to_digit(10).unwrap_or(0));
                None
            },
            Some(';') => {
                self.consume();
                self.finish_numeric_char_ref();
                self.state = self.return_state;
                None
            },
            _ => {
                if self.char_ref_code == 0 {
                    self.flush_temp_buffer();
                } else {
                    self.finish_numeric_char_ref();
                }
                self.state = self.return_state;
                None
            },
        }
    }

    /// **NamedCharacterReference**.
    fn state_named_char_ref(&mut self) -> Option<Token> {
        let mut name = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() {
                self.consume();
                name.push(ch);
            } else if ch == ';' {
                self.consume();
                name.push(';');
                break;
            } else {
                break;
            }
        }

        if let Some(replacement) = resolve_named_ref(&name) {
            self.emit_char_ref_text(replacement);
        } else {
            let literal = format!("&{name}");
            self.emit_char_ref_text(&literal);
        }

        self.state = self.return_state;
        None
    }

    // -- character reference helpers ----------------------------------------

    /// Convert accumulated numeric code point to character and emit.
    fn finish_numeric_char_ref(&mut self) {
        let ch = match self.char_ref_code {
            0 | 0xD800..=0xDFFF => '\u{FFFD}',
            c if c > 0x10FFFF => '\u{FFFD}',
            c => char::from_u32(c).unwrap_or('\u{FFFD}'),
        };
        let s = ch.to_string();
        self.emit_char_ref_text(&s);
    }

    /// Emit resolved character reference text to the correct
    /// destination (attribute value or temp_buffer for data emission).
    fn emit_char_ref_text(&mut self, text: &str) {
        if self.return_state_is_attr() {
            // Clear temp_buffer so the `&` pushed by state_char_ref
            // is not spuriously drained as a Character token.
            self.temp_buffer.clear();
            if let Some(ref mut tag) = self.current_tag {
                tag.current_attr_value.push_str(text);
            }
        } else {
            self.temp_buffer.clear();
            self.temp_buffer.push_str(text);
        }
    }

    /// Flush `temp_buffer` to attribute value (when `&` did not
    /// resolve and we are inside an attribute) or leave it in
    /// `temp_buffer` for the `next_token` drain.
    fn flush_temp_buffer(&mut self) {
        if self.return_state_is_attr() {
            let buf = std::mem::take(&mut self.temp_buffer);
            if let Some(ref mut tag) = self.current_tag {
                tag.current_attr_value.push_str(&buf);
            }
        }
        // For data-like return states, temp_buffer is drained at the
        // top of next_token().
    }

    fn return_state_is_attr(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
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
        let tag_name = self.last_start_tag.clone().unwrap();
        self.current_tag = Some(TagBuilder::new(true));
        self.current_tag.as_mut().unwrap().name = tag_name;
        // Skip to `>`.
        loop {
            match self.consume() {
                Some('>') | None => break,
                _ => {},
            }
        }
        self.current_tag.take().unwrap().into_token()
    }

    /// **RawText** -- for `<script>`, `<style>`, etc.
    fn state_rawtext(&mut self) -> Option<Token> {
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
    fn state_rcdata(&mut self) -> Option<Token> {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: tokenize and strip the trailing Eof.
    fn tok(input: &str) -> Vec<Token> {
        let mut t = Tokenizer::new(input);
        let mut tokens = t.tokenize();
        if matches!(tokens.last(), Some(Token::Eof)) {
            tokens.pop();
        }
        tokens
    }

    // -- basic tags ---------------------------------------------------------

    #[test]
    fn basic_paragraph() {
        let tokens = tok("<p>Hello</p>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("Hello".into()));
        assert_eq!(tokens[2], Token::EndTag(EndTagToken { name: "p".into() }));
    }

    #[test]
    fn self_closing_br() {
        let tokens = tok("<br/>");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: true,
            })
        );
    }

    #[test]
    fn self_closing_img_with_attr() {
        let tokens = tok(r#"<img src="test.png"/>"#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "img".into(),
                attributes: vec![Attribute {
                    name: "src".into(),
                    value: "test.png".into(),
                }],
                self_closing: true,
            })
        );
    }

    // -- attributes ---------------------------------------------------------

    #[test]
    fn double_quoted_attribute() {
        let tokens = tok(r#"<a href="http://example.com">link</a>"#);
        assert_eq!(tokens.len(), 3);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.name, "a");
            assert_eq!(tag.attributes.len(), 1);
            assert_eq!(tag.attributes[0].name, "href");
            assert_eq!(tag.attributes[0].value, "http://example.com");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn single_quoted_attribute() {
        let tokens = tok("<div class='main'>");
        assert_eq!(tokens.len(), 1);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes[0].name, "class");
            assert_eq!(tag.attributes[0].value, "main");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn unquoted_attribute() {
        let tokens = tok("<input type=text>");
        assert_eq!(tokens.len(), 1);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes[0].name, "type");
            assert_eq!(tag.attributes[0].value, "text");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn boolean_attribute() {
        let tokens = tok("<input disabled>");
        assert_eq!(tokens.len(), 1);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes.len(), 1);
            assert_eq!(tag.attributes[0].name, "disabled");
            assert_eq!(tag.attributes[0].value, "");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn multiple_attributes() {
        let tokens = tok(r#"<input type="text" name="q" value="search">"#);
        assert_eq!(tokens.len(), 1);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes.len(), 3);
            assert_eq!(tag.attributes[0].name, "type");
            assert_eq!(tag.attributes[0].value, "text");
            assert_eq!(tag.attributes[1].name, "name");
            assert_eq!(tag.attributes[1].value, "q");
            assert_eq!(tag.attributes[2].name, "value");
            assert_eq!(tag.attributes[2].value, "search");
        } else {
            panic!("expected start tag");
        }
    }

    // -- character references -----------------------------------------------

    #[test]
    fn named_char_ref_amp() {
        let tokens = tok("a&amp;b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a&b".into()));
    }

    #[test]
    fn named_char_ref_lt_gt() {
        let tokens = tok("&lt;div&gt;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<div>".into()));
    }

    #[test]
    fn decimal_char_ref() {
        let tokens = tok("&#60;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn hex_char_ref_lower() {
        let tokens = tok("&#x3c;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn hex_char_ref_upper() {
        let tokens = tok("&#x3C;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn char_ref_in_attribute() {
        let tokens = tok(r#"<a href="?a=1&amp;b=2">x</a>"#);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes[0].value, "?a=1&b=2");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn named_char_ref_nbsp() {
        let tokens = tok("hello&nbsp;world");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("hello\u{00A0}world".into()));
    }

    // -- comments -----------------------------------------------------------

    #[test]
    fn basic_comment() {
        let tokens = tok("<!-- comment -->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment(" comment ".into()));
    }

    #[test]
    fn empty_comment() {
        let tokens = tok("<!---->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment("".into()));
    }

    #[test]
    fn comment_with_dashes() {
        let tokens = tok("<!-- a -- b -->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment(" a -- b ".into()));
    }

    // -- doctype ------------------------------------------------------------

    #[test]
    fn doctype_html() {
        let tokens = tok("<!DOCTYPE html>");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Doctype(DoctypeToken {
                name: Some("html".into()),
                force_quirks: false,
            })
        );
    }

    #[test]
    fn doctype_case_insensitive() {
        let tokens = tok("<!doctype HTML>");
        assert_eq!(tokens.len(), 1);
        if let Token::Doctype(ref dt) = tokens[0] {
            assert_eq!(dt.name, Some("html".into()));
        } else {
            panic!("expected doctype");
        }
    }

    // -- nested tags --------------------------------------------------------

    #[test]
    fn nested_tags() {
        let tokens = tok("<div><p>text</p></div>");
        assert_eq!(tokens.len(), 5);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "div".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[2], Token::Character("text".into()));
        assert_eq!(tokens[3], Token::EndTag(EndTagToken { name: "p".into() }));
        assert_eq!(tokens[4], Token::EndTag(EndTagToken { name: "div".into() }));
    }

    // -- malformed input ----------------------------------------------------

    #[test]
    fn unclosed_tag() {
        let tokens = tok("<p>hello");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("hello".into()));
    }

    #[test]
    fn bare_less_than() {
        let tokens = tok("a < b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a < b".into()));
    }

    #[test]
    fn empty_input() {
        let tokens = tok("");
        assert!(tokens.is_empty());
    }

    // -- script content (RAWTEXT) -------------------------------------------

    #[test]
    fn script_content() {
        let tokens = tok("<script>var x = 1 < 2;</script>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "script".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("var x = 1 < 2;".into()));
        assert_eq!(
            tokens[2],
            Token::EndTag(EndTagToken {
                name: "script".into(),
            })
        );
    }

    #[test]
    fn style_content() {
        let tokens = tok("<style>body { color: red; }</style>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1], Token::Character("body { color: red; }".into()));
    }

    // -- RCDATA -------------------------------------------------------------

    #[test]
    fn title_with_char_ref() {
        let tokens = tok("<title>Page &amp; Title</title>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "title".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("Page & Title".into()));
        assert_eq!(
            tokens[2],
            Token::EndTag(EndTagToken {
                name: "title".into(),
            })
        );
    }

    #[test]
    fn textarea_rcdata() {
        let tokens = tok("<textarea>some &lt;text&gt;</textarea>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1], Token::Character("some <text>".into()));
    }

    // -- mixed content ------------------------------------------------------

    #[test]
    fn mixed_content() {
        // "Hello <b>world</b> and <i>friends</i>!" produces 9 tokens:
        // Character, StartTag, Character, EndTag, Character, StartTag,
        // Character, EndTag, Character.
        let tokens = tok("Hello <b>world</b> and <i>friends</i>!");
        assert_eq!(tokens.len(), 9);
        assert_eq!(tokens[0], Token::Character("Hello ".into()));
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "b".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[2], Token::Character("world".into()));
        assert_eq!(tokens[3], Token::EndTag(EndTagToken { name: "b".into() }));
        assert_eq!(tokens[4], Token::Character(" and ".into()));
        assert_eq!(
            tokens[5],
            Token::StartTag(StartTagToken {
                name: "i".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[6], Token::Character("friends".into()));
        assert_eq!(tokens[7], Token::EndTag(EndTagToken { name: "i".into() }));
        assert_eq!(tokens[8], Token::Character("!".into()));
    }

    // -- void elements ------------------------------------------------------

    #[test]
    fn void_elements() {
        let tokens = tok("<br><hr><img src=\"a.png\">");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "hr".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[2],
            Token::StartTag(StartTagToken {
                name: "img".into(),
                attributes: vec![Attribute {
                    name: "src".into(),
                    value: "a.png".into(),
                }],
                self_closing: false,
            })
        );
    }

    // -- full document ------------------------------------------------------

    #[test]
    fn full_document() {
        let html = concat!(
            "<!DOCTYPE html>",
            "<html><head><title>Test</title></head>",
            "<body><p>Hello</p></body></html>",
        );
        let tokens = tok(html);
        // DOCTYPE, <html>, <head>, <title>, "Test",
        // </title>, </head>, <body>, <p>, "Hello",
        // </p>, </body>, </html>
        assert_eq!(tokens.len(), 13);
        assert_eq!(
            tokens[0],
            Token::Doctype(DoctypeToken {
                name: Some("html".into()),
                force_quirks: false,
            })
        );
    }

    // -- edge cases ---------------------------------------------------------

    #[test]
    fn tag_name_case_insensitive() {
        let tokens = tok("<DIV>x</DIV>");
        assert_eq!(tokens.len(), 3);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.name, "div");
        } else {
            panic!("expected start tag");
        }
        if let Token::EndTag(ref tag) = tokens[2] {
            assert_eq!(tag.name, "div");
        } else {
            panic!("expected end tag");
        }
    }

    #[test]
    fn attribute_name_case_insensitive() {
        let tokens = tok(r#"<div CLASS="x">"#);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.attributes[0].name, "class");
        } else {
            panic!("expected start tag");
        }
    }

    #[test]
    fn bogus_comment_from_question_mark() {
        let tokens = tok("<?xml version=\"1.0\"?>");
        assert_eq!(tokens.len(), 1);
        if let Token::Comment(_) = tokens[0] {
            // Good -- treated as bogus comment.
        } else {
            panic!("expected comment");
        }
    }

    #[test]
    fn self_closing_with_space() {
        let tokens = tok("<br />");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: true,
            })
        );
    }

    #[test]
    fn unknown_entity_passthrough() {
        let tokens = tok("&foobar;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("&foobar;".into()));
    }

    #[test]
    fn numeric_ref_zero_becomes_replacement() {
        let tokens = tok("&#0;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("\u{FFFD}".into()));
    }

    #[test]
    fn multiple_char_refs_coalesce() {
        let tokens = tok("&lt;&gt;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<>".into()));
    }

    #[test]
    fn script_with_attributes() {
        let tokens = tok(r#"<script type="text/javascript">alert(1)</script>"#);
        assert_eq!(tokens.len(), 3);
        if let Token::StartTag(ref tag) = tokens[0] {
            assert_eq!(tag.name, "script");
            assert_eq!(tag.attributes[0].name, "type");
            assert_eq!(tag.attributes[0].value, "text/javascript");
        } else {
            panic!("expected start tag");
        }
        assert_eq!(tokens[1], Token::Character("alert(1)".into()));
    }

    #[test]
    fn bare_ampersand_not_ref() {
        let tokens = tok("a & b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a & b".into()));
    }
}

//! Token types, state enum, and internal builders for the HTML tokenizer.

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
#[repr(u32)]
pub(crate) enum State {
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
    pub(crate) fn is_char_ref(self) -> bool {
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
pub(crate) struct TagBuilder {
    pub(crate) name: String,
    pub(crate) attributes: Vec<Attribute>,
    pub(crate) self_closing: bool,
    pub(crate) is_end_tag: bool,
    pub(crate) current_attr_name: String,
    pub(crate) current_attr_value: String,
}

impl TagBuilder {
    pub(crate) fn new(is_end_tag: bool) -> Self {
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
    pub(crate) fn finish_attribute(&mut self) {
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
    pub(crate) fn into_token(mut self) -> Token {
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
pub(crate) struct DoctypeBuilder {
    pub(crate) name: Option<String>,
    pub(crate) force_quirks: bool,
}

impl DoctypeBuilder {
    pub(crate) fn new() -> Self {
        Self {
            name: None,
            force_quirks: false,
        }
    }

    pub(crate) fn into_token(self) -> Token {
        Token::Doctype(DoctypeToken {
            name: self.name,
            force_quirks: self.force_quirks,
        })
    }
}

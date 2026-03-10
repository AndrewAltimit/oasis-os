//! Character reference handling for the HTML tokenizer.
//!
//! Implements the character-reference sub-machine (named, numeric,
//! hexadecimal) and helper methods for resolving references both in
//! the main state machine and inline within RCDATA content.

use super::Tokenizer;
use super::types::{State, Token};

// ---------------------------------------------------------------------------
// Named character reference table
// ---------------------------------------------------------------------------

/// Resolve a named character reference (without the leading `&`).
/// `name` may include a trailing semicolon.
pub(crate) fn resolve_named_ref(name: &str) -> Option<&'static str> {
    let key = name.strip_suffix(';').unwrap_or(name);
    // Delegate to the central entity table first.
    if let Some(s) = crate::html::entities::lookup_entity(key) {
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
// Character reference state implementations
// ---------------------------------------------------------------------------

impl Tokenizer {
    /// **CharacterReference** -- after `&`.
    pub(crate) fn state_char_ref(&mut self) -> Option<Token> {
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
    pub(crate) fn state_numeric_char_ref(&mut self) -> Option<Token> {
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
    pub(crate) fn state_hex_char_ref_start(&mut self) -> Option<Token> {
        if matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
            self.state = State::HexCharacterReference;
        } else {
            self.flush_temp_buffer();
            self.state = self.return_state;
        }
        None
    }

    /// **HexCharacterReference**.
    pub(crate) fn state_hex_char_ref(&mut self) -> Option<Token> {
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
    pub(crate) fn state_dec_char_ref(&mut self) -> Option<Token> {
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
    pub(crate) fn state_named_char_ref(&mut self) -> Option<Token> {
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
    pub(crate) fn finish_numeric_char_ref(&mut self) {
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
    pub(crate) fn emit_char_ref_text(&mut self, text: &str) {
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
    pub(crate) fn flush_temp_buffer(&mut self) {
        if self.return_state_is_attr() {
            let buf = std::mem::take(&mut self.temp_buffer);
            if let Some(ref mut tag) = self.current_tag {
                tag.current_attr_value.push_str(&buf);
            }
        }
        // For data-like return states, temp_buffer is drained at the
        // top of next_token().
    }

    pub(crate) fn return_state_is_attr(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
    }
}

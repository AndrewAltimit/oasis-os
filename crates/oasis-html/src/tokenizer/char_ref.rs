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

/// Match a named character reference (WHATWG §13.2.5.73). `name` is the
/// run of ASCII alphanumerics after the `&`, with the `;` that ended it
/// if any. Returns the replacement and how many bytes of `name` it used;
/// the rest stays literal text.
///
/// With a `;` any known name matches. Without one only the legacy
/// references do, as the longest such prefix (`&copy2024` is `©2024`,
/// `&notin` without `;` is `¬in`); `&mdash` stays literal.
///
/// An unknown name *with* a `;` stays literal instead of matching a legacy
/// prefix: the table covers only part of HTML5's names, so `&centerdot;`
/// or `&parallel;` (real names it lacks) would otherwise come out as
/// `¢erdot;` / `¶llel;`.
pub(crate) fn match_named_ref(name: &str) -> Option<(&'static str, usize)> {
    use crate::entities::{is_legacy_entity, lookup_entity};
    if let Some(base) = name.strip_suffix(';') {
        return lookup_entity(base).map(|replacement| (replacement, name.len()));
    }
    let base = name;
    // The longest legacy name is six characters.
    (1..=base.len().min(6)).rev().find_map(|n| {
        let prefix = base.get(..n)?;
        if is_legacy_entity(prefix) {
            lookup_entity(prefix).map(|r| (r, n))
        } else {
            None
        }
    })
}

/// [`match_named_ref`] as text: replacement plus the unused tail, or
/// the literal `&name`.
pub(crate) fn resolve_named_ref_text(name: &str) -> String {
    match match_named_ref(name) {
        Some((replacement, used)) => format!("{replacement}{}", &name[used..]),
        None => format!("&{name}"),
    }
}

/// The character a numeric reference `&#code;` stands for (§13.2.5.80):
/// U+FFFD for NUL, surrogates and out-of-range values, and C1 controls
/// as the Windows-1252 characters pages mean (`&#150;` is an en dash).
pub(crate) fn numeric_ref_char(code: u32) -> char {
    match code {
        0 | 0xD800..=0xDFFF => '\u{FFFD}',
        c if c > 0x10FFFF => '\u{FFFD}',
        c @ 0x80..=0x9F => windows_1252_c1(c),
        c => char::from_u32(c).unwrap_or('\u{FFFD}'),
    }
}

/// The numeric-reference replacement for a C1 control code point
/// (0x80-0x9F): its Windows-1252 character, or the code point itself for
/// the five 1252 leaves unassigned.
fn windows_1252_c1(code: u32) -> char {
    const TABLE: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    usize::try_from(code - 0x80)
        .ok()
        .and_then(|i| TABLE.get(i).copied())
        .unwrap_or('\u{FFFD}')
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
            // Decimal start: without a digit, `&#` stays literal (`&#;`
            // used to become U+FFFD and `&#abc` lost nothing but luck).
            Some(c) if c.is_ascii_digit() => {
                self.state = State::DecimalCharacterReference;
                None
            },
            _ => {
                self.flush_temp_buffer();
                self.state = self.return_state;
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
            // At least one digit was consumed (see the start state), so
            // a missing `;` still resolves: `&#0abc` is U+FFFD + `abc`
            // (it used to emit `&#abc`, dropping the digit).
            _ => {
                self.finish_numeric_char_ref();
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

        // In an attribute, a reference without `;` followed by `=` or an
        // alphanumeric is left alone (§13.2.5.73, "historical reasons"):
        // `?a=1&copy=2` is a query string, not `?a=1©=2`.
        let text = match match_named_ref(&name) {
            Some((replacement, used)) => {
                let rest = &name[used..];
                let next = rest.chars().next().or(self.peek());
                let historical_attr_case = !name[..used].ends_with(';')
                    && self.return_state_is_attr()
                    && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
                if historical_attr_case {
                    format!("&{name}")
                } else {
                    format!("{replacement}{rest}")
                }
            },
            None => format!("&{name}"),
        };
        self.emit_char_ref_text(&text);

        self.state = self.return_state;
        None
    }

    // -- character reference helpers ----------------------------------------

    /// Convert accumulated numeric code point to character and emit.
    pub(crate) fn finish_numeric_char_ref(&mut self) {
        let s = numeric_ref_char(self.char_ref_code).to_string();
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

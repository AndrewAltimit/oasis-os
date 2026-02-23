//! Text shaping and line-breaking utilities.
//!
//! Provides helpers for word-breaking, whitespace collapsing, and text
//! measurement used by the inline layout algorithm.

use super::block::TextMeasurer;
use crate::css::values::{TextTransform, WhiteSpace};

// -------------------------------------------------------------------
// Text word
// -------------------------------------------------------------------

/// A single word extracted from a text run, used for line breaking.
#[derive(Debug, Clone, PartialEq)]
pub struct TextWord {
    /// The actual word content.
    pub text: String,
    /// Whether this word had trailing whitespace in the source text.
    pub trailing_space: bool,
}

// -------------------------------------------------------------------
// Whitespace collapsing
// -------------------------------------------------------------------

/// Collapse whitespace according to the CSS `white-space` property.
///
/// - `Normal` / `NoWrap`: collapse runs of whitespace to a single
///   space and strip leading/trailing whitespace.
/// - `Pre` / `PreWrap`: preserve all whitespace as-is.
/// - `PreLine`: collapse spaces/tabs to a single space but preserve
///   newlines.
pub fn collapse_whitespace(text: &str, white_space: WhiteSpace) -> String {
    match white_space {
        WhiteSpace::Normal | WhiteSpace::NoWrap => {
            let mut result = String::with_capacity(text.len());
            let mut in_space = true; // treat leading ws as collapsible
            for ch in text.chars() {
                if ch.is_ascii_whitespace() {
                    if !in_space {
                        result.push(' ');
                        in_space = true;
                    }
                } else {
                    result.push(ch);
                    in_space = false;
                }
            }
            // Strip trailing space.
            if result.ends_with(' ') {
                result.pop();
            }
            result
        },
        WhiteSpace::Pre | WhiteSpace::PreWrap => text.to_string(),
        WhiteSpace::PreLine => {
            let mut result = String::with_capacity(text.len());
            let mut in_space = false;
            for ch in text.chars() {
                if ch == '\n' {
                    // Drop any pending collapsed space before newline.
                    if result.ends_with(' ') {
                        result.pop();
                    }
                    result.push('\n');
                    in_space = false;
                } else if ch == ' ' || ch == '\t' {
                    if !in_space {
                        result.push(' ');
                        in_space = true;
                    }
                } else {
                    result.push(ch);
                    in_space = false;
                }
            }
            result
        },
    }
}

// -------------------------------------------------------------------
// Word splitting
// -------------------------------------------------------------------

/// Split text into words for line breaking, respecting the CSS
/// `white-space` property.
///
/// Each word carries a `trailing_space` flag indicating whether there
/// was whitespace after it in the source (relevant for measuring
/// inter-word spacing).
pub fn split_into_words(text: &str, white_space: WhiteSpace) -> Vec<TextWord> {
    match white_space {
        WhiteSpace::Pre | WhiteSpace::PreWrap => {
            // In pre modes, split only on newlines; preserve spaces
            // within each line as a single chunk.
            let mut words = Vec::new();
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    words.push(TextWord {
                        text: "\n".to_string(),
                        trailing_space: false,
                    });
                }
                if !line.is_empty() {
                    words.push(TextWord {
                        text: line.to_string(),
                        trailing_space: false,
                    });
                }
            }
            words
        },
        WhiteSpace::PreLine => {
            let collapsed = collapse_whitespace(text, WhiteSpace::PreLine);
            let mut words = Vec::new();
            for (i, line) in collapsed.split('\n').enumerate() {
                if i > 0 {
                    words.push(TextWord {
                        text: "\n".to_string(),
                        trailing_space: false,
                    });
                }
                split_line_into_words(line, &mut words);
            }
            words
        },
        WhiteSpace::Normal | WhiteSpace::NoWrap => {
            let collapsed = collapse_whitespace(text, WhiteSpace::Normal);
            let mut words = Vec::new();
            split_line_into_words(&collapsed, &mut words);
            words
        },
    }
}

/// Split a single line (no embedded newlines) into space-separated
/// words.
fn split_line_into_words(line: &str, out: &mut Vec<TextWord>) {
    let parts: Vec<&str> = line.split(' ').collect();
    let last_idx = parts.len().saturating_sub(1);
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        out.push(TextWord {
            text: (*part).to_string(),
            trailing_space: i < last_idx,
        });
    }
}

// -------------------------------------------------------------------
// Text measurement
// -------------------------------------------------------------------

/// Measure a word's pixel width using the backend text measurer.
///
/// Adds `letter_spacing` between each pair of characters.
pub fn measure_word(
    word: &str,
    font_size: f32,
    letter_spacing: f32,
    measurer: &dyn TextMeasurer,
) -> f32 {
    let base = measurer.measure_text(word, font_size as u16) as f32;
    if letter_spacing != 0.0 {
        let chars = word.chars().count();
        if chars > 1 {
            return (base + letter_spacing * (chars - 1) as f32).max(0.0);
        }
    }
    base
}

/// Measure the width of a single space character at the given font
/// size.
///
/// Adds `word_spacing` to the natural space width.
pub fn measure_space(font_size: f32, word_spacing: f32, measurer: &dyn TextMeasurer) -> f32 {
    measurer.measure_text(" ", font_size as u16) as f32 + word_spacing
}

// -------------------------------------------------------------------
// Text transform
// -------------------------------------------------------------------

/// Apply the CSS `text-transform` property to a string.
pub fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => capitalize_words(text),
    }
}

/// Capitalize the first letter of each whitespace-delimited word.
fn capitalize_words(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true;
    for ch in text.chars() {
        if ch.is_ascii_whitespace() {
            result.push(ch);
            capitalize_next = true;
        } else if capitalize_next {
            for upper in ch.to_uppercase() {
                result.push(upper);
            }
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub measurer: each character is 8 pixels wide.
    struct StubMeasurer;

    impl TextMeasurer for StubMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    // -- whitespace collapsing ----------------------------------------

    #[test]
    fn collapse_normal_multiple_spaces() {
        let result = collapse_whitespace("hello   world", WhiteSpace::Normal);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn collapse_normal_leading_trailing() {
        let result = collapse_whitespace("  hello  ", WhiteSpace::Normal);
        assert_eq!(result, "hello");
    }

    #[test]
    fn collapse_normal_tabs_and_newlines() {
        let result = collapse_whitespace("hello\t\n  world", WhiteSpace::Normal);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn collapse_nowrap_same_as_normal() {
        let result = collapse_whitespace("  a   b  ", WhiteSpace::NoWrap);
        assert_eq!(result, "a b");
    }

    #[test]
    fn preserve_pre_whitespace() {
        let input = "  hello\n  world  ";
        let result = collapse_whitespace(input, WhiteSpace::Pre);
        assert_eq!(result, input);
    }

    #[test]
    fn preserve_pre_wrap_whitespace() {
        let input = "hello   world";
        let result = collapse_whitespace(input, WhiteSpace::PreWrap);
        assert_eq!(result, input);
    }

    #[test]
    fn pre_line_collapses_spaces_preserves_newlines() {
        let result = collapse_whitespace("hello   world\n  next", WhiteSpace::PreLine);
        assert_eq!(result, "hello world\n next");
    }

    // -- word splitting -----------------------------------------------

    #[test]
    fn split_normal_simple() {
        let words = split_into_words("hello world", WhiteSpace::Normal);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "hello");
        assert!(words[0].trailing_space);
        assert_eq!(words[1].text, "world");
        assert!(!words[1].trailing_space);
    }

    #[test]
    fn split_normal_collapses_spaces() {
        let words = split_into_words("  hello   world  ", WhiteSpace::Normal);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "hello");
        assert_eq!(words[1].text, "world");
    }

    #[test]
    fn split_pre_preserves_spaces() {
        let words = split_into_words("hello  world", WhiteSpace::Pre);
        // In pre mode, "hello  world" is one continuous chunk
        // (no newline to split on).
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "hello  world");
    }

    #[test]
    fn split_pre_splits_on_newlines() {
        let words = split_into_words("line1\nline2", WhiteSpace::Pre);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "line1");
        assert_eq!(words[1].text, "\n");
        assert_eq!(words[2].text, "line2");
    }

    // -- text transform -----------------------------------------------

    #[test]
    fn transform_none() {
        assert_eq!(
            apply_text_transform("Hello World", TextTransform::None),
            "Hello World",
        );
    }

    #[test]
    fn transform_uppercase() {
        assert_eq!(
            apply_text_transform("hello world", TextTransform::Uppercase,),
            "HELLO WORLD",
        );
    }

    #[test]
    fn transform_lowercase() {
        assert_eq!(
            apply_text_transform("HELLO WORLD", TextTransform::Lowercase,),
            "hello world",
        );
    }

    #[test]
    fn transform_capitalize() {
        assert_eq!(
            apply_text_transform("hello world foo", TextTransform::Capitalize,),
            "Hello World Foo",
        );
    }

    #[test]
    fn transform_capitalize_already_capitalized() {
        assert_eq!(
            apply_text_transform("Hello World", TextTransform::Capitalize,),
            "Hello World",
        );
    }

    // -- text measurement ---------------------------------------------

    #[test]
    fn measure_word_stub() {
        let m = StubMeasurer;
        let w = measure_word("hello", 16.0, 0.0, &m);
        // Proportional: h(7)+e(7)+l(5)+l(5)+o(7) = 31, scale=2 at font_size 16 => 62
        assert_eq!(w, 62.0);
    }

    #[test]
    fn measure_word_with_letter_spacing() {
        let m = StubMeasurer;
        // "hello" = 62px base, 5 chars, letter_spacing 2.0 => 62 + 2*(5-1) = 70
        let w = measure_word("hello", 16.0, 2.0, &m);
        assert_eq!(w, 70.0);
    }

    #[test]
    fn measure_space_stub() {
        let m = StubMeasurer;
        let w = measure_space(16.0, 0.0, &m);
        assert_eq!(w, 8.0);
    }

    #[test]
    fn measure_space_with_word_spacing() {
        let m = StubMeasurer;
        let w = measure_space(16.0, 3.0, &m);
        assert_eq!(w, 11.0);
    }

    #[test]
    fn test_negative_letter_spacing_no_negative_width() {
        let m = StubMeasurer;
        // "ab" base width = 16 (8*2 at font_size 16 via StubMeasurer)
        // letter_spacing = -100 => 16 + (-100 * 1) = -84
        // Should be clamped to 0.
        let w = measure_word("ab", 16.0, -100.0, &m);
        assert!(
            w >= 0.0,
            "negative letter-spacing should not produce negative width, got {w}",
        );
    }
}

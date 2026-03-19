//! Text shaping and line-breaking utilities.
//!
//! Provides helpers for word-breaking, whitespace collapsing, and text
//! measurement used by the inline layout algorithm.

use super::block::TextMeasurer;
use crate::css::values::{TextTransform, WhiteSpace};

// -------------------------------------------------------------------
// Unicode fallback
// -------------------------------------------------------------------

/// Pass through all characters, preserving unknown ones for tofu-box
/// rendering by the bitmap font (which returns an outlined □ fallback
/// for any character without a dedicated glyph).
pub fn replace_unrenderable(text: &str) -> String {
    text.to_string()
}

// -------------------------------------------------------------------
// Text word
// -------------------------------------------------------------------

/// A single word extracted from a text run, used for line breaking.
#[derive(Debug, Clone, PartialEq)]
pub struct TextWord {
    /// The actual word content.
    pub text: String,
    /// Whether this word had leading whitespace in the source text.
    pub leading_space: bool,
    /// Whether this word had trailing whitespace in the source text.
    pub trailing_space: bool,
    /// If true, a visible hyphen should be rendered when this word
    /// is at the end of a line break (soft hyphen U+00AD boundary).
    pub soft_hyphen: bool,
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
            let mut in_space = false;
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
            result
        },
        WhiteSpace::Pre | WhiteSpace::PreWrap => {
            // Expand tabs to spaces using `tab-size` (default 8).
            if text.contains('\t') {
                let mut result = String::with_capacity(text.len());
                let mut col = 0usize;
                let tab = 8; // Default; caller should pre-expand with correct tab_size.
                for ch in text.chars() {
                    if ch == '\t' {
                        let spaces = tab - (col % tab);
                        for _ in 0..spaces {
                            result.push(' ');
                        }
                        col += spaces;
                    } else if ch == '\n' {
                        result.push(ch);
                        col = 0;
                    } else {
                        result.push(ch);
                        col += 1;
                    }
                }
                result
            } else {
                text.to_string()
            }
        },
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
                        leading_space: false,
                        trailing_space: false,
                        soft_hyphen: false,
                    });
                }
                if !line.is_empty() {
                    words.push(TextWord {
                        text: line.to_string(),
                        leading_space: false,
                        trailing_space: false,
                        soft_hyphen: false,
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
                        leading_space: false,
                        trailing_space: false,
                        soft_hyphen: false,
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
/// words, further splitting on soft hyphen (U+00AD) boundaries.
fn split_line_into_words(line: &str, out: &mut Vec<TextWord>) {
    let parts: Vec<&str> = line.split(' ').collect();
    let last_idx = parts.len().saturating_sub(1);
    let mut saw_empty = false;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            saw_empty = true;
            continue;
        }
        // Further split each word on soft hyphen boundaries.
        let shy_parts: Vec<&str> = part.split('\u{00AD}').collect();
        if shy_parts.len() > 1 {
            for (j, sp) in shy_parts.iter().enumerate() {
                if !sp.is_empty() {
                    out.push(TextWord {
                        text: sp.to_string(),
                        leading_space: if j == 0 { saw_empty } else { false },
                        trailing_space: if j == shy_parts.len() - 1 {
                            i < last_idx
                        } else {
                            false
                        },
                        soft_hyphen: j < shy_parts.len() - 1,
                    });
                }
            }
        } else {
            out.push(TextWord {
                text: (*part).to_string(),
                leading_space: saw_empty,
                trailing_space: i < last_idx,
                soft_hyphen: false,
            });
        }
        saw_empty = false;
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
// Bidi text direction detection
// -------------------------------------------------------------------

use crate::css::values::TextDirection;

/// Check if a character is in an RTL Unicode block.
pub fn is_rtl_char(ch: char) -> bool {
    matches!(ch as u32,
        0x0590..=0x05FF |  // Hebrew
        0x0600..=0x06FF |  // Arabic
        0x0700..=0x074F |  // Syriac
        0xFB50..=0xFDFF |  // Arabic Presentation Forms-A
        0xFE70..=0xFEFF    // Arabic Presentation Forms-B
    )
}

/// Detect the dominant text direction from content.
///
/// Counts RTL vs LTR alphabetic characters and returns the
/// direction of the majority. Falls back to LTR when counts are
/// equal or the text contains no alphabetic characters.
pub fn detect_direction(text: &str) -> TextDirection {
    let mut rtl_count = 0u32;
    let mut ltr_count = 0u32;
    for ch in text.chars() {
        if is_rtl_char(ch) {
            rtl_count += 1;
        } else if ch.is_alphabetic() {
            ltr_count += 1;
        }
    }
    if rtl_count > ltr_count {
        TextDirection::Rtl
    } else {
        TextDirection::Ltr
    }
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

    // -- replace_unrenderable -----------------------------------------

    #[test]
    fn unrenderable_preserves_all_chars() {
        // Unknown characters should pass through (rendered as tofu □
        // by the bitmap font fallback, not replaced with '?').
        let input = "hello \u{4E00} world";
        assert_eq!(replace_unrenderable(input), input);
    }

    #[test]
    fn unrenderable_ascii_unchanged() {
        let input = "Hello, World! 123";
        assert_eq!(replace_unrenderable(input), input);
    }

    // -- whitespace collapsing ----------------------------------------

    #[test]
    fn collapse_normal_multiple_spaces() {
        let result = collapse_whitespace("hello   world", WhiteSpace::Normal);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn collapse_normal_leading_trailing() {
        // Leading/trailing whitespace is now preserved as single
        // spaces (trimmed at line boundaries, not text-node boundaries).
        let result = collapse_whitespace("  hello  ", WhiteSpace::Normal);
        assert_eq!(result, " hello ");
    }

    #[test]
    fn collapse_normal_tabs_and_newlines() {
        let result = collapse_whitespace("hello\t\n  world", WhiteSpace::Normal);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn collapse_nowrap_same_as_normal() {
        let result = collapse_whitespace("  a   b  ", WhiteSpace::NoWrap);
        assert_eq!(result, " a b ");
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
        assert!(
            words[0].leading_space,
            "leading space from collapsed leading ws"
        );
        assert_eq!(words[1].text, "world");
        assert!(
            words[1].trailing_space,
            "trailing space from collapsed trailing ws"
        );
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

    #[test]
    fn collapse_preserves_inter_element_space() {
        // " and " between inline elements should become " and "
        let result = collapse_whitespace(" and ", WhiteSpace::Normal);
        assert_eq!(result, " and ");
    }

    #[test]
    fn split_inter_element_space() {
        // " and " → one word "and" with leading_space=true, trailing_space=true
        let words = split_into_words(" and ", WhiteSpace::Normal);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "and");
        assert!(words[0].leading_space);
        assert!(words[0].trailing_space);
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

    // -- soft hyphen splitting ----------------------------------------

    #[test]
    fn split_soft_hyphen_produces_pieces() {
        let words = split_into_words("sup\u{00AD}er\u{00AD}cal", WhiteSpace::Normal);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "sup");
        assert!(words[0].soft_hyphen);
        assert_eq!(words[1].text, "er");
        assert!(words[1].soft_hyphen);
        assert_eq!(words[2].text, "cal");
        assert!(!words[2].soft_hyphen);
    }

    #[test]
    fn split_soft_hyphen_no_leading_trailing_space_between_parts() {
        let words = split_into_words("ab\u{00AD}cd ef", WhiteSpace::Normal);
        // "ab" (soft_hyphen=true), "cd" (trailing_space), "ef"
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "ab");
        assert!(words[0].soft_hyphen);
        assert!(!words[0].trailing_space);
        assert!(!words[0].leading_space);
        assert_eq!(words[1].text, "cd");
        assert!(!words[1].soft_hyphen);
        assert!(words[1].trailing_space);
        assert!(!words[1].leading_space);
        assert_eq!(words[2].text, "ef");
    }

    #[test]
    fn no_soft_hyphen_unchanged() {
        let words = split_into_words("hello world", WhiteSpace::Normal);
        assert_eq!(words.len(), 2);
        assert!(!words[0].soft_hyphen);
        assert!(!words[1].soft_hyphen);
    }

    // -- bidi detection -----------------------------------------------

    #[test]
    fn detect_direction_ltr() {
        assert_eq!(detect_direction("hello world"), TextDirection::Ltr);
    }

    #[test]
    fn detect_direction_rtl_arabic() {
        // Arabic text should be detected as RTL.
        assert_eq!(
            detect_direction("\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
            TextDirection::Rtl
        );
    }

    #[test]
    fn detect_direction_rtl_hebrew() {
        assert_eq!(
            detect_direction("\u{05E9}\u{05DC}\u{05D5}\u{05DD}"),
            TextDirection::Rtl
        );
    }

    #[test]
    fn detect_direction_mixed_majority_ltr() {
        // More Latin than Arabic characters.
        assert_eq!(
            detect_direction("hello \u{0645}\u{0631}"),
            TextDirection::Ltr,
        );
    }

    #[test]
    fn detect_direction_empty() {
        assert_eq!(detect_direction(""), TextDirection::Ltr);
    }

    #[test]
    fn is_rtl_char_arabic() {
        assert!(is_rtl_char('\u{0645}'));
        assert!(is_rtl_char('\u{0627}'));
    }

    #[test]
    fn is_rtl_char_latin() {
        assert!(!is_rtl_char('a'));
        assert!(!is_rtl_char('Z'));
    }
}

//! Syntax highlighting for shell input.
//!
//! Tokenizes a shell input line and classifies each region by syntax role
//! (command, keyword, string, variable, operator, etc.). A theme maps each
//! role to a color so the terminal can render colored spans.
//!
//! This module holds the span types, the keyword table and [`colorize`];
//! submodules hold the rest:
//!
//! - `tokenize` -- [`highlight`], the tokenizer/classifier
//! - `theme` -- [`HighlightTheme`], the kind-to-color mapping

#[cfg(test)]
mod tests;
mod theme;
mod tokenize;

pub use theme::HighlightTheme;
pub use tokenize::highlight;

use oasis_types::backend::Color;

/// Syntax element types for shell input highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// Normal text / unknown.
    Normal,
    /// Valid command name (green).
    ValidCommand,
    /// Unknown command name (red).
    UnknownCommand,
    /// Shell builtin or keyword (blue).
    /// `if`, `then`, `else`, `fi`, `while`, `for`, `do`, `done`, `function`.
    Keyword,
    /// String literal in quotes.
    StringLiteral,
    /// Variable reference (`$VAR`, `${VAR}`).
    Variable,
    /// Pipe operator (`|`).
    Pipe,
    /// Redirect operator (`>`, `>>`, `2>`, etc.).
    Redirect,
    /// Chain operator (`&&`, `||`, `;`).
    Chain,
    /// Comment (`#` to end of line).
    Comment,
    /// Glob pattern (`*`, `?`, `[...]`).
    Glob,
    /// Argument/flag (starts with `-`).
    Flag,
    /// Numeric literal.
    Number,
}

/// A highlighted region of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Byte offset of the start of this span.
    pub start: usize,
    /// Byte offset of the end of this span (exclusive).
    pub end: usize,
    /// The syntax element type for this span.
    pub kind: HighlightKind,
}

/// Shell keywords recognised by the highlighter.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "do", "done", "for", "in", "case", "esac",
    "function", "return", "break", "continue",
];

/// Check whether `word` is a shell keyword.
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// Convert highlight spans into colored text segments.
///
/// Each entry in the returned vector is a `(text, color)` pair. The
/// concatenation of all text segments reproduces the original `input`.
pub fn colorize(
    spans: &[HighlightSpan],
    input: &str,
    theme: &HighlightTheme,
) -> Vec<(String, Color)> {
    spans
        .iter()
        .map(|span| {
            let text = input.get(span.start..span.end).unwrap_or("").to_string();
            let color = theme.color_for(span.kind);
            (text, color)
        })
        .collect()
}

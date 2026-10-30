//! [`HighlightTheme`]: the color for each [`HighlightKind`].

use oasis_types::backend::Color;

use super::HighlightKind;

/// Color mapping for each highlight kind.
pub struct HighlightTheme {
    /// Color for normal / unclassified text.
    pub normal: Color,
    /// Color for valid command names.
    pub valid_command: Color,
    /// Color for unknown command names.
    pub unknown_command: Color,
    /// Color for shell keywords.
    pub keyword: Color,
    /// Color for string literals.
    pub string_literal: Color,
    /// Color for variable references.
    pub variable: Color,
    /// Color for the pipe operator.
    pub pipe: Color,
    /// Color for redirect operators.
    pub redirect: Color,
    /// Color for chain operators.
    pub chain: Color,
    /// Color for comments.
    pub comment: Color,
    /// Color for glob patterns.
    pub glob: Color,
    /// Color for flags / options.
    pub flag: Color,
    /// Color for numeric literals.
    pub number: Color,
}

impl Default for HighlightTheme {
    /// Dark theme colors suitable for a dark terminal background.
    fn default() -> Self {
        Self {
            normal: Color::rgb(204, 204, 204),
            valid_command: Color::rgb(80, 200, 80),
            unknown_command: Color::rgb(220, 60, 60),
            keyword: Color::rgb(80, 140, 220),
            string_literal: Color::rgb(200, 170, 80),
            variable: Color::rgb(180, 120, 220),
            pipe: Color::rgb(200, 200, 80),
            redirect: Color::rgb(200, 200, 80),
            chain: Color::rgb(200, 200, 80),
            comment: Color::rgb(120, 120, 120),
            glob: Color::rgb(100, 200, 200),
            flag: Color::rgb(160, 200, 240),
            number: Color::rgb(220, 140, 80),
        }
    }
}

impl HighlightTheme {
    /// Light theme colors suitable for a light terminal background.
    pub fn light() -> Self {
        Self {
            normal: Color::rgb(40, 40, 40),
            valid_command: Color::rgb(0, 128, 0),
            unknown_command: Color::rgb(180, 0, 0),
            keyword: Color::rgb(0, 0, 180),
            string_literal: Color::rgb(160, 100, 0),
            variable: Color::rgb(120, 60, 180),
            pipe: Color::rgb(140, 140, 0),
            redirect: Color::rgb(140, 140, 0),
            chain: Color::rgb(140, 140, 0),
            comment: Color::rgb(140, 140, 140),
            glob: Color::rgb(0, 140, 140),
            flag: Color::rgb(60, 100, 160),
            number: Color::rgb(180, 80, 0),
        }
    }

    /// Return the color for a given highlight kind.
    pub fn color_for(&self, kind: HighlightKind) -> Color {
        match kind {
            HighlightKind::Normal => self.normal,
            HighlightKind::ValidCommand => self.valid_command,
            HighlightKind::UnknownCommand => self.unknown_command,
            HighlightKind::Keyword => self.keyword,
            HighlightKind::StringLiteral => self.string_literal,
            HighlightKind::Variable => self.variable,
            HighlightKind::Pipe => self.pipe,
            HighlightKind::Redirect => self.redirect,
            HighlightKind::Chain => self.chain,
            HighlightKind::Comment => self.comment,
            HighlightKind::Glob => self.glob,
            HighlightKind::Flag => self.flag,
            HighlightKind::Number => self.number,
        }
    }
}

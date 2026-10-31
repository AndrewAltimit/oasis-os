//! Syntax highlighting for the text editor.
//!
//! Provides minimal keyword-based highlighting for common file types:
//! Rust, C/C++, TOML, HTML, CSS, JavaScript, Markdown, and shell scripts.
//! Each line is tokenized into colored spans for rendering.

use oasis_types::backend::Color;

/// A colored span within a single line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorSpan {
    /// Byte offset of the start of this span within the line.
    pub start: usize,
    /// Byte offset of the end of this span (exclusive).
    pub end: usize,
    /// The syntax role for this span.
    pub kind: SyntaxKind,
}

/// Syntax element classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    /// Plain text.
    Normal,
    /// Language keyword (`fn`, `let`, `if`, `for`, etc.).
    Keyword,
    /// Type name or built-in type.
    Type,
    /// String literal (quoted text).
    StringLiteral,
    /// Numeric literal.
    Number,
    /// Comment (line or block).
    Comment,
    /// Attribute, decorator, or preprocessor directive.
    Attribute,
    /// Punctuation / operator.
    Operator,
    /// HTML/XML tag name.
    Tag,
    /// HTML/XML attribute name.
    TagAttribute,
    /// TOML section header `[section]`.
    Section,
    /// Markdown heading.
    Heading,
    /// Markdown bold/italic markers.
    Emphasis,
    /// Markdown code span.
    CodeSpan,
    /// Markdown link/URL.
    Link,
}

/// File type for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Rust,
    /// C and C++ (sources and headers).
    C,
    Toml,
    Html,
    Css,
    JavaScript,
    Markdown,
    Shell,
    /// No highlighting.
    Plain,
}

/// Detect file type from a file path extension.
pub fn detect_file_type(path: &str) -> FileType {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => FileType::Rust,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => FileType::C,
        "toml" => FileType::Toml,
        "html" | "htm" => FileType::Html,
        "css" => FileType::Css,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => FileType::JavaScript,
        "md" | "markdown" => FileType::Markdown,
        "sh" | "bash" | "zsh" => FileType::Shell,
        _ => FileType::Plain,
    }
}

/// Color theme for syntax highlighting.
///
/// The editor renders with `EditorColors::from_theme`, which fills this from
/// the active skin; `Default` is only a fallback palette for hosts without a
/// theme.
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    pub normal: Color,
    pub keyword: Color,
    pub type_name: Color,
    pub string_literal: Color,
    pub number: Color,
    pub comment: Color,
    pub attribute: Color,
    pub operator: Color,
    pub tag: Color,
    pub tag_attribute: Color,
    pub section: Color,
    pub heading: Color,
    pub emphasis: Color,
    pub code_span: Color,
    pub link: Color,
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        Self {
            normal: Color::rgb(204, 204, 204),
            keyword: Color::rgb(86, 156, 214),         // blue
            type_name: Color::rgb(78, 201, 176),       // teal
            string_literal: Color::rgb(206, 145, 120), // orange-brown
            number: Color::rgb(181, 206, 168),         // light green
            comment: Color::rgb(106, 153, 85),         // green
            attribute: Color::rgb(200, 200, 100),      // yellow
            operator: Color::rgb(180, 180, 180),       // light gray
            tag: Color::rgb(86, 156, 214),             // blue
            tag_attribute: Color::rgb(156, 220, 254),  // light blue
            section: Color::rgb(220, 220, 100),        // yellow
            heading: Color::rgb(86, 156, 214),         // blue
            emphasis: Color::rgb(200, 170, 80),        // gold
            code_span: Color::rgb(206, 145, 120),      // orange-brown
            link: Color::rgb(86, 156, 214),            // blue
        }
    }
}

impl SyntaxTheme {
    /// Return the color for a given syntax kind.
    pub fn color_for(&self, kind: SyntaxKind) -> Color {
        match kind {
            SyntaxKind::Normal => self.normal,
            SyntaxKind::Keyword => self.keyword,
            SyntaxKind::Type => self.type_name,
            SyntaxKind::StringLiteral => self.string_literal,
            SyntaxKind::Number => self.number,
            SyntaxKind::Comment => self.comment,
            SyntaxKind::Attribute => self.attribute,
            SyntaxKind::Operator => self.operator,
            SyntaxKind::Tag => self.tag,
            SyntaxKind::TagAttribute => self.tag_attribute,
            SyntaxKind::Section => self.section,
            SyntaxKind::Heading => self.heading,
            SyntaxKind::Emphasis => self.emphasis,
            SyntaxKind::CodeSpan => self.code_span,
            SyntaxKind::Link => self.link,
        }
    }
}

mod c_family;
mod css;
mod html;
mod markdown;
mod scan;
mod shell;
#[cfg(test)]
mod tests;
mod toml;

use c_family::{JS_KEYWORDS, RUST_KEYWORDS, RUST_TYPES, highlight_c, highlight_c_family};
use css::highlight_css;
use html::highlight_html;
use markdown::highlight_markdown;
use shell::highlight_shell;
use toml::highlight_toml;

/// Highlight a single line of source code.
///
/// `in_block_comment` indicates whether the line starts inside a
/// block comment (for multi-line `/* ... */` in Rust/JS/CSS).
/// Returns the spans and whether a block comment is still open at
/// the end of this line.
pub fn highlight_line(
    line: &str,
    file_type: FileType,
    in_block_comment: bool,
) -> (Vec<ColorSpan>, bool) {
    match file_type {
        FileType::Rust => highlight_c_family(line, RUST_KEYWORDS, RUST_TYPES, in_block_comment),
        FileType::C => highlight_c(line, in_block_comment),
        FileType::JavaScript => highlight_c_family(line, JS_KEYWORDS, &[], in_block_comment),
        FileType::Css => highlight_css(line, in_block_comment),
        FileType::Toml => (highlight_toml(line), false),
        FileType::Html => (highlight_html(line), false),
        FileType::Markdown => (highlight_markdown(line), false),
        FileType::Shell => (highlight_shell(line), false),
        FileType::Plain => {
            if line.is_empty() {
                (Vec::new(), false)
            } else {
                (
                    vec![ColorSpan {
                        start: 0,
                        end: line.len(),
                        kind: SyntaxKind::Normal,
                    }],
                    false,
                )
            }
        },
    }
}

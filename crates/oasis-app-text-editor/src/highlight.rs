//! Syntax highlighting for the text editor.
//!
//! Provides minimal keyword-based highlighting for common file types:
//! Rust, TOML, HTML, CSS, JavaScript, Markdown, and shell scripts.
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

// -----------------------------------------------------------------------
// Rust keywords and types
// -----------------------------------------------------------------------

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    "HashMap", "HashSet", "BTreeMap", "BTreeSet",
];

// -----------------------------------------------------------------------
// JavaScript keywords
// -----------------------------------------------------------------------

const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

// -----------------------------------------------------------------------
// Shell keywords
// -----------------------------------------------------------------------

const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "do", "done", "for", "in", "case", "esac",
    "function", "return", "break", "continue", "export", "source", "local", "readonly", "declare",
    "set", "unset", "shift", "exit", "exec", "eval", "trap",
];

// -----------------------------------------------------------------------
// CSS keywords
// -----------------------------------------------------------------------

const CSS_AT_RULES: &[&str] = &[
    "@media",
    "@import",
    "@keyframes",
    "@font-face",
    "@charset",
    "@supports",
    "@namespace",
    "@page",
    "@layer",
];

// -----------------------------------------------------------------------
// Highlighting entry point
// -----------------------------------------------------------------------

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

// -----------------------------------------------------------------------
// Shared scanning helpers
// -----------------------------------------------------------------------

/// Scan a quoted string starting at `pos` (which must point to the opening
/// quote character). Handles backslash escapes when `with_escapes` is true.
/// Returns the position after the closing quote (or end of bytes).
fn scan_quoted(bytes: &[u8], pos: usize, quote: u8, with_escapes: bool) -> usize {
    let len = bytes.len();
    let mut p = pos + 1;
    while p < len && bytes[p] != quote {
        if with_escapes && bytes[p] == b'\\' && p + 1 < len {
            p += 2;
        } else {
            p += 1;
        }
    }
    if p < len {
        p + 1 // skip closing quote
    } else {
        p
    }
}

/// Scan a `/* ... */` block comment. `pos` should point to the first byte
/// to scan (either the `/*` opener or continuation from previous line).
/// When `include_opener` is true, `pos` points at `/*` and we skip 2 bytes.
/// Returns `(end_pos, still_in_comment)`.
fn scan_block_comment(bytes: &[u8], mut pos: usize, include_opener: bool) -> (usize, bool) {
    let len = bytes.len();
    if include_opener {
        pos += 2;
    }
    loop {
        if pos + 1 < len && bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
            return (pos + 2, false);
        }
        pos += 1;
        if pos >= len {
            return (len, true);
        }
    }
}

/// Scan contiguous ASCII whitespace starting at `pos`.
/// Returns the position after the last whitespace byte.
fn scan_whitespace(bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    while pos < len && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

/// Scan a C-family numeric literal (decimal, hex, float with type suffix).
/// Returns the position after the number.
fn scan_c_number(bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    // Hex: 0x...
    if bytes[pos] == b'0' && pos + 1 < len && (bytes[pos + 1] == b'x' || bytes[pos + 1] == b'X') {
        pos += 2;
        while pos < len && (bytes[pos].is_ascii_hexdigit() || bytes[pos] == b'_') {
            pos += 1;
        }
    } else {
        while pos < len
            && (bytes[pos].is_ascii_digit()
                || bytes[pos] == b'.'
                || bytes[pos] == b'_'
                || bytes[pos] == b'e'
                || bytes[pos] == b'E')
        {
            pos += 1;
        }
    }
    // Type suffix (e.g., `42u32`, `1.0f64`).
    while pos < len && bytes[pos].is_ascii_alphanumeric() {
        pos += 1;
    }
    pos
}

/// Scan an identifier/word. Returns the position after the word.
/// `extra_chars` lists additional bytes (beyond alphanumeric/underscore)
/// that are part of a word (e.g., `b'-'` for shell).
fn scan_word(bytes: &[u8], mut pos: usize, extra_chars: &[u8]) -> usize {
    let len = bytes.len();
    while pos < len
        && (bytes[pos].is_ascii_alphanumeric()
            || bytes[pos] == b'_'
            || extra_chars.contains(&bytes[pos]))
    {
        pos += 1;
    }
    pos
}

/// Advance past a single UTF-8 character, returning the position after it.
fn advance_utf8_char(line: &str, pos: usize) -> usize {
    pos + line[pos..].chars().next().map_or(1, |c| c.len_utf8())
}

fn is_operator(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'!'
            | b'<'
            | b'>'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b';'
            | b':'
            | b','
            | b'.'
            | b'?'
            | b'@'
    )
}

// -----------------------------------------------------------------------
// C-family highlighter (Rust, JavaScript)
// -----------------------------------------------------------------------

fn highlight_c_family(
    line: &str,
    keywords: &[&str],
    types: &[&str],
    mut in_block_comment: bool,
) -> (Vec<ColorSpan>, bool) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return (Vec::new(), in_block_comment);
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Inside a block comment: scan for `*/`.
        if in_block_comment {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, false);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Line comment: `//` to end of line.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            spans.push(ColorSpan {
                start: pos,
                end: len,
                kind: SyntaxKind::Comment,
            });
            pos = len;
            continue;
        }

        // Block comment start: `/*`.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, true);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Attribute: `#[...]` or `#![...]` (Rust).
        if bytes[pos] == b'#'
            && pos + 1 < len
            && (bytes[pos + 1] == b'['
                || (bytes[pos + 1] == b'!' && pos + 2 < len && bytes[pos + 2] == b'['))
        {
            let start = pos;
            // Find matching `]`.
            let mut depth = 0;
            while pos < len {
                if bytes[pos] == b'[' {
                    depth += 1;
                } else if bytes[pos] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        pos += 1;
                        break;
                    }
                }
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Attribute,
            });
            continue;
        }

        // String literals: double-quoted, single-quoted, backtick (template).
        if matches!(bytes[pos], b'"' | b'\'' | b'`') {
            let start = pos;
            pos = scan_quoted(bytes, pos, bytes[pos], true);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Numeric literal.
        if bytes[pos].is_ascii_digit()
            || (bytes[pos] == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            let start = pos;
            pos = scan_c_number(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Number,
            });
            continue;
        }

        // Whitespace.
        if bytes[pos].is_ascii_whitespace() {
            let start = pos;
            pos = scan_whitespace(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
            continue;
        }

        // Operator / punctuation.
        if is_operator(bytes[pos]) {
            let start = pos;
            while pos < len && is_operator(bytes[pos]) {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Operator,
            });
            continue;
        }

        // Word (identifier / keyword).
        if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' {
            let start = pos;
            pos = scan_word(bytes, pos, &[]);
            let word = &line[start..pos];
            let kind = if keywords.contains(&word) {
                SyntaxKind::Keyword
            } else if types.contains(&word) {
                SyntaxKind::Type
            } else {
                SyntaxKind::Normal
            };
            spans.push(ColorSpan {
                start,
                end: pos,
                kind,
            });
            continue;
        }

        // Fallback: single character (advance by full UTF-8 char width).
        let end = advance_utf8_char(line, pos);
        spans.push(ColorSpan {
            start: pos,
            end,
            kind: SyntaxKind::Normal,
        });
        pos = end;
    }

    (spans, in_block_comment)
}

// -----------------------------------------------------------------------
// TOML highlighter
// -----------------------------------------------------------------------

fn highlight_toml(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = line.trim_start();
    let leading_ws = len - trimmed.len();

    // Comment line.
    if trimmed.starts_with('#') {
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(ColorSpan {
                start: 0,
                end: leading_ws,
                kind: SyntaxKind::Normal,
            });
        }
        spans.push(ColorSpan {
            start: leading_ws,
            end: len,
            kind: SyntaxKind::Comment,
        });
        return spans;
    }

    // Section header: `[section]` or `[[array]]`.
    if trimmed.starts_with('[') {
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(ColorSpan {
                start: 0,
                end: leading_ws,
                kind: SyntaxKind::Normal,
            });
        }
        spans.push(ColorSpan {
            start: leading_ws,
            end: len,
            kind: SyntaxKind::Section,
        });
        return spans;
    }

    // Key = value line.
    if let Some(eq_pos) = line.find('=') {
        let mut spans = Vec::new();
        // Key part (before `=`).
        spans.push(ColorSpan {
            start: 0,
            end: eq_pos,
            kind: SyntaxKind::Keyword,
        });
        // `=` sign.
        spans.push(ColorSpan {
            start: eq_pos,
            end: eq_pos + 1,
            kind: SyntaxKind::Operator,
        });
        // Value part (after `=`).
        let rest = &line[eq_pos + 1..];
        let rest_trimmed = rest.trim_start();
        let val_start = eq_pos + 1 + (rest.len() - rest_trimmed.len());
        if val_start < len {
            // Whitespace between = and value.
            if val_start > eq_pos + 1 {
                spans.push(ColorSpan {
                    start: eq_pos + 1,
                    end: val_start,
                    kind: SyntaxKind::Normal,
                });
            }
            let kind = if rest_trimmed.starts_with('"') || rest_trimmed.starts_with('\'') {
                SyntaxKind::StringLiteral
            } else if rest_trimmed == "true" || rest_trimmed == "false" {
                SyntaxKind::Keyword
            } else if rest_trimmed
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
            {
                SyntaxKind::Number
            } else {
                SyntaxKind::Normal
            };
            spans.push(ColorSpan {
                start: val_start,
                end: len,
                kind,
            });
        }
        return spans;
    }

    vec![ColorSpan {
        start: 0,
        end: len,
        kind: SyntaxKind::Normal,
    }]
}

// -----------------------------------------------------------------------
// HTML highlighter
// -----------------------------------------------------------------------

fn highlight_html(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // HTML comment: `<!-- ... -->`.
        if pos + 3 < len && &line[pos..pos + 4] == "<!--" {
            let start = pos;
            if let Some(end_pos) = line[pos..].find("-->") {
                pos += end_pos + 3;
            } else {
                pos = len;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Tag: `<...>`.
        if bytes[pos] == b'<' {
            let start = pos;
            pos += 1;
            // Skip `/` for closing tags.
            if pos < len && bytes[pos] == b'/' {
                pos += 1;
            }
            // Tag name.
            let tag_name_start = pos;
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'-') {
                pos += 1;
            }
            if pos > tag_name_start {
                // Opening bracket + optional slash + tag name.
                spans.push(ColorSpan {
                    start,
                    end: pos,
                    kind: SyntaxKind::Tag,
                });
            } else {
                spans.push(ColorSpan {
                    start,
                    end: pos.max(start + 1),
                    kind: SyntaxKind::Operator,
                });
            }

            // Attributes within the tag until `>`.
            while pos < len && bytes[pos] != b'>' {
                if bytes[pos].is_ascii_whitespace() {
                    let ws_start = pos;
                    while pos < len && bytes[pos].is_ascii_whitespace() {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: ws_start,
                        end: pos,
                        kind: SyntaxKind::Normal,
                    });
                } else if bytes[pos] == b'=' {
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + 1,
                        kind: SyntaxKind::Operator,
                    });
                    pos += 1;
                } else if bytes[pos] == b'"' || bytes[pos] == b'\'' {
                    let q = bytes[pos];
                    let str_start = pos;
                    pos += 1;
                    while pos < len && bytes[pos] != q {
                        pos += 1;
                    }
                    if pos < len {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: str_start,
                        end: pos,
                        kind: SyntaxKind::StringLiteral,
                    });
                } else if bytes[pos] == b'/' {
                    // Self-closing slash.
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + 1,
                        kind: SyntaxKind::Tag,
                    });
                    pos += 1;
                } else if bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'-'
                    || bytes[pos] == b'_'
                {
                    let attr_start = pos;
                    while pos < len
                        && (bytes[pos].is_ascii_alphanumeric()
                            || bytes[pos] == b'-'
                            || bytes[pos] == b'_')
                    {
                        pos += 1;
                    }
                    spans.push(ColorSpan {
                        start: attr_start,
                        end: pos,
                        kind: SyntaxKind::TagAttribute,
                    });
                } else {
                    spans.push(ColorSpan {
                        start: pos,
                        end: pos + 1,
                        kind: SyntaxKind::Normal,
                    });
                    pos += 1;
                }
            }
            // Closing `>`.
            if pos < len && bytes[pos] == b'>' {
                spans.push(ColorSpan {
                    start: pos,
                    end: pos + 1,
                    kind: SyntaxKind::Tag,
                });
                pos += 1;
            }
            continue;
        }

        // Plain text between tags.
        let start = pos;
        while pos < len && bytes[pos] != b'<' {
            pos += 1;
        }
        if pos > start {
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
        }
    }

    spans
}

// -----------------------------------------------------------------------
// CSS highlighter
// -----------------------------------------------------------------------

fn highlight_css(line: &str, mut in_block_comment: bool) -> (Vec<ColorSpan>, bool) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return (Vec::new(), in_block_comment);
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        if in_block_comment {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, false);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // Block comment.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            let start = pos;
            let (end, still_open) = scan_block_comment(bytes, pos, true);
            in_block_comment = still_open;
            pos = end;
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Comment,
            });
            continue;
        }

        // At-rule.
        if bytes[pos] == b'@' {
            let start = pos;
            pos += 1;
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'-') {
                pos += 1;
            }
            // All @-rules are highlighted as keywords.
            let _ = CSS_AT_RULES; // keyword list available for future refinement
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Keyword,
            });
            continue;
        }

        // String.
        if matches!(bytes[pos], b'"' | b'\'') {
            let start = pos;
            pos = scan_quoted(bytes, pos, bytes[pos], true);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Number (including units like `10px`).
        if bytes[pos].is_ascii_digit()
            || (bytes[pos] == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            let start = pos;
            while pos < len && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
                pos += 1;
            }
            // CSS units.
            while pos < len && bytes[pos].is_ascii_alphabetic() {
                pos += 1;
            }
            // Percent.
            if pos < len && bytes[pos] == b'%' {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Number,
            });
            continue;
        }

        // Braces / operators.
        if matches!(bytes[pos], b'{' | b'}' | b':' | b';' | b',' | b'(' | b')') {
            spans.push(ColorSpan {
                start: pos,
                end: pos + 1,
                kind: SyntaxKind::Operator,
            });
            pos += 1;
            continue;
        }

        // Whitespace.
        if bytes[pos].is_ascii_whitespace() {
            let start = pos;
            pos = scan_whitespace(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
            continue;
        }

        // Selector / property / value word.
        let start = pos;
        while pos < len
            && !bytes[pos].is_ascii_whitespace()
            && !matches!(
                bytes[pos],
                b'{' | b'}' | b':' | b';' | b',' | b'(' | b')' | b'"' | b'\'' | b'/'
            )
        {
            pos += 1;
        }
        if pos > start {
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
        }
    }

    (spans, in_block_comment)
}

// -----------------------------------------------------------------------
// Markdown highlighter
// -----------------------------------------------------------------------

fn highlight_markdown(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = line.trim_start();

    // Heading: lines starting with `#`.
    if trimmed.starts_with('#') {
        return vec![ColorSpan {
            start: 0,
            end: len,
            kind: SyntaxKind::Heading,
        }];
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Inline code: `...`.
        if bytes[pos] == b'`' {
            let start = pos;
            pos += 1;
            while pos < len && bytes[pos] != b'`' {
                pos += 1;
            }
            if pos < len {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::CodeSpan,
            });
            continue;
        }

        // Bold/italic markers: `**`, `__`, `*`, `_`.
        if bytes[pos] == b'*' || bytes[pos] == b'_' {
            let marker = bytes[pos];
            let start = pos;
            while pos < len && bytes[pos] == marker {
                pos += 1;
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Emphasis,
            });
            continue;
        }

        // Link: `[text](url)`.
        if bytes[pos] == b'[' {
            let start = pos;
            pos += 1;
            while pos < len && bytes[pos] != b']' {
                pos += 1;
            }
            if pos < len {
                pos += 1; // `]`
            }
            if pos < len && bytes[pos] == b'(' {
                pos += 1;
                while pos < len && bytes[pos] != b')' {
                    pos += 1;
                }
                if pos < len {
                    pos += 1;
                }
            }
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Link,
            });
            continue;
        }

        // Regular text.
        let start = pos;
        while pos < len && !matches!(bytes[pos], b'`' | b'*' | b'_' | b'[') {
            pos += 1;
        }
        if pos > start {
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
        }
    }

    spans
}

// -----------------------------------------------------------------------
// Shell highlighter
// -----------------------------------------------------------------------

/// Scan a shell variable reference: `$VAR` or `${VAR}`.
/// `pos` must point at the `$`. Returns the position after the variable.
fn scan_shell_variable(bytes: &[u8], pos: usize) -> usize {
    let len = bytes.len();
    let mut p = pos + 1;
    if p < len && bytes[p] == b'{' {
        p += 1;
        while p < len && bytes[p] != b'}' {
            p += 1;
        }
        if p < len {
            p += 1;
        }
    } else {
        while p < len && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_') {
            p += 1;
        }
    }
    p
}

/// Scan a shell operator (|, ;, &&, ||, >, <, >>).
/// Returns the position after the operator.
fn scan_shell_operator(bytes: &[u8], pos: usize) -> usize {
    let len = bytes.len();
    let b = bytes[pos];
    let mut p = pos + 1;
    if p < len
        && ((b == b'&' && bytes[p] == b'&')
            || (b == b'|' && bytes[p] == b'|')
            || (b == b'>' && bytes[p] == b'>'))
    {
        p += 1;
    }
    p
}

fn highlight_shell(line: &str) -> Vec<ColorSpan> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return Vec::new();
    }

    let mut spans: Vec<ColorSpan> = Vec::new();
    let mut pos: usize = 0;

    while pos < len {
        // Comment.
        if bytes[pos] == b'#' {
            spans.push(ColorSpan {
                start: pos,
                end: len,
                kind: SyntaxKind::Comment,
            });
            pos = len;
            continue;
        }

        // String: double-quoted (with escapes), single-quoted (no escapes).
        if matches!(bytes[pos], b'"' | b'\'') {
            let start = pos;
            let with_escapes = bytes[pos] == b'"';
            pos = scan_quoted(bytes, pos, bytes[pos], with_escapes);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::StringLiteral,
            });
            continue;
        }

        // Variable: $VAR, ${VAR}.
        if bytes[pos] == b'$' && pos + 1 < len {
            let start = pos;
            pos = scan_shell_variable(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Attribute,
            });
            continue;
        }

        // Whitespace.
        if bytes[pos].is_ascii_whitespace() {
            let start = pos;
            pos = scan_whitespace(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Normal,
            });
            continue;
        }

        // Operators: |, ;, &&, ||, >, <, >>.
        if matches!(bytes[pos], b'|' | b';' | b'&' | b'>' | b'<') {
            let start = pos;
            pos = scan_shell_operator(bytes, pos);
            spans.push(ColorSpan {
                start,
                end: pos,
                kind: SyntaxKind::Operator,
            });
            continue;
        }

        // Word (identifier / keyword).
        if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' || bytes[pos] == b'-' {
            let start = pos;
            pos = scan_word(bytes, pos, b"-");
            let word = &line[start..pos];
            let kind = if SHELL_KEYWORDS.contains(&word) {
                SyntaxKind::Keyword
            } else {
                SyntaxKind::Normal
            };
            spans.push(ColorSpan {
                start,
                end: pos,
                kind,
            });
            continue;
        }

        // Fallback: single character (advance by full UTF-8 char width).
        let end = advance_utf8_char(line, pos);
        spans.push(ColorSpan {
            start: pos,
            end,
            kind: SyntaxKind::Normal,
        });
        pos = end;
    }

    spans
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- File type detection --

    #[test]
    fn detect_rust() {
        assert_eq!(detect_file_type("/foo/bar.rs"), FileType::Rust);
    }

    #[test]
    fn detect_toml() {
        assert_eq!(detect_file_type("Cargo.toml"), FileType::Toml);
    }

    #[test]
    fn detect_html() {
        assert_eq!(detect_file_type("index.html"), FileType::Html);
        assert_eq!(detect_file_type("page.htm"), FileType::Html);
    }

    #[test]
    fn detect_css() {
        assert_eq!(detect_file_type("style.css"), FileType::Css);
    }

    #[test]
    fn detect_javascript() {
        assert_eq!(detect_file_type("app.js"), FileType::JavaScript);
        assert_eq!(detect_file_type("module.ts"), FileType::JavaScript);
        assert_eq!(detect_file_type("component.tsx"), FileType::JavaScript);
    }

    #[test]
    fn detect_markdown() {
        assert_eq!(detect_file_type("README.md"), FileType::Markdown);
    }

    #[test]
    fn detect_shell() {
        assert_eq!(detect_file_type("build.sh"), FileType::Shell);
        assert_eq!(detect_file_type("init.bash"), FileType::Shell);
    }

    #[test]
    fn detect_plain() {
        assert_eq!(detect_file_type("notes.txt"), FileType::Plain);
        assert_eq!(detect_file_type("data"), FileType::Plain);
    }

    #[test]
    fn detect_case_insensitive_extension() {
        assert_eq!(detect_file_type("FILE.RS"), FileType::Rust);
        assert_eq!(detect_file_type("PAGE.HTML"), FileType::Html);
    }

    // -- Plain highlighting --

    #[test]
    fn plain_returns_single_span() {
        let (spans, _) = highlight_line("hello world", FileType::Plain, false);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Normal);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 11);
    }

    // -- Rust highlighting --

    #[test]
    fn rust_keyword() {
        let (spans, _) = highlight_line("fn main() {", FileType::Rust, false);
        assert_eq!(spans[0].kind, SyntaxKind::Keyword);
        assert_eq!(&"fn main() {"[spans[0].start..spans[0].end], "fn");
    }

    #[test]
    fn rust_type() {
        let (spans, _) = highlight_line("let x: u32 = 42;", FileType::Rust, false);
        // Find the u32 span.
        let u32_span = spans
            .iter()
            .find(|s| &"let x: u32 = 42;"[s.start..s.end] == "u32");
        assert!(u32_span.is_some());
        assert_eq!(u32_span.unwrap().kind, SyntaxKind::Type);
    }

    #[test]
    fn rust_string() {
        let (spans, _) = highlight_line("let s = \"hello\";", FileType::Rust, false);
        let str_span = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
        assert!(str_span.is_some());
        assert_eq!(
            &"let s = \"hello\";"[str_span.unwrap().start..str_span.unwrap().end],
            "\"hello\""
        );
    }

    #[test]
    fn rust_number() {
        let (spans, _) = highlight_line("let x = 42;", FileType::Rust, false);
        let num = spans.iter().find(|s| s.kind == SyntaxKind::Number);
        assert!(num.is_some());
        assert_eq!(&"let x = 42;"[num.unwrap().start..num.unwrap().end], "42");
    }

    #[test]
    fn rust_line_comment() {
        let (spans, _) = highlight_line("// this is a comment", FileType::Rust, false);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Comment);
    }

    #[test]
    fn rust_block_comment_single_line() {
        let (spans, in_comment) = highlight_line("x /* comment */ y", FileType::Rust, false);
        assert!(!in_comment);
        let comment = spans.iter().find(|s| s.kind == SyntaxKind::Comment);
        assert!(comment.is_some());
    }

    #[test]
    fn rust_block_comment_spans_lines() {
        let (spans1, in_comment) = highlight_line("x /* start", FileType::Rust, false);
        assert!(in_comment);
        let comment1 = spans1.iter().find(|s| s.kind == SyntaxKind::Comment);
        assert!(comment1.is_some());

        let (spans2, in_comment) = highlight_line("still comment */ y", FileType::Rust, true);
        assert!(!in_comment);
        assert_eq!(spans2[0].kind, SyntaxKind::Comment);
    }

    #[test]
    fn rust_attribute() {
        let (spans, _) = highlight_line("#[derive(Debug)]", FileType::Rust, false);
        assert_eq!(spans[0].kind, SyntaxKind::Attribute);
    }

    // -- TOML highlighting --

    #[test]
    fn toml_section() {
        let (spans, _) = highlight_line("[package]", FileType::Toml, false);
        assert_eq!(spans[0].kind, SyntaxKind::Section);
    }

    #[test]
    fn toml_comment() {
        let (spans, _) = highlight_line("# comment", FileType::Toml, false);
        assert_eq!(spans[0].kind, SyntaxKind::Comment);
    }

    #[test]
    fn toml_key_value() {
        let (spans, _) = highlight_line("name = \"oasis\"", FileType::Toml, false);
        assert_eq!(spans[0].kind, SyntaxKind::Keyword); // key
        assert_eq!(spans[1].kind, SyntaxKind::Operator); // =
    }

    // -- HTML highlighting --

    #[test]
    fn html_tag() {
        let (spans, _) = highlight_line("<div class=\"main\">", FileType::Html, false);
        assert_eq!(spans[0].kind, SyntaxKind::Tag); // <div
        let attr = spans.iter().find(|s| s.kind == SyntaxKind::TagAttribute);
        assert!(attr.is_some());
        let str_span = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
        assert!(str_span.is_some());
    }

    // -- Markdown highlighting --

    #[test]
    fn markdown_heading() {
        let (spans, _) = highlight_line("# Hello", FileType::Markdown, false);
        assert_eq!(spans[0].kind, SyntaxKind::Heading);
    }

    #[test]
    fn markdown_code_span() {
        let (spans, _) = highlight_line("use `code` here", FileType::Markdown, false);
        let code = spans.iter().find(|s| s.kind == SyntaxKind::CodeSpan);
        assert!(code.is_some());
    }

    // -- Shell highlighting --

    #[test]
    fn shell_keyword() {
        let (spans, _) = highlight_line("if true; then", FileType::Shell, false);
        assert_eq!(spans[0].kind, SyntaxKind::Keyword);
        assert_eq!(&"if true; then"[spans[0].start..spans[0].end], "if");
    }

    #[test]
    fn shell_variable() {
        let (spans, _) = highlight_line("echo $HOME", FileType::Shell, false);
        let var = spans.iter().find(|s| s.kind == SyntaxKind::Attribute);
        assert!(var.is_some());
    }

    #[test]
    fn shell_comment() {
        let (spans, _) = highlight_line("# comment", FileType::Shell, false);
        assert_eq!(spans[0].kind, SyntaxKind::Comment);
    }

    // -- JavaScript highlighting --

    #[test]
    fn js_keyword() {
        let (spans, _) = highlight_line("const x = 42;", FileType::JavaScript, false);
        assert_eq!(spans[0].kind, SyntaxKind::Keyword);
        assert_eq!(&"const x = 42;"[spans[0].start..spans[0].end], "const");
    }

    #[test]
    fn js_template_literal() {
        let (spans, _) = highlight_line("let s = `hello`;", FileType::JavaScript, false);
        let tpl = spans.iter().find(|s| s.kind == SyntaxKind::StringLiteral);
        assert!(tpl.is_some());
    }

    // -- SyntaxTheme --

    #[test]
    fn theme_color_for_returns_correct_color() {
        let theme = SyntaxTheme::default();
        assert_eq!(theme.color_for(SyntaxKind::Keyword), theme.keyword);
        assert_eq!(theme.color_for(SyntaxKind::Normal), theme.normal);
        assert_eq!(theme.color_for(SyntaxKind::Comment), theme.comment);
    }

    // -- Empty lines --

    #[test]
    fn empty_line_returns_empty_spans() {
        for ft in [
            FileType::Rust,
            FileType::Toml,
            FileType::Html,
            FileType::Css,
            FileType::JavaScript,
            FileType::Markdown,
            FileType::Shell,
            FileType::Plain,
        ] {
            let (spans, _) = highlight_line("", ft, false);
            assert!(spans.is_empty(), "non-empty spans for {ft:?}");
        }
    }

    // -- Spans cover full line --

    #[test]
    fn rust_spans_cover_full_line() {
        let line = "fn main() { let x: u32 = 42; }";
        let (spans, _) = highlight_line(line, FileType::Rust, false);
        assert_spans_contiguous(&spans, line.len());
    }

    #[test]
    fn toml_spans_cover_full_line() {
        let line = "name = \"oasis\"";
        let (spans, _) = highlight_line(line, FileType::Toml, false);
        assert_spans_contiguous(&spans, line.len());
    }

    #[test]
    fn html_spans_cover_full_line() {
        let line = "<div class=\"x\">hello</div>";
        let (spans, _) = highlight_line(line, FileType::Html, false);
        assert_spans_contiguous(&spans, line.len());
    }

    fn assert_spans_contiguous(spans: &[ColorSpan], expected_len: usize) {
        if expected_len == 0 {
            assert!(spans.is_empty());
            return;
        }
        let mut expected_start = 0;
        for span in spans {
            assert_eq!(
                span.start, expected_start,
                "gap before span at {}",
                span.start
            );
            assert!(span.end > span.start, "empty span at {}", span.start);
            expected_start = span.end;
        }
        assert_eq!(expected_start, expected_len, "spans don't cover full line");
    }
}

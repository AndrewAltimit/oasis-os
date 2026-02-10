//! CSS tokenizer.
//!
//! Consumes a CSS source string and produces a flat token stream. Handles
//! comments, quoted strings, numbers with units/percentages, hash tokens,
//! at-keywords, function notation, and whitespace coalescing.

/// A single CSS token.
#[derive(Debug, Clone, PartialEq)]
pub enum CssToken {
    /// Keyword or property name.
    Ident(String),
    /// `#id` or `#color`.
    Hash(String),
    /// Quoted string (`"..."` or `'...'`).
    String(String),
    /// Bare number: `42`, `3.14`.
    Number(f32),
    /// Percentage: `50%`.
    Percentage(f32),
    /// Number with unit: `10px`, `1.5em`, `2rem`.
    Dimension(f32, String),
    /// `:`.
    Colon,
    /// `;`.
    Semicolon,
    /// `,`.
    Comma,
    /// `{`.
    OpenBrace,
    /// `}`.
    CloseBrace,
    /// `(`.
    OpenParen,
    /// `)`.
    CloseParen,
    /// `[`.
    OpenBracket,
    /// `]`.
    CloseBracket,
    /// `.`.
    Dot,
    /// `>`.
    Greater,
    /// `+`.
    Plus,
    /// `*`.
    Star,
    /// `/`.
    Slash,
    /// Any other single character.
    Delim(char),
    /// Coalesced whitespace (spaces, tabs, newlines).
    Whitespace,
    /// At-keyword: `@import`, `@media`, etc.
    AtKeyword(String),
    /// Function name immediately followed by `(`: `rgb(`, `url(`.
    Function(String),
    /// End-of-file sentinel.
    Eof,
}

/// A streaming CSS tokenizer.
pub struct CssTokenizer {
    input: Vec<char>,
    pos: usize,
}

impl CssTokenizer {
    /// Create a new tokenizer from the given CSS source text.
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
        }
    }

    /// Consume the entire input and return a `Vec` of tokens (including
    /// a trailing `Eof`).
    pub fn tokenize(&mut self) -> Vec<CssToken> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok == CssToken::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn skip_whitespace_chars(&mut self) -> bool {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
        self.pos > start
    }

    fn skip_comment(&mut self) -> bool {
        if self.peek() == Some('/') && self.peek_at(1) == Some('*') {
            self.pos += 2;
            while !self.is_eof() {
                if self.peek() == Some('*') && self.peek_at(1) == Some('/') {
                    self.pos += 2;
                    return true;
                }
                self.advance();
            }
            // Unterminated comment -- still consumed.
            return true;
        }
        false
    }

    fn is_ident_start(ch: char) -> bool {
        ch.is_ascii_alphabetic() || ch == '_' || ch == '-'
    }

    fn is_ident_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
    }

    fn consume_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if Self::is_ident_char(ch) {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    fn consume_number(&mut self) -> f32 {
        let mut s = String::new();
        // Optional leading minus already handled by caller if needed.
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        // Decimal part.
        if self.peek() == Some('.')
            && let Some(next) = self.peek_at(1)
            && next.is_ascii_digit()
        {
            s.push('.');
            self.advance(); // '.'
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    s.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        s.parse::<f32>().unwrap_or(0.0)
    }

    fn consume_string(&mut self, quote: char) -> String {
        self.advance(); // skip opening quote
        let mut s = String::new();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                if let Some(escaped) = self.advance() {
                    match escaped {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        other => s.push(other),
                    }
                }
            } else if ch == quote {
                break;
            } else {
                s.push(ch);
            }
        }
        s
    }

    fn starts_number(&self) -> bool {
        match self.peek() {
            Some(ch) if ch.is_ascii_digit() => true,
            Some('.') => matches!(
                self.peek_at(1),
                Some(d) if d.is_ascii_digit()
            ),
            _ => false,
        }
    }

    // ---------------------------------------------------------------
    // Main tokenisation loop
    // ---------------------------------------------------------------

    fn next_token(&mut self) -> CssToken {
        // Consume whitespace and comments, emitting a single Whitespace
        // token when any whitespace was encountered.
        let mut saw_ws = false;
        loop {
            if self.skip_whitespace_chars() {
                saw_ws = true;
                continue;
            }
            if self.skip_comment() {
                // Comments act as whitespace.
                saw_ws = true;
                continue;
            }
            break;
        }
        if saw_ws && !self.is_eof() {
            return CssToken::Whitespace;
        }

        if self.is_eof() {
            return CssToken::Eof;
        }

        let ch = self.peek().expect("not eof");

        // Quoted strings.
        if ch == '"' || ch == '\'' {
            return CssToken::String(self.consume_string(ch));
        }

        // Numbers (and things that start with a digit or `.digit`).
        if self.starts_number() {
            return self.tokenize_numeric();
        }

        // Hash token.
        if ch == '#' {
            self.advance();
            let name = self.consume_hash_name();
            return CssToken::Hash(name);
        }

        // At-keyword.
        if ch == '@' {
            self.advance();
            let name = self.consume_ident();
            return CssToken::AtKeyword(name);
        }

        // Ident or function.
        if Self::is_ident_start(ch) {
            let ident = self.consume_ident();
            if self.peek() == Some('(') {
                self.advance(); // consume '('
                return CssToken::Function(ident);
            }
            return CssToken::Ident(ident);
        }

        // Single-character tokens.
        self.advance();
        match ch {
            ':' => CssToken::Colon,
            ';' => CssToken::Semicolon,
            ',' => CssToken::Comma,
            '{' => CssToken::OpenBrace,
            '}' => CssToken::CloseBrace,
            '(' => CssToken::OpenParen,
            ')' => CssToken::CloseParen,
            '[' => CssToken::OpenBracket,
            ']' => CssToken::CloseBracket,
            '.' => {
                // Could be start of a number like `.5`.
                if let Some(d) = self.peek()
                    && d.is_ascii_digit()
                {
                    // Put the dot back conceptually and parse number.
                    self.pos -= 1;
                    return self.tokenize_numeric();
                }
                CssToken::Dot
            },
            '>' => CssToken::Greater,
            '+' => CssToken::Plus,
            '*' => CssToken::Star,
            '/' => CssToken::Slash,
            other => CssToken::Delim(other),
        }
    }

    /// Consume a numeric token (Number, Percentage, or Dimension).
    fn tokenize_numeric(&mut self) -> CssToken {
        let value = self.consume_number();
        if self.peek() == Some('%') {
            self.advance();
            return CssToken::Percentage(value);
        }
        if let Some(ch) = self.peek()
            && Self::is_ident_start(ch)
        {
            let unit = self.consume_ident();
            return CssToken::Dimension(value, unit);
        }
        CssToken::Number(value)
    }

    /// Consume the name portion of a hash token (allows digits at start,
    /// unlike normal idents).
    fn consume_hash_name(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<CssToken> {
        CssTokenizer::new(input).tokenize()
    }

    #[test]
    fn simple_property() {
        let tokens = tokenize("color: red;");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("color".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::Ident("red".into()),
                CssToken::Semicolon,
                CssToken::Eof,
            ]
        );
    }

    #[test]
    fn multiple_declarations() {
        let tokens = tokenize("color: red; margin: 10px;");
        // Spot-check key tokens.
        assert!(tokens.contains(&CssToken::Ident("color".into())));
        assert!(tokens.contains(&CssToken::Ident("margin".into())));
        assert!(tokens.contains(&CssToken::Dimension(10.0, "px".into())));
    }

    #[test]
    fn comments() {
        let tokens = tokenize("/* hello */ color: red;");
        // Comment becomes whitespace; first real token is `color`.
        assert_eq!(tokens[0], CssToken::Whitespace);
        assert_eq!(tokens[1], CssToken::Ident("color".into()));
    }

    #[test]
    fn strings_with_escapes() {
        let tokens = tokenize(r#""hello \"world\"""#);
        assert_eq!(tokens[0], CssToken::String(r#"hello "world""#.into()));
    }

    #[test]
    fn numbers_with_units() {
        let tokens = tokenize("10px 1.5em 50% 42");
        assert!(tokens.contains(&CssToken::Dimension(10.0, "px".into())));
        assert!(tokens.contains(&CssToken::Dimension(1.5, "em".into())));
        assert!(tokens.contains(&CssToken::Percentage(50.0)));
        assert!(tokens.contains(&CssToken::Number(42.0)));
    }

    #[test]
    fn hash_colors() {
        let tokens = tokenize("#fff #333333 #header");
        assert!(tokens.contains(&CssToken::Hash("fff".into())));
        assert!(tokens.contains(&CssToken::Hash("333333".into())));
        assert!(tokens.contains(&CssToken::Hash("header".into())));
    }

    #[test]
    fn at_keyword() {
        let tokens = tokenize("@import url('a.css');");
        assert_eq!(tokens[0], CssToken::AtKeyword("import".into()));
        assert_eq!(tokens[2], CssToken::Function("url".into()));
    }

    #[test]
    fn function_token() {
        let tokens = tokenize("rgb(255, 0, 128)");
        assert_eq!(tokens[0], CssToken::Function("rgb".into()));
        assert_eq!(tokens[1], CssToken::Number(255.0));
        assert_eq!(tokens[2], CssToken::Comma);
    }

    #[test]
    fn whitespace_coalescing() {
        let tokens = tokenize("a   \n\t  b");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("a".into()),
                CssToken::Whitespace,
                CssToken::Ident("b".into()),
                CssToken::Eof,
            ]
        );
    }

    #[test]
    fn single_char_tokens() {
        let tokens = tokenize(":;,{}()[].*>+/~");
        let expected = vec![
            CssToken::Colon,
            CssToken::Semicolon,
            CssToken::Comma,
            CssToken::OpenBrace,
            CssToken::CloseBrace,
            CssToken::OpenParen,
            CssToken::CloseParen,
            CssToken::OpenBracket,
            CssToken::CloseBracket,
            CssToken::Dot,
            CssToken::Star,
            CssToken::Greater,
            CssToken::Plus,
            CssToken::Slash,
            CssToken::Delim('~'),
            CssToken::Eof,
        ];
        assert_eq!(tokens, expected);
    }

    #[test]
    fn single_quoted_string() {
        let tokens = tokenize("'hello world'");
        assert_eq!(tokens[0], CssToken::String("hello world".into()));
    }

    #[test]
    fn decimal_only_number() {
        let tokens = tokenize(".75em");
        assert_eq!(tokens[0], CssToken::Dimension(0.75, "em".into()));
    }

    #[test]
    fn empty_input() {
        let tokens = tokenize("");
        assert_eq!(tokens, vec![CssToken::Eof]);
    }

    #[test]
    fn unterminated_comment() {
        // Should consume to EOF without panicking.
        let tokens = tokenize("/* oops");
        assert_eq!(tokens, vec![CssToken::Eof]);
    }

    #[test]
    fn unterminated_string() {
        let tokens = tokenize("\"oops");
        assert_eq!(tokens[0], CssToken::String("oops".into()));
    }
}

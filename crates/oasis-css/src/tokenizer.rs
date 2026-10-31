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
    /// Tokens already produced, returned before reading more input
    /// (last first): an unquoted `url(...)` becomes three tokens.
    pending: Vec<CssToken>,
}

impl CssTokenizer {
    /// Create a new tokenizer from the given CSS source text.
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            pending: Vec::new(),
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

    /// Ident-start code point (CSS Syntax §4.2): a letter, `_`, `-`
    /// (the full `-`-rules are approximated) or any non-ASCII character.
    fn is_ident_start(ch: char) -> bool {
        ch.is_ascii_alphabetic() || ch == '_' || ch == '-' || !ch.is_ascii()
    }

    fn is_ident_char(ch: char) -> bool {
        Self::is_ident_start(ch) || ch.is_ascii_digit()
    }

    /// Whether the input at `self.pos + offset` is a valid escape: `\`
    /// not followed by a newline (§4.3.8).
    fn valid_escape_at(&self, offset: usize) -> bool {
        self.peek_at(offset) == Some('\\') && !matches!(self.peek_at(offset + 1), Some('\n'))
    }

    /// Consume an escape after its `\` (§4.3.7): up to six hex digits
    /// (plus one optional whitespace) name a code point, anything else
    /// stands for itself. NUL, surrogates, out-of-range values and EOF
    /// give U+FFFD.
    fn consume_escape(&mut self) -> char {
        let Some(first) = self.advance() else {
            return '\u{FFFD}';
        };
        if !first.is_ascii_hexdigit() {
            return first;
        }
        let mut code = first.to_digit(16).unwrap_or(0);
        for _ in 0..5 {
            match self.peek().and_then(|c| c.to_digit(16)) {
                Some(d) => {
                    code = code * 16 + d;
                    self.advance();
                },
                None => break,
            }
        }
        if self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.advance();
        }
        match code {
            0 => '\u{FFFD}',
            c => char::from_u32(c).unwrap_or('\u{FFFD}'),
        }
    }

    /// Consume an identifier sequence, escapes included (`.md\:flex`,
    /// `\31 0`), non-ASCII characters too.
    fn consume_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if Self::is_ident_char(ch) {
                s.push(ch);
                self.advance();
            } else if self.valid_escape_at(0) {
                self.advance(); // '\'
                s.push(self.consume_escape());
            } else {
                break;
            }
        }
        s
    }

    /// Whether an identifier starts at the current position.
    fn starts_ident(&self) -> bool {
        match self.peek() {
            Some('\\') => self.valid_escape_at(0),
            // A `-` always starts one (a lone `-` is the ident `-`, which
            // calc() text relies on); `-` + digit was taken as a number.
            Some('-') => true,
            Some(c) => Self::is_ident_start(c),
            None => false,
        }
    }

    /// After `url(`: an unquoted URL is consumed raw (§4.3.6) and
    /// queued as `String(url)` + `)`. Regression: it used to be split
    /// into tokens and re-joined, so `url(images/a.png)` became
    /// `imagesapng` and `url(data:...;base64,...)` stopped at the `;`.
    /// Quoted URLs (`url("...")`) keep the normal token stream.
    fn tokenize_url_function(&mut self) -> CssToken {
        let mut look = 0;
        while self.peek_at(look).is_some_and(|c| c.is_ascii_whitespace()) {
            look += 1;
        }
        if matches!(self.peek_at(look), Some('"' | '\'')) {
            return CssToken::Function("url".into());
        }
        self.pos += look;
        let mut url = String::new();
        loop {
            match self.advance() {
                None | Some(')') => break,
                Some(c) if c.is_ascii_whitespace() => {
                    while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                        self.advance();
                    }
                    if matches!(self.peek(), None | Some(')')) {
                        self.advance();
                        break;
                    }
                    return self.consume_bad_url();
                },
                Some('"' | '\'' | '(') => return self.consume_bad_url(),
                Some(c) if c.is_control() => return self.consume_bad_url(),
                Some('\\') => {
                    if matches!(self.peek(), Some('\n')) {
                        return self.consume_bad_url();
                    }
                    url.push(self.consume_escape());
                },
                Some(c) => url.push(c),
            }
        }
        self.pending.push(CssToken::CloseParen);
        self.pending.push(CssToken::String(url));
        CssToken::Function("url".into())
    }

    /// Skip the rest of a malformed unquoted URL through its `)` (§4.3.14).
    /// The bad-url token is a lone U+FFFD delimiter: no URL is loaded and
    /// the declaration does not parse as one.
    fn consume_bad_url(&mut self) -> CssToken {
        while let Some(c) = self.advance() {
            if c == ')' {
                break;
            }
            if c == '\\' && self.peek().is_some() {
                self.advance();
            }
        }
        CssToken::Delim('\u{FFFD}')
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

    /// Consume a quoted string (§4.3.5). Escapes are decoded per CSS:
    /// `\f101` is U+F101 (it used to be the text `f101`, breaking every
    /// icon font) and `\` + newline is a line continuation.
    fn consume_string(&mut self, quote: char) -> String {
        self.advance(); // skip opening quote
        let mut s = String::new();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                match self.peek() {
                    None => {},
                    Some('\n') => {
                        self.advance();
                    },
                    Some(_) => s.push(self.consume_escape()),
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
        self.starts_number_at(0)
    }

    /// Check whether a number starts at `self.pos + offset`.
    fn starts_number_at(&self, offset: usize) -> bool {
        match self.input.get(self.pos + offset).copied() {
            Some(ch) if ch.is_ascii_digit() => true,
            Some('.') => matches!(
                self.input.get(self.pos + offset + 1).copied(),
                Some(d) if d.is_ascii_digit()
            ),
            _ => false,
        }
    }

    // ---------------------------------------------------------------
    // Main tokenisation loop
    // ---------------------------------------------------------------

    fn next_token(&mut self) -> CssToken {
        if let Some(tok) = self.pending.pop() {
            return tok;
        }
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

        let Some(ch) = self.peek() else {
            return CssToken::Eof;
        };

        // Quoted strings.
        if ch == '"' || ch == '\'' {
            return CssToken::String(self.consume_string(ch));
        }

        // Numbers (and things that start with a digit or `.digit`).
        if self.starts_number() {
            return self.tokenize_numeric(1.0);
        }

        // Negative number: `-` followed by digit or `.digit`.
        if ch == '-' && self.starts_number_at(1) {
            self.advance(); // consume '-'
            return self.tokenize_numeric(-1.0);
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
        if self.starts_ident() {
            let ident = self.consume_ident();
            if self.peek() == Some('(') {
                self.advance(); // consume '('
                if ident.eq_ignore_ascii_case("url") {
                    return self.tokenize_url_function();
                }
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
                    return self.tokenize_numeric(1.0);
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
    /// `sign` is `1.0` for positive, `-1.0` for negative.
    fn tokenize_numeric(&mut self, sign: f32) -> CssToken {
        let value = self.consume_number() * sign;
        if self.peek() == Some('%') {
            self.advance();
            return CssToken::Percentage(value);
        }
        if self.starts_ident() {
            let unit = self.consume_ident();
            return CssToken::Dimension(value, unit);
        }
        CssToken::Number(value)
    }

    /// Consume the name portion of a hash token (allows digits at start,
    /// unlike normal idents).
    fn consume_hash_name(&mut self) -> String {
        self.consume_ident()
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

    /// Unquoted URLs used to be tokenized and re-joined, losing `/`, `.`
    /// and everything after a `;`.
    #[test]
    fn unquoted_url_is_one_string() {
        let url_of = |css: &str| {
            let toks = tokenize(css);
            assert_eq!(toks[0], CssToken::Function("url".into()), "{css}");
            toks[1].clone()
        };
        assert_eq!(
            url_of("url(images/a.png)"),
            CssToken::String("images/a.png".into())
        );
        assert_eq!(
            url_of("url(  data:image/png;base64,iVBOR==  )"),
            CssToken::String("data:image/png;base64,iVBOR==".into())
        );
        assert_eq!(url_of("URL(a\\)b)"), CssToken::String("a)b".into()));
        // Quoted URLs keep the ordinary token stream.
        assert_eq!(url_of("url( 'x.png' )"), CssToken::Whitespace);
        // Bad URLs are swallowed through their `)`.
        let toks = tokenize("url(a b) red");
        assert_eq!(toks[0], CssToken::Delim('\u{FFFD}'));
        assert_eq!(toks[2], CssToken::Ident("red".into()));
        let toks = tokenize("url(a(b) c");
        assert_eq!(toks[0], CssToken::Delim('\u{FFFD}'));
    }

    #[test]
    fn string_escapes_are_decoded() {
        assert_eq!(
            tokenize(r#""\f101""#)[0],
            CssToken::String("\u{f101}".into())
        );
        assert_eq!(tokenize(r#""\41 B""#)[0], CssToken::String("AB".into()));
        assert_eq!(tokenize("\"a\\\nb\"")[0], CssToken::String("ab".into()));
        assert_eq!(tokenize(r#""\"\\""#)[0], CssToken::String("\"\\".into()));
        assert_eq!(tokenize(r#""\0""#)[0], CssToken::String("\u{fffd}".into()));
        assert_eq!(
            tokenize(r#""\110000""#)[0],
            CssToken::String("\u{fffd}".into())
        );
    }

    #[test]
    fn idents_take_escapes_and_non_ascii() {
        // Tailwind-style escaped class names.
        assert_eq!(tokenize(r"md\:flex")[0], CssToken::Ident("md:flex".into()));
        assert_eq!(tokenize(r"\31 0")[0], CssToken::Ident("10".into()));
        assert_eq!(tokenize("café")[0], CssToken::Ident("café".into()));
        assert_eq!(tokenize("#ab\\.c")[0], CssToken::Hash("ab.c".into()));
        let sheet = crate::parser::Stylesheet::parse(r".md\:flex{display:flex}");
        assert_eq!(sheet.rules.len(), 1);
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

    #[test]
    fn negative_dimension() {
        let tokens = tokenize("-10px");
        assert_eq!(
            tokens,
            vec![CssToken::Dimension(-10.0, "px".into()), CssToken::Eof]
        );
    }

    #[test]
    fn negative_number() {
        let tokens = tokenize("-42");
        assert_eq!(tokens, vec![CssToken::Number(-42.0), CssToken::Eof]);
    }

    #[test]
    fn negative_percentage() {
        let tokens = tokenize("-50%");
        assert_eq!(tokens, vec![CssToken::Percentage(-50.0), CssToken::Eof]);
    }

    #[test]
    fn negative_decimal_dimension() {
        let tokens = tokenize("-1.5em");
        assert_eq!(
            tokens,
            vec![CssToken::Dimension(-1.5, "em".into()), CssToken::Eof]
        );
    }

    #[test]
    fn negative_in_declaration() {
        let tokens = tokenize("margin: -10px;");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("margin".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::Dimension(-10.0, "px".into()),
                CssToken::Semicolon,
                CssToken::Eof,
            ]
        );
    }

    #[test]
    fn hyphenated_ident_not_negative() {
        // `-webkit-transform` is an ident, not a negative number.
        let tokens = tokenize("-webkit-transform");
        assert_eq!(
            tokens,
            vec![CssToken::Ident("-webkit-transform".into()), CssToken::Eof,]
        );
    }

    #[test]
    fn negative_decimal_only() {
        // `-.5px` should tokenize as a negative dimension.
        let tokens = tokenize("-.5px");
        assert_eq!(
            tokens,
            vec![CssToken::Dimension(-0.5, "px".into()), CssToken::Eof,]
        );
    }
}

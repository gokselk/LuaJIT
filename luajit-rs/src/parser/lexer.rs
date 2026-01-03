//! Lua lexer (tokenizer).
//!
//! Tokenizes Lua source code into a stream of tokens.

use crate::value::{LuaError, LuaResult};
use std::str::Chars;
use std::iter::Peekable;

/// Token kinds
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Name(String),
    String(Vec<u8>),  // Lua strings are byte arrays
    Number(f64),

    // Keywords
    And,
    Break,
    Do,
    Else,
    Elseif,
    End,
    False,
    For,
    Function,
    Goto,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,

    // Operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Caret,      // ^
    Hash,       // #
    Ampersand,  // &
    Tilde,      // ~
    Pipe,       // |
    LtLt,       // <<
    GtGt,       // >>
    SlashSlash, // //
    EqEq,       // ==
    TildeEq,    // ~=
    LtEq,       // <=
    GtEq,       // >=
    Lt,         // <
    Gt,         // >
    Eq,         // =
    LParen,     // (
    RParen,     // )
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    ColonColon, // ::
    Semicolon,  // ;
    Colon,      // :
    Comma,      // ,
    Dot,        // .
    DotDot,     // ..
    DotDotDot,  // ...

    // Special
    Eof,
}

/// A token with position information
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: u32,
    pub column: u32,
}

impl Token {
    pub fn new(kind: TokenKind, line: u32, column: u32) -> Self {
        Self { kind, line, column }
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }
}

/// Lua lexer
pub struct Lexer<'a> {
    source: &'a str,
    chars: Peekable<Chars<'a>>,
    current_pos: usize,
    line: u32,
    column: u32,
    /// Lookahead token
    peeked: Option<Token>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given source
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().peekable(),
            current_pos: 0,
            line: 1,
            column: 1,
            peeked: None,
        }
    }

    /// Get the current line number
    pub fn line(&self) -> u32 {
        self.line
    }

    /// Peek at the next token without consuming it
    pub fn peek(&mut self) -> LuaResult<&Token> {
        if self.peeked.is_none() {
            self.peeked = Some(self.next_token()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    /// Get the next token
    pub fn next(&mut self) -> LuaResult<Token> {
        if let Some(token) = self.peeked.take() {
            Ok(token)
        } else {
            self.next_token()
        }
    }

    /// Check if next token matches the given kind
    pub fn check(&mut self, kind: &TokenKind) -> LuaResult<bool> {
        Ok(&self.peek()?.kind == kind)
    }

    /// Consume the next token if it matches
    pub fn match_token(&mut self, kind: &TokenKind) -> LuaResult<bool> {
        if self.check(kind)? {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Expect a specific token kind
    pub fn expect(&mut self, kind: TokenKind) -> LuaResult<Token> {
        let token = self.next()?;
        if token.kind == kind {
            Ok(token)
        } else {
            Err(LuaError::SyntaxError(format!(
                "expected {:?}, got {:?} at line {}",
                kind, token.kind, token.line
            )))
        }
    }

    /// Internal: get next token
    fn next_token(&mut self) -> LuaResult<Token> {
        self.skip_whitespace_and_comments();

        let line = self.line;
        let column = self.column;

        let Some(c) = self.advance() else {
            return Ok(Token::new(TokenKind::Eof, line, column));
        };

        let kind = match c {
            // Single-character tokens
            '+' => TokenKind::Plus,
            '*' => TokenKind::Star,
            '%' => TokenKind::Percent,
            '^' => TokenKind::Caret,
            '#' => TokenKind::Hash,
            '&' => TokenKind::Ampersand,
            '|' => TokenKind::Pipe,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,

            // Multi-character tokens
            '-' => {
                if self.match_char('-') {
                    // Comment
                    self.skip_comment();
                    return self.next_token();
                }
                TokenKind::Minus
            }

            '/' => {
                if self.match_char('/') {
                    TokenKind::SlashSlash
                } else {
                    TokenKind::Slash
                }
            }

            '=' => {
                if self.match_char('=') {
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }

            '~' => {
                if self.match_char('=') {
                    TokenKind::TildeEq
                } else {
                    TokenKind::Tilde
                }
            }

            '<' => {
                if self.match_char('=') {
                    TokenKind::LtEq
                } else if self.match_char('<') {
                    TokenKind::LtLt
                } else {
                    TokenKind::Lt
                }
            }

            '>' => {
                if self.match_char('=') {
                    TokenKind::GtEq
                } else if self.match_char('>') {
                    TokenKind::GtGt
                } else {
                    TokenKind::Gt
                }
            }

            ':' => {
                if self.match_char(':') {
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }

            '.' => {
                if self.match_char('.') {
                    if self.match_char('.') {
                        TokenKind::DotDotDot
                    } else {
                        TokenKind::DotDot
                    }
                } else if self.peek_char().map_or(false, |c| c.is_ascii_digit()) {
                    // Number starting with .
                    self.read_number_from_dot()?
                } else {
                    TokenKind::Dot
                }
            }

            '[' => {
                if self.check_char('=') || self.check_char('[') {
                    // Long string
                    let s = self.read_long_string()?;
                    TokenKind::String(s)
                } else {
                    TokenKind::LBracket
                }
            }

            '"' | '\'' => {
                let s = self.read_string(c)?;
                TokenKind::String(s)
            }

            '0'..='9' => self.read_number(c)?,

            c if c.is_alphabetic() || c == '_' => self.read_name(c),

            _ => {
                return Err(LuaError::SyntaxError(format!(
                    "unexpected character '{}' at line {}",
                    c, line
                )));
            }
        };

        Ok(Token::new(kind, line, column))
    }

    /// Advance to the next character
    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.current_pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    /// Peek at the next character without consuming
    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    /// Check if next char matches
    fn check_char(&mut self, expected: char) -> bool {
        self.peek_char() == Some(expected)
    }

    /// Match and consume a character
    fn match_char(&mut self, expected: char) -> bool {
        if self.check_char(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Skip whitespace and comments
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek_char() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    /// Skip a comment (after --)
    fn skip_comment(&mut self) {
        // Check for long comment
        if self.check_char('[') {
            self.advance();
            if self.check_char('=') || self.check_char('[') {
                // Long comment
                let _ = self.read_long_string();
                return;
            }
        }

        // Short comment - skip to end of line
        while let Some(c) = self.peek_char() {
            if c == '\n' {
                break;
            }
            self.advance();
        }
    }

    /// Read a name/identifier
    fn read_name(&mut self, first: char) -> TokenKind {
        let mut name = String::new();
        name.push(first);

        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Check for keywords
        match name.as_str() {
            "and" => TokenKind::And,
            "break" => TokenKind::Break,
            "do" => TokenKind::Do,
            "else" => TokenKind::Else,
            "elseif" => TokenKind::Elseif,
            "end" => TokenKind::End,
            "false" => TokenKind::False,
            "for" => TokenKind::For,
            "function" => TokenKind::Function,
            "goto" => TokenKind::Goto,
            "if" => TokenKind::If,
            "in" => TokenKind::In,
            "local" => TokenKind::Local,
            "nil" => TokenKind::Nil,
            "not" => TokenKind::Not,
            "or" => TokenKind::Or,
            "repeat" => TokenKind::Repeat,
            "return" => TokenKind::Return,
            "then" => TokenKind::Then,
            "true" => TokenKind::True,
            "until" => TokenKind::Until,
            "while" => TokenKind::While,
            _ => TokenKind::Name(name),
        }
    }

    /// Read a string literal (returns raw bytes)
    fn read_string(&mut self, quote: char) -> LuaResult<Vec<u8>> {
        let mut s: Vec<u8> = Vec::new();

        loop {
            match self.advance() {
                None => {
                    return Err(LuaError::SyntaxError(
                        "unterminated string".to_string(),
                    ));
                }
                Some(c) if c == quote => break,
                Some('\\') => {
                    let escaped: u8 = match self.advance() {
                        Some('a') => 0x07,
                        Some('b') => 0x08,
                        Some('f') => 0x0c,
                        Some('n') => b'\n',
                        Some('r') => b'\r',
                        Some('t') => b'\t',
                        Some('v') => 0x0b,
                        Some('\\') => b'\\',
                        Some('"') => b'"',
                        Some('\'') => b'\'',
                        Some('\n') => b'\n',
                        Some(d1 @ '0'..='9') => {
                            // Decimal escape \ddd (up to 3 digits, value <= 255)
                            let mut num = d1.to_digit(10).unwrap();

                            // Try to read second digit
                            if let Some(d2 @ '0'..='9') = self.peek_char() {
                                self.advance();
                                num = num * 10 + d2.to_digit(10).unwrap();

                                // Try to read third digit
                                if let Some(d3 @ '0'..='9') = self.peek_char() {
                                    let new_num = num * 10 + d3.to_digit(10).unwrap();
                                    // Only consume if result <= 255
                                    if new_num <= 255 {
                                        self.advance();
                                        num = new_num;
                                    }
                                }
                            }

                            if num > 255 {
                                return Err(LuaError::SyntaxError(
                                    format!("decimal escape too large: {}", num),
                                ));
                            }
                            num as u8
                        }
                        Some('x') => {
                            // Hex escape \xXX
                            let h1 = self.advance().and_then(|c| c.to_digit(16));
                            let h2 = self.advance().and_then(|c| c.to_digit(16));
                            match (h1, h2) {
                                (Some(d1), Some(d2)) => {
                                    (d1 * 16 + d2) as u8
                                }
                                _ => {
                                    return Err(LuaError::SyntaxError(
                                        "invalid hex escape".to_string(),
                                    ));
                                }
                            }
                        }
                        Some('z') => {
                            // Skip whitespace
                            self.skip_whitespace_and_comments();
                            continue;
                        }
                        Some(c) => c as u8,
                        None => {
                            return Err(LuaError::SyntaxError(
                                "unterminated string".to_string(),
                            ));
                        }
                    };
                    s.push(escaped);
                }
                Some(c) => {
                    // Handle multi-byte UTF-8 chars in source by encoding as bytes
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    s.extend_from_slice(encoded.as_bytes());
                }
            }
        }

        Ok(s)
    }

    /// Read a long string [[...]] or [=[...]=]
    fn read_long_string(&mut self) -> LuaResult<Vec<u8>> {
        // Count the = signs
        let mut level = 0;
        while self.match_char('=') {
            level += 1;
        }

        if !self.match_char('[') {
            return Err(LuaError::SyntaxError(
                "invalid long string delimiter".to_string(),
            ));
        }

        // Skip initial newline if present
        if self.check_char('\n') {
            self.advance();
        }

        let mut s: Vec<u8> = Vec::new();

        loop {
            match self.advance() {
                None => {
                    return Err(LuaError::SyntaxError(
                        "unterminated long string".to_string(),
                    ));
                }
                Some(']') => {
                    // Check for closing ]=*]
                    let mut closing_level = 0;
                    while self.check_char('=') {
                        closing_level += 1;
                        self.advance();
                    }
                    if closing_level == level && self.match_char(']') {
                        break;
                    }
                    // Not a closing delimiter, add to string
                    s.push(b']');
                    for _ in 0..closing_level {
                        s.push(b'=');
                    }
                }
                Some(c) => {
                    // Handle multi-byte UTF-8 chars in source
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    s.extend_from_slice(encoded.as_bytes());
                }
            }
        }

        Ok(s)
    }

    /// Read a number
    fn read_number(&mut self, first: char) -> LuaResult<TokenKind> {
        let mut s = String::new();
        s.push(first);

        // Check for hex
        if first == '0' && (self.check_char('x') || self.check_char('X')) {
            s.push(self.advance().unwrap());
            return self.read_hex_number(s);
        }

        // Read integer part
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Read fractional part
        if self.check_char('.') {
            s.push(self.advance().unwrap());
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        // Read exponent
        if self.check_char('e') || self.check_char('E') {
            s.push(self.advance().unwrap());
            if self.check_char('+') || self.check_char('-') {
                s.push(self.advance().unwrap());
            }
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        let num: f64 = s.parse().map_err(|_| {
            LuaError::SyntaxError(format!("malformed number: {}", s))
        })?;

        Ok(TokenKind::Number(num))
    }

    /// Read hex number after 0x
    fn read_hex_number(&mut self, mut s: String) -> LuaResult<TokenKind> {
        // Read hex digits
        while let Some(c) = self.peek_char() {
            if c.is_ascii_hexdigit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Check for fractional part
        if self.check_char('.') {
            s.push(self.advance().unwrap());
            while let Some(c) = self.peek_char() {
                if c.is_ascii_hexdigit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        // Check for exponent (p or P for hex)
        if self.check_char('p') || self.check_char('P') {
            s.push(self.advance().unwrap());
            if self.check_char('+') || self.check_char('-') {
                s.push(self.advance().unwrap());
            }
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        // Parse hex float
        let num = parse_hex_float(&s)?;
        Ok(TokenKind::Number(num))
    }

    /// Read a number starting with decimal point
    fn read_number_from_dot(&mut self) -> LuaResult<TokenKind> {
        let mut s = String::from("0.");

        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Read exponent
        if self.check_char('e') || self.check_char('E') {
            s.push(self.advance().unwrap());
            if self.check_char('+') || self.check_char('-') {
                s.push(self.advance().unwrap());
            }
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        let num: f64 = s.parse().map_err(|_| {
            LuaError::SyntaxError(format!("malformed number: {}", s))
        })?;

        Ok(TokenKind::Number(num))
    }
}

/// Parse a hex float (0x... with optional fraction and exponent)
fn parse_hex_float(s: &str) -> LuaResult<f64> {
    // Simple implementation: handle common cases
    let s = s.trim_start_matches("0x").trim_start_matches("0X");

    // Check for exponent
    let (mantissa_str, exp_str) = if let Some(pos) = s.find(|c| c == 'p' || c == 'P') {
        (&s[..pos], Some(&s[pos + 1..]))
    } else {
        (s, None)
    };

    // Parse mantissa
    let (int_part, frac_part) = if let Some(dot_pos) = mantissa_str.find('.') {
        (&mantissa_str[..dot_pos], Some(&mantissa_str[dot_pos + 1..]))
    } else {
        (mantissa_str, None)
    };

    // Parse integer part
    let int_val = if int_part.is_empty() {
        0u64
    } else {
        u64::from_str_radix(int_part, 16).map_err(|_| {
            LuaError::SyntaxError(format!("malformed hex number"))
        })?
    };

    // Parse fractional part
    let frac_val = if let Some(frac) = frac_part {
        if frac.is_empty() {
            0.0
        } else {
            let frac_int = u64::from_str_radix(frac, 16).map_err(|_| {
                LuaError::SyntaxError(format!("malformed hex number"))
            })?;
            frac_int as f64 / (16.0_f64).powi(frac.len() as i32)
        }
    } else {
        0.0
    };

    let mut result = int_val as f64 + frac_val;

    // Apply exponent
    if let Some(exp) = exp_str {
        let exp_val: i32 = exp.parse().map_err(|_| {
            LuaError::SyntaxError(format!("malformed hex exponent"))
        })?;
        result *= (2.0_f64).powi(exp_val);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_tokens() {
        let mut lexer = Lexer::new("+ - * / % ^");
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Plus));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Minus));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Star));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Slash));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Percent));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Caret));
    }

    #[test]
    fn test_keywords() {
        let mut lexer = Lexer::new("if then else end function");
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::If));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Then));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Else));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::End));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Function));
    }

    #[test]
    fn test_numbers() {
        let mut lexer = Lexer::new("123 45.67 0xFF 1e10");

        match lexer.next().unwrap().kind {
            TokenKind::Number(n) => assert_eq!(n, 123.0),
            _ => panic!("expected number"),
        }
        match lexer.next().unwrap().kind {
            TokenKind::Number(n) => assert!((n - 45.67).abs() < 0.001),
            _ => panic!("expected number"),
        }
        match lexer.next().unwrap().kind {
            TokenKind::Number(n) => assert_eq!(n, 255.0),
            _ => panic!("expected number"),
        }
        match lexer.next().unwrap().kind {
            TokenKind::Number(n) => assert_eq!(n, 1e10),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_strings() {
        let mut lexer = Lexer::new(r#""hello" 'world' [[long]]"#);

        match lexer.next().unwrap().kind {
            TokenKind::String(s) => assert_eq!(s, b"hello"),
            _ => panic!("expected string"),
        }
        match lexer.next().unwrap().kind {
            TokenKind::String(s) => assert_eq!(s, b"world"),
            _ => panic!("expected string"),
        }
        match lexer.next().unwrap().kind {
            TokenKind::String(s) => assert_eq!(s, b"long"),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_comments() {
        let mut lexer = Lexer::new("a -- comment\nb --[[ long ]] c");
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Name(_)));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Name(_)));
        assert!(matches!(lexer.next().unwrap().kind, TokenKind::Name(_)));
    }
}

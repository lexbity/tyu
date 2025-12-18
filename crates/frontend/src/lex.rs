use crate::{
    span::Span,
    token::{Token, TokenKind},
};

pub struct Lexer<'a> {
    src: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, i: 0 }
    }

    pub fn src(&self) -> &'a [u8] {
        self.src
    }

    pub fn next(&mut self) -> Token {
        self.skip_ws_and_comments();
        if self.i >= self.src.len() {
            return Token::new(TokenKind::Eof, Span::new(self.i, self.i));
        }

        let start = self.i;
        let b = self.bump();

        let kind = match b {
            b':' => TokenKind::PunctColon,
            b';' => TokenKind::PunctSemi,
            b'(' => TokenKind::PunctLParen,
            b')' => TokenKind::PunctRParen,
            b'[' => TokenKind::PunctLBracket,
            b']' => TokenKind::PunctRBracket,
            b'{' => TokenKind::PunctLBrace,
            b'}' => TokenKind::PunctRBrace,
            b',' => TokenKind::PunctComma,
            b'.' => {
                if self.peek() == Some(b'.') {
                    self.i += 1;
                    TokenKind::PunctDblDot
                } else {
                    TokenKind::PunctDot
                }
            }
            b'=' => match self.peek() {
                Some(b'>') => {
                    self.i += 1;
                    TokenKind::PunctArrowBind
                }
                Some(b'=') => {
                    self.i += 1;
                    TokenKind::PunctEqEq
                }
                _ => TokenKind::PunctEq,
            },
            b'-' => {
                if self.peek() == Some(b'-') {
                    self.i += 1;
                    TokenKind::PunctDashDash
                } else {
                    return self.lex_number_or_ident(start);
                }
            }
            b'<' => {
                if self.peek() == Some(b'|') {
                    self.i += 1;
                    TokenKind::PunctLessPipe
                } else if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::PunctLe
                } else {
                    return self.lex_ident(start);
                }
            }
            b'|' => {
                if self.peek() == Some(b'>') {
                    self.i += 1;
                    TokenKind::PunctPipeGreater
                } else {
                    return self.lex_ident(start);
                }
            }
            b'>' => {
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::PunctGe
                } else {
                    return self.lex_ident(start);
                }
            }
            b'!' => {
                if self.peek() == Some(b'{') {
                    return self.lex_effect_set(start);
                }
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    return Token::new(TokenKind::PunctNe, Span::new(start, self.i));
                }
                return self.lex_ident(start);
            }
            b'"' => return self.lex_string(start),
            c if is_ident_start(c) => return self.lex_ident(start),
            c if is_digit(c) => return self.lex_number(start),
            _ => TokenKind::Ident, // treat unknown punctuation as ident-like token for recovery
        };

        Token::new(kind, Span::new(start, self.i))
    }

    fn lex_string(&mut self, start: usize) -> Token {
        while let Some(b) = self.peek() {
            self.i += 1;
            if b == b'\\' {
                if self.i < self.src.len() {
                    self.i += 1;
                }
                continue;
            }
            if b == b'"' {
                break;
            }
        }
        Token::new(TokenKind::String, Span::new(start, self.i))
    }

    fn lex_number_or_ident(&mut self, start: usize) -> Token {
        if let Some(b) = self.peek() {
            if is_digit(b) {
                return self.lex_number(start);
            }
        }
        self.lex_ident(start)
    }

    fn lex_number(&mut self, start: usize) -> Token {
        while let Some(b) = self.peek() {
            if is_digit(b) || matches!(b, b'x' | b'X' | b'b' | b'B' | b'a'..=b'f' | b'A'..=b'F' | b'_') {
                self.i += 1;
            } else {
                break;
            }
        }
        Token::new(TokenKind::Number, Span::new(start, self.i))
    }

    fn lex_ident(&mut self, start: usize) -> Token {
        while let Some(b) = self.peek() {
            if is_ident_continue(b) {
                self.i += 1;
            } else {
                break;
            }
        }
        let span = Span::new(start, self.i);
        let kind = match self.slice(span) {
            b"module" => TokenKind::KwModule,
            b"import" => TokenKind::KwImport,
            b"export" => TokenKind::KwExport,
            b"end" => TokenKind::KwEnd,
            b"type" => TokenKind::KwType,
            b"subtype" => TokenKind::KwSubtype,
            b"struct" => TokenKind::KwStruct,
            b"enum" => TokenKind::KwEnum,
            b"const" => TokenKind::KwConst,
            b"resource" => TokenKind::KwResource,
            b"register-map" => TokenKind::KwRegisterMap,
            b"requires" => TokenKind::KwRequires,
            b"ensures" => TokenKind::KwEnsures,
            _ => TokenKind::Ident,
        };
        Token::new(kind, span)
    }

    fn lex_effect_set(&mut self, start: usize) -> Token {
        // We have already consumed '!' and peeked '{'
        self.i += 1; // consume '{'
        while let Some(b) = self.peek() {
            self.i += 1;
            if b == b'}' {
                break;
            }
        }
        Token::new(TokenKind::EffectSet, Span::new(start, self.i))
    }

    fn slice(&self, span: Span) -> &'a [u8] {
        &self.src[span.start..span.end]
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while let Some(b) = self.peek() {
                if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' {
                    self.i += 1;
                } else {
                    break;
                }
            }
            if self.peek() == Some(b'#') {
                while let Some(b) = self.peek() {
                    self.i += 1;
                    if b == b'\n' {
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn bump(&mut self) -> u8 {
        let b = self.src[self.i];
        self.i += 1;
        b
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.i).copied()
    }
}

fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'@' || b == b'!'
}

fn is_ident_continue(b: u8) -> bool {
    is_ident_start(b) || is_digit(b) || matches!(b, b'-' | b'?' | b'!' | b'/' | b'[' | b']')
}

use crate::{
    fixed::FixedVec,
    lex::Lexer,
    span::Span,
    token::{Token, TokenKind},
};

pub trait Output {
    fn write(&mut self, bytes: &[u8]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub code: u32,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclKind {
    Word,
    Type,
    Subtype,
    Struct,
    Enum,
    Const,
    Resource,
    RegisterMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtypeAst {
    pub name: Span,
    pub base: Span,
    pub min: i64,
    pub max: i64,
}

pub struct ImportAst {
    pub module: Span,
    pub names: FixedVec<Span, 64>,
}

pub struct DeclAst {
    pub kind: DeclKind,
    pub name: Span,
    pub sig: Option<Span>,
    pub attrs: FixedVec<Span, 16>,
    pub body: Option<Span>,
    pub requires: Option<Span>,
    pub ensures: Option<Span>,
}

pub struct ModuleAst {
    pub name: Span,
    pub imports: FixedVec<ImportAst, 64>,
    pub exports: FixedVec<Span, 256>,
    pub decls: FixedVec<DeclAst, 256>,
    pub has_export_stmt: bool,
    pub subtypes: FixedVec<SubtypeAst, 64>,
}

pub struct Parser<'a> {
    lex: Lexer<'a>,
    look: Token,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        let mut lex = Lexer::new(src);
        let look = lex.next();
        Self { lex, look }
    }

    pub fn parse_module_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.expect(TokenKind::KwModule, 2100)?;
        self.bump();

        let name = self.expect(TokenKind::Ident, 2101)?;
        self.bump();

        self.expect(TokenKind::PunctSemi, 2102)?;
        self.bump();

        out.write(b"module ");
        out.write(self.slice(name.span));
        out.write(b"\n");

        while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
            self.parse_item_dump(out)?;
        }

        self.expect(TokenKind::KwEnd, 2103)?;
        self.bump();
        self.expect(TokenKind::PunctSemi, 2104)?;
        self.bump();

        Ok(())
    }

    pub fn parse_module_ast(&mut self) -> Result<ModuleAst, ParseError> {
        self.expect(TokenKind::KwModule, 2100)?;
        self.bump();
        let name = self.expect(TokenKind::Ident, 2101)?;
        self.bump();
        self.expect(TokenKind::PunctSemi, 2102)?;
        self.bump();

        let mut ast = ModuleAst {
            name: name.span,
            imports: FixedVec::new(),
            exports: FixedVec::new(),
            decls: FixedVec::new(),
            has_export_stmt: false,
            subtypes: FixedVec::new(),
        };

        let mut pending_attrs: FixedVec<Span, 16> = FixedVec::new();

        while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
            if self.look.kind == TokenKind::Ident && self.slice(self.look.span).starts_with(b"@") {
                let _ = pending_attrs.push(self.look.span);
                self.bump();
                continue;
            }

            match self.look.kind {
                TokenKind::KwImport => {
                    let imp = self.parse_import_ast()?;
                    let _ = ast.imports.push(imp);
                }
                TokenKind::KwExport => {
                    self.parse_export_ast(&mut ast)?;
                }
                TokenKind::PunctColon => {
                    let decl = self.parse_word_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwStruct => {
                    let decl = self.parse_block_decl_ast(DeclKind::Struct, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwEnum => {
                    let decl = self.parse_block_decl_ast(DeclKind::Enum, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwRegisterMap => {
                    let decl =
                        self.parse_block_decl_ast(DeclKind::RegisterMap, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwType => {
                    let decl = self.parse_semi_decl_ast(DeclKind::Type, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwSubtype => {
                    let (decl, st) = self.parse_subtype_decl_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                    if let Some(st) = st {
                        let _ = ast.subtypes.push(st);
                    }
                }
                TokenKind::KwConst => {
                    let decl = self.parse_semi_decl_ast(DeclKind::Const, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwResource => {
                    let decl = self.parse_semi_decl_ast(DeclKind::Resource, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                _ => {
                    // recovery: skip token and reset pending attrs (attributes only apply to decls)
                    pending_attrs = FixedVec::new();
                    self.bump();
                    let _ = self.skip_until_semi();
                }
            }
        }

        self.expect(TokenKind::KwEnd, 2103)?;
        self.bump();
        self.expect(TokenKind::PunctSemi, 2104)?;
        self.bump();

        Ok(ast)
    }

    fn parse_item_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        match self.look.kind {
            TokenKind::KwImport => self.parse_import_dump(out),
            TokenKind::KwExport => self.parse_export_dump(out),
            TokenKind::PunctColon => self.parse_word_dump(out),
            TokenKind::KwStruct | TokenKind::KwEnum | TokenKind::KwRegisterMap => {
                self.parse_block_decl_dump(out)
            }
            TokenKind::KwType
            | TokenKind::KwSubtype
            | TokenKind::KwConst
            | TokenKind::KwResource => self.parse_semi_decl_dump(out),
            _ => self.parse_unknown_stmt_dump(out),
        }
    }

    fn parse_import_ast(&mut self) -> Result<ImportAst, ParseError> {
        self.bump(); // import
        let name = self.expect(TokenKind::Ident, 2110)?;
        self.bump();

        let mut names: FixedVec<Span, 64> = FixedVec::new();
        if self.look.kind == TokenKind::PunctLBrace {
            self.bump();
            while self.look.kind != TokenKind::PunctRBrace && self.look.kind != TokenKind::Eof {
                if self.look.kind == TokenKind::Ident {
                    let _ = names.push(self.look.span);
                    self.bump();
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, 2111)?;
            self.bump();
        }
        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }
        Ok(ImportAst {
            module: name.span,
            names,
        })
    }

    fn parse_export_ast(&mut self, ast: &mut ModuleAst) -> Result<(), ParseError> {
        self.bump(); // export
        ast.has_export_stmt = true;

        if self.look.kind == TokenKind::PunctLBrace {
            self.bump();
            while self.look.kind != TokenKind::PunctRBrace && self.look.kind != TokenKind::Eof {
                if self.look.kind == TokenKind::Ident {
                    let _ = ast.exports.push(self.look.span);
                    self.bump();
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, 2120)?;
            self.bump();
        } else if self.look.kind == TokenKind::Ident {
            let _ = ast.exports.push(self.look.span);
            self.bump();
        }

        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }
        Ok(())
    }

    fn parse_word_ast(&mut self, pending_attrs: &mut FixedVec<Span, 16>) -> Result<DeclAst, ParseError> {
        self.bump(); // :
        let name = self.expect(TokenKind::Ident, 2130)?;
        self.bump();

        let sig = if self.look.kind == TokenKind::PunctLParen {
            Some(self.capture_balanced(TokenKind::PunctLParen, TokenKind::PunctRParen, 2131)?)
        } else {
            None
        };

        let mut requires: Option<Span> = None;
        let mut ensures: Option<Span> = None;
        loop {
            match self.look.kind {
                TokenKind::KwRequires | TokenKind::KwEnsures => {
                    let is_requires = self.look.kind == TokenKind::KwRequires;
                    self.bump();
                    let q = self.capture_quotation(2132)?;
                    if is_requires {
                        requires = Some(q);
                    } else {
                        ensures = Some(q);
                    }
                }
                _ => break,
            }
        }

        let body_start = self.look.span.start;
        self.dump_terms_until(&mut NullOut, TokenKind::PunctSemi)?;
        let body_end = self.look.span.start;
        self.expect(TokenKind::PunctSemi, 2134)?;
        self.bump();

        let attrs = core::mem::replace(pending_attrs, FixedVec::new());
        Ok(DeclAst {
            kind: DeclKind::Word,
            name: name.span,
            sig,
            attrs,
            body: Some(Span::new(body_start, body_end)),
            requires,
            ensures,
        })
    }

    fn parse_block_decl_ast(
        &mut self,
        kind: DeclKind,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // keyword already matched by caller
        let name = self.expect(TokenKind::Ident, 2150)?;
        self.bump();
        self.skip_until_end_semi()?;
        let attrs = core::mem::replace(pending_attrs, FixedVec::new());
        Ok(DeclAst {
            kind,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
        })
    }

    fn parse_semi_decl_ast(
        &mut self,
        kind: DeclKind,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // keyword already matched by caller
        let name = self.expect(TokenKind::Ident, 2151)?;
        self.bump();
        self.skip_until_semi()?;
        let attrs = core::mem::replace(pending_attrs, FixedVec::new());
        Ok(DeclAst {
            kind,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
        })
    }

    fn parse_subtype_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<(DeclAst, Option<SubtypeAst>), ParseError> {
        self.bump(); // subtype
        let name = self.expect(TokenKind::Ident, 2170)?;
        self.bump();
        self.expect(TokenKind::PunctEq, 2171)?;
        self.bump();
        let base = self.expect(TokenKind::Ident, 2172)?;
        self.bump();

        // Expect "range" keyword as ident.
        if self.look.kind != TokenKind::Ident || self.slice(self.look.span) != b"range" {
            return Err(ParseError {
                code: 2173,
                span: self.look.span,
            });
        }
        self.bump();

        let min_tok = self.expect(TokenKind::Number, 2174)?;
        let min = parse_i64(self.slice(min_tok.span)).ok_or(ParseError {
            code: 2175,
            span: min_tok.span,
        })?;
        self.bump();
        self.expect(TokenKind::PunctDblDot, 2176)?;
        self.bump();
        let max_tok = self.expect(TokenKind::Number, 2177)?;
        let max = parse_i64(self.slice(max_tok.span)).ok_or(ParseError {
            code: 2178,
            span: max_tok.span,
        })?;
        self.bump();
        self.expect(TokenKind::PunctSemi, 2179)?;
        self.bump();

        let attrs = core::mem::replace(pending_attrs, FixedVec::new());
        let decl = DeclAst {
            kind: DeclKind::Subtype,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
        };
        let st = Some(SubtypeAst {
            name: name.span,
            base: base.span,
            min,
            max,
        });
        Ok((decl, st))
    }

    fn parse_import_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.bump();
        let name = self.expect(TokenKind::Ident, 2110)?;
        self.bump();

        out.write(b"  import ");
        out.write(self.slice(name.span));

        if self.look.kind == TokenKind::PunctLBrace {
            out.write(b" {");
            self.bump();
            let mut first = true;
            while self.look.kind != TokenKind::PunctRBrace && self.look.kind != TokenKind::Eof {
                if self.look.kind == TokenKind::Ident {
                    if !first {
                        out.write(b" ");
                    }
                    first = false;
                    out.write(self.slice(self.look.span));
                    self.bump();
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                // recovery: skip unknown tokens
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, 2111)?;
            self.bump();
            out.write(b"}");
        }

        // optional trailing ';'
        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }

        out.write(b"\n");
        Ok(())
    }

    fn parse_export_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.bump(); // export
        out.write(b"  export ");

        if self.look.kind == TokenKind::PunctLBrace {
            out.write(b"{");
            self.bump();
            let mut first = true;
            while self.look.kind != TokenKind::PunctRBrace && self.look.kind != TokenKind::Eof {
                if self.look.kind == TokenKind::Ident {
                    if !first {
                        out.write(b" ");
                    }
                    first = false;
                    out.write(self.slice(self.look.span));
                    self.bump();
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, 2120)?;
            self.bump();
            out.write(b"}");
        } else if self.look.kind == TokenKind::Ident {
            out.write(self.slice(self.look.span));
            self.bump();
        }

        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }
        out.write(b"\n");
        Ok(())
    }

    fn parse_word_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.bump(); // :
        let name = self.expect(TokenKind::Ident, 2130)?;
        self.bump();

        out.write(b"  word ");
        out.write(self.slice(name.span));
        out.write(b"\n");

        if self.look.kind == TokenKind::PunctLParen {
            out.write(b"    sig ");
            self.dump_balanced(out, TokenKind::PunctLParen, TokenKind::PunctRParen, 2131)?;
            out.write(b"\n");
        }

        loop {
            match self.look.kind {
                TokenKind::KwRequires => {
                    self.bump();
                    out.write(b"    requires ");
                    self.dump_quotation_like(out, 2132)?;
                    out.write(b"\n");
                }
                TokenKind::KwEnsures => {
                    self.bump();
                    out.write(b"    ensures ");
                    self.dump_quotation_like(out, 2133)?;
                    out.write(b"\n");
                }
                _ => break,
            }
        }

        out.write(b"    body ");
        self.dump_terms_until(out, TokenKind::PunctSemi)?;
        self.expect(TokenKind::PunctSemi, 2134)?;
        self.bump();
        out.write(b"\n");
        Ok(())
    }

    fn parse_block_decl_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        let kind = self.look.kind;
        self.bump();

        out.write(b"  decl ");
        match kind {
            TokenKind::KwStruct => out.write(b"struct "),
            TokenKind::KwEnum => out.write(b"enum "),
            TokenKind::KwRegisterMap => out.write(b"register-map "),
            _ => out.write(b"block "),
        }

        if self.look.kind == TokenKind::Ident {
            out.write(self.slice(self.look.span));
            self.bump();
        }
        out.write(b"\n");

        self.skip_until_end_semi()?;
        Ok(())
    }

    fn parse_semi_decl_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        let kind = self.look.kind;
        self.bump();

        out.write(b"  decl ");
        match kind {
            TokenKind::KwType => out.write(b"type "),
            TokenKind::KwSubtype => out.write(b"subtype "),
            TokenKind::KwConst => out.write(b"const "),
            TokenKind::KwResource => out.write(b"resource "),
            _ => out.write(b"stmt "),
        }

        if self.look.kind == TokenKind::Ident {
            out.write(self.slice(self.look.span));
            self.bump();
        }
        out.write(b"\n");

        self.skip_until_semi()?;
        Ok(())
    }

    fn parse_unknown_stmt_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        out.write(b"  stmt ");
        out.write(self.slice(self.look.span));
        out.write(b"\n");
        self.bump();
        // attempt to resync to ';' or next top-level item
        let _ = self.skip_until_semi();
        Ok(())
    }

    fn dump_terms_until(&mut self, out: &mut impl Output, stop: TokenKind) -> Result<(), ParseError> {
        let mut first = true;
        while self.look.kind != stop && self.look.kind != TokenKind::Eof {
            if !first {
                out.write(b" ");
            }
            first = false;
            match self.look.kind {
                TokenKind::PunctLBracket => {
                    self.dump_balanced(out, TokenKind::PunctLBracket, TokenKind::PunctRBracket, 2140)?;
                }
                TokenKind::PunctLBrace => {
                    self.dump_balanced(out, TokenKind::PunctLBrace, TokenKind::PunctRBrace, 2141)?;
                }
                TokenKind::PunctLParen => {
                    self.dump_balanced(out, TokenKind::PunctLParen, TokenKind::PunctRParen, 2142)?;
                }
                _ => {
                    out.write(self.slice(self.look.span));
                    self.bump();
                }
            }
        }
        Ok(())
    }

    fn dump_quotation_like(&mut self, out: &mut impl Output, code: u32) -> Result<(), ParseError> {
        if self.look.kind == TokenKind::PunctLBracket {
            self.dump_balanced(out, TokenKind::PunctLBracket, TokenKind::PunctRBracket, code)
        } else {
            Err(ParseError {
                code,
                span: self.look.span,
            })
        }
    }

    fn capture_balanced(
        &mut self,
        open: TokenKind,
        close: TokenKind,
        code: u32,
    ) -> Result<Span, ParseError> {
        let open_tok = self.expect(open, code)?;
        let start = open_tok.span.start;
        self.bump();

        let mut depth = 1usize;
        let mut end = open_tok.span.end;
        while depth > 0 {
            let tok = self.look;
            if tok.kind == TokenKind::Eof {
                return Err(ParseError { code, span: tok.span });
            }
            self.bump();
            if tok.kind == open {
                depth += 1;
            } else if tok.kind == close {
                depth -= 1;
                if depth == 0 {
                    end = tok.span.end;
                    break;
                }
            }
        }
        Ok(Span::new(start, end))
    }

    fn capture_quotation(&mut self, code: u32) -> Result<Span, ParseError> {
        self.capture_balanced(TokenKind::PunctLBracket, TokenKind::PunctRBracket, code)
    }

    fn dump_balanced(
        &mut self,
        out: &mut impl Output,
        open: TokenKind,
        close: TokenKind,
        code: u32,
    ) -> Result<(), ParseError> {
        let open_tok = self.expect(open, code)?;
        out.write(self.slice(open_tok.span));
        self.bump();

        let mut depth = 1usize;
        while depth > 0 {
            let tok = self.look;
            if tok.kind == TokenKind::Eof {
                return Err(ParseError { code, span: tok.span });
            }

            out.write(b" ");

            out.write(self.slice(tok.span));
            self.bump();

            if tok.kind == open {
                depth += 1;
            } else if tok.kind == close {
                depth -= 1;
            }
        }
        Ok(())
    }

    fn skip_until_semi(&mut self) -> Result<(), ParseError> {
        let mut depth_paren = 0usize;
        let mut depth_brace = 0usize;
        let mut depth_bracket = 0usize;
        while self.look.kind != TokenKind::Eof {
            match self.look.kind {
                TokenKind::PunctLParen => depth_paren += 1,
                TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                TokenKind::PunctLBrace => depth_brace += 1,
                TokenKind::PunctRBrace => depth_brace = depth_brace.saturating_sub(1),
                TokenKind::PunctLBracket => depth_bracket += 1,
                TokenKind::PunctRBracket => depth_bracket = depth_bracket.saturating_sub(1),
                TokenKind::PunctSemi if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                    self.bump();
                    return Ok(());
                }
                _ => {}
            }
            self.bump();
        }
        Err(ParseError {
            code: 2199,
            span: self.look.span,
        })
    }

    fn skip_until_end_semi(&mut self) -> Result<(), ParseError> {
        let mut depth_paren = 0usize;
        let mut depth_brace = 0usize;
        let mut depth_bracket = 0usize;
        while self.look.kind != TokenKind::Eof {
            match self.look.kind {
                TokenKind::PunctLParen => depth_paren += 1,
                TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                TokenKind::PunctLBrace => depth_brace += 1,
                TokenKind::PunctRBrace => depth_brace = depth_brace.saturating_sub(1),
                TokenKind::PunctLBracket => depth_bracket += 1,
                TokenKind::PunctRBracket => depth_bracket = depth_bracket.saturating_sub(1),
                TokenKind::KwEnd if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                    self.bump();
                    self.expect(TokenKind::PunctSemi, 2160)?;
                    self.bump();
                    return Ok(());
                }
                _ => {}
            }
            self.bump();
        }
        Err(ParseError {
            code: 2161,
            span: self.look.span,
        })
    }

    fn bump(&mut self) -> Token {
        let prev = self.look;
        self.look = self.lex.next();
        prev
    }

    fn expect(&self, kind: TokenKind, code: u32) -> Result<Token, ParseError> {
        if self.look.kind == kind {
            Ok(self.look)
        } else {
            Err(ParseError {
                code,
                span: self.look.span,
            })
        }
    }

    fn slice(&self, span: Span) -> &'a [u8] {
        &self.lex.src()[span.start..span.end]
    }
}

struct NullOut;

impl Output for NullOut {
    fn write(&mut self, _bytes: &[u8]) {}
}

fn parse_i64(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let mut sign = 1i64;
    if bytes[0] == b'-' {
        sign = -1;
        i = 1;
    }
    let mut v: i64 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'_' {
            i += 1;
            continue;
        }
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?;
        v = v.checked_add((b - b'0') as i64)?;
        i += 1;
    }
    Some(v * sign)
}

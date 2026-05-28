mod ast;
mod decl;

pub use ast::{
    Output, ParseError, DeclKind, SubtypeAst, RegMapInstanceAst, ImportAst,
    StructFieldAst, EnumVariantAst, StructDeclAst, EnumDeclAst, DeclAst, ModuleAst,
};

use crate::{
    fixed::FixedVec,
    lex::Lexer,
    span::Span,
    token::{Token, TokenKind},
};

pub struct Parser<'a> {
    pub(super) lex: Lexer<'a>,
    pub(super) look: Token,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        let mut lex = Lexer::new(src);
        let look = lex.next();
        Self { lex, look }
    }

    pub fn parse_module_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.expect(TokenKind::KwModule, ParseError::ExpectedModule { span: self.look.span })?;
        self.bump();

        let name = self.expect(TokenKind::Ident, ParseError::ExpectedModuleName { span: self.look.span })?;
        self.bump();

        self.expect(TokenKind::PunctSemi, ParseError::ExpectedSemiAfterModule { span: self.look.span })?;
        self.bump();

        out.write(b"module ");
        out.write(self.slice(name.span));
        out.write(b"\n");

        while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
            self.parse_item_dump(out)?;
        }

        self.expect(TokenKind::KwEnd, ParseError::ExpectedEnd { span: self.look.span })?;
        self.bump();
        self.expect(TokenKind::PunctSemi, ParseError::ExpectedSemiAfterEnd { span: self.look.span })?;
        self.bump();

        Ok(())
    }

    pub fn parse_module_ast(&mut self) -> Result<ModuleAst, ParseError> {
        self.expect(TokenKind::KwModule, ParseError::ExpectedModule { span: self.look.span })?;
        self.bump();
        let name = self.expect(TokenKind::Ident, ParseError::ExpectedModuleName { span: self.look.span })?;
        self.bump();
        self.expect(TokenKind::PunctSemi, ParseError::ExpectedSemiAfterModule { span: self.look.span })?;
        self.bump();

        let mut ast = ModuleAst {
            name: name.span,
            imports: FixedVec::new(),
            exports: FixedVec::new(),
            decls: FixedVec::new(),
            has_export_stmt: false,
            subtypes: FixedVec::new(),
            instances: FixedVec::new(),
            structs: FixedVec::new(),
            enums: FixedVec::new(),
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
                    let (decl, sdecl) = self.parse_struct_decl_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                    let _ = ast.structs.push(sdecl);
                }
                TokenKind::KwEnum => {
                    let (decl, edecl) = self.parse_enum_decl_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                    let _ = ast.enums.push(edecl);
                }
                TokenKind::KwRegisterMap => {
                    let decl = self.parse_register_map_decl_ast(&mut pending_attrs)?;
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
                    let (decl, inst) = self.parse_const_decl_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                    if let Some(inst) = inst {
                        let _ = ast.instances.push(inst);
                    }
                }
                TokenKind::KwResource => {
                    let decl = self.parse_resource_decl_ast(&mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwOwned => {
                    let decl = self.parse_semi_decl_ast(DeclKind::Owned, &mut pending_attrs)?;
                    let _ = ast.decls.push(decl);
                }
                TokenKind::KwIso => {
                    let decl = self.parse_semi_decl_ast(DeclKind::Iso, &mut pending_attrs)?;
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

        self.expect(TokenKind::KwEnd, ParseError::ExpectedEnd { span: self.look.span })?;
        self.bump();
        self.expect(TokenKind::PunctSemi, ParseError::ExpectedSemiAfterEnd { span: self.look.span })?;
        self.bump();

        Ok(ast)
    }

    pub(super) fn parse_item_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
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
            | TokenKind::KwResource
            | TokenKind::KwOwned
            | TokenKind::KwIso => self.parse_semi_decl_dump(out),
            _ => self.parse_unknown_stmt_dump(out),
        }
    }

    fn parse_import_ast(&mut self) -> Result<ImportAst, ParseError> {
        self.bump(); // import
        let name = self.expect(TokenKind::Ident, ParseError::ExpectedImportName { span: self.look.span })?;
        self.bump();

        let mut names: FixedVec<Span, 64> = FixedVec::new();
        if self.look.kind == TokenKind::PunctLBrace {
            self.bump();
            while self.look.kind != TokenKind::PunctRBrace && self.look.kind != TokenKind::Eof {
                if self.look.kind == TokenKind::Ident {
                    let q = self.capture_qualified_name(ParseError::ExpectedQualIdent { span: self.look.span })?;
                    let _ = names.push(q);
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, ParseError::ExpectedRBrace { span: self.look.span })?;
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
                    let q = self.capture_qualified_name(ParseError::ExpectedExportName { span: self.look.span })?;
                    let _ = ast.exports.push(q);
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(TokenKind::PunctRBrace, ParseError::ExpectedRBraceExport { span: self.look.span })?;
            self.bump();
        } else if self.look.kind == TokenKind::Ident {
            let q = self.capture_qualified_name(ParseError::ExpectedExportName { span: self.look.span })?;
            let _ = ast.exports.push(q);
        }

        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }
        Ok(())
    }

    pub(super) fn capture_qualified_name(&mut self, err: ParseError) -> Result<Span, ParseError> {
        let first = self.expect(TokenKind::Ident, err)?;
        let start = first.span.start;
        let mut end = first.span.end;
        self.bump();
        while self.look.kind == TokenKind::PunctDot {
            self.bump(); // dot
            let seg = self.expect(TokenKind::Ident, err)?;
            end = seg.span.end;
            self.bump();
        }
        Ok(Span::new(start, end))
    }

    pub(super) fn dump_terms_until(&mut self, out: &mut impl Output, stop: TokenKind) -> Result<(), ParseError> {
        let mut first = true;
        while self.look.kind != stop && self.look.kind != TokenKind::Eof {
            if !first {
                out.write(b" ");
            }
            first = false;
            match self.look.kind {
                TokenKind::PunctLBracket => {
                    self.dump_balanced(out, TokenKind::PunctLBracket, TokenKind::PunctRBracket, ParseError::UnmatchedBracket { span: self.look.span })?;
                }
                TokenKind::PunctLBrace => {
                    self.dump_balanced(out, TokenKind::PunctLBrace, TokenKind::PunctRBrace, ParseError::UnmatchedBrace { span: self.look.span })?;
                }
                TokenKind::PunctLParen => {
                    self.dump_balanced(out, TokenKind::PunctLParen, TokenKind::PunctRParen, ParseError::UnmatchedParen { span: self.look.span })?;
                }
                _ => {
                    out.write(self.slice(self.look.span));
                    self.bump();
                }
            }
        }
        Ok(())
    }

    pub(super) fn dump_quotation_like(&mut self, out: &mut impl Output, err: ParseError) -> Result<(), ParseError> {
        if self.look.kind == TokenKind::PunctLBracket {
            self.dump_balanced(out, TokenKind::PunctLBracket, TokenKind::PunctRBracket, err)
        } else {
            Err(err)
        }
    }

    pub(super) fn capture_balanced(
        &mut self,
        open: TokenKind,
        close: TokenKind,
        err: ParseError,
    ) -> Result<Span, ParseError> {
        let open_tok = self.expect(open, err)?;
        let start = open_tok.span.start;
        self.bump();

        let mut depth = 1usize;
        let mut end = open_tok.span.end;
        while depth > 0 {
            let tok = self.look;
            if tok.kind == TokenKind::Eof {
                return Err(err);
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

    pub(super) fn capture_quotation(&mut self, err: ParseError) -> Result<Span, ParseError> {
        self.capture_balanced(TokenKind::PunctLBracket, TokenKind::PunctRBracket, err)
    }

    pub(super) fn dump_balanced(
        &mut self,
        out: &mut impl Output,
        open: TokenKind,
        close: TokenKind,
        err: ParseError,
    ) -> Result<(), ParseError> {
        let open_tok = self.expect(open, err)?;
        out.write(self.slice(open_tok.span));
        self.bump();

        let mut depth = 1usize;
        while depth > 0 {
            let tok = self.look;
            if tok.kind == TokenKind::Eof {
                return Err(err);
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

    pub(super) fn skip_until_semi(&mut self) -> Result<(), ParseError> {
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
        Err(ParseError::ExpectedSemiSkip { span: self.look.span })
    }

    pub(super) fn skip_until_end_semi(&mut self) -> Result<(), ParseError> {
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
                    self.expect(TokenKind::PunctSemi, ParseError::ExpectedSemiOrEnd { span: self.look.span })?;
                    self.bump();
                    return Ok(());
                }
                _ => {}
            }
            self.bump();
        }
        Err(ParseError::ExpectedEndSemi { span: self.look.span })
    }

    pub(super) fn bump(&mut self) -> Token {
        let prev = self.look;
        self.look = self.lex.next();
        prev
    }

    pub(super) fn expect(&self, kind: TokenKind, err: ParseError) -> Result<Token, ParseError> {
        if self.look.kind == kind {
            Ok(self.look)
        } else {
            Err(err)
        }
    }

    pub(super) fn slice(&self, span: Span) -> &'a [u8] {
        &self.lex.src()[span.start..span.end]
    }
}

pub(super) struct NullOut;

impl Output for NullOut {
    fn write(&mut self, _bytes: &[u8]) {}
}

pub(super) fn parse_i64(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let mut sign = 1i64;
    if bytes[0] == b'-' {
        sign = -1;
        i = 1;
    }
    if i >= bytes.len() {
        return None;
    }

    let (radix, mut j) = if i + 1 < bytes.len() && bytes[i] == b'0' {
        match bytes[i + 1] {
            b'x' | b'X' => (16i64, i + 2),
            b'b' | b'B' => (2i64, i + 2),
            _ => (10i64, i),
        }
    } else {
        (10i64, i)
    };

    let mut v: i64 = 0;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'_' {
            j += 1;
            continue;
        }
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as i64,
            b'a'..=b'f' if radix == 16 => (b - b'a') as i64 + 10,
            b'A'..=b'F' if radix == 16 => (b - b'A') as i64 + 10,
            _ => return None,
        };
        if digit >= radix {
            return None;
        }
        v = v.checked_mul(radix)?;
        v = v.checked_add(digit)?;
        j += 1;
    }
    Some(v * sign)
}

pub(super) fn effect_has_suspend(effect_token: &[u8]) -> bool {
    // token includes "!{...}"
    if effect_token.len() < 4 {
        return false;
    }
    // Scan for substring "suspend"
    let needle = b"suspend";
    effect_token
        .windows(needle.len())
        .any(|w| w == needle)
}

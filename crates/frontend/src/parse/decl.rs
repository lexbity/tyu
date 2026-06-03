use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_word_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // :
        let name_span = self.capture_qualified_name(ParseError::ExpectedWordName {
            span: self.look.span,
        })?;

        let sig = if self.look.kind == TokenKind::PunctLParen {
            Some(self.capture_balanced(
                TokenKind::PunctLParen,
                TokenKind::PunctRParen,
                ParseError::ExpectedSigParen {
                    span: self.look.span,
                },
            )?)
        } else {
            None
        };

        let mut effect_bits = 0u16;
        let mut effect_net: i16 = 0;
        let mut effect_high: u32 = 0;
        if self.look.kind == TokenKind::EffectSet {
            let (bits, net, high) = parse_effect_bits(self.slice(self.look.span));
            effect_bits = bits;
            effect_net = net;
            effect_high = high;
            self.bump();
        }

        let mut requires: Option<Span> = None;
        let mut ensures: Option<Span> = None;
        while let TokenKind::KwRequires | TokenKind::KwEnsures = self.look.kind {
            let is_requires = self.look.kind == TokenKind::KwRequires;
            self.bump();
            let q = self.capture_quotation(ParseError::ExpectedQuotation {
                span: self.look.span,
            })?;
            if is_requires {
                requires = Some(q);
            } else {
                ensures = Some(q);
            }
        }

        let body_start = self.look.span.start;
        self.dump_terms_until(&mut NullOut, TokenKind::PunctSemi)?;
        let body_end = self.look.span.start;
        self.expect(
            TokenKind::PunctSemi,
            ParseError::ExpectedSemi {
                span: self.look.span,
            },
        )?;
        self.bump();

        let attrs = core::mem::take(pending_attrs);
        Ok(DeclAst {
            kind: DeclKind::Word,
            name: name_span,
            sig,
            attrs,
            body: Some(Span::new(body_start, body_end)),
            requires,
            ensures,
            effect_bits,
            effect_net,
            effect_high,
        })
    }

    pub(super) fn parse_struct_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<(DeclAst, StructDeclAst), ParseError> {
        self.bump(); // struct
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedEnumName {
                span: self.look.span,
            },
        )?;
        self.bump();

        let mut fields: FixedVec<StructFieldAst, 32> = FixedVec::new();
        while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
            if self.look.kind != TokenKind::Ident {
                self.bump();
                continue;
            }

            // Heuristic: treat `Ident :` as a field starter.
            let field_name = self.look;
            let mut probe = self.lex;
            let next = probe.next();
            if next.kind != TokenKind::PunctColon {
                self.bump();
                continue;
            }

            self.bump(); // field name
            self.bump(); // ':'
            let start = self.look.span.start;
            let mut end = start;

            let mut depth_paren = 0usize;
            let mut depth_bracket = 0usize;
            loop {
                if self.look.kind == TokenKind::Eof || self.look.kind == TokenKind::KwEnd {
                    break;
                }

                // If we see `Ident :` at top-level, that's the next field.
                if depth_paren == 0 && depth_bracket == 0 && self.look.kind == TokenKind::Ident {
                    let mut probe = self.lex;
                    let next = probe.next();
                    if next.kind == TokenKind::PunctColon {
                        break;
                    }
                }

                match self.look.kind {
                    TokenKind::PunctLParen => depth_paren += 1,
                    TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                    TokenKind::PunctLBracket => depth_bracket += 1,
                    TokenKind::PunctRBracket => depth_bracket = depth_bracket.saturating_sub(1),
                    _ => {}
                }
                end = self.look.span.end;
                self.bump();
            }

            fields
                .push(StructFieldAst {
                    name: field_name.span,
                    ty: Span::new(start, end),
                })
                .map_err(|_| ParseError::TooManyItems {
                    span: self.look.span,
                })?;
        }

        self.expect(
            TokenKind::KwEnd,
            ParseError::ExpectedEnd {
                span: self.look.span,
            },
        )?;
        self.bump();
        self.expect(
            TokenKind::PunctSemi,
            ParseError::ExpectedSemiOrEnd {
                span: self.look.span,
            },
        )?;
        self.bump();

        let attrs = core::mem::take(pending_attrs);
        let decl = DeclAst {
            kind: DeclKind::Struct,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        };
        let sdecl = StructDeclAst {
            name: name.span,
            fields,
        };
        Ok((decl, sdecl))
    }

    pub(super) fn parse_enum_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<(DeclAst, EnumDeclAst), ParseError> {
        self.bump(); // enum
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedEnumName {
                span: self.look.span,
            },
        )?;
        self.bump();

        // Minimal v1 parsing: `enum Name : BaseTy ... end;`
        let mut base_ty: Option<Span> = None;
        if self.look.kind == TokenKind::PunctColon {
            self.bump();
            let start = self.look.span.start;
            let mut end = start;
            let mut depth_paren = 0usize;
            while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
                // Stop the base type when we hit the first top-level `Variant = ...`.
                if depth_paren == 0 && self.look.kind == TokenKind::Ident {
                    let mut probe = self.lex;
                    let next = probe.next();
                    if next.kind == TokenKind::PunctEq {
                        break;
                    }
                }
                match self.look.kind {
                    TokenKind::PunctLParen => depth_paren += 1,
                    TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                    _ => {}
                }
                end = self.look.span.end;
                self.bump();
            }
            if end > start {
                base_ty = Some(Span::new(start, end));
            }
        }

        let mut variants: FixedVec<EnumVariantAst, 64> = FixedVec::new();
        while self.look.kind != TokenKind::KwEnd && self.look.kind != TokenKind::Eof {
            if self.look.kind != TokenKind::Ident {
                self.bump();
                continue;
            }
            let vname = self.look;
            self.bump();
            self.expect(
                TokenKind::PunctEq,
                ParseError::ExpectedEq {
                    span: self.look.span,
                },
            )?;
            self.bump();
            let vnum = self.expect(
                TokenKind::Number,
                ParseError::ExpectedNumber {
                    span: self.look.span,
                },
            )?;
            let val = parse_i64(self.slice(vnum.span))
                .ok_or(ParseError::InvalidInteger { span: vnum.span })?;
            self.bump();
            variants
                .push(EnumVariantAst {
                    name: vname.span,
                    value: val,
                })
                .map_err(|_| ParseError::TooManyItems {
                    span: self.look.span,
                })?;
        }

        self.expect(
            TokenKind::KwEnd,
            ParseError::ExpectedEnd {
                span: self.look.span,
            },
        )?;
        self.bump();
        self.expect(
            TokenKind::PunctSemi,
            ParseError::ExpectedSemiOrEnd {
                span: self.look.span,
            },
        )?;
        self.bump();

        let attrs = core::mem::take(pending_attrs);
        let decl = DeclAst {
            kind: DeclKind::Enum,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        };
        let edecl = EnumDeclAst {
            name: name.span,
            base: base_ty,
            variants,
        };
        Ok((decl, edecl))
    }

    pub(super) fn parse_semi_decl_ast(
        &mut self,
        kind: DeclKind,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // keyword already matched by caller
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedFieldIdent {
                span: self.look.span,
            },
        )?;
        self.bump();
        self.skip_until_semi()?;
        let attrs = core::mem::take(pending_attrs);
        Ok(DeclAst {
            kind,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        })
    }

    pub(super) fn parse_resource_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // resource
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedFieldIdent {
                span: self.look.span,
            },
        )?;
        self.bump();

        // Minimal v1 parsing: `resource NAME : Type ... ;`
        // Store the `Type` span in `sig` (reusing `sig` field for non-word decls).
        let mut ty_span: Option<Span> = None;
        if self.look.kind == TokenKind::PunctColon {
            self.bump();
            let start = self.look.span.start;
            let mut end = start;
            let mut depth_paren = 0usize;
            while self.look.kind != TokenKind::PunctSemi && self.look.kind != TokenKind::Eof {
                // stop at '=' (initializer) or 'ceiling' (attributes)
                if depth_paren == 0 {
                    if self.look.kind == TokenKind::PunctEq {
                        break;
                    }
                    if self.look.kind == TokenKind::Ident
                        && self.slice(self.look.span) == b"ceiling"
                    {
                        break;
                    }
                }
                match self.look.kind {
                    TokenKind::PunctLParen => depth_paren += 1,
                    TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                    _ => {}
                }
                end = self.look.span.end;
                self.bump();
            }
            if end > start {
                ty_span = Some(Span::new(start, end));
            }
        }

        let _ = self.skip_until_semi();
        let attrs = core::mem::take(pending_attrs);
        Ok(DeclAst {
            kind: DeclKind::Resource,
            name: name.span,
            sig: ty_span,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        })
    }

    pub(super) fn parse_const_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<(DeclAst, Option<RegMapInstanceAst>), ParseError> {
        self.bump(); // const
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedConstName {
                span: self.look.span,
            },
        )?;
        self.bump();

        // attempt to parse: "= MAP @ <num> ;"
        let mut inst: Option<RegMapInstanceAst> = None;
        if self.look.kind == TokenKind::PunctEq {
            self.bump();
            if self.look.kind == TokenKind::Ident {
                let map = self.look.span;
                self.bump();
                if self.look.kind == TokenKind::Ident && self.slice(self.look.span) == b"@" {
                    self.bump();
                    if self.look.kind == TokenKind::Number {
                        let base_addr = self.look.span;
                        self.bump();
                        if self.look.kind == TokenKind::PunctSemi {
                            inst = Some(RegMapInstanceAst {
                                name: name.span,
                                map,
                                base_addr,
                            });
                        }
                    }
                }
            }
        }
        // regardless of whether pattern matched, skip to ';'
        let _ = self.skip_until_semi();

        let attrs = core::mem::take(pending_attrs);
        let decl = DeclAst {
            kind: DeclKind::Const,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        };
        Ok((decl, inst))
    }

    pub(super) fn parse_register_map_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<DeclAst, ParseError> {
        self.bump(); // register-map
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedRegisterName {
                span: self.look.span,
            },
        )?;
        self.bump();

        let body_start = self.look.span.start;
        let mut depth_paren = 0usize;
        let mut depth_brace = 0usize;
        let mut depth_bracket = 0usize;
        let body_end;
        loop {
            if self.look.kind == TokenKind::Eof {
                return Err(ParseError::ExpectedEndSemi {
                    span: self.look.span,
                });
            }
            match self.look.kind {
                TokenKind::PunctLParen => depth_paren += 1,
                TokenKind::PunctRParen => depth_paren = depth_paren.saturating_sub(1),
                TokenKind::PunctLBrace => depth_brace += 1,
                TokenKind::PunctRBrace => depth_brace = depth_brace.saturating_sub(1),
                TokenKind::PunctLBracket => depth_bracket += 1,
                TokenKind::PunctRBracket => depth_bracket = depth_bracket.saturating_sub(1),
                TokenKind::KwEnd if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                    body_end = self.look.span.start;
                    self.bump();
                    self.expect(
                        TokenKind::PunctSemi,
                        ParseError::ExpectedSemiOrEnd {
                            span: self.look.span,
                        },
                    )?;
                    self.bump();
                    break;
                }
                _ => {}
            }
            self.bump();
        }

        let attrs = core::mem::take(pending_attrs);
        Ok(DeclAst {
            kind: DeclKind::RegisterMap,
            name: name.span,
            sig: None,
            attrs,
            body: Some(Span::new(body_start, body_end)),
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        })
    }

    pub(super) fn parse_subtype_decl_ast(
        &mut self,
        pending_attrs: &mut FixedVec<Span, 16>,
    ) -> Result<(DeclAst, Option<SubtypeAst>), ParseError> {
        self.bump(); // subtype
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedSubtypeName {
                span: self.look.span,
            },
        )?;
        self.bump();
        self.expect(
            TokenKind::PunctEq,
            ParseError::ExpectedEqSubtype {
                span: self.look.span,
            },
        )?;
        self.bump();
        let base = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedBaseType {
                span: self.look.span,
            },
        )?;
        self.bump();

        // Expect "range" keyword as ident.
        if self.look.kind != TokenKind::Ident || self.slice(self.look.span) != b"range" {
            return Err(ParseError::ExpectedRangeKeyword {
                span: self.look.span,
            });
        }
        self.bump();

        let min_tok = self.expect(
            TokenKind::Number,
            ParseError::ExpectedRangeMin {
                span: self.look.span,
            },
        )?;
        let min = parse_i64(self.slice(min_tok.span))
            .ok_or(ParseError::InvalidRangeMin { span: min_tok.span })?;
        self.bump();
        self.expect(
            TokenKind::PunctDblDot,
            ParseError::ExpectedDblDot {
                span: self.look.span,
            },
        )?;
        self.bump();
        let max_tok = self.expect(
            TokenKind::Number,
            ParseError::ExpectedRangeMax {
                span: self.look.span,
            },
        )?;
        let max = parse_i64(self.slice(max_tok.span))
            .ok_or(ParseError::InvalidRangeMax { span: max_tok.span })?;
        self.bump();
        self.expect(
            TokenKind::PunctSemi,
            ParseError::ExpectedSemiSubtype {
                span: self.look.span,
            },
        )?;
        self.bump();

        let attrs = core::mem::take(pending_attrs);
        let decl = DeclAst {
            kind: DeclKind::Subtype,
            name: name.span,
            sig: None,
            attrs,
            body: None,
            requires: None,
            ensures: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
        };
        let st = Some(SubtypeAst {
            name: name.span,
            base: base.span,
            min,
            max,
        });
        Ok((decl, st))
    }

    pub(super) fn parse_import_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.bump();
        let name = self.expect(
            TokenKind::Ident,
            ParseError::ExpectedImportName {
                span: self.look.span,
            },
        )?;
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
                    let q = self.capture_qualified_name(ParseError::ExpectedQualIdent {
                        span: self.look.span,
                    })?;
                    out.write(self.slice(q));
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                // recovery: skip unknown tokens
                self.bump();
            }
            self.expect(
                TokenKind::PunctRBrace,
                ParseError::ExpectedRBrace {
                    span: self.look.span,
                },
            )?;
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

    pub(super) fn parse_export_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
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
                    let q = self.capture_qualified_name(ParseError::ExpectedExportName {
                        span: self.look.span,
                    })?;
                    out.write(self.slice(q));
                    continue;
                }
                if self.look.kind == TokenKind::PunctComma {
                    self.bump();
                    continue;
                }
                self.bump();
            }
            self.expect(
                TokenKind::PunctRBrace,
                ParseError::ExpectedRBraceExport {
                    span: self.look.span,
                },
            )?;
            self.bump();
            out.write(b"}");
        } else if self.look.kind == TokenKind::Ident {
            let q = self.capture_qualified_name(ParseError::ExpectedExportName {
                span: self.look.span,
            })?;
            out.write(self.slice(q));
        }

        if self.look.kind == TokenKind::PunctSemi {
            self.bump();
        }
        out.write(b"\n");
        Ok(())
    }

    pub(super) fn parse_word_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        self.bump(); // :
        let name = self.capture_qualified_name(ParseError::ExpectedWordName {
            span: self.look.span,
        })?;

        out.write(b"  word ");
        out.write(self.slice(name));
        out.write(b"\n");

        if self.look.kind == TokenKind::PunctLParen {
            out.write(b"    sig ");
            self.dump_balanced(
                out,
                TokenKind::PunctLParen,
                TokenKind::PunctRParen,
                ParseError::ExpectedSigParen {
                    span: self.look.span,
                },
            )?;
            out.write(b"\n");
        }

        loop {
            match self.look.kind {
                TokenKind::KwRequires => {
                    self.bump();
                    out.write(b"    requires ");
                    self.dump_quotation_like(
                        out,
                        ParseError::ExpectedQuotation {
                            span: self.look.span,
                        },
                    )?;
                    out.write(b"\n");
                }
                TokenKind::KwEnsures => {
                    self.bump();
                    out.write(b"    ensures ");
                    self.dump_quotation_like(
                        out,
                        ParseError::ExpectedQuotationEffect {
                            span: self.look.span,
                        },
                    )?;
                    out.write(b"\n");
                }
                _ => break,
            }
        }

        out.write(b"    body ");
        self.dump_terms_until(out, TokenKind::PunctSemi)?;
        self.expect(
            TokenKind::PunctSemi,
            ParseError::ExpectedSemi {
                span: self.look.span,
            },
        )?;
        self.bump();
        out.write(b"\n");
        Ok(())
    }

    pub(super) fn parse_block_decl_dump(
        &mut self,
        out: &mut impl Output,
    ) -> Result<(), ParseError> {
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

    pub(super) fn parse_semi_decl_dump(&mut self, out: &mut impl Output) -> Result<(), ParseError> {
        let kind = self.look.kind;
        self.bump();

        out.write(b"  decl ");
        match kind {
            TokenKind::KwType => out.write(b"type "),
            TokenKind::KwSubtype => out.write(b"subtype "),
            TokenKind::KwConst => out.write(b"const "),
            TokenKind::KwResource => out.write(b"resource "),
            TokenKind::KwOwned => out.write(b"owned "),
            TokenKind::KwIso => out.write(b"iso "),
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

    pub(super) fn parse_unknown_stmt_dump(
        &mut self,
        out: &mut impl Output,
    ) -> Result<(), ParseError> {
        out.write(b"  stmt ");
        out.write(self.slice(self.look.span));
        out.write(b"\n");
        self.bump();
        // attempt to resync to ';' or next top-level item
        let _ = self.skip_until_semi();
        Ok(())
    }
}

use frontend::{
    parse::{DeclKind, ModuleAst, ParseError, Parser},
    span::Span,
};

/// Parse a module source, returning either the AST or the error.
fn parse(src: &str) -> Result<ModuleAst, ParseError> {
    Parser::new(src.as_bytes()).parse_module_ast()
}

/// Assert that parsing succeeds and returns an AST with the given name.
fn assert_parse_ok(src: &str) -> ModuleAst {
    parse(src).expect("expected parse to succeed")
}

/// Assert that parsing fails with an error matching the given pattern.
macro_rules! assert_parse_err {
    ($src:expr, $pattern:pat) => {
        match parse($src) {
            Err($pattern) => {} // ok
            Err(e) => panic!(
                "error variant mismatch for input: {:?}\n  actual: {:?}",
                $src, e
            ),
            Ok(_) => panic!("expected parse to fail for input: {:?}", $src),
        }
    };
}

/// Assert span positions match
fn assert_span(span: Span, start: usize, end: usize) {
    assert_eq!(span.start, start, "span.start");
    assert_eq!(span.end, end, "span.end");
}

// ---------------------------------------------------------------------------
// Minimal valid module (baseline for all parse_success tests)
// ---------------------------------------------------------------------------

const MINIMAL: &str = "module m; end;";

#[test]
fn minimal_module() {
    let ast = assert_parse_ok(MINIMAL);
    assert_span(ast.name, 7, 8);
    assert_eq!(ast.imports.len(), 0);
    assert_eq!(ast.decls.len(), 0);
}

// ---------------------------------------------------------------------------
// Module header errors
// ---------------------------------------------------------------------------

#[test]
fn err_module_missing_kw() {
    assert_parse_err!("end;", ParseError::ExpectedModule { .. });
}

#[test]
fn err_module_after_kw() {
    assert_parse_err!("module ; end;", ParseError::ExpectedModuleName { .. });
}

#[test]
fn err_module_semi() {
    assert_parse_err!("module m\nend;", ParseError::ExpectedSemiAfterModule { .. });
}

// ---------------------------------------------------------------------------
// End marker errors
// ---------------------------------------------------------------------------

#[test]
fn err_end_missing() {
    assert_parse_err!("module m;", ParseError::ExpectedEnd { .. });
}

#[test]
fn err_end_semi() {
    assert_parse_err!("module m; end", ParseError::ExpectedSemiAfterEnd { .. });
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

#[test]
fn word_empty_body() {
    // Word with qualified name: `: name` — this parses as word with empty body
    let src = "module m; : foo ; end;";
    let ast = assert_parse_ok(src);
    assert_eq!(ast.decls.len(), 1);
    let d = &ast.decls.get(0).unwrap();
    assert_eq!(d.kind, DeclKind::Word);
}

#[test]
fn word_with_sig() {
    let ast = assert_parse_ok("module m; : foo ( i64 -- i64 ) ; end;");
    assert_eq!(ast.decls.len(), 1);
    let d = &ast.decls.get(0).unwrap();
    assert!(d.sig.is_some());
}

#[test]
fn word_with_sig_and_body() {
    let ast = assert_parse_ok("module m; : foo ( -- ) 42 ; end;");
    let d = &ast.decls.get(0).unwrap();
    assert!(d.sig.is_some());
    assert!(d.body.is_some());
}

#[test]
fn word_with_requires() {
    let ast = assert_parse_ok("module m; : foo requires [ 0 > ] ; end;");
    let d = &ast.decls.get(0).unwrap();
    assert!(d.requires.is_some());
}

#[test]
fn word_with_ensures() {
    let ast = assert_parse_ok("module m; : foo ensures [ 0 > ] ; end;");
    let d = &ast.decls.get(0).unwrap();
    assert!(d.ensures.is_some());
}

#[test]
fn word_with_requires_ensures() {
    let ast = assert_parse_ok("module m; : foo requires [ true ] ensures [ true ] ; end;");
    let d = &ast.decls.get(0).unwrap();
    assert!(d.requires.is_some());
    assert!(d.ensures.is_some());
}

#[test]
fn err_word_name() {
    assert_parse_err!("module m; : ; end;", ParseError::ExpectedWordName { .. });
}

#[test]
fn err_word_semi() {
    // word with body but no semi — body bounces until semi, then hits end marker issues
    let result = parse("module m; : foo 42 end;");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[test]
fn struct_empty() {
    let ast = assert_parse_ok("module m; struct S end; end;");
    assert_eq!(ast.structs.len(), 1);
    let s = &ast.structs.get(0).unwrap();
    assert_span(s.name, 17, 18);
    assert_eq!(s.fields.len(), 0);
}

#[test]
fn struct_one_field() {
    let ast = assert_parse_ok("module m; struct S x : i64 end; end;");
    let s = &ast.structs.get(0).unwrap();
    assert_eq!(s.fields.len(), 1);
    let f = &s.fields.get(0).unwrap();
    assert_span(f.name, 19, 20);
}

#[test]
fn struct_multi_field() {
    let src = "module m; struct S x : i64 y : bool end; end;";
    let ast = assert_parse_ok(src);
    assert_eq!(ast.structs.get(0).unwrap().fields.len(), 2);
}

#[test]
fn err_struct_name() {
    assert_parse_err!(
        "module m; struct ; end; end;",
        ParseError::ExpectedEnumName { .. }
    );
}

#[test]
fn err_struct_end_missing() {
    assert_parse_err!("module m; struct S ; end;", ParseError::ExpectedEnd { .. });
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[test]
fn enum_empty() {
    let ast = assert_parse_ok("module m; enum E end; end;");
    assert_eq!(ast.enums.len(), 1);
}

#[test]
fn enum_with_variant() {
    let src = "module m; enum E A = 0 end; end;";
    let ast = assert_parse_ok(src);
    let e = &ast.enums.get(0).unwrap();
    assert_eq!(e.variants.len(), 1);
    assert_eq!(e.variants.get(0).unwrap().value, 0);
}

#[test]
fn enum_with_base() {
    let ast = assert_parse_ok("module m; enum E : i64 A = 1 end; end;");
    let e = &ast.enums.get(0).unwrap();
    assert!(e.base.is_some());
}

#[test]
fn err_enum_name() {
    assert_parse_err!(
        "module m; enum ; end; end;",
        ParseError::ExpectedEnumName { .. }
    );
}

// ---------------------------------------------------------------------------
// Subtypes
// ---------------------------------------------------------------------------

#[test]
fn subtype_decl() {
    let ast = assert_parse_ok("module m; subtype Age = i64 range 0..150 ; end;");
    assert_eq!(ast.subtypes.len(), 1);
    let st = &ast.subtypes.get(0).unwrap();
    assert_span(st.name, 18, 21);
    assert_eq!(st.min, 0);
    assert_eq!(st.max, 150);
}

#[test]
fn err_subtype_name() {
    assert_parse_err!(
        "module m; subtype ; end;",
        ParseError::ExpectedSubtypeName { .. }
    );
}

#[test]
fn err_subtype_eq() {
    assert_parse_err!(
        "module m; subtype Age ; end;",
        ParseError::ExpectedEqSubtype { .. }
    );
}

#[test]
fn err_subtype_base() {
    assert_parse_err!(
        "module m; subtype Age = ; end;",
        ParseError::ExpectedBaseType { .. }
    );
}

#[test]
fn err_subtype_range_kw() {
    // Expect `range` keyword after base type; using `;` without `range` should fail
    assert_parse_err!(
        "module m; subtype Age = i64 ; end;",
        ParseError::ExpectedRangeKeyword { .. }
    );
}

// ---------------------------------------------------------------------------
// Const
// ---------------------------------------------------------------------------

#[test]
fn const_decl() {
    // Const syntax: `const NAME ;` for simple decl
    let ast = assert_parse_ok("module m; const FOO ; end;");
    assert_eq!(ast.decls.len(), 1);
}

#[test]
fn err_const_name() {
    assert_parse_err!(
        "module m; const ; end;",
        ParseError::ExpectedConstName { .. }
    );
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

#[test]
fn resource_decl() {
    let ast = assert_parse_ok("module m; resource MyResource ; end;");
    assert_eq!(ast.decls.len(), 1);
}

// ---------------------------------------------------------------------------
// Owned / Iso
// ---------------------------------------------------------------------------

#[test]
fn owned_decl() {
    let ast = assert_parse_ok("module m; owned MyOwned ; end;");
    assert_eq!(ast.decls.len(), 1);
    assert_eq!(ast.decls.get(0).unwrap().kind, DeclKind::Owned);
}

#[test]
fn iso_decl() {
    let ast = assert_parse_ok("module m; iso MyIso ; end;");
    assert_eq!(ast.decls.len(), 1);
    assert_eq!(ast.decls.get(0).unwrap().kind, DeclKind::Iso);
}

// ---------------------------------------------------------------------------
// Type (stub)
// ---------------------------------------------------------------------------

#[test]
fn type_decl() {
    let ast = assert_parse_ok("module m; type MyType ; end;");
    assert_eq!(ast.decls.len(), 1);
    assert_eq!(ast.decls.get(0).unwrap().kind, DeclKind::Type);
}

// ---------------------------------------------------------------------------
// Register-map
// ---------------------------------------------------------------------------

#[test]
fn register_map_decl() {
    let ast = assert_parse_ok("module m; register-map UART0 ( addr=0x1000 ) end; end;");
    assert_eq!(ast.decls.len(), 1);
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

#[test]
fn import_wildcard() {
    let ast = assert_parse_ok("module m; import Foo ; end;");
    assert_eq!(ast.imports.len(), 1);
    assert_span(ast.imports.get(0).unwrap().module, 17, 20);
    assert_eq!(ast.imports.get(0).unwrap().names.len(), 0);
}

#[test]
fn import_selective() {
    let ast = assert_parse_ok("module m; import Foo { bar, baz } ; end;");
    assert_eq!(ast.imports.len(), 1);
    assert_eq!(ast.imports.get(0).unwrap().names.len(), 2);
}

#[test]
fn err_import_name() {
    assert_parse_err!(
        "module m; import ; end;",
        ParseError::ExpectedImportName { .. }
    );
}

#[test]
fn err_import_rbrace() {
    assert_parse_err!(
        "module m; import Foo { ; end;",
        ParseError::ExpectedRBrace { .. }
    );
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

#[test]
fn export_list() {
    let ast = assert_parse_ok("module m; export { foo, bar } ; end;");
    assert!(ast.has_export_stmt);
    assert_eq!(ast.exports.len(), 2);
}

#[test]
fn err_export_rbrace() {
    assert_parse_err!(
        "module m; export { ; end;",
        ParseError::ExpectedRBraceExport { .. }
    );
}

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

#[test]
fn attribute_on_word() {
    let src = "module m; @export : foo ; end;";
    let ast = assert_parse_ok(src);
    let d = &ast.decls.get(0).unwrap();
    assert_eq!(d.attrs.len(), 1);
    // "@export" starts at byte 10, ends at 17
    assert_span(*d.attrs.get(0).unwrap(), 10, 17);
}

#[test]
fn attribute_on_struct() {
    let src = "module m; @export struct S end; end;";
    let ast = assert_parse_ok(src);
    let d = &ast.decls.get(0).unwrap();
    assert_eq!(d.attrs.len(), 1);
}

// ---------------------------------------------------------------------------
// Effect sets on word declarations
// ---------------------------------------------------------------------------

#[test]
fn word_no_effect() {
    let ast = assert_parse_ok("module m; : foo ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits, 0);
}

#[test]
fn word_effect_suspend() {
    let ast = assert_parse_ok("module m; : foo !{suspend} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits & 1, 1);
}

#[test]
fn word_effect_interrupt() {
    let ast = assert_parse_ok("module m; : foo !{interrupt} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits & 2, 2);
}

#[test]
fn word_effect_diverge() {
    let ast = assert_parse_ok("module m; : foo !{diverge} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits & 4, 4);
}

#[test]
fn word_effect_mmio() {
    let ast = assert_parse_ok("module m; : foo !{mmio} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits & 8, 8);
}

#[test]
fn word_effect_alloc() {
    let ast = assert_parse_ok("module m; : foo !{alloc} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits & 16, 16);
}

#[test]
fn word_effect_multiple() {
    let ast = assert_parse_ok("module m; : foo !{suspend, mmio} ; end;");
    let bits = ast.decls.get(0).unwrap().effect_bits;
    assert_eq!(bits & 1, 1); // suspend
    assert_eq!(bits & 8, 8); // mmio
    assert_eq!(bits & 2, 0); // no interrupt
}

#[test]
fn word_effect_unknown_name() {
    let ast = assert_parse_ok("module m; : foo !{unknown} ; end;");
    // unknown effect names are silently ignored (no bit set)
    assert_eq!(ast.decls.get(0).unwrap().effect_bits, 0);
}

#[test]
fn word_effect_empty() {
    let ast = assert_parse_ok("module m; : foo !{} ; end;");
    assert_eq!(ast.decls.get(0).unwrap().effect_bits, 0);
}

// ---------------------------------------------------------------------------
// Multiple words / ordering
// ---------------------------------------------------------------------------

#[test]
fn two_words() {
    let ast = assert_parse_ok("module m; : a ; : b ; end;");
    assert_eq!(ast.decls.len(), 2);
}

// ---------------------------------------------------------------------------
// Edge case: source with Unicode/UTF-8 (not ASCII)
// ---------------------------------------------------------------------------

#[test]
fn utf8_identifiers() {
    // The lexer accepts any byte that is_ascii_alphabetic or '_' '@' '!'
    // Non-ASCII bytes fall through to _ => TokenKind::Ident recovery.
    // So a UTF-8 encoding of 'é' (0xC3 0xA9) would be two ident tokens.
    // That's acceptable — just verify no panic.
    let result = parse("module m; : café ; end;");
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Unknown token recovery
// ---------------------------------------------------------------------------

#[test]
fn unknown_token_skipped() {
    // '^' and '%' are not recognized tokens — lexer returns Ident, parser skips
    let ast = assert_parse_ok("module m; ^ % ; end;");
    assert_eq!(ast.decls.len(), 0);
}

// ---------------------------------------------------------------------------
// Additional error variants
// ---------------------------------------------------------------------------

#[test]
fn err_word_sig_missing_rparen() {
    assert_parse_err!(
        "module m; : foo ( i64 ; end;",
        ParseError::ExpectedSigParen { .. }
    );
}

#[test]
fn err_register_map_name() {
    assert_parse_err!(
        "module m; register-map ; end; end;",
        ParseError::ExpectedRegisterName { .. }
    );
}

#[test]
fn err_unknown_kw_skipped() {
    // Unknown keyword `bogus` triggers recovery skip-until-semi
    let result = parse("module m; bogus ; end;");
    assert!(result.is_ok(), "parser should recover from unknown keyword");
    let ast = result.unwrap();
    assert_eq!(ast.decls.len(), 0);
}

// ---------------------------------------------------------------------------
// Error code coverage helpers
// ---------------------------------------------------------------------------

#[test]
fn error_codes_are_distinct() {
    // Verify no two error variants share the same code (just a sanity check)
    let codes = [
        ParseError::ExpectedModule {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedModuleName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemiAfterModule {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedEnd {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemiAfterEnd {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedImportName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRBrace {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedQualIdent {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRBraceExport {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedExportName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedWordName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSigParen {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedQuotation {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedQuotationEffect {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemi {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::UnmatchedBracket {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::UnmatchedBrace {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::UnmatchedParen {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedEnumName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedFieldIdent {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemiOrEnd {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedEndSemi {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedEq {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedNumber {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::InvalidInteger {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSubtypeName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedEqSubtype {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedBaseType {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRangeKeyword {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRangeMin {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::InvalidRangeMin {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedDblDot {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRangeMax {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::InvalidRangeMax {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemiSubtype {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedConstName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedRegisterName {
            span: Span::UNKNOWN,
        }
        .code(),
        ParseError::ExpectedSemiSkip {
            span: Span::UNKNOWN,
        }
        .code(),
    ];
    let mut sorted = codes.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), codes.len(), "duplicate error codes found");
}

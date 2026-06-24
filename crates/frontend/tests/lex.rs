use frontend::{
    lex::Lexer,
    span::Span,
    token::{Token, TokenKind},
};

fn lex(s: &str) -> Vec<Token> {
    let mut lex = Lexer::new(s.as_bytes());
    let mut toks = Vec::new();
    loop {
        let t = lex.next();
        let is_eof = t.kind == TokenKind::Eof;
        toks.push(t);
        if is_eof {
            break;
        }
    }
    toks
}

fn kind(s: &str) -> Vec<TokenKind> {
    lex(s).into_iter().map(|t| t.kind).collect()
}

// ---------------------------------------------------------------------------
// Keywords
// ---------------------------------------------------------------------------

#[test]
fn kw_module() {
    assert_eq!(kind("module"), [TokenKind::KwModule, TokenKind::Eof]);
}

#[test]
fn kw_import() {
    assert_eq!(kind("import"), [TokenKind::KwImport, TokenKind::Eof]);
}

#[test]
fn kw_export() {
    assert_eq!(kind("export"), [TokenKind::KwExport, TokenKind::Eof]);
}

#[test]
fn kw_end() {
    assert_eq!(kind("end"), [TokenKind::KwEnd, TokenKind::Eof]);
}

#[test]
fn kw_type() {
    assert_eq!(kind("type"), [TokenKind::KwType, TokenKind::Eof]);
}

#[test]
fn kw_subtype() {
    assert_eq!(kind("subtype"), [TokenKind::KwSubtype, TokenKind::Eof]);
}

#[test]
fn kw_struct() {
    assert_eq!(kind("struct"), [TokenKind::KwStruct, TokenKind::Eof]);
}

#[test]
fn kw_enum() {
    assert_eq!(kind("enum"), [TokenKind::KwEnum, TokenKind::Eof]);
}

#[test]
fn kw_const() {
    assert_eq!(kind("const"), [TokenKind::KwConst, TokenKind::Eof]);
}

#[test]
fn kw_resource() {
    assert_eq!(kind("resource"), [TokenKind::KwResource, TokenKind::Eof]);
}

#[test]
fn kw_register_map() {
    assert_eq!(
        kind("register-map"),
        [TokenKind::KwRegisterMap, TokenKind::Eof]
    );
}

#[test]
fn kw_owned() {
    assert_eq!(kind("owned"), [TokenKind::KwOwned, TokenKind::Eof]);
}

#[test]
fn kw_iso() {
    assert_eq!(kind("iso"), [TokenKind::KwIso, TokenKind::Eof]);
}

#[test]
fn kw_requires() {
    assert_eq!(kind("requires"), [TokenKind::KwRequires, TokenKind::Eof]);
}

#[test]
fn kw_ensures() {
    assert_eq!(kind("ensures"), [TokenKind::KwEnsures, TokenKind::Eof]);
}

// ---------------------------------------------------------------------------
// Punctuation
// ---------------------------------------------------------------------------

#[test]
fn punct_colon() {
    assert_eq!(kind(":"), [TokenKind::PunctColon, TokenKind::Eof]);
}

#[test]
fn punct_semi() {
    assert_eq!(kind(";"), [TokenKind::PunctSemi, TokenKind::Eof]);
}

#[test]
fn punct_lparen() {
    assert_eq!(kind("("), [TokenKind::PunctLParen, TokenKind::Eof]);
}

#[test]
fn punct_rparen() {
    assert_eq!(kind(")"), [TokenKind::PunctRParen, TokenKind::Eof]);
}

#[test]
fn punct_lbracket() {
    assert_eq!(kind("["), [TokenKind::PunctLBracket, TokenKind::Eof]);
}

#[test]
fn punct_rbracket() {
    assert_eq!(kind("]"), [TokenKind::PunctRBracket, TokenKind::Eof]);
}

#[test]
fn punct_lbrace() {
    assert_eq!(kind("{"), [TokenKind::PunctLBrace, TokenKind::Eof]);
}

#[test]
fn punct_rbrace() {
    assert_eq!(kind("}"), [TokenKind::PunctRBrace, TokenKind::Eof]);
}

#[test]
fn punct_comma() {
    assert_eq!(kind(","), [TokenKind::PunctComma, TokenKind::Eof]);
}

#[test]
fn punct_dot() {
    assert_eq!(kind("."), [TokenKind::PunctDot, TokenKind::Eof]);
}

#[test]
fn punct_dotdot() {
    assert_eq!(kind(".."), [TokenKind::PunctDblDot, TokenKind::Eof]);
}

#[test]
fn punct_eq() {
    assert_eq!(kind("="), [TokenKind::PunctEq, TokenKind::Eof]);
}

#[test]
fn punct_arrow_bind() {
    assert_eq!(kind("=>"), [TokenKind::PunctArrowBind, TokenKind::Eof]);
}

#[test]
fn punct_eqeq() {
    assert_eq!(kind("=="), [TokenKind::PunctEqEq, TokenKind::Eof]);
}

#[test]
fn punct_arrow() {
    assert_eq!(kind("->"), [TokenKind::PunctArrow, TokenKind::Eof]);
}

#[test]
fn punct_dashdash() {
    assert_eq!(kind("--"), [TokenKind::PunctDashDash, TokenKind::Eof]);
}

#[test]
fn punct_ge() {
    assert_eq!(kind(">="), [TokenKind::PunctGe, TokenKind::Eof]);
}

#[test]
fn punct_le() {
    assert_eq!(kind("<="), [TokenKind::PunctLe, TokenKind::Eof]);
}

#[test]
fn punct_ne() {
    assert_eq!(kind("!="), [TokenKind::PunctNe, TokenKind::Eof]);
}

#[test]
fn punct_lesspipe() {
    assert_eq!(kind("<|"), [TokenKind::PunctLessPipe, TokenKind::Eof]);
}

#[test]
fn punct_pipegreater() {
    assert_eq!(kind("|>"), [TokenKind::PunctPipeGreater, TokenKind::Eof]);
}

#[test]
fn punct_pipe() {
    assert_eq!(kind("|"), [TokenKind::PunctPipe, TokenKind::Eof]);
}

#[test]
fn punct_apostrophe() {
    assert_eq!(kind("'"), [TokenKind::PunctApostrophe, TokenKind::Eof]);
}

#[test]
fn punct_amp() {
    assert_eq!(kind("&"), [TokenKind::PunctAmp, TokenKind::Eof]);
}

#[test]
fn punct_ampbang() {
    assert_eq!(kind("&!"), [TokenKind::PunctAmpBang, TokenKind::Eof]);
}

#[test]
fn punct_amp_lbracket() {
    assert_eq!(kind("&["), [TokenKind::PunctAmpLBracket, TokenKind::Eof]);
}

#[test]
fn punct_ampbang_lbracket() {
    assert_eq!(
        kind("&!["),
        [TokenKind::PunctAmpBangLBracket, TokenKind::Eof]
    );
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

#[test]
fn ident_basic() {
    assert_eq!(kind("foo"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_underscore_prefix() {
    assert_eq!(kind("_myvar"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_with_dash() {
    assert_eq!(kind("my-var"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_with_question() {
    assert_eq!(kind("is_ok?"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_with_bang() {
    assert_eq!(kind("not!"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_with_slash() {
    // S-13: `/` is no longer part of identifiers.
    assert_eq!(
        kind("a/b"),
        [
            TokenKind::Ident,
            TokenKind::PunctSlash,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
}

#[test]
fn ident_with_brackets() {
    // S-13: `[` and `]` are no longer part of identifiers.
    assert_eq!(
        kind("arr[0]"),
        [
            TokenKind::Ident,
            TokenKind::PunctLBracket,
            TokenKind::Number,
            TokenKind::PunctRBracket,
            TokenKind::Eof
        ]
    );
}

#[test]
fn ident_at_prefix() {
    assert_eq!(kind("@addr"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_bang_prefix() {
    assert_eq!(kind("!value"), [TokenKind::Ident, TokenKind::Eof]);
}

#[test]
fn ident_followed_by_space() {
    let toks = kind("abc 123");
    assert_eq!(toks[0], TokenKind::Ident);
    assert_eq!(toks[1], TokenKind::Number);
    assert_eq!(toks[2], TokenKind::Eof);
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn number_decimal() {
    assert_eq!(kind("42"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_zero() {
    assert_eq!(kind("0"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_hex() {
    assert_eq!(kind("0xFF"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_hex_lowercase() {
    assert_eq!(kind("0xff"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_binary() {
    assert_eq!(kind("0b1010"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_with_underscores() {
    assert_eq!(kind("1_000_000"), [TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn number_negative() {
    // '-' followed by digit → lex_number_or_ident → lex_number
    let toks = kind("-1");
    assert_eq!(toks[0], TokenKind::Number);
    assert_eq!(toks[1], TokenKind::Eof);
}

#[test]
fn minus_non_digit_becomes_ident() {
    // '-' followed by a letter: lex_number_or_ident peeks 'a' (not digit) → lex_ident.
    // lex_ident reads the whole "-a" as a single identifier since '-' is ident-continue
    // and 'a' is ident-start.
    assert_eq!(kind("-a"), [TokenKind::Ident, TokenKind::Eof]);
    assert_eq!(kind("-abc"), [TokenKind::Ident, TokenKind::Eof]);
}

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

#[test]
fn string_empty() {
    assert_eq!(kind(r#""""#), [TokenKind::String, TokenKind::Eof]);
}

#[test]
fn string_basic() {
    assert_eq!(kind(r#""hello""#), [TokenKind::String, TokenKind::Eof]);
}

#[test]
fn string_escape_backslash() {
    assert_eq!(kind(r#""\\""#), [TokenKind::String, TokenKind::Eof]);
}

#[test]
fn string_escape_quote() {
    assert_eq!(kind(r#""\"""#), [TokenKind::String, TokenKind::Eof]);
}

#[test]
fn string_unterminated() {
    // Unterminated string should still produce a String token (lexer reads to EOF)
    assert_eq!(kind(r#""hello"#), [TokenKind::String, TokenKind::Eof]);
}

#[test]
fn string_multiple() {
    let toks = kind(r#""a" "b""#);
    assert_eq!(toks[0], TokenKind::String);
    assert_eq!(toks[1], TokenKind::String);
    assert_eq!(toks[2], TokenKind::Eof);
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn comment_line_ignored() {
    let toks = kind("123 # this is a comment\n456");
    assert_eq!(toks, [TokenKind::Number, TokenKind::Number, TokenKind::Eof]);
}

#[test]
fn comment_at_eof() {
    assert_eq!(
        kind("123 # comment without newline"),
        [TokenKind::Number, TokenKind::Eof]
    );
}

#[test]
fn comment_only() {
    assert_eq!(kind("# just a comment"), [TokenKind::Eof]);
}

#[test]
fn comment_empty_line() {
    assert_eq!(kind("#\n"), [TokenKind::Eof]);
}

// ---------------------------------------------------------------------------
// Whitespace
// ---------------------------------------------------------------------------

#[test]
fn whitespace_only() {
    assert_eq!(kind("   \n  \t  "), [TokenKind::Eof]);
}

#[test]
fn empty_input() {
    assert_eq!(kind(""), [TokenKind::Eof]);
}

// ---------------------------------------------------------------------------
// Effect sets
// ---------------------------------------------------------------------------

#[test]
fn performs_empty() {
    assert_eq!(
        kind("performs {}"),
        [
            TokenKind::KwPerforms,
            TokenKind::PunctLBrace,
            TokenKind::PunctRBrace,
            TokenKind::Eof
        ]
    );
}

#[test]
fn performs_with_effect() {
    assert_eq!(
        kind("performs {suspend}"),
        [
            TokenKind::KwPerforms,
            TokenKind::PunctLBrace,
            TokenKind::Ident,
            TokenKind::PunctRBrace,
            TokenKind::Eof
        ]
    );
}

#[test]
fn performs_with_multiple() {
    assert_eq!(
        kind("performs {suspend, mmio}"),
        [
            TokenKind::KwPerforms,
            TokenKind::PunctLBrace,
            TokenKind::Ident,
            TokenKind::PunctComma,
            TokenKind::Ident,
            TokenKind::PunctRBrace,
            TokenKind::Eof
        ]
    );
}

#[test]
fn old_bang_brace_is_two_tokens() {
    // Old `!{` syntax is no longer a special token — it's Ident(!) + PunctLBrace.
    assert_eq!(
        kind("!{suspend}"),
        [
            TokenKind::Ident,
            TokenKind::PunctLBrace,
            TokenKind::Ident,
            TokenKind::PunctRBrace,
            TokenKind::Eof
        ]
    );
}

// ---------------------------------------------------------------------------
// FR-23 charset: [ ] / no longer part of identifiers
// ---------------------------------------------------------------------------

#[test]
fn bracket_index_is_not_one_ident() {
    // `buf[3]` must lex as `buf`, `[`, `3`, `]`, not one ident
    assert_eq!(
        kind("buf[3]"),
        [
            TokenKind::Ident,
            TokenKind::PunctLBracket,
            TokenKind::Number,
            TokenKind::PunctRBracket,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn module_path_uses_slash_token() {
    // `platform/linux` must lex as `platform`, `/`, `linux`
    assert_eq!(
        kind("platform/linux"),
        [
            TokenKind::Ident,
            TokenKind::PunctSlash,
            TokenKind::Ident,
            TokenKind::Eof,
        ]
    );
}

// ---------------------------------------------------------------------------
// Regression: hyphen, question, bang in identifiers still work
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Multi-token sequences
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_expression() {
    assert_eq!(
        kind("1 + 2 * 3"),
        [
            TokenKind::Number,
            TokenKind::Ident, // '+'
            TokenKind::Number,
            TokenKind::Ident, // '*'
            TokenKind::Number,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn if_expression() {
    assert_eq!(
        kind("if true { 1 } else { 2 }"),
        [
            TokenKind::Ident, // 'if'
            TokenKind::Ident, // 'true'
            TokenKind::PunctLBrace,
            TokenKind::Number,
            TokenKind::PunctRBrace,
            TokenKind::Ident, // 'else'
            TokenKind::PunctLBrace,
            TokenKind::Number,
            TokenKind::PunctRBrace,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn module_header() {
    assert_eq!(
        kind("module Foo;"),
        [
            TokenKind::KwModule,
            TokenKind::Ident,
            TokenKind::PunctSemi,
            TokenKind::Eof
        ]
    );
}

#[test]
fn keyword_not_ident_when_part_of_word() {
    // Keyword matching is exact: "module" is a keyword, "moduleX" is an ident
    let toks = kind("moduleX");
    assert_eq!(toks[0], TokenKind::Ident);
    assert_eq!(toks[1], TokenKind::Eof);
}

// ---------------------------------------------------------------------------
// Span verification
// ---------------------------------------------------------------------------

#[test]
fn span_of_keyword() {
    let toks = lex("module");
    assert_eq!(toks[0].span, Span::new(0, 6));
}

#[test]
fn span_of_number() {
    let toks = lex("12345");
    assert_eq!(toks[0].span, Span::new(0, 5));
}

#[test]
fn span_after_whitespace() {
    let toks = lex("  foo");
    assert_eq!(toks[0].span, Span::new(2, 5));
}

// ---------------------------------------------------------------------------
// Error recovery — unknown bytes treated as Ident
// ---------------------------------------------------------------------------

#[test]
fn unknown_byte_treated_as_ident() {
    // The lexer fallback for unrecognized bytes: _ => TokenKind::Ident
    let toks = kind("@");
    assert_eq!(toks[0], TokenKind::Ident);
    assert_eq!(toks[1], TokenKind::Eof);
}

#[test]
fn caret_is_ident() {
    let toks = kind("^");
    assert_eq!(toks[0], TokenKind::Ident);
    assert_eq!(toks[1], TokenKind::Eof);
}

#[test]
fn percent_is_ident() {
    let toks = kind("%");
    assert_eq!(toks[0], TokenKind::Ident);
    assert_eq!(toks[1], TokenKind::Eof);
}

// ---------------------------------------------------------------------------
// EOF behavior
// ---------------------------------------------------------------------------

#[test]
fn eof_only_once() {
    let toks = lex("");
    assert_eq!(toks.len(), 1); // only the Eof token
    assert_eq!(toks[0].kind, TokenKind::Eof);
}

#[test]
fn eof_repeated_returns_eof_each_time() {
    let mut lex = Lexer::new(b"");
    for _ in 0..3 {
        assert_eq!(lex.next().kind, TokenKind::Eof);
    }
}

// ---------------------------------------------------------------------------
// set_pos
// ---------------------------------------------------------------------------

#[test]
fn set_pos_resets_position() {
    let mut lex = Lexer::new(b"abc def");
    assert_eq!(lex.next().kind, TokenKind::Ident);
    lex.set_pos(0);
    assert_eq!(lex.next().kind, TokenKind::Ident);
    assert_eq!(lex.next().kind, TokenKind::Ident);
    assert_eq!(lex.next().kind, TokenKind::Eof);
}

#[test]
fn set_pos_beyond_end_clamps() {
    let mut lex = Lexer::new(b"abc");
    lex.set_pos(100);
    assert_eq!(lex.next().kind, TokenKind::Eof);
}

// ---------------------------------------------------------------------------
// src accessor
// ---------------------------------------------------------------------------

#[test]
fn src_returns_original_bytes() {
    let lex = Lexer::new(b"test");
    assert_eq!(lex.src(), b"test");
}

// ---------------------------------------------------------------------------
// Property-based tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod proptests {
    use frontend::lex::Lexer;
    use frontend::token::TokenKind;
    use proptest::prelude::*;

    proptest! {
        /// The lexer must never panic on any byte sequence, and must always
        /// produce at least one token (the Eof token).
        #[test]
        fn lexer_never_panics(bytes: Vec<u8>) {
            let mut lex = Lexer::new(&bytes);
            let mut count = 0;
            loop {
                let tok = lex.next();
                count += 1;
                if tok.kind == TokenKind::Eof {
                    break;
                }
                if count > bytes.len() + 1 {
                    panic!("lexer did not terminate after {} tokens for {} bytes of input", count, bytes.len());
                }
            }
            assert!(count >= 1, "must produce at least Eof token");
        }

        /// All non-Eof tokens have valid spans within the input bounds.
        #[test]
        fn lexer_spans_are_valid(bytes: Vec<u8>) {
            let mut lex = Lexer::new(&bytes);
            loop {
                let tok = lex.next();
                if tok.kind == TokenKind::Eof {
                    assert_eq!(tok.span.start, bytes.len(), "Eof span should point to end of input");
                    assert_eq!(tok.span.end, bytes.len());
                    break;
                }
                assert!(tok.span.start < bytes.len(), "token start {0} >= len {1}", tok.span.start, bytes.len());
                assert!(tok.span.end <= bytes.len(), "token end {0} > len {1}", tok.span.end, bytes.len());
                assert!(tok.span.start <= tok.span.end, "token span start {0} > end {1}", tok.span.start, tok.span.end);
            }
        }

        /// Lexer token positions are monotonic (non-decreasing).
        #[test]
        fn lexer_positions_monotonic(bytes: Vec<u8>) {
            let mut lex = Lexer::new(&bytes);
            let mut prev_end = 0usize;
            loop {
                let tok = lex.next();
                if tok.kind == TokenKind::Eof {
                    break;
                }
                assert!(tok.span.start >= prev_end,
                    "token start {0} < previous end {1}", tok.span.start, prev_end);
                prev_end = tok.span.end;
            }
        }

        /// Re-scanning from position 0 produces the same sequence of tokens.
        #[test]
        fn lexer_rescan_consistent(bytes: Vec<u8>) {
            let mut lex = Lexer::new(&bytes);
            let first_pass: Vec<_> = {
                let mut toks = Vec::new();
                loop {
                    let tok = lex.next();
                    let is_eof = tok.kind == TokenKind::Eof;
                    toks.push(tok.kind);
                    if is_eof { break; }
                }
                toks
            };
            lex.set_pos(0);
            let second_pass: Vec<_> = {
                let mut toks = Vec::new();
                loop {
                    let tok = lex.next();
                    let is_eof = tok.kind == TokenKind::Eof;
                    toks.push(tok.kind);
                    if is_eof { break; }
                }
                toks
            };
            assert_eq!(first_pass, second_pass, "re-scan produced different token sequence");
        }

        /// Eof is a terminal: after the first Eof, all subsequent calls
        /// return Eof with the identical span (end-of-input point).
        #[test]
        fn lexer_eof_is_terminal(bytes: Vec<u8>) {
            let mut lex = Lexer::new(&bytes);
            // Consume all tokens until Eof.
            let mut first_eof = None;
            loop {
                let tok = lex.next();
                if tok.kind == TokenKind::Eof {
                    first_eof = Some(tok);
                    break;
                }
            }
            let first = first_eof.unwrap();
            // Subsequent calls must return Eof with the same span.
            for _ in 0..3 {
                let tok = lex.next();
                assert_eq!(tok.kind, TokenKind::Eof);
                assert_eq!(tok.span, first.span,
                    "post-Eof calls must return identical span");
            }
        }
    }
}

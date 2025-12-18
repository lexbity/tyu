use crate::span::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Eof,

    Ident,
    Number,
    String,
    EffectSet, // !{...}

    KwModule,
    KwImport,
    KwExport,
    KwEnd,

    KwType,
    KwSubtype,
    KwStruct,
    KwEnum,
    KwConst,
    KwResource,
    KwRegisterMap,
    KwRequires,
    KwEnsures,

    PunctColon,
    PunctSemi,
    PunctLParen,
    PunctRParen,
    PunctLBracket,
    PunctRBracket,
    PunctLBrace,
    PunctRBrace,
    PunctComma,
    PunctDot,
    PunctEq,
    PunctGe, // >=
    PunctLe, // <=
    PunctEqEq, // ==
    PunctNe, // !=

    PunctArrowBind, // =>
    PunctDblDot,    // ..
    PunctDashDash,  // --
    PunctLessPipe,  // <|
    PunctPipeGreater, // |>
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub const fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

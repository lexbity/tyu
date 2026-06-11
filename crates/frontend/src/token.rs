use crate::span::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Eof,

    Ident,
    Number,
    String,
    KwPerforms,

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
    KwOwned,
    KwIso,
    KwRequires,
    KwEnsures,
    KwNeeds,

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
    PunctGe,   // >=
    PunctLe,   // <=
    PunctEqEq, // ==
    PunctNe,   // !=

    PunctArrow,       // ->
    PunctArrowBind,   // =>
    PunctDblDot,      // ..
    PunctDashDash,    // --
    PunctLessPipe,    // <|
    PunctPipeGreater, // |>
    PunctPipe,        // |
    PunctApostrophe,  // '

    PunctAmp,             // &
    PunctAmpBang,         // &!
    PunctAmpLBracket,     // &[
    PunctAmpBangLBracket, // &![
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

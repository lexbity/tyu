use crate::fixed::FixedVec;
use crate::span::Span;

pub trait Output {
    fn write(&mut self, bytes: &[u8]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    ExpectedModule {
        span: Span,
    },
    ExpectedModuleName {
        span: Span,
    },
    ExpectedSemiAfterModule {
        span: Span,
    },
    ExpectedEnd {
        span: Span,
    },
    ExpectedSemiAfterEnd {
        span: Span,
    },
    ExpectedImportName {
        span: Span,
    },
    ExpectedRBrace {
        span: Span,
    },
    ExpectedQualIdent {
        span: Span,
    },
    ExpectedRBraceExport {
        span: Span,
    },
    ExpectedExportName {
        span: Span,
    },
    ExpectedWordName {
        span: Span,
    },
    ExpectedSigParen {
        span: Span,
    },
    ExpectedQuotation {
        span: Span,
    },
    ExpectedQuotationEffect {
        span: Span,
    },
    ExpectedSemi {
        span: Span,
    },
    UnmatchedBracket {
        span: Span,
    },
    UnmatchedBrace {
        span: Span,
    },
    UnmatchedParen {
        span: Span,
    },
    ExpectedEnumName {
        span: Span,
    },
    ExpectedFieldIdent {
        span: Span,
    },
    ExpectedSemiOrEnd {
        span: Span,
    },
    ExpectedEndSemi {
        span: Span,
    },
    ExpectedEq {
        span: Span,
    },
    ExpectedNumber {
        span: Span,
    },
    InvalidInteger {
        span: Span,
    },
    ExpectedSubtypeName {
        span: Span,
    },
    ExpectedEqSubtype {
        span: Span,
    },
    ExpectedBaseType {
        span: Span,
    },
    ExpectedRangeKeyword {
        span: Span,
    },
    ExpectedRangeMin {
        span: Span,
    },
    InvalidRangeMin {
        span: Span,
    },
    ExpectedDblDot {
        span: Span,
    },
    ExpectedRangeMax {
        span: Span,
    },
    InvalidRangeMax {
        span: Span,
    },
    ExpectedSemiSubtype {
        span: Span,
    },
    ExpectedConstName {
        span: Span,
    },
    ExpectedInterruptVector {
        span: Span,
    },
    ExpectedRegisterName {
        span: Span,
    },
    ExpectedSemiSkip {
        span: Span,
    },
    /// Too many items of a given kind; the parser's static capacity was exceeded.
    TooManyItems {
        span: Span,
    },
    /// An unknown effect name was used in a `performs` declaration.
    UnknownEffect {
        span: Span,
        name: crate::span::Span,
    },
    /// Skipped unrecognized input at the top level (recovery marker).
    Skipped {
        span: Span,
    },
}

impl ParseError {
    pub fn code(&self) -> u32 {
        match self {
            Self::ExpectedModule { .. } => 2100,
            Self::ExpectedModuleName { .. } => 2101,
            Self::ExpectedSemiAfterModule { .. } => 2102,
            Self::ExpectedEnd { .. } => 2103,
            Self::ExpectedSemiAfterEnd { .. } => 2104,
            Self::ExpectedImportName { .. } => 2110,
            Self::ExpectedRBrace { .. } => 2111,
            Self::ExpectedQualIdent { .. } => 2112,
            Self::ExpectedRBraceExport { .. } => 2120,
            Self::ExpectedExportName { .. } => 2121,
            Self::ExpectedWordName { .. } => 2130,
            Self::ExpectedSigParen { .. } => 2131,
            Self::ExpectedQuotation { .. } => 2132,
            Self::ExpectedQuotationEffect { .. } => 2133,
            Self::ExpectedSemi { .. } => 2134,
            Self::UnmatchedBracket { .. } => 2140,
            Self::UnmatchedBrace { .. } => 2141,
            Self::UnmatchedParen { .. } => 2142,
            Self::ExpectedEnumName { .. } => 2150,
            Self::ExpectedFieldIdent { .. } => 2151,
            Self::ExpectedSemiOrEnd { .. } => 2160,
            Self::ExpectedEndSemi { .. } => 2161,
            Self::ExpectedEq { .. } => 2162,
            Self::ExpectedNumber { .. } => 2163,
            Self::InvalidInteger { .. } => 2164,
            Self::ExpectedSubtypeName { .. } => 2170,
            Self::ExpectedEqSubtype { .. } => 2171,
            Self::ExpectedBaseType { .. } => 2172,
            Self::ExpectedRangeKeyword { .. } => 2173,
            Self::ExpectedRangeMin { .. } => 2174,
            Self::InvalidRangeMin { .. } => 2175,
            Self::ExpectedDblDot { .. } => 2176,
            Self::ExpectedRangeMax { .. } => 2177,
            Self::InvalidRangeMax { .. } => 2178,
            Self::ExpectedSemiSubtype { .. } => 2179,
            Self::ExpectedConstName { .. } => 2180,
            Self::ExpectedInterruptVector { .. } => 2185,
            Self::ExpectedRegisterName { .. } => 2186,
            Self::ExpectedSemiSkip { .. } => 2199,
            Self::TooManyItems { .. } => 2198,
            Self::UnknownEffect { .. } => 2143,
            Self::Skipped { .. } => 2144,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::ExpectedModule { span }
            | Self::ExpectedModuleName { span }
            | Self::ExpectedSemiAfterModule { span }
            | Self::ExpectedEnd { span }
            | Self::ExpectedSemiAfterEnd { span }
            | Self::ExpectedImportName { span }
            | Self::ExpectedRBrace { span }
            | Self::ExpectedQualIdent { span }
            | Self::ExpectedRBraceExport { span }
            | Self::ExpectedExportName { span }
            | Self::ExpectedWordName { span }
            | Self::ExpectedSigParen { span }
            | Self::ExpectedQuotation { span }
            | Self::ExpectedQuotationEffect { span }
            | Self::ExpectedSemi { span }
            | Self::UnmatchedBracket { span }
            | Self::UnmatchedBrace { span }
            | Self::UnmatchedParen { span }
            | Self::ExpectedEnumName { span }
            | Self::ExpectedFieldIdent { span }
            | Self::ExpectedSemiOrEnd { span }
            | Self::ExpectedEndSemi { span }
            | Self::ExpectedEq { span }
            | Self::ExpectedNumber { span }
            | Self::InvalidInteger { span }
            | Self::ExpectedSubtypeName { span }
            | Self::ExpectedEqSubtype { span }
            | Self::ExpectedBaseType { span }
            | Self::ExpectedRangeKeyword { span }
            | Self::ExpectedRangeMin { span }
            | Self::InvalidRangeMin { span }
            | Self::ExpectedDblDot { span }
            | Self::ExpectedRangeMax { span }
            | Self::InvalidRangeMax { span }
            | Self::ExpectedSemiSubtype { span }
            | Self::ExpectedConstName { span }
            | Self::ExpectedInterruptVector { span }
            | Self::ExpectedRegisterName { span }
            | Self::ExpectedSemiSkip { span }
            | Self::TooManyItems { span }
            | Self::Skipped { span } => *span,
            Self::UnknownEffect { name, .. } => *name,
        }
    }
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
    Owned,
    Iso,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtypeAst {
    pub name: Span,
    pub base: Span,
    pub min: i64,
    pub max: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegMapInstanceAst {
    pub name: Span,
    pub map: Span,
    pub base_addr: Span,
}

pub struct ImportAst {
    pub module: Span,
    pub names: FixedVec<Span, 64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructFieldAst {
    pub name: Span,
    pub ty: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnumVariantAst {
    pub name: Span,
    pub value: i64,
}

pub struct StructDeclAst {
    pub name: Span,
    pub fields: FixedVec<StructFieldAst, 32>,
}

pub struct EnumDeclAst {
    pub name: Span,
    pub base: Option<Span>,
    pub variants: FixedVec<EnumVariantAst, 64>,
}

/// A parsed attribute attached to a declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttrAst {
    Interrupt { vector: Span },
    Other(Span),
}

pub struct DeclAst {
    pub kind: DeclKind,
    pub name: Span,
    pub sig: Option<Span>,
    pub attrs: FixedVec<AttrAst, 16>,
    pub body: Option<Span>,
    pub requires: Option<Span>,
    pub ensures: Option<Span>,
    pub cap_set: Option<Span>,
    pub effect_bits: u16,
    pub effect_net: i16,
    pub effect_high: u32,
    pub has_explicit_performs: bool,
}

pub struct ModuleAst {
    pub name: Span,
    pub imports: FixedVec<ImportAst, 64>,
    pub exports: FixedVec<Span, 256>,
    pub decls: FixedVec<DeclAst, 256>,
    pub has_export_stmt: bool,
    pub subtypes: FixedVec<SubtypeAst, 64>,
    pub instances: FixedVec<RegMapInstanceAst, 64>,
    pub structs: FixedVec<StructDeclAst, 32>,
    pub enums: FixedVec<EnumDeclAst, 32>,
}

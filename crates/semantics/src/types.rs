use frontend::span::Span;
use ir::{CapSet, EffectSet, StackBound};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeAtom {
    len: u8,
    bytes: [u8; 32],
}

impl TypeAtom {
    // Static constants for built-in type atoms.
    // These avoid repeated `TypeAtom::new(b"...").expect(...)` calls.
    pub const EMPTY: TypeAtom = builtin_type_atom(b"");
    pub const I64: TypeAtom = builtin_type_atom(b"i64");
    pub const BOOL: TypeAtom = builtin_type_atom(b"bool");
    pub const STR: TypeAtom = builtin_type_atom(b"str");
    pub const PTR: TypeAtom = builtin_type_atom(b"ptr");
    pub const PTR_MUT: TypeAtom = builtin_type_atom(b"ptr_mut");
    pub const MMIO: TypeAtom = builtin_type_atom(b"mmio");
    pub const QUOT: TypeAtom = builtin_type_atom(b"quot");
    pub const RESOURCE: TypeAtom = builtin_type_atom(b"resource");
    pub const SCOPED: TypeAtom = builtin_type_atom(b"scoped");

    pub const fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 32 {
            return None;
        }
        let mut out = [0u8; 32];
        let mut i = 0usize;
        while i < bytes.len() {
            out[i] = bytes[i];
            i += 1;
        }
        Some(Self {
            len: bytes.len() as u8,
            bytes: out,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Compiler-computed type class tag (decision D-13). The single source of
    /// truth for class computation is `ir::TypeClass::class_of`; this method
    /// is the semantics-side accessor so irgen and the typechecker never
    /// duplicate the byte-matching logic.
    pub fn class(&self) -> ir::TypeClass {
        ir::TypeClass::class_of(self.as_bytes())
    }
}

const fn builtin_type_atom(bytes: &[u8]) -> TypeAtom {
    assert!(bytes.len() <= 32, "built-in TypeAtom exceeds 32 bytes");
    let mut out = [0u8; 32];
    let mut i = 0usize;
    while i < bytes.len() {
        out[i] = bytes[i];
        i += 1;
    }
    TypeAtom {
        len: bytes.len() as u8,
        bytes: out,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordSig {
    pub in_len: u8,
    pub out_len: u8,
    pub inputs: [TypeAtom; 8],
    pub outputs: [TypeAtom; 8],
}

impl WordSig {
    pub const fn empty() -> Self {
        const Z: TypeAtom = TypeAtom {
            len: 0,
            bytes: [0u8; 32],
        };
        Self {
            in_len: 0,
            out_len: 0,
            inputs: [Z; 8],
            outputs: [Z; 8],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordEntry {
    pub name: TypeAtom,
    pub sig: WordSig,
    pub performs: EffectSet,
    pub requires: CapSet,
    pub bound: StackBound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigParseError {
    pub code: u32,
    pub span: Span,
}

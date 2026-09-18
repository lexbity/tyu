#![no_std]
#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
//! Intermediate representation, ABI contract, and verifier support for Tyu.
//!
//! The crate defines the load-bearing IR and related compatibility types used
//! by the front end, semantics pass, and code generators.

use frontend::{fixed::FixedVec, parse::Output, span::Span};

pub mod contract;

pub use contract::{abi_hash, CapSet, Context, EffectSet, High, StackBound, ABI_CONTRACT_VERSION};

/// Version of the `--emit=ir` text format (design doc §5.4, D-8). Every
/// consumer of the text format (golden tooling, corpus tooling, the future
/// Lean-side parser) checks the first emitted line against this constant and
/// fails fast on mismatch.
pub const FORMAT_VER: u32 = 4;

/// A `format_ver` header that does not match this reader's [`FORMAT_VER`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatVerMismatch {
    /// The version the artifact actually carried. `None` when the first line
    /// was not a `format_ver` header at all (pre-D-8 artifact or not IR text).
    pub found: Option<u32>,
}

impl core::fmt::Display for FormatVerMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.found {
            Some(found) => write!(
                f,
                "format mismatch: reader expects {}, artifact says {}",
                FORMAT_VER, found
            ),
            None => write!(
                f,
                "format mismatch: reader expects {}, artifact has no format_ver header",
                FORMAT_VER
            ),
        }
    }
}

/// Verify the `format_ver` header of a `--emit=ir` text artifact against the
/// reader's [`FORMAT_VER`] (decision D-8). Accepts the whole artifact or just
/// its first line — the header is taken from the first line either way. Every
/// consumer of the text format MUST call this before parsing anything else;
/// the error's Display is the fail-fast message the spec requires.
pub fn check_format_ver(artifact: &[u8]) -> Result<(), FormatVerMismatch> {
    let mismatch = FormatVerMismatch { found: None };
    let first_line = artifact.split(|&b| b == b'\n').next().unwrap_or(artifact);
    let rest = first_line.strip_prefix(b"format_ver ").ok_or(mismatch)?;
    let text = core::str::from_utf8(rest).map_err(|_| mismatch)?;
    let found: u32 = text.trim_end().parse().map_err(|_| mismatch)?;
    if found == FORMAT_VER {
        Ok(())
    } else {
        Err(FormatVerMismatch { found: Some(found) })
    }
}

/// Fused per-window access-mask bits (design doc §5.5).
pub const ACCESS_READ: u8 = 1;
pub const ACCESS_WRITE: u8 = 2;
pub const ACCESS_W1S: u8 = 4;
pub const ACCESS_W1C: u8 = 8;
pub const ACCESS_EFFECTFUL_READ: u8 = 16;

/// How a memory-mapped window is backed (design doc §5.2, D-7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Bus,
    Emulated,
}

/// A window a module touches, with the fused access mask derived by irgen
/// (design doc §5.4/§5.5). Carried on the `Word` for the verifier's
/// place-bounds check and on the `Module` for text emit; the modinfo
/// projection (name_hash + id + size + access_mask) is derived at pack time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowUse {
    pub id: u16,
    pub name: Atom,
    pub kind: WindowKind,
    /// Absolute base. `None` = link-time symbol (emulated window).
    pub base: Option<u64>,
    pub size: u32,
    pub access_mask: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Atom {
    len: u8,
    bytes: [u8; 32],
}

impl Atom {
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
}

const fn builtin_atom(bytes: &[u8]) -> Atom {
    assert!(bytes.len() <= 32, "built-in Atom exceeds 32 bytes");
    let mut out = [0u8; 32];
    let mut i = 0usize;
    while i < bytes.len() {
        out[i] = bytes[i];
        i += 1;
    }
    Atom {
        len: bytes.len() as u8,
        bytes: out,
    }
}

// Static Atom constants for built-in types.
// These are used throughout the compiler pipeline to avoid repeated
// Atom::new(b"...").expect() calls. All fit in the 32-byte limit.
pub const AT_EMPTY: Atom = builtin_atom(b"");
pub const AT_I64: Atom = builtin_atom(b"i64");
pub const AT_BOOL: Atom = builtin_atom(b"bool");
pub const AT_STR: Atom = builtin_atom(b"str");
pub const AT_PTR: Atom = builtin_atom(b"ptr");
pub const AT_PTR_MUT: Atom = builtin_atom(b"ptr_mut");
pub const AT_MMIO: Atom = builtin_atom(b"mmio");
pub const AT_QUOT: Atom = builtin_atom(b"quot");
pub const AT_RESOURCE: Atom = builtin_atom(b"resource");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeId(pub u8);

pub const TY_EMPTY: TypeId = TypeId(0);
pub const TY_I64: TypeId = TypeId(1);
pub const TY_BOOL: TypeId = TypeId(2);
pub const TY_STR: TypeId = TypeId(3);
pub const TY_PTR: TypeId = TypeId(4);
pub const TY_PTR_MUT: TypeId = TypeId(5);
pub const TY_MMIO: TypeId = TypeId(6);

/// Compiler-computed type class tag used by codegen dispatch (decision D-13).
///
/// The class is computed once in semantics (irgen) at word finalization and
/// carried on the `Word` in a vector parallel to `types`; backends dispatch on
/// the tag and never byte-match type names. An unmatched class is a loud
/// `UnsupportedOp`, never a silent no-op.
///
/// The class is IR-internal: it is *not* serialized in `--emit=ir` text (no
/// format bump), so a reader re-derives it via [`TypeClass::class_of`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeClass {
    I64,
    Bool,
    Str,
    Ptr,
    PtrMut,
    Mmio,
    /// `Slice(...)` / `SliceMut(...)` — a scoped slice whose descriptor is
    /// materialized in a per-word metadata slot.
    Slice,
    RegionRef,
    RegionRefMut,
    /// Everything else (integer prims, nominals, subtypes, quotes, ...).
    Other,
}

impl TypeClass {
    /// Derive the class of a type by name — the single source of truth for
    /// class computation. Used by irgen at word finalization and by
    /// `TypeAtom::class` in semantics; never duplicated in backends.
    pub fn class_of(name: &[u8]) -> Self {
        match name {
            b"i64" => Self::I64,
            b"bool" => Self::Bool,
            b"str" => Self::Str,
            b"ptr" => Self::Ptr,
            b"ptr_mut" => Self::PtrMut,
            b"mmio" => Self::Mmio,
            b"RegionRef" => Self::RegionRef,
            b"RegionRefMut" => Self::RegionRefMut,
            _ if name.starts_with(b"Slice(") || name.starts_with(b"SliceMut(") => Self::Slice,
            _ => Self::Other,
        }
    }
}

/// True when the class is one a `ScopedEnter` may carry: every scoped type
/// the semantics can enter is a slice or a region reference (D-13, verifier
/// rule, codegen dispatch).
pub fn is_scoped_enter_class(c: TypeClass) -> bool {
    matches!(
        c,
        TypeClass::Slice | TypeClass::RegionRef | TypeClass::RegionRefMut
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prim {
    U8,
    U16,
    U32,
    U64,
    Usize,
    I8,
    I16,
    I32,
    I64,
    Isize,
    Bool,
    Ptr,
    PtrMut,
    Str,
    Mmio,
    Chan,
    Task,
}

impl Prim {
    pub fn from_type_name(name: &[u8]) -> Option<Self> {
        Some(match name {
            b"u8" => Self::U8,
            b"u16" => Self::U16,
            b"u32" => Self::U32,
            b"u64" => Self::U64,
            b"usize" => Self::Usize,
            b"i8" => Self::I8,
            b"i16" => Self::I16,
            b"i32" => Self::I32,
            b"i64" => Self::I64,
            b"isize" => Self::Isize,
            b"bool" => Self::Bool,
            b"ptr" => Self::Ptr,
            b"ptr_mut" => Self::PtrMut,
            b"str" => Self::Str,
            b"mmio" => Self::Mmio,
            b"Chan" => Self::Chan,
            b"Task" => Self::Task,
            _ if name.starts_with(b"Chan(") => Self::Chan,
            _ => return None,
        })
    }

    pub fn bits(self, ptr_bits: u16) -> u16 {
        match self {
            Self::U8 => 8,
            Self::U16 => 16,
            Self::U32 => 32,
            Self::U64 => 64,
            Self::Usize => ptr_bits,
            Self::I8 => 8,
            Self::I16 => 16,
            Self::I32 => 32,
            Self::I64 => 64,
            Self::Isize => ptr_bits,
            Self::Bool => 8,
            Self::Ptr => ptr_bits,
            Self::PtrMut => ptr_bits,
            Self::Str => ptr_bits,
            Self::Mmio => ptr_bits,
            Self::Chan => ptr_bits,
            Self::Task => ptr_bits,
        }
    }

    pub fn is_signed(self) -> bool {
        match self {
            Self::U8 => false,
            Self::U16 => false,
            Self::U32 => false,
            Self::U64 => false,
            Self::Usize => false,
            Self::I8 => true,
            Self::I16 => true,
            Self::I32 => true,
            Self::I64 => true,
            Self::Isize => true,
            Self::Bool => false,
            Self::Ptr => false,
            Self::PtrMut => false,
            Self::Str => false,
            Self::Mmio => false,
            Self::Chan => false,
            Self::Task => false,
        }
    }

    pub fn bits_signed(self, ptr_bits: u16) -> (u16, bool) {
        (self.bits(ptr_bits), self.is_signed())
    }
}

#[cfg(test)]
mod prim_tests {
    use super::Prim;

    #[test]
    fn prim_resolver_matches_legacy_32_bit_backend_table() {
        let cases: &[(&[u8], Prim, u16, bool)] = &[
            (b"u8", Prim::U8, 8, false),
            (b"u16", Prim::U16, 16, false),
            (b"u32", Prim::U32, 32, false),
            (b"u64", Prim::U64, 64, false),
            (b"usize", Prim::Usize, 32, false),
            (b"i8", Prim::I8, 8, true),
            (b"i16", Prim::I16, 16, true),
            (b"i32", Prim::I32, 32, true),
            (b"i64", Prim::I64, 64, true),
            (b"isize", Prim::Isize, 32, true),
            (b"bool", Prim::Bool, 8, false),
            (b"ptr", Prim::Ptr, 32, false),
            (b"ptr_mut", Prim::PtrMut, 32, false),
            (b"str", Prim::Str, 32, false),
            (b"mmio", Prim::Mmio, 32, false),
            (b"Chan", Prim::Chan, 32, false),
            (b"Task", Prim::Task, 32, false),
        ];
        for &(name, prim, bits, signed) in cases {
            let resolved = Prim::from_type_name(name);
            assert_eq!(resolved, Some(prim), "name {:?}", name);
            assert_eq!(prim.bits_signed(32), (bits, signed), "name {:?}", name);
        }
    }

    #[test]
    fn prim_resolver_matches_legacy_x86_64_backend_table() {
        let cases: &[(&[u8], Prim, u16, bool)] = &[
            (b"u8", Prim::U8, 8, false),
            (b"u16", Prim::U16, 16, false),
            (b"u32", Prim::U32, 32, false),
            (b"u64", Prim::U64, 64, false),
            (b"usize", Prim::Usize, 64, false),
            (b"i8", Prim::I8, 8, true),
            (b"i16", Prim::I16, 16, true),
            (b"i32", Prim::I32, 32, true),
            (b"i64", Prim::I64, 64, true),
            (b"isize", Prim::Isize, 64, true),
            (b"bool", Prim::Bool, 8, false),
            (b"ptr", Prim::Ptr, 64, false),
            (b"ptr_mut", Prim::PtrMut, 64, false),
            (b"str", Prim::Str, 64, false),
            (b"mmio", Prim::Mmio, 64, false),
            (b"Chan(i64)", Prim::Chan, 64, false),
        ];
        for &(name, prim, bits, signed) in cases {
            let resolved = Prim::from_type_name(name);
            assert_eq!(resolved, Some(prim), "name {:?}", name);
            assert_eq!(prim.bits_signed(64), (bits, signed), "name {:?}", name);
        }
    }

    #[test]
    fn prim_resolver_rejects_non_primitives() {
        assert_eq!(Prim::from_type_name(b"Percent"), None);
        assert_eq!(Prim::from_type_name(b"Slice(i64)"), None);
        assert_eq!(Prim::from_type_name(b"RegionRef"), None);
    }
}

#[cfg(test)]
mod type_class_tests {
    use super::{TypeClass, is_scoped_enter_class};

    #[test]
    fn class_of_maps_every_builtin_name() {
        let cases: &[(&[u8], TypeClass)] = &[
            (b"i64", TypeClass::I64),
            (b"bool", TypeClass::Bool),
            (b"str", TypeClass::Str),
            (b"ptr", TypeClass::Ptr),
            (b"ptr_mut", TypeClass::PtrMut),
            (b"mmio", TypeClass::Mmio),
            (b"Slice(u8)", TypeClass::Slice),
            (b"SliceMut(i64)", TypeClass::Slice),
            (b"Slice(i64)", TypeClass::Slice),
            (b"RegionRef", TypeClass::RegionRef),
            (b"RegionRefMut", TypeClass::RegionRefMut),
            (b"", TypeClass::Other),
            (b"u8", TypeClass::Other),
            (b"i32", TypeClass::Other),
            (b"Percent", TypeClass::Other),
            (b"Task", TypeClass::Other),
        ];
        for &(name, class) in cases {
            assert_eq!(TypeClass::class_of(name), class, "name {:?}", name);
        }
    }

    #[test]
    fn slice_prefix_matching_is_exact_not_ambiguous() {
        // A name that merely *contains* the slice prefix must not classify.
        assert_eq!(TypeClass::class_of(b"NotSlice(u8)"), TypeClass::Other);
        assert_eq!(TypeClass::class_of(b"Slice(u8)x"), TypeClass::Slice);
    }

    #[test]
    fn scoped_enter_classes_are_slice_and_region_refs() {
        assert!(is_scoped_enter_class(TypeClass::Slice));
        assert!(is_scoped_enter_class(TypeClass::RegionRef));
        assert!(is_scoped_enter_class(TypeClass::RegionRefMut));
        assert!(!is_scoped_enter_class(TypeClass::I64));
        assert!(!is_scoped_enter_class(TypeClass::Bool));
        assert!(!is_scoped_enter_class(TypeClass::Other));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sig {
    pub in_len: u8,
    pub out_len: u8,
    pub inputs: [TypeId; 8],
    pub outputs: [TypeId; 8],
}

impl Sig {
    pub const fn empty() -> Self {
        const Z: TypeId = TY_EMPTY;
        Self {
            in_len: 0,
            out_len: 0,
            inputs: [Z; 8],
            outputs: [Z; 8],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrapCode {
    ContractFail,
    SubtypeFail,
    AssertFail,
    StackOverflow,
    TaskQueueOverflow,
    Unreachable,
    Deadlock,
}

pub const fn trap_code_u32(code: TrapCode) -> u32 {
    match code {
        TrapCode::ContractFail => 20,
        TrapCode::SubtypeFail => 21,
        TrapCode::AssertFail => 22,
        TrapCode::Unreachable => 23,
        TrapCode::TaskQueueOverflow => 24,
        TrapCode::StackOverflow => 10,
        TrapCode::Deadlock => 25,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpKind {
    ConstI64(i64),
    ConstBool(bool),
    ConstStr(Span),

    AddrOf {
        place: Atom,
        mutable: bool,
        base: AddrOfBase,
    },
    MmioPlace {
        place: Atom,
        window: u16,
        offset: u32,
    },
    ScopedEnter {
        ty: TypeId,
        len: u32,
    },
    TaskSpawn {
        name: Atom,
        task_ty: TypeId,
    },
    PtrAddConst {
        ty: TypeId,
        offset: u32,
    },
    PtrAddIndex {
        ty: TypeId,
        scale: u32,
    },

    Dup {
        ty: TypeId,
    },
    Drop {
        ty: TypeId,
    },
    Swap {
        a: TypeId,
        b: TypeId,
    },

    AddI64,
    SubI64,
    MulI64,
    Cmp {
        out: TypeId,
        kind: CmpKind,
    },
    AndBool,
    OrBool,
    NotBool,
    InterruptDisable,
    InterruptEnable,

    LocalSet {
        slot: u16,
        ty: TypeId,
    },
    LocalGet {
        slot: u16,
        ty: TypeId,
    },

    Cast {
        from: TypeId,
        to: TypeId,
    },
    Bitcast {
        from: TypeId,
        to: TypeId,
    },

    Call {
        name: Atom,
        sig: Sig,
        performs: EffectSet,
        requires: CapSet,
        bound: StackBound,
    },

    Load {
        ty: TypeId,
    },
    Store {
        ty: TypeId,
    },

    MmioVolLoad {
        ty: TypeId,
        place: Atom,
    },
    MmioVolStore {
        ty: TypeId,
        place: Atom,
        access: MmioAccess,
    },
    MmioVolLoadField {
        reg_ty: TypeId,
        field_ty: TypeId,
        place: Atom,
        mask: u64,
        shift: u8,
    },
    MmioVolStoreField {
        reg_ty: TypeId,
        field_ty: TypeId,
        place: Atom,
        mask: u64,
        shift: u8,
    },

    // Produces `bool` while preserving the value (so `trap_if_false` can consume the bool).
    // Stack effect: `( ty -- ty bool )`
    CheckSubtype {
        ty: TypeId,
    },
    TrapIfFalse {
        code: TrapCode,
    },

    Br {
        target: BlockId,
    },
    BrIf {
        then_tgt: BlockId,
        else_tgt: BlockId,
    },
    Ret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CmpKind {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// MMIO register access mode, describing how reads and writes behave.
/// This is a subset of the full `AccessMode` from the semantics crate,
/// defined here so the IR and codegen can make code-generation decisions
/// without depending on the semantics crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioAccess {
    /// Read-write: plain load/store.
    Rw,
    /// Write-1-to-clear: writing 1 clears the bit; writing 0 has no effect.
    W1c,
    /// Write-1-to-set: writing 1 sets the bit; writing 0 has no effect.
    W1s,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Op {
    pub kind: OpKind,
    pub span: Span,
}

/// The base of an `AddrOf` (P4): a runtime/resource address, or a
/// window-relative MMIO register address resolved from the descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddrOfBase {
    /// Address computed at runtime (a local/resource/struct place).
    Runtime,
    /// Window-relative MMIO register address.
    Mmio { window: u16, offset: u32 },
}

pub struct Block {
    pub id: BlockId,
    pub entry_stack: FixedVec<TypeId, 32>,
    pub ops: FixedVec<Op, 96>,
}

pub struct Word {
    pub name: Atom,
    pub sig: Sig,
    pub performs: EffectSet,
    pub requires: CapSet,
    pub bound: StackBound,
    pub entry: BlockId,
    pub types: FixedVec<Atom, 64>,
    pub type_sizes: FixedVec<u32, 64>,
    /// Compiler-computed class tag per entry in `types` (decision D-13).
    /// Kept parallel to `types`; backends dispatch on this, never on type
    /// name bytes. Populated once by irgen at word finalization; not
    /// serialized in `--emit=ir` text (no format bump).
    pub type_classes: FixedVec<TypeClass, 64>,
    /// The windows this word touches, derived from its `MmioPlace` ops at
    /// word finalization (P4). The verifier checks place bounds against it.
    pub windows: FixedVec<WindowUse, 8>,
    /// `subtype_bases[i]` is the base `TypeId` of `types[i]` when it is a
    /// subtype, otherwise `TY_EMPTY`.  The verifier uses it to accept a
    /// subtype value where its base is declared (subsumption).
    pub subtype_bases: FixedVec<TypeId, 64>,
    pub blocks: FixedVec<Block, 16>,
}

pub struct Module {
    pub name: Atom,
    pub words: FixedVec<Word, 64>,
    /// The union of every word's window-use table (P4).
    pub windows: FixedVec<WindowUse, 8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyError {
    EntryBlockNotFound { span: Span },
    EntryStackLenMismatch { span: Span },
    EntryStackTypeMismatch { span: Span },
    CodeAfterTerminator { span: Span },
    TypeMismatch { span: Span },
    DropTypeMismatch { span: Span },
    SwapTypeMismatch { span: Span },
    LocalTypeMismatch { span: Span },
    CallStackUnderflow { span: Span },
    CallInputTypeMismatch { span: Span },
    LoadAddrNotPtr { span: Span },
    StoreValueTypeMismatch { span: Span },
    StoreAddrNotMutPtr { span: Span },
    MmioFieldAddrNotMmio { span: Span },
    MmioFieldTypeMismatch { span: Span },
    CheckSubtypeTypeMismatch { span: Span },
    TrapIfFalseNotBool { span: Span },
    BrTargetNotFound { span: Span },
    BrStackDepthMismatch { span: Span },
    BrStackContentMismatch { span: Span },
    BrIfCondNotBool { span: Span },
    BrIfTargetNotFound { span: Span },
    BrIfStackDepthMismatch { span: Span },
    BrIfStackContentMismatch { span: Span },
    RetStackDepthMismatch { span: Span },
    RetOutputTypeMismatch { span: Span },
    NotTerminated { span: Span },
    PopEmptyStack { span: Span },
    PushFullStack { span: Span },
    ScopedEnterTypeNotScoped { span: Span },
    MmioWindowOutOfRange { span: Span },
    MmioPlaceOutOfBounds { span: Span },
}

impl VerifyError {
    pub fn code(self) -> u32 {
        match self {
            VerifyError::EntryBlockNotFound { .. } => 9001,
            VerifyError::EntryStackLenMismatch { .. } => 9002,
            VerifyError::EntryStackTypeMismatch { .. } => 9003,
            VerifyError::CodeAfterTerminator { .. } => 9010,
            VerifyError::TypeMismatch { .. } => 9011,
            VerifyError::DropTypeMismatch { .. } => 9012,
            VerifyError::SwapTypeMismatch { .. } => 9013,
            VerifyError::LocalTypeMismatch { .. } => 9016,
            VerifyError::CallStackUnderflow { .. } => 9017,
            VerifyError::CallInputTypeMismatch { .. } => 9018,
            VerifyError::LoadAddrNotPtr { .. } => 9019,
            VerifyError::StoreValueTypeMismatch { .. } => 9020,
            VerifyError::StoreAddrNotMutPtr { .. } => 9021,
            VerifyError::MmioFieldAddrNotMmio { .. } => 9022,
            VerifyError::MmioFieldTypeMismatch { .. } => 9023,
            VerifyError::CheckSubtypeTypeMismatch { .. } => 9024,
            VerifyError::TrapIfFalseNotBool { .. } => 9025,
            VerifyError::BrTargetNotFound { .. } => 9026,
            VerifyError::BrStackDepthMismatch { .. } => 9027,
            VerifyError::BrStackContentMismatch { .. } => 9028,
            VerifyError::BrIfCondNotBool { .. } => 9029,
            VerifyError::BrIfTargetNotFound { .. } => 9030,
            VerifyError::BrIfStackDepthMismatch { .. } => 9031,
            VerifyError::BrIfStackContentMismatch { .. } => 9032,
            VerifyError::RetStackDepthMismatch { .. } => 9033,
            VerifyError::RetOutputTypeMismatch { .. } => 9034,
            VerifyError::NotTerminated { .. } => 9035,
            VerifyError::PopEmptyStack { .. } => 9098,
            VerifyError::PushFullStack { .. } => 9099,
            VerifyError::ScopedEnterTypeNotScoped { .. } => 9036,
            VerifyError::MmioWindowOutOfRange { .. } => 9037,
            VerifyError::MmioPlaceOutOfBounds { .. } => 9038,
        }
    }

    pub fn span(self) -> Span {
        match self {
            VerifyError::EntryBlockNotFound { span }
            | VerifyError::EntryStackLenMismatch { span }
            | VerifyError::EntryStackTypeMismatch { span }
            | VerifyError::CodeAfterTerminator { span }
            | VerifyError::TypeMismatch { span }
            | VerifyError::DropTypeMismatch { span }
            | VerifyError::SwapTypeMismatch { span }
            | VerifyError::LocalTypeMismatch { span }
            | VerifyError::CallStackUnderflow { span }
            | VerifyError::CallInputTypeMismatch { span }
            | VerifyError::LoadAddrNotPtr { span }
            | VerifyError::StoreValueTypeMismatch { span }
            | VerifyError::StoreAddrNotMutPtr { span }
            | VerifyError::MmioFieldAddrNotMmio { span }
            | VerifyError::MmioFieldTypeMismatch { span }
            | VerifyError::CheckSubtypeTypeMismatch { span }
            | VerifyError::TrapIfFalseNotBool { span }
            | VerifyError::BrTargetNotFound { span }
            | VerifyError::BrStackDepthMismatch { span }
            | VerifyError::BrStackContentMismatch { span }
            | VerifyError::BrIfCondNotBool { span }
            | VerifyError::BrIfTargetNotFound { span }
            | VerifyError::BrIfStackDepthMismatch { span }
            | VerifyError::BrIfStackContentMismatch { span }
            | VerifyError::RetStackDepthMismatch { span }
            | VerifyError::RetOutputTypeMismatch { span }
            | VerifyError::NotTerminated { span }
            | VerifyError::PopEmptyStack { span }
            | VerifyError::PushFullStack { span }
            | VerifyError::ScopedEnterTypeNotScoped { span }
            | VerifyError::MmioWindowOutOfRange { span }
            | VerifyError::MmioPlaceOutOfBounds { span } => span,
        }
    }
}

pub fn verify_module(m: &Module) -> Result<(), VerifyError> {
    for w in m.words.iter() {
        verify_word(w)?;
    }
    Ok(())
}

pub fn verify_word(w: &Word) -> Result<(), VerifyError> {
    let mut entry = None;
    for b in w.blocks.iter() {
        if b.id == w.entry {
            entry = Some(b);
            break;
        }
    }
    let Some(entry_block) = entry else {
        return Err(VerifyError::EntryBlockNotFound {
            span: Span::UNKNOWN,
        });
    };
    if entry_block.entry_stack.len() != w.sig.in_len as usize {
        return Err(VerifyError::EntryStackLenMismatch {
            span: Span::UNKNOWN,
        });
    }
    for i in 0..(w.sig.in_len as usize) {
        if *entry_block
            .entry_stack
            .get(i)
            .expect("verified entry stack len")
            != w.sig.inputs[i]
        {
            return Err(VerifyError::EntryStackTypeMismatch {
                span: Span::UNKNOWN,
            });
        }
    }

    for b in w.blocks.iter() {
        verify_block(w, b)?;
    }

    Ok(())
}

/// Look up a window-use entry by id in a word's table.
fn find_window_use(w: &Word, id: u16) -> Option<&WindowUse> {
    w.windows.iter().find(|wu| wu.id == id)
}

/// Byte width of a type in the word's table (for place-bounds checking).
fn type_width_bytes(w: &Word, ty: TypeId) -> Option<u32> {
    let name = w.types.get(ty.0 as usize)?.as_bytes();
    let prim = Prim::from_type_name(name)?;
    Some(prim.bits(64) as u32 / 8)
}

/// Check that a volatile access at `place` (offset + width) stays inside the
/// window its `MmioPlace` site declared. Best-effort within a block: sites
/// recorded in the same block are correlated; a missing site (e.g. the place
/// was materialized in another block) skips the width check.
fn check_site_bounds(
    w: &Word,
    sites: &[(Atom, u16, u32)],
    place: Atom,
    width: u32,
    span: Span,
) -> Result<(), VerifyError> {
    let Some((_, window, offset)) = sites.iter().find(|(p, _, _)| *p == place) else {
        return Ok(());
    };
    let Some(wu) = find_window_use(w, *window) else {
        return Err(VerifyError::MmioWindowOutOfRange { span });
    };
    if offset.saturating_add(width) > wu.size {
        return Err(VerifyError::MmioPlaceOutOfBounds { span });
    }
    Ok(())
}

fn find_block(w: &Word, id: BlockId) -> Option<&Block> {
    w.blocks.iter().find(|b| b.id == id)
}

fn verify_block(w: &Word, b: &Block) -> Result<(), VerifyError> {
    let mut stack: [TypeId; 64] = [TY_EMPTY; 64];
    let mut sp = 0usize;
    for a in b.entry_stack.iter() {
        stack[sp] = *a;
        sp += 1;
    }

    // P4: `MmioPlace` sites in this block, keyed by place atom, so the
    // consuming volatile ops can check offset + width within the window.
    let mut sites: [(Atom, u16, u32); 8] = [(AT_EMPTY, 0, 0); 8];
    let mut site_count = 0usize;

    let mut terminated = false;
    for op in b.ops.iter() {
        if terminated {
            return Err(VerifyError::CodeAfterTerminator { span: op.span });
        }
        match op.kind {
            OpKind::ConstI64(_) => {
                push(&mut stack, &mut sp, TY_I64, op.span)?;
            }
            OpKind::ConstBool(_) => {
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::ConstStr(_) => {
                push(&mut stack, &mut sp, TY_STR, op.span)?;
            }
            OpKind::AddrOf { mutable: false, .. } => {
                push(&mut stack, &mut sp, TY_PTR, op.span)?;
            }
            OpKind::AddrOf { mutable: true, .. } => {
                push(&mut stack, &mut sp, TY_PTR_MUT, op.span)?;
            }
            OpKind::MmioPlace { place, window, offset } => {
                // P4: the referenced window must be declared in the word's
                // use-table, and the offset must fall inside it.
                let size = match find_window_use(w, window) {
                    Some(wu) => wu.size,
                    None => return Err(VerifyError::MmioWindowOutOfRange { span: op.span }),
                };
                if offset >= size {
                    return Err(VerifyError::MmioPlaceOutOfBounds { span: op.span });
                }
                if site_count < sites.len() {
                    sites[site_count] = (place, window, offset);
                    site_count += 1;
                }
                push(&mut stack, &mut sp, TY_MMIO, op.span)?;
            }
            OpKind::ScopedEnter { ty, .. } => {
                // D-13 / verifier rule: a ScopedEnter may only carry a
                // scoped class (Slice / RegionRef / RegionRefMut). A
                // hand-built word with an I64 (or any Other) class is
                // rejected here — the same class check the backends rely on
                // to avoid the silent no-op. Defense in depth, mirroring the
                // stack checker.
                let class = w.type_classes.get(ty.0 as usize).copied().unwrap_or(TypeClass::Other);
                if !is_scoped_enter_class(class) {
                    return Err(VerifyError::ScopedEnterTypeNotScoped { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::TaskSpawn { task_ty, .. } => {
                push(&mut stack, &mut sp, task_ty, op.span)?;
            }
            OpKind::PtrAddConst { ty, .. } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::PtrAddIndex { ty, .. } => {
                let idx = pop(&mut stack, &mut sp, op.span)?;
                if idx != TY_I64 {
                    return Err(VerifyError::CallInputTypeMismatch { span: op.span });
                }
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::Dup { ty } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::Drop { ty } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::DropTypeMismatch { span: op.span });
                }
            }
            OpKind::Swap { a, b } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                let below = pop(&mut stack, &mut sp, op.span)?;
                if top != b || below != a {
                    return Err(VerifyError::SwapTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
                push(&mut stack, &mut sp, below, op.span)?;
            }
            OpKind::AddI64 | OpKind::SubI64 | OpKind::MulI64 => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_I64, op.span)?;
            }
            OpKind::Cmp { out, .. } => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, out, op.span)?;
            }
            OpKind::AndBool | OpKind::OrBool => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::NotBool => {
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::InterruptDisable | OpKind::InterruptEnable => {}
            OpKind::LocalSet { ty, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if !type_ok(w, v, ty) {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
            }
            OpKind::LocalGet { ty, .. } => {
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::Cast { from, to } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != from {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, to, op.span)?;
            }
            OpKind::Bitcast { from, to } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != from {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, to, op.span)?;
            }
            OpKind::Call { sig, .. } => {
                let need = sig.in_len as usize;
                if sp < need {
                    return Err(VerifyError::CallStackUnderflow { span: op.span });
                }
                for i in 0..need {
                    if !type_ok(w, stack[sp - need + i], sig.inputs[i]) {
                        return Err(VerifyError::CallInputTypeMismatch { span: op.span });
                    }
                }
                sp -= need;
                for i in 0..(sig.out_len as usize) {
                    push(&mut stack, &mut sp, sig.outputs[i], op.span)?;
                }
            }
            OpKind::Load { ty } => {
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if addr != TY_PTR && addr != TY_PTR_MUT {
                    return Err(VerifyError::LoadAddrNotPtr { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::Store { ty } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::StoreValueTypeMismatch { span: op.span });
                }
                if addr != TY_PTR_MUT {
                    return Err(VerifyError::StoreAddrNotMutPtr { span: op.span });
                }
            }
            OpKind::MmioVolLoad { ty, place } => {
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if addr != TY_MMIO && addr != TY_PTR && addr != TY_PTR_MUT {
                    return Err(VerifyError::LoadAddrNotPtr { span: op.span });
                }
                if let Some(width) = type_width_bytes(w, ty) {
                    if let Err(e) = check_site_bounds(w, &sites[..site_count], place, width, op.span)
                    {
                        return Err(e);
                    }
                }
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::MmioVolStore { ty, place, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::StoreValueTypeMismatch { span: op.span });
                }
                if addr != TY_MMIO && addr != TY_PTR_MUT {
                    return Err(VerifyError::StoreAddrNotMutPtr { span: op.span });
                }
                if let Some(width) = type_width_bytes(w, ty) {
                    if let Err(e) = check_site_bounds(w, &sites[..site_count], place, width, op.span)
                    {
                        return Err(e);
                    }
                }
            }
            OpKind::MmioVolLoadField {
                reg_ty,
                field_ty,
                place,
                ..
            } => {
                let pl = pop(&mut stack, &mut sp, op.span)?;
                if pl != TY_MMIO {
                    return Err(VerifyError::MmioFieldAddrNotMmio { span: op.span });
                }
                if let Some(width) = type_width_bytes(w, reg_ty) {
                    if let Err(e) = check_site_bounds(w, &sites[..site_count], place, width, op.span)
                    {
                        return Err(e);
                    }
                }
                push(&mut stack, &mut sp, field_ty, op.span)?;
            }
            OpKind::MmioVolStoreField { reg_ty, field_ty, place, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let pl = pop(&mut stack, &mut sp, op.span)?;
                if pl != TY_MMIO || v != field_ty {
                    return Err(VerifyError::MmioFieldTypeMismatch { span: op.span });
                }
                if let Some(width) = type_width_bytes(w, reg_ty) {
                    if let Err(e) = check_site_bounds(w, &sites[..site_count], place, width, op.span)
                    {
                        return Err(e);
                    }
                }
            }
            OpKind::CheckSubtype { ty } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::CheckSubtypeTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::TrapIfFalse { .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != TY_BOOL {
                    return Err(VerifyError::TrapIfFalseNotBool { span: op.span });
                }
            }
            OpKind::Br { target } => {
                let Some(t) = find_block(w, target) else {
                    return Err(VerifyError::BrTargetNotFound { span: op.span });
                };
                if t.entry_stack.len() != sp {
                    return Err(VerifyError::BrStackDepthMismatch { span: op.span });
                }
                for (i, item) in stack.iter().enumerate().take(sp) {
                    if *t.entry_stack.get(i).expect("verified length matches sp") != *item {
                        return Err(VerifyError::BrStackContentMismatch { span: op.span });
                    }
                }
                terminated = true;
            }
            OpKind::BrIf { then_tgt, else_tgt } => {
                let cond = pop(&mut stack, &mut sp, op.span)?;
                if cond != TY_BOOL {
                    return Err(VerifyError::BrIfCondNotBool { span: op.span });
                }
                for &tgt in &[then_tgt, else_tgt] {
                    let Some(t) = find_block(w, tgt) else {
                        return Err(VerifyError::BrIfTargetNotFound { span: op.span });
                    };
                    if t.entry_stack.len() != sp {
                        return Err(VerifyError::BrIfStackDepthMismatch { span: op.span });
                    }
                    for (i, item) in stack.iter().enumerate().take(sp) {
                        if *t.entry_stack.get(i).expect("verified length matches sp") != *item {
                            return Err(VerifyError::BrIfStackContentMismatch { span: op.span });
                        }
                    }
                }
                terminated = true;
            }
            OpKind::Ret => {
                if sp != w.sig.out_len as usize {
                    return Err(VerifyError::RetStackDepthMismatch { span: op.span });
                }
                for (i, item) in stack.iter().enumerate().take(sp) {
                    if !type_ok(w, *item, w.sig.outputs[i]) {
                        return Err(VerifyError::RetOutputTypeMismatch { span: op.span });
                    }
                }
                terminated = true;
            }
        }
    }
    if !terminated {
        return Err(VerifyError::NotTerminated {
            span: Span::UNKNOWN,
        });
    }
    Ok(())
}

fn push(
    stack: &mut [TypeId; 64],
    sp: &mut usize,
    ty: TypeId,
    span: Span,
) -> Result<(), VerifyError> {
    if *sp >= stack.len() {
        return Err(VerifyError::PushFullStack { span });
    }
    stack[*sp] = ty;
    *sp += 1;
    Ok(())
}

fn pop(stack: &mut [TypeId; 64], sp: &mut usize, span: Span) -> Result<TypeId, VerifyError> {
    if *sp == 0 {
        return Err(VerifyError::PopEmptyStack { span });
    }
    *sp -= 1;
    Ok(stack[*sp])
}

/// True when a value of type `got` may be used where `want` is declared:
/// exact match, or `got` is a subtype of `want` (subsumption).  Keeps the
/// verifier in agreement with the typechecker's `type_compatible`.
fn type_ok(w: &Word, got: TypeId, want: TypeId) -> bool {
    if got == want {
        return true;
    }
    w.subtype_bases
        .get(got.0 as usize)
        .map(|&base| base == want)
        .unwrap_or(false)
}

pub fn write_module(out: &mut impl Output, m: &Module) {
    out.write(b"format_ver ");
    write_u32(out, FORMAT_VER);
    out.write(b"\n");
    out.write(b"module ");
    out.write(m.name.as_bytes());
    out.write(b"\n");
    out.write(b"windows ");
    write_u32(out, m.windows.len() as u32);
    out.write(b"\n");
    for wu in m.windows.iter() {
        out.write(b"window ");
        write_u32(out, wu.id as u32);
        out.write(b" ");
        out.write(wu.name.as_bytes());
        out.write(b" ");
        out.write(match wu.kind {
            WindowKind::Bus => b"bus",
            WindowKind::Emulated => b"emulated",
        });
        out.write(b" ");
        match wu.base {
            Some(base) => {
                out.write(b"0x");
                write_u64_hex(out, base);
            }
            None => out.write(b"link"),
        }
        out.write(b" ");
        out.write(b"0x");
        write_u32_hex(out, wu.size);
        out.write(b"\n");
    }
    for w in m.words.iter() {
        write_word(out, w);
    }
}

pub fn write_word(out: &mut impl Output, w: &Word) {
    out.write(b"word ");
    out.write(w.name.as_bytes());
    out.write(b" ");
    write_sig(out, w, &w.sig);
    out.write(b"\n");
    for b in w.blocks.iter() {
        out.write(b"  block b");
        write_u32(out, b.id.0 as u32);
        out.write(b" (");
        write_stack(out, w, &b.entry_stack);
        out.write(b")\n");
        for op in b.ops.iter() {
            out.write(b"    ");
            write_op(out, w, op);
            out.write(b"\n");
        }
    }
}

fn type_atom(w: &Word, id: TypeId) -> &Atom {
    w.types.get(id.0 as usize).unwrap_or(&AT_EMPTY)
}

fn write_sig(out: &mut impl Output, w: &Word, sig: &Sig) {
    out.write(b"( ");
    for i in 0..(sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type_atom(out, type_atom(w, sig.inputs[i]), 4);
    }
    out.write(b" --");
    if sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type_atom(out, type_atom(w, sig.outputs[i]), 4);
    }
    out.write(b" )");
}

fn write_stack(out: &mut impl Output, w: &Word, stack: &FixedVec<TypeId, 32>) {
    let mut first = true;
    for a in stack.iter() {
        if !first {
            out.write(b" ");
        }
        first = false;
        write_type_atom(out, type_atom(w, *a), 4);
    }
}

fn write_type_atom(out: &mut impl Output, atom: &Atom, depth: u8) {
    if depth == 0 {
        out.write(atom.as_bytes());
        return;
    }
    let bytes = atom.as_bytes();
    if bytes.starts_with(b"Chan(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Chan(".len()..bytes.len() - 1];
        out.write(b"|");
        if let Some(a) = Atom::new(inner) {
            write_type_atom(out, &a, depth - 1);
        } else {
            out.write(inner);
        }
        out.write(b"|");
        return;
    }
    if bytes.starts_with(b"Array(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Array(".len()..bytes.len() - 1];
        let mut depth_paren = 0u32;
        for (i, &c) in inner.iter().enumerate() {
            match c {
                b'(' => depth_paren = depth_paren.wrapping_add(1),
                b')' => depth_paren = depth_paren.wrapping_sub(1),
                b',' if depth_paren == 0 => {
                    let elem = &inner[..i];
                    let len = &inner[i + 1..];
                    if let Some(a) = Atom::new(elem) {
                        write_type_atom(out, &a, depth - 1);
                    } else {
                        out.write(elem);
                    }
                    out.write(b"'");
                    out.write(len);
                    return;
                }
                _ => {}
            }
        }
    }
    out.write(bytes);
}

fn write_op(out: &mut impl Output, w: &Word, op: &Op) {
    match op.kind {
        OpKind::ConstI64(v) => {
            out.write(b"const_i64 ");
            write_i64(out, v);
        }
        OpKind::ConstBool(true) => out.write(b"const_bool true"),
        OpKind::ConstBool(false) => out.write(b"const_bool false"),
        OpKind::ConstStr(_) => out.write(b"const_str"),
        OpKind::AddrOf {
            place,
            mutable: false,
            base,
        } => {
            out.write(b"addr_of ");
            out.write(place.as_bytes());
            write_addr_of_base(out, base);
        }
        OpKind::AddrOf {
            place,
            mutable: true,
            base,
        } => {
            out.write(b"addr_of_mut ");
            out.write(place.as_bytes());
            write_addr_of_base(out, base);
        }
        OpKind::MmioPlace { place, window, offset } => {
            out.write(b"mmio_place ");
            out.write(place.as_bytes());
            out.write(b" window=");
            write_u32(out, window as u32);
            out.write(b" offset=0x");
            write_u32_hex(out, offset);
        }
        OpKind::ScopedEnter { ty, .. } => {
            out.write(b"scoped_enter ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::TaskSpawn { name, .. } => {
            out.write(b"task_spawn ");
            out.write(name.as_bytes());
        }
        OpKind::PtrAddConst { ty, offset } => {
            out.write(b"ptr_add_const ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            write_u32(out, offset);
        }
        OpKind::PtrAddIndex { ty, scale } => {
            out.write(b"ptr_add_index ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            write_u32(out, scale);
        }
        OpKind::Dup { ty } => {
            out.write(b"dup ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Drop { ty } => {
            out.write(b"drop ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Swap { a, b } => {
            out.write(b"swap ");
            out.write(type_atom(w, a).as_bytes());
            out.write(b" ");
            out.write(type_atom(w, b).as_bytes());
        }
        OpKind::AddI64 => out.write(b"add_i64"),
        OpKind::SubI64 => out.write(b"sub_i64"),
        OpKind::MulI64 => out.write(b"mul_i64"),
        OpKind::AndBool => out.write(b"and_bool"),
        OpKind::OrBool => out.write(b"or_bool"),
        OpKind::NotBool => out.write(b"not_bool"),
        OpKind::InterruptDisable => out.write(b"interrupt_disable"),
        OpKind::InterruptEnable => out.write(b"interrupt_enable"),
        OpKind::Cmp { kind, .. } => match kind {
            CmpKind::Lt => out.write(b"cmp_lt"),
            CmpKind::Le => out.write(b"cmp_le"),
            CmpKind::Gt => out.write(b"cmp_gt"),
            CmpKind::Ge => out.write(b"cmp_ge"),
            CmpKind::Eq => out.write(b"cmp_eq"),
            CmpKind::Ne => out.write(b"cmp_ne"),
        },
        OpKind::LocalSet { slot, .. } => {
            out.write(b"local_set ");
            write_u32(out, slot as u32);
        }
        OpKind::LocalGet { slot, .. } => {
            out.write(b"local_get ");
            write_u32(out, slot as u32);
        }
        OpKind::Cast { to, .. } => {
            out.write(b"cast ");
            out.write(type_atom(w, to).as_bytes());
        }
        OpKind::Bitcast { to, .. } => {
            out.write(b"bitcast ");
            out.write(type_atom(w, to).as_bytes());
        }
        OpKind::Call { name, .. } => {
            out.write(b"call ");
            out.write(name.as_bytes());
        }
        OpKind::Load { ty } => {
            out.write(b"load ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Store { ty } => {
            out.write(b"store ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::MmioVolLoad { ty, place } => {
            out.write(b"vol_load ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
        }
        OpKind::MmioVolStore { ty, place, access } => {
            out.write(b"vol_store ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
            out.write(match access {
                MmioAccess::W1c => b" w1c",
                MmioAccess::W1s => b" w1s",
                MmioAccess::Rw => b"",
            });
        }
        OpKind::MmioVolLoadField {
            reg_ty,
            field_ty,
            place,
            mask,
            shift,
        } => {
            out.write(b"vol_load_field ");
            out.write(type_atom(w, field_ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
            out.write(b" reg=");
            out.write(type_atom(w, reg_ty).as_bytes());
            out.write(b" mask=0x");
            write_u64_hex(out, mask);
            out.write(b" shift=");
            write_u32(out, shift as u32);
        }
        OpKind::MmioVolStoreField {
            reg_ty,
            field_ty,
            place,
            mask,
            shift,
        } => {
            out.write(b"vol_store_field ");
            out.write(type_atom(w, field_ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
            out.write(b" reg=");
            out.write(type_atom(w, reg_ty).as_bytes());
            out.write(b" mask=0x");
            write_u64_hex(out, mask);
            out.write(b" shift=");
            write_u32(out, shift as u32);
        }
        OpKind::CheckSubtype { ty } => {
            out.write(b"check_subtype ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::TrapIfFalse { code } => {
            out.write(b"trap_if_false ");
            out.write(match code {
                TrapCode::ContractFail => b"CONTRACT_FAIL",
                TrapCode::SubtypeFail => b"SUBTYPE_FAIL",
                TrapCode::AssertFail => b"ASSERT_FAIL",
                TrapCode::StackOverflow => b"STACK_OVERFLOW",
                TrapCode::TaskQueueOverflow => b"TASK_QUEUE_OVERFLOW",
                TrapCode::Unreachable => b"UNREACHABLE",
                TrapCode::Deadlock => b"DEADLOCK",
            });
        }
        OpKind::Br { target } => {
            out.write(b"br b");
            write_u32(out, target.0 as u32);
        }
        OpKind::BrIf { then_tgt, else_tgt } => {
            out.write(b"br_if b");
            write_u32(out, then_tgt.0 as u32);
            out.write(b" b");
            write_u32(out, else_tgt.0 as u32);
        }
        OpKind::Ret => out.write(b"ret"),
    }
}

fn write_u32(out: &mut impl Output, mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

fn write_u32_hex(out: &mut impl Output, mut v: u32) {
    let mut buf = [0u8; 8];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            let d = (v & 0xF) as u8;
            buf[n] = match d {
                0..=9 => b'0' + d,
                _ => b'a' + (d - 10),
            };
            n += 1;
            v >>= 4;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

fn write_addr_of_base(out: &mut impl Output, base: AddrOfBase) {
    match base {
        AddrOfBase::Runtime => {}
        AddrOfBase::Mmio { window, offset } => {
            out.write(b" window=");
            write_u32(out, window as u32);
            out.write(b" offset=0x");
            write_u32_hex(out, offset);
        }
    }
}

fn write_i64(out: &mut impl Output, v: i64) {
    if v == 0 {
        out.write(b"0");
        return;
    }
    let mut buf = [0u8; 24];
    let mut n = 0usize;
    let mut x = v;
    if x < 0 {
        out.write(b"-");
        x = -x;
    }
    let mut u = x as u64;
    while u > 0 && n < buf.len() {
        buf[n] = b'0' + (u % 10) as u8;
        n += 1;
        u /= 10;
    }
    buf[..n].reverse();
    out.write(&buf[..n]);
}

fn write_u64_hex(out: &mut impl Output, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            let d = (v & 0xF) as u8;
            buf[n] = match d {
                0..=9 => b'0' + d,
                _ => b'a' + (d - 10),
            };
            n += 1;
            v >>= 4;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

#[cfg(test)]
mod format_ver_tests {
    extern crate alloc;
    use alloc::string::ToString;
    use super::{check_format_ver, FormatVerMismatch, FORMAT_VER};

    #[test]
    fn accepts_current_version_with_trailing_newline() {
        assert_eq!(check_format_ver(b"format_ver 4\nmodule M;"), Ok(()));
        assert_eq!(check_format_ver(b"format_ver 4"), Ok(()));
    }

    #[test]
    fn rejects_older_version_with_found_value() {
        assert_eq!(
            check_format_ver(b"format_ver 3\n"),
            Err(FormatVerMismatch { found: Some(3) })
        );
    }

    #[test]
    fn rejects_newer_version_with_found_value() {
        assert_eq!(
            check_format_ver(b"format_ver 5\n"),
            Err(FormatVerMismatch {
                found: Some(5)
            })
        );
    }

    #[test]
    fn rejects_missing_header_and_garbage() {
        assert_eq!(
            check_format_ver(b"module M;\n"),
            Err(FormatVerMismatch { found: None })
        );
        assert_eq!(check_format_ver(b""), Err(FormatVerMismatch { found: None }));
        assert_eq!(
            check_format_ver(b"format_ver x\n"),
            Err(FormatVerMismatch { found: None })
        );
        assert_eq!(
            check_format_ver(b"format_ver\n"),
            Err(FormatVerMismatch { found: None })
        );
    }

    #[test]
    fn mismatch_display_is_the_fail_fast_message() {
        let e = check_format_ver(b"format_ver 3\n").unwrap_err();
        assert_eq!(
            e.to_string(),
            "format mismatch: reader expects 4, artifact says 3"
        );
        assert_eq!(FORMAT_VER, 4);
    }
}

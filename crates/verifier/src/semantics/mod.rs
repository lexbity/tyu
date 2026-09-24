//! The normative IR op semantics table (`tyu.ir-sem/1.0`).
//!
//! This module is the machine-readable projection of
//! `devdocs/plans/design-doc/ir-op-semantics.md`: one [`SemanticsRow`] per
//! printable op form, carrying the op's canonical text mnemonic, data-stack
//! transition, intrinsic effect contribution, and OEL projection.
//!
//! Enforcement (static-verification.md FR-3): the table is built by a
//! wildcard-free `match` over every `OpKind` variant (and, nested, over every
//! `CmpKind` and `AddrOf` mutability). Adding a variant without adding its
//! row here is a compile error — the artifact cannot silently lag the enum.
//! `src/semantics/ops.json` is generated data mirrored from this match (the
//! match is the source of truth); the drift test
//! `crates/verifier/tests/semantics_coverage.rs::semantics_artifact_is_current`
//! regenerates it in-memory and fails on any difference. Regenerate the
//! committed file with `TYU_EXPORT_SEMANTICS=1 cargo test -p verifier`.

use ir::{
    AddrOfBase, Atom, BlockId, CapSet, CmpKind, EffectSet, OpKind, Sig, StackBound, TrapCode,
    TY_BOOL, TY_I64,
};

/// Version of the op semantics this table (and every artifact encoding
/// meaning derived from it) carries. MUST be bumped when a row's meaning
/// changes; verdict caches are keyed on it (static-verification.md Q3,
/// ir-op-semantics.md §7).
pub const SEMANTICS_VERSION: &str = "tyu.ir-sem/1.0";

/// The OEL (Obligation Expression Language) projection of an op
/// (static-verification.md §4 Q2, §6.3, §7.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OelProj {
    /// The op contributes the value operator to obligation formulas and its
    /// result is a pure function of the abstract state (tracked intervals).
    Value(ValueOp),
    /// No OEL projection — obligations treat this op's effect as opaque:
    /// abstract evaluation yields `⊤` (never discharges on its own).
    Opaque,
    /// Control flow / trap termination — not a value step in a formula.
    Control,
}

/// The value-level OEL operators, one per value-projecting op family
/// (ir-op-semantics.md §5, OEL column).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueOp {
    /// `ConstI64` / `ConstBool` — a known constant.
    Const,
    Add,
    Sub,
    Mul,
    /// `Cast` — narrowing/widening conversion; the interval engine intersects
    /// with the subtype range (`[lo,hi]`), which is where `InRange` heads
    /// come from.
    Cast,
    /// `Bitcast` — same-width reinterpretation; identity on intervals.
    Bitcast,
    Cmp(CmpKind),
    And,
    Or,
    Not,
    /// Read of a tracked local slot.
    LocalGet,
    /// Write of a tracked local slot.
    LocalSet,
    /// Slot permutation (`Origin` moves with the slot — the E3312 substrate).
    Dup,
    Drop,
    Swap,
}

/// One row of the semantics table: everything verification needs to know
/// about one printable op form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticsRow {
    /// Representative op instance for this row (payload values are
    /// placeholders; only the variant and payload-keyed discriminants —
    /// `CmpKind`, `AddrOf` mutability — are meaningful).
    pub op: OpKind,
    /// Canonical op-text form: the exact first token `--emit=ir` prints for
    /// this op form (one canonical printer, two consumers —
    /// static-verification.md §6.1).
    pub mnemonic: &'static str,
    /// Data-stack slots popped.
    pub pops: u8,
    /// Data-stack slots pushed.
    pub pushes: u8,
    /// Intrinsic effect contribution of executing this op
    /// (abi-contract.md §4.1 wire bits). `Call` additionally carries its
    /// callee's declared `performs` in the op payload; the intrinsic
    /// contribution of the `call` form itself is empty.
    pub effect: EffectSet,
    pub oel: OelProj,
}

/// Row count: one per printable op form — 34 single-form variants, plus
/// six `Cmp` kinds and two `AddrOf` mutabilities.
pub const SEMANTICS_ROWS: usize = 42;

/// The semantics table, in IR text-printer order (the order `OpKind` is
/// declared). Deterministic; no heap.
pub fn semantics() -> [SemanticsRow; SEMANTICS_ROWS] {
    // Placeholder used only to size the buffer before the match fills it;
    // never observable (every slot is overwritten below, checked by the
    // `filled` assert).
    const PLACEHOLDER: SemanticsRow = SemanticsRow {
        op: OpKind::Ret,
        mnemonic: "\0unfilled",
        pops: 0,
        pushes: 0,
        effect: EffectSet::empty(),
        oel: OelProj::Opaque,
    };
    let mut rows = [PLACEHOLDER; SEMANTICS_ROWS];
    let mut n = 0usize;
    macro_rules! push {
        ($row:expr) => {{
            rows[n] = $row;
            n += 1;
        }};
    }

    // Wildcard-free over `OpKind` — adding a variant is a compile error until
    // a row (or rows) is added here. The row and its representative in
    // `representative_ops` (directly below) are one edit: this match and that
    // list must stay in lockstep, and the generated ops.json drift test pins
    // the result.
    for op in representative_ops() {
        match op {
            OpKind::ConstI64(_) => push!(SemanticsRow {
                op,
                mnemonic: "const_i64",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Const),
            }),
            OpKind::ConstBool(_) => push!(SemanticsRow {
                op,
                mnemonic: "const_bool",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Const),
            }),
            OpKind::ConstStr(_) => push!(SemanticsRow {
                op,
                mnemonic: "const_str",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::AddrOf { mutable, .. } => push!(SemanticsRow {
                op,
                mnemonic: if mutable { "addr_of_mut" } else { "addr_of" },
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                // Addresses are not in the integer value domain (memory is
                // not modeled — Q4); MMIO-bounds formulas use descriptor
                // facts, not AddrOf evaluation.
                oel: OelProj::Opaque,
            }),
            OpKind::MmioPlace { .. } => push!(SemanticsRow {
                op,
                mnemonic: "mmio_place",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::ScopedEnter { .. } => push!(SemanticsRow {
                op,
                mnemonic: "scoped_enter",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::TaskSpawn { .. } => push!(SemanticsRow {
                op,
                mnemonic: "task_spawn",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::PtrAddConst { .. } => push!(SemanticsRow {
                op,
                mnemonic: "ptr_add_const",
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::PtrAddIndex { .. } => push!(SemanticsRow {
                op,
                mnemonic: "ptr_add_index",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::Dup { .. } => push!(SemanticsRow {
                op,
                mnemonic: "dup",
                pops: 1,
                pushes: 2,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Dup),
            }),
            OpKind::Drop { .. } => push!(SemanticsRow {
                op,
                mnemonic: "drop",
                pops: 1,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Drop),
            }),
            OpKind::Swap { .. } => push!(SemanticsRow {
                op,
                mnemonic: "swap",
                pops: 2,
                pushes: 2,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Swap),
            }),
            OpKind::AddI64 => push!(SemanticsRow {
                op,
                mnemonic: "add_i64",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Add),
            }),
            OpKind::SubI64 => push!(SemanticsRow {
                op,
                mnemonic: "sub_i64",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Sub),
            }),
            OpKind::MulI64 => push!(SemanticsRow {
                op,
                mnemonic: "mul_i64",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Mul),
            }),
            OpKind::Cmp { kind, .. } => {
                let oel = OelProj::Value(ValueOp::Cmp(kind));
                let mnemonic = match kind {
                    CmpKind::Lt => "cmp_lt",
                    CmpKind::Le => "cmp_le",
                    CmpKind::Gt => "cmp_gt",
                    CmpKind::Ge => "cmp_ge",
                    CmpKind::Eq => "cmp_eq",
                    CmpKind::Ne => "cmp_ne",
                };
                push!(SemanticsRow {
                    op,
                    mnemonic,
                    pops: 2,
                    pushes: 1,
                    effect: EffectSet::empty(),
                    oel,
                });
            }
            OpKind::AndBool => push!(SemanticsRow {
                op,
                mnemonic: "and_bool",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::And),
            }),
            OpKind::OrBool => push!(SemanticsRow {
                op,
                mnemonic: "or_bool",
                pops: 2,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Or),
            }),
            OpKind::NotBool => push!(SemanticsRow {
                op,
                mnemonic: "not_bool",
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Not),
            }),
            OpKind::InterruptDisable => push!(SemanticsRow {
                op,
                mnemonic: "interrupt_disable",
                pops: 0,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::InterruptEnable => push!(SemanticsRow {
                op,
                mnemonic: "interrupt_enable",
                pops: 0,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::LocalSet { .. } => push!(SemanticsRow {
                op,
                mnemonic: "local_set",
                pops: 1,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::LocalSet),
            }),
            OpKind::LocalGet { .. } => push!(SemanticsRow {
                op,
                mnemonic: "local_get",
                pops: 0,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::LocalGet),
            }),
            OpKind::Cast { .. } => push!(SemanticsRow {
                op,
                mnemonic: "cast",
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Cast),
            }),
            OpKind::Bitcast { .. } => push!(SemanticsRow {
                op,
                mnemonic: "bitcast",
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Value(ValueOp::Bitcast),
            }),
            OpKind::Call { .. } => push!(SemanticsRow {
                op,
                mnemonic: "call",
                // Per the representative sig (1 -> 1); the real transition is
                // `( sig.in_len -- sig.out_len )` (ir-op-semantics.md §5).
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::Load { .. } => push!(SemanticsRow {
                op,
                mnemonic: "load",
                pops: 1,
                pushes: 1,
                effect: EffectSet::empty(),
                // Load yields ⊤ unconditionally (memory not modeled — Q4).
                oel: OelProj::Opaque,
            }),
            OpKind::Store { .. } => push!(SemanticsRow {
                op,
                mnemonic: "store",
                pops: 2,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Opaque,
            }),
            OpKind::MmioVolLoad { .. } => push!(SemanticsRow {
                op,
                mnemonic: "vol_load",
                pops: 1,
                pushes: 1,
                effect: EffectSet::from_bits(EffectSet::MMIO),
                oel: OelProj::Opaque,
            }),
            OpKind::MmioVolStore { .. } => push!(SemanticsRow {
                op,
                mnemonic: "vol_store",
                pops: 2,
                pushes: 0,
                effect: EffectSet::from_bits(EffectSet::MMIO),
                oel: OelProj::Opaque,
            }),
            OpKind::MmioVolLoadField { .. } => push!(SemanticsRow {
                op,
                mnemonic: "vol_load_field",
                pops: 1,
                pushes: 1,
                effect: EffectSet::from_bits(EffectSet::MMIO),
                oel: OelProj::Opaque,
            }),
            OpKind::MmioVolStoreField { .. } => push!(SemanticsRow {
                op,
                mnemonic: "vol_store_field",
                pops: 2,
                pushes: 0,
                effect: EffectSet::from_bits(EffectSet::MMIO),
                oel: OelProj::Opaque,
            }),
            OpKind::TrapIfFalse { .. } => push!(SemanticsRow {
                op,
                mnemonic: "trap_if_false",
                pops: 1,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Control,
            }),
            OpKind::Br { .. } => push!(SemanticsRow {
                op,
                mnemonic: "br",
                pops: 0,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Control,
            }),
            OpKind::BrIf { .. } => push!(SemanticsRow {
                op,
                mnemonic: "br_if",
                pops: 1,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Control,
            }),
            OpKind::Ret => push!(SemanticsRow {
                op,
                mnemonic: "ret",
                pops: 0,
                pushes: 0,
                effect: EffectSet::empty(),
                oel: OelProj::Control,
            }),
        }
    }

    debug_assert!(n == SEMANTICS_ROWS, "representative list out of sync");
    for row in rows.iter() {
        debug_assert!(row.mnemonic != PLACEHOLDER.mnemonic, "unfilled row");
    }
    rows
}

/// One representative op per printable form, in table order. The payloads are
/// placeholders; keep this list in lockstep with the `match` above.
fn representative_ops() -> [OpKind; SEMANTICS_ROWS] {
    let atom = |b: &[u8]| Atom::new(b).expect("representative atom fits");
    let sig = Sig {
        in_len: 1,
        out_len: 1,
        ..Sig::empty()
    };
    [
        OpKind::ConstI64(0),
        OpKind::ConstBool(false),
        OpKind::ConstStr(ir::Span::UNKNOWN),
        OpKind::AddrOf {
            place: atom(b"p"),
            mutable: false,
            base: AddrOfBase::Runtime,
        },
        OpKind::AddrOf {
            place: atom(b"p"),
            mutable: true,
            base: AddrOfBase::Runtime,
        },
        OpKind::MmioPlace {
            place: atom(b"r"),
            aperture: 0,
            offset: 0,
        },
        OpKind::ScopedEnter { ty: TY_I64, len: 0 },
        OpKind::TaskSpawn {
            name: atom(b"t"),
            task_ty: TY_I64,
        },
        OpKind::PtrAddConst {
            ty: TY_I64,
            offset: 0,
        },
        OpKind::PtrAddIndex {
            ty: TY_I64,
            scale: 1,
        },
        OpKind::Dup { ty: TY_I64 },
        OpKind::Drop { ty: TY_I64 },
        OpKind::Swap {
            a: TY_I64,
            b: TY_I64,
        },
        OpKind::AddI64,
        OpKind::SubI64,
        OpKind::MulI64,
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Lt,
        },
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Le,
        },
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Gt,
        },
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Ge,
        },
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Eq,
        },
        OpKind::Cmp {
            out: TY_BOOL,
            kind: CmpKind::Ne,
        },
        OpKind::AndBool,
        OpKind::OrBool,
        OpKind::NotBool,
        OpKind::InterruptDisable,
        OpKind::InterruptEnable,
        OpKind::LocalSet { slot: 0, ty: TY_I64 },
        OpKind::LocalGet { slot: 0, ty: TY_I64 },
        OpKind::Cast {
            from: TY_I64,
            to: TY_I64,
        },
        OpKind::Bitcast {
            from: TY_I64,
            to: TY_I64,
        },
        OpKind::Call {
            name: atom(b"f"),
            sig,
            performs: EffectSet::empty(),
            requires: CapSet::empty(),
            bound: StackBound::ID,
        },
        OpKind::Load { ty: TY_I64 },
        OpKind::Store { ty: TY_I64 },
        OpKind::MmioVolLoad {
            ty: TY_I64,
            place: atom(b"r"),
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        OpKind::MmioVolStore {
            ty: TY_I64,
            place: atom(b"r"),
            write_kind: ir::WriteKind::Plain,
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        OpKind::MmioVolLoadField {
            reg_ty: TY_I64,
            field_ty: TY_I64,
            place: atom(b"r"),
            mask: 1,
            shift: 0,
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        OpKind::MmioVolStoreField {
            reg_ty: TY_I64,
            field_ty: TY_I64,
            place: atom(b"r"),
            mask: 1,
            shift: 0,
            write_kind: ir::WriteKind::Plain,
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        OpKind::TrapIfFalse {
            code: TrapCode::ContractFail,
        },
        OpKind::Br {
            target: BlockId(0),
        },
        OpKind::BrIf {
            then_tgt: BlockId(0),
            else_tgt: BlockId(1),
        },
        OpKind::Ret,
    ]
}

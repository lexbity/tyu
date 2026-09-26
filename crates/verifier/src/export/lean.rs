//! The Lean 4 port renderer (PLAN-VERIFY-3 P3.1).
//!
//! Renders the generated data layer of `verification/ports/lean/`:
//!
//! - `Tyu/IR/Op.lean` — the op-form enum (one constructor per printable
//!   `--emit=ir` mnemonic) + the mnemonic projection;
//! - `Tyu/IR/Semantics.lean` — the semantics table (`rows`, mirroring
//!   `src/semantics/ops.json`) + `SEMANTICS_total` (the completeness proof:
//!   every op form has a row; a hand-edited generated file dropping a row
//!   fails the port build);
//! - `Tyu/IR/Target.lean` — the `TargetSpec` records (the four recognized
//!   triples), mirroring `verifier::target::TARGETS`;
//! - `Tyu/Mem.lean` — the MemModel/Device/Services *interfaces* with
//!   law-statement placeholders (structure fields, never axioms).
//!
//! Every row's representation is produced by a **wildcard-free match over
//! the row mnemonic** (`op_ctor`): adding an op form to the semantics table
//! without adding its Lean constructor here is a Rust compile error (FR-12,
//! "new row without rendering = Rust compile error"), and the drift test
//! byte-pins the committed files.
//!
//! The renderer is `host-side, render-time only` — it is compiled into the
//! verifier lib but never linked into any hosted artifact, so
//! `alloc::format!` is acceptable here (the crate's no-`format!` rule governs
//! the runtime JSON writers in `codec.rs`/`interp.rs`).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::export::{GeneratedFile, PortRenderer};
use crate::semantics::{
    semantics, OelProj, SemanticsRow, ValueOp, SEMANTICS_ROWS, SEMANTICS_VERSION,
};
use crate::target::{TargetSpec, TARGETS};

/// The row-mnemonic registry of the rendered layer. The renderer's
/// constructor projection is a **wildcard-free match over this enum**: a
/// variant added without a rendering arm is a Rust compile error (FR-12,
/// "new row without rendering = Rust compile error"). `Mne::parse` fails
/// closed on an unregistered mnemonic — every row of `semantics()` flows
/// through it in `render()`, so a new row without a registry variant trips
/// the drift/coverage tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mne {
    ConstI64,
    ConstBool,
    ConstStr,
    AddrOf,
    AddrOfMut,
    MmioPlace,
    ScopedEnter,
    TaskSpawn,
    PtrAddConst,
    PtrAddIndex,
    Dup,
    Drop,
    Swap,
    AddI64,
    SubI64,
    MulI64,
    CmpLt,
    CmpLe,
    CmpGt,
    CmpGe,
    CmpEq,
    CmpNe,
    AndBool,
    OrBool,
    NotBool,
    InterruptDisable,
    InterruptEnable,
    LocalSet,
    LocalGet,
    Cast,
    Bitcast,
    Call,
    Load,
    Store,
    VolLoad,
    VolStore,
    VolLoadField,
    VolStoreField,
    TrapIfFalse,
    Br,
    BrIf,
    Ret,
}

impl Mne {
    /// The canonical `--emit=ir` mnemonic of the row. `None` means the
    /// mnemonic has no registry entry (a semantics row the renderer does
    /// not know — the render path aborts on it; the coverage drift test
    /// pins every row through this function).
    pub fn parse(mnemonic: &str) -> Option<Mne> {
        match mnemonic {
            "const_i64" => Some(Mne::ConstI64),
            "const_bool" => Some(Mne::ConstBool),
            "const_str" => Some(Mne::ConstStr),
            "addr_of" => Some(Mne::AddrOf),
            "addr_of_mut" => Some(Mne::AddrOfMut),
            "mmio_place" => Some(Mne::MmioPlace),
            "scoped_enter" => Some(Mne::ScopedEnter),
            "task_spawn" => Some(Mne::TaskSpawn),
            "ptr_add_const" => Some(Mne::PtrAddConst),
            "ptr_add_index" => Some(Mne::PtrAddIndex),
            "dup" => Some(Mne::Dup),
            "drop" => Some(Mne::Drop),
            "swap" => Some(Mne::Swap),
            "add_i64" => Some(Mne::AddI64),
            "sub_i64" => Some(Mne::SubI64),
            "mul_i64" => Some(Mne::MulI64),
            "cmp_lt" => Some(Mne::CmpLt),
            "cmp_le" => Some(Mne::CmpLe),
            "cmp_gt" => Some(Mne::CmpGt),
            "cmp_ge" => Some(Mne::CmpGe),
            "cmp_eq" => Some(Mne::CmpEq),
            "cmp_ne" => Some(Mne::CmpNe),
            "and_bool" => Some(Mne::AndBool),
            "or_bool" => Some(Mne::OrBool),
            "not_bool" => Some(Mne::NotBool),
            "interrupt_disable" => Some(Mne::InterruptDisable),
            "interrupt_enable" => Some(Mne::InterruptEnable),
            "local_set" => Some(Mne::LocalSet),
            "local_get" => Some(Mne::LocalGet),
            "cast" => Some(Mne::Cast),
            "bitcast" => Some(Mne::Bitcast),
            "call" => Some(Mne::Call),
            "load" => Some(Mne::Load),
            "store" => Some(Mne::Store),
            "vol_load" => Some(Mne::VolLoad),
            "vol_store" => Some(Mne::VolStore),
            "vol_load_field" => Some(Mne::VolLoadField),
            "vol_store_field" => Some(Mne::VolStoreField),
            "trap_if_false" => Some(Mne::TrapIfFalse),
            "br" => Some(Mne::Br),
            "br_if" => Some(Mne::BrIf),
            "ret" => Some(Mne::Ret),
            _ => None,
        }
    }

    /// The Lean op-form constructor name (equal to the mnemonic by
    /// construction). Wildcard-free over `Mne`: a new variant without a
    /// rendering arm is a compile error.
    pub fn ctor(self) -> &'static str {
        match self {
            Mne::ConstI64 => "const_i64",
            Mne::ConstBool => "const_bool",
            Mne::ConstStr => "const_str",
            Mne::AddrOf => "addr_of",
            Mne::AddrOfMut => "addr_of_mut",
            Mne::MmioPlace => "mmio_place",
            Mne::ScopedEnter => "scoped_enter",
            Mne::TaskSpawn => "task_spawn",
            Mne::PtrAddConst => "ptr_add_const",
            Mne::PtrAddIndex => "ptr_add_index",
            Mne::Dup => "dup",
            Mne::Drop => "drop",
            Mne::Swap => "swap",
            Mne::AddI64 => "add_i64",
            Mne::SubI64 => "sub_i64",
            Mne::MulI64 => "mul_i64",
            Mne::CmpLt => "cmp_lt",
            Mne::CmpLe => "cmp_le",
            Mne::CmpGt => "cmp_gt",
            Mne::CmpGe => "cmp_ge",
            Mne::CmpEq => "cmp_eq",
            Mne::CmpNe => "cmp_ne",
            Mne::AndBool => "and_bool",
            Mne::OrBool => "or_bool",
            Mne::NotBool => "not_bool",
            Mne::InterruptDisable => "interrupt_disable",
            Mne::InterruptEnable => "interrupt_enable",
            Mne::LocalSet => "local_set",
            Mne::LocalGet => "local_get",
            Mne::Cast => "cast",
            Mne::Bitcast => "bitcast",
            Mne::Call => "call",
            Mne::Load => "load",
            Mne::Store => "store",
            Mne::VolLoad => "vol_load",
            Mne::VolStore => "vol_store",
            Mne::VolLoadField => "vol_load_field",
            Mne::VolStoreField => "vol_store_field",
            Mne::TrapIfFalse => "trap_if_false",
            Mne::Br => "br",
            Mne::BrIf => "br_if",
            Mne::Ret => "ret",
        }
    }
}

/// The Lean op-form constructor name of a row mnemonic. `None` when the
/// mnemonic has no registry entry. A registered-but-unrendered `Mne`
/// variant is a Rust compile error (the wildcard-free `ctor` match);
/// a row without a registry entry fails the coverage drift test and the
/// render path (`.expect` below — fail-closed, never a silent gap).
pub fn op_ctor(mnemonic: &str) -> Option<&'static str> {
    Mne::parse(mnemonic).map(Mne::ctor)
}

/// The fail-closed message for an unregistered row at render time.
const REGISTRY_MISSING: &str =
    "verifier::export::lean: semantics row has no Mne registry entry — add it to Mne (FR-12)";

/// The OEL projection's stable text form (the same rendering
/// `semantics_coverage` pins into `ops.json`).
fn oel_str(oel: OelProj) -> String {
    match oel {
        OelProj::Value(v) => {
            let name = match v {
                ValueOp::Const => "const",
                ValueOp::Add => "add",
                ValueOp::Sub => "sub",
                ValueOp::Mul => "mul",
                ValueOp::Cast => "cast",
                ValueOp::Bitcast => "bitcast",
                ValueOp::Cmp(k) => match k {
                    ir::CmpKind::Lt => "cmp_lt",
                    ir::CmpKind::Le => "cmp_le",
                    ir::CmpKind::Gt => "cmp_gt",
                    ir::CmpKind::Ge => "cmp_ge",
                    ir::CmpKind::Eq => "cmp_eq",
                    ir::CmpKind::Ne => "cmp_ne",
                },
                ValueOp::And => "and",
                ValueOp::Or => "or",
                ValueOp::Not => "not",
                ValueOp::LocalGet => "local_get",
                ValueOp::LocalSet => "local_set",
                ValueOp::Dup => "dup",
                ValueOp::Drop => "drop",
                ValueOp::Swap => "swap",
            };
            format!("value:{name}")
        }
        OelProj::Opaque => "opaque".to_string(),
        OelProj::Control => "control".to_string(),
    }
}

fn file(path: &'static str, content: String) -> GeneratedFile {
    GeneratedFile { path, content }
}

/// Render `Tyu/IR/Op.lean`: the op-form enum + the mnemonic projection.
fn render_op_lean(rows: &[SemanticsRow]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "namespace Tyu.IR\n\n\
         /- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by\n\
            `crates/verifier/src/export/lean.rs` — DO NOT EDIT. Drift-locked:\n\
            `TYU_EXPORT_PORTS=1 cargo test -p verifier --test export_drift`\n\
            regenerates; the committed file must byte-match. -/\n\
         /-- One printable op form: the canonical `--emit=ir` mnemonic (one\n\
         constructor per row of `verifier::semantics`, {SEMANTICS_ROWS} rows). -/\n\
         inductive OpForm where\n"
    ));
    for row in rows {
        let ctor = op_ctor(row.mnemonic).expect(REGISTRY_MISSING);
        out.push_str(&format!("  | {ctor}\n"));
    }
    out.push_str("  deriving DecidableEq, Repr, Inhabited\n\n");
    out.push_str(
        "/-- The canonical `--emit=ir` mnemonic of each op form (the `mnemonic`\n\
         column of the semantics table; the op-text parser dispatches on it). -/\n\
         def OpForm.mnemonic : OpForm → String\n",
    );
    for row in rows {
        let ctor = op_ctor(row.mnemonic).expect(REGISTRY_MISSING);
        out.push_str(&format!("  | .{ctor} => \"{}\"\n", row.mnemonic));
    }
    out.push_str("\nend Tyu.IR\n");
    out
}

/// Render `Tyu/IR/Semantics.lean`: the rows list (mirroring `ops.json`) and
/// the completeness theorem `SEMANTICS_total`.
fn render_semantics_lean(rows: &[SemanticsRow]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "import Tyu.IR.Op\n\
         import Std\n\n\
         namespace Tyu.IR\n\n\
         /- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by\n\
            `crates/verifier/src/export/lean.rs` — DO NOT EDIT. Mirrors\n\
            `src/semantics/ops.json` ({SEMANTICS_VERSION}). -/\n\
         /-- One row of the normative semantics table (`verifier::semantics::SemanticsRow`). -/\n\
         structure SemanticsRow where\n\
           form : OpForm\n\
           mnemonic : String\n\
           pops : Nat\n\
           pushes : Nat\n\
           effect_bits : Nat\n\
           oel : String\n\
           deriving DecidableEq, Repr\n\n\
         /-- The table, in IR text-printer order. -/\n\
         def rows : List SemanticsRow := [\n"
    ));
    for (i, row) in rows.iter().enumerate() {
        let comma = if i + 1 == rows.len() { "" } else { "," };
        out.push_str(&format!(
            "  {{ form := .{ctor}, mnemonic := \"{mn}\", pops := {pops}, pushes := {pushes}, \
             effect_bits := {eff}, oel := \"{oel}\" }}{comma}\n",
            ctor = op_ctor(row.mnemonic).expect(REGISTRY_MISSING),
            mn = row.mnemonic,
            pops = row.pops,
            pushes = row.pushes,
            eff = row.effect.bits(),
            oel = oel_str(row.oel),
        ));
    }
    out.push_str("]\n\n");
    out.push_str(&format!(
        "theorem row_count : rows.length = {SEMANTICS_ROWS} := by\n  decide\n\n"
    ));
    // The per-op stack effect, generated from the same rows (one per form):
    // T-C (stack algebra, P4) proves the concrete step against these.
    out.push_str(
        "/-- The table's stack effect, per form (P4's T-C substrate):\n\
         `pops` slots consumed, `pushes` produced, `net = pushes - pops`. -/\n\
         def OpForm.pops : OpForm → Nat\n",
    );
    for row in rows {
        out.push_str(&format!(
            "  | .{} => {}\n",
            op_ctor(row.mnemonic).expect(REGISTRY_MISSING),
            row.pops
        ));
    }
    out.push_str("\ndef OpForm.pushes : OpForm → Nat\n");
    for row in rows {
        out.push_str(&format!(
            "  | .{} => {}\n",
            op_ctor(row.mnemonic).expect(REGISTRY_MISSING),
            row.pushes
        ));
    }
    out.push_str("\ndef OpForm.net : OpForm → Int\n");
    for row in rows {
        out.push_str(&format!(
            "  | .{} => {}\n",
            op_ctor(row.mnemonic).expect(REGISTRY_MISSING),
            row.pushes as i64 - row.pops as i64
        ));
    }
    out.push_str("\n-- sanity pin: the generated table is internally consistent\n");
    out.push_str(
        "theorem pops_eq_row : ∀ f : OpForm, ∃ r : SemanticsRow, r ∈ rows ∧ r.form = f ∧ r.pops = OpForm.pops f := by\n  intro f\n  cases f with\n"
    );
    for row in rows {
        let ctor = op_ctor(row.mnemonic).expect(REGISTRY_MISSING);
        out.push_str(&format!("    | {ctor} => decide\n"));
    }
    out.push_str(
        "/-- Completeness: every op form has a row in the table (P3.1's\n\
         `SEMANTICS_total`; kernel-checked by `decide` — a hand-edited generated\n\
         file dropping a row fails the port build). -/\n\
         theorem SEMANTICS_total : ∀ f : OpForm, ∃ r : SemanticsRow, r ∈ rows ∧ r.form = f := by\n\
         \x20 intro f\n\
         \x20 cases f with\n",
    );
    for row in rows {
        let ctor = op_ctor(row.mnemonic).expect(REGISTRY_MISSING);
        out.push_str(&format!("    | {ctor} => decide\n"));
    }
    out.push_str("\nend Tyu.IR\n");
    out
}

/// Render `Tyu/IR/Target.lean`: the recognized `TargetSpec` records in the
/// verifier's deterministic order.
fn render_target_lean(targets: &[TargetSpec]) -> String {
    let mut out = String::new();
    out.push_str(
        "import Std\n\n\
         namespace Tyu.IR\n\n\
         /- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by\n\
            `crates/verifier/src/export/lean.rs` — DO NOT EDIT. Mirrors\n\
            `verifier::target::TARGETS`. \"Semantics is a function of (TargetSpec,\n\
            MemModel-instance)\" (§Q3): the four fields relativize every statement. -/\n\
         /-- The target-identity record (`verifier::target::TargetSpec`): triple,\n\
         data-stack slot width, integer width, ABI arch tag. -/\n\
         structure TargetSpec where\n\
           triple : String\n\
           slotBytes : Nat\n\
           wordBits : Nat\n\
           archTag : Nat\n\
           deriving DecidableEq, Repr, Inhabited\n\n\
         /-- The recognized targets, in `verifier::target::TARGETS` order. -/\n",
    );
    for t in targets {
        out.push_str(&format!(
            "def {name} : TargetSpec := {{ triple := \"{triple}\", slotBytes := {slot}, wordBits := {bits}, archTag := {tag} }}\n",
            name = target_ident(t.triple),
            triple = t.triple,
            slot = t.slot_bytes,
            bits = t.word_bits,
            tag = t.arch_tag,
        ));
    }
    out.push_str("\ndef TARGETS : List TargetSpec := [");
    for (i, t) in targets.iter().enumerate() {
        let sep = if i + 1 == targets.len() { "" } else { ", " };
        out.push_str(&format!(" {}{sep}", target_ident(t.triple)));
    }
    out.push_str(" ]\n\n");
    out.push_str(&format!(
        "theorem target_count : TARGETS.length = {} := by\n  decide\n\nend Tyu.IR\n",
        targets.len()
    ));
    out
}

/// A Lean-safe identifier for a target record, from a triple (mapping
/// punctuation to underscores).
fn target_ident(triple: &str) -> String {
    triple.replace(['-', '.'], "_")
}

/// Render `Tyu/Mem.lean`: the memory-model interfaces + law-statement
/// placeholders (structure fields — never axioms). Built from a raw string
/// so indentation is preserved (Lean record literals require indented
/// continuation lines).
fn render_mem_lean() -> String {
    r#"namespace Tyu.Mem

/- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by
   `crates/verifier/src/export/lean.rs` — DO NOT EDIT. The
   MemModel/DeviceModel/Services *interfaces* rendered from the verifier's
   memory-model boundary (`verifier::mem::MemModel`), with law-statement
   placeholders (P3.1: "GEN interfaces + law-statement placeholders"). The
   placeholders are structure fields — **data, not axioms** — so the library
   adds no axioms; the T-D memory-model laws are proven in Phase P12 over
   real bundle instances. -/
/-- The abstract value domain of the memory-model boundary (mirrors the
value lattice `Interval` of `verifier::interval`). -/
inductive IntervalVal where
  | bottom
  | top
  | range (lo : Int) (hi : Int)
  deriving DecidableEq, Repr, Inhabited

/-- The memory-model interface (`verifier::mem::MemModel`): an abstract
load/store and the MMIO aperture read/write oracle (§Q13: the aperture
read is the injected nondeterminism, width-bounded by the register). -/
structure MemModel where
  load : IntervalVal → Nat → IntervalVal
  store : IntervalVal → IntervalVal → IntervalVal
  apertureRead : String → Nat → IntervalVal
  apertureWrite : String → IntervalVal → IntervalVal
  deriving Inhabited

/-- Law-statement placeholders for the interface (the T-D family). These
are Prop fields of a structure parameterized by the model — data, not
axioms; Phase P12 states and proves the real laws. -/
structure MemModelLaws (M : MemModel) where
  loadSubsetTop : Prop
  storeLoadPoint : Prop
  apertureWidthBound : Prop
  deriving Inhabited

/-- The default model (`verifier::mem::FlatMem`): memory unmodeled, MMIO
reads entirely nondeterministic (top — sound for any register width). -/
def flatMem : MemModel := {
  load := fun _ _ => IntervalVal.top,
  store := fun _ v => v,
  apertureRead := fun _ _ => IntervalVal.top,
  apertureWrite := fun _ _ => IntervalVal.top }

end Tyu.Mem
"#
    .to_string()
}

/// The Lean port renderer (P3.1).
pub struct LeanRenderer;

impl PortRenderer for LeanRenderer {
    fn port_name(&self) -> &'static str {
        "lean"
    }

    fn render(&self) -> Vec<GeneratedFile> {
        let rows = semantics();
        vec![
            file("Tyu/IR/Op.lean", render_op_lean(&rows)),
            file("Tyu/IR/Semantics.lean", render_semantics_lean(&rows)),
            file("Tyu/IR/Target.lean", render_target_lean(&TARGETS)),
            file("Tyu/Mem.lean", render_mem_lean()),
        ]
    }
}

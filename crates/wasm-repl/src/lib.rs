//! The Tyu console interpreter fork (PLAN-WASM-1, `devdocs/plans/wasm-repl.md`).
//!
//! `wasm-repl` is a *fork* of the verifier's test-only source-fragment
//! interpreter (`crates/verifier/src/src_interp.rs`, frozen upstream) reworked
//! into a full, concrete, fuel-metered interpreter over the production IR
//! (`ir::Module` structs). It is a product, not a test surface (DEC-1, §Q1).
//!
//! P1 shipped the fork's provenance ([`frag`], pinned to the frozen upstream
//! by the G52 differential). P2 shipped the production engine: [`compile`]
//! (source → `build_ir` → `ir::Module` through the [`SourceTable`] seam),
//! [`engine`] (the exhaustive concrete `OpKind` interpreter), [`heap`]
//! (the §6.7 region arena), [`builtins`] (`testio.*` + the region ABI),
//! plus [`value`]/[`fuel`]/[`trace`]/[`diag`]/[`periph`]. Byte-parity with
//! hosted native is the G53 differential (`tests/exec_parity.rs`). P3 adds
//! the REPL **session** layer — [`ReplSession`] with persistent stack/heap,
//! transactional phrases (rollback via the [`journal`], DEC-4..DEC-6),
//! phrase signature inference ([`phrase`]), and the byte-pinned session
//! goldens (`test-goldens/repl/`, G56). P4 completes `OpKind` coverage:
//! the MMIO family (`AddrOf{Mmio}`/`MmioPlace`/`MmioVol*`) executes
//! concretely against the device boundary — [`periph`] gains the scripted
//! device and the emulated-aperture RAM device, [`mmio`] owns the
//! `WriteKind` RMW bit math + aperture table, and the mmio fixture
//! differential extends G53 (compile-parity + self-checking device runs).
//! Task/interrupt constructs remain rejected at submit (E9156, FR-3).
//!
//! Discipline (FR-1, I-6): `#![no_std]` + `alloc`, in-tree dependencies only
//! (R-A), `#![forbid(unsafe_code)]`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

// Re-exported by the tests/ harnesses for the G52 differential.
pub mod frag;

/// The production engine modules (P2). Declaration order is the crate's
/// logical surface: `frag` first for its G52 provenance role, then the
/// compiler-front seam, the diagnostics, the engine proper, and its
/// building blocks.
pub mod builtins;
pub mod compile;
pub mod diag;
pub mod engine;
pub mod fuel;
pub mod heap;
pub mod journal;
pub mod mmio;
pub mod periph;
pub mod phrase;
pub mod session;
pub mod trace;
pub mod value;

pub use compile::{compile, Compiled, DictTarget, SourceTable, X86_64_LINUX_SPEC};
pub use diag::Diag;
pub use engine::{Engine, ExecState, Outcome};
pub use fuel::Fuel;
pub use journal::{Effect, Journal};
pub use mmio::{apply_write_kind, extract_field, insert_field, ApertureEntry};
pub use periph::{Access, PeriphError, Peripheral, RamDevice, ScriptStep, ScriptedPeripheral};
/// The canonical `--emit=ir` renderer (DEC-13): the IR pane and the G53-b
/// parity tests render through this — the SAME bytes `langc --emit=ir` prints.
pub use semantics::typecheck::render_ir;
pub use session::{ReplSession, SessionOpts, StackCell, Status, SubmitResult};
pub use trace::Trace;

/// Engine surface version (PLAN-WASM-1 §10): bumped on any change to the
/// REPL's execution semantics. The browser exposes it for bug reports; the
/// wasm artifact hash plus this version identify a page's engine exactly.
pub const ENGINE_VERSION: u32 = 4;

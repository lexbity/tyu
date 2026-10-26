//! The Tyu console interpreter fork (PLAN-WASM-1, archived at
//! `devdocs/plans/done/wasm-repl.md`; the plan of record is
//! `devdocs/plans/web-ide.md`).
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

// The wasm32 cdylib (P5) needs a global allocator and panic handler for the
// bindgen boundary; std on `wasm32-unknown-unknown` provides both (std is an
// in-toolchain crate, not an external dependency — I-6/R-A unaffected).
#[cfg(target_arch = "wasm32")]
extern crate std;

// The HOST cdylib (a byproduct of the crate-type above — only the wasm32
// build is ever consumed) links `std` for the same allocator/panic-handler
// need: a `forbid(unsafe_code)` crate cannot hand-roll either. Every byte of
// this crate's own code still compiles against alloc only (FR-1).
#[cfg(not(target_arch = "wasm32"))]
extern crate std;

// Re-exported by the tests/ harnesses for the G52 differential.
pub mod frag;

/// The production engine modules (P2). Declaration order is the crate's
/// logical surface: `frag` first for its G52 provenance role, then the
/// compiler-front seam, the diagnostics, the engine proper, and its
/// building blocks.
#[cfg(target_arch = "wasm32")]
pub mod abi;
pub mod builtins;
pub mod compile;
pub mod debug;
pub mod diag;
pub mod engine;
pub mod examples_pack;
pub mod fuel;
pub mod heap;
pub mod journal;
pub mod mmio;
pub mod periph;
pub mod phrase;
pub mod session;
pub mod sysroot;
pub mod trace;
pub mod value;

pub use compile::{compile, Compiled, DictTarget, SourceTable, X86_64_LINUX_SPEC};
pub use diag::Diag;
pub use engine::{DetailKind, Engine, ExecState, Outcome, TrapDetail};
pub use fuel::Fuel;
pub use journal::{Effect, Journal};
pub use mmio::{apply_write_kind, extract_field, insert_field, ApertureEntry};
pub use periph::{Access, PeriphError, Peripheral, RamDevice, ScriptStep, ScriptedPeripheral};
/// The canonical `--emit=ir` renderer (DEC-13): the IR pane and the G53-b
/// parity tests render through this — the SAME bytes `langc --emit=ir` prints.
pub use semantics::typecheck::render_ir;
pub use session::{ReplSession, SessionOpts, StackCell, Status, SubmitResult};
pub use sysroot::{decode_blob, encode_blob, sha256, SysrootTable};
pub use trace::Trace;

/// Property tests over the crate-internal transaction machinery (P2 §7.3,
/// R-3): in-crate so the private checkpoint/rollback surface is exercised
/// directly. Compiled only under `cfg(test)`.
#[cfg(test)]
mod transactional_props;

/// Engine surface version (PLAN-WASM-1 §10): bumped on any change to the
/// REPL's execution semantics. The browser exposes it for bug reports; the
/// wasm artifact hash plus this version identify a page's engine exactly.
pub const ENGINE_VERSION: u32 = 6;
// v6 (PLAN-WASM-2 W1): L2 completed — checked pops (L2a), per-slot Call
// input contracts + word-local type retagging (L2c), block-entry validation
// (L2b, E9158); fabricated-value paths deleted; contract traps carry the
// §Q6/DEC-22 TrapDetail; `tyu.trace/1` gains the FR-35 version+header record;
// the debug snapshot projections (`debug`) and the pooled phrase buffers
// (D-W4) land. Correct programs are unaffected (G53 zero-delta is the proof).
// v5: E9150/E9155/E9157/E9158 diagnostic surfaces, fuel suspension + step
// (§Q8), heap-reservation rollback reclamation (§7.3), dotted-name phrase
// calls, module-form submissions, and the wasm abi boundary (P5).

//! The Tyu console interpreter fork (PLAN-WASM-1, `devdocs/plans/wasm-repl.md`).
//!
//! `wasm-repl` is a *fork* of the verifier's test-only source-fragment
//! interpreter (`crates/verifier/src/src_interp.rs`, frozen upstream) reworked
//! into a full, concrete, fuel-metered interpreter over the production IR
//! (`ir::Module` structs). It is a product, not a test surface (DEC-1, §Q1).
//!
//! Slice P1 ships the fork's **provenance only**: [`frag`] is the verbatim
//! adaptation of the frozen upstream fragment engine, differentially pinned
//! against it by the G52 gate (`tests/frag_parity.rs`, invoked from
//! `ci/wasm.sh`). The full-execution engine (`engine.rs`, [`Value`],
//! [`RegionHeap`], sessions) lands in P2+; nothing here executes beyond the
//! fragment surface on purpose.
//!
//! Discipline (FR-1, I-6): `#![no_std]` + `alloc`, zero external
//! dependencies, `#![forbid(unsafe_code)]`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

// Re-exported by the tests/ harnesses for the G52 differential.
pub mod frag;

/// Engine surface version (PLAN-WASM-1 §10): bumped on any change to the
/// REPL's execution semantics. The browser exposes it for bug reports; the
/// wasm artifact hash plus this version identify a page's engine exactly.
pub const ENGINE_VERSION: u32 = 1;

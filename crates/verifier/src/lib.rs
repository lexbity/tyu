//! Static verification subsystem for Tyu (PLAN-VERIFY-1,
//! `devdocs/plans/static-verification.md`).
//!
//! The verifier converts the toolchain's language-invariant runtime checks
//! into proof obligations, discharges what is provable, and leaves runtime
//! checks exactly for the open residue. This crate is the pure core: data
//! models and algorithms, no I/O.
//!
//! Dependency direction (static-verification.md §5): `ir` → `verifier` →
//! (`semantics`, `langc`) → `tyu`. This crate MUST NOT depend on `semantics`
//! or `langc`; it knows only `ir` and its own model, so external tool authors
//! can reuse its types and codecs without the compiler.
//!
//! Currently implemented:
//! - slice P1: the normative IR op semantics table ([`semantics`]) that every
//!   later slice proves against;
//! - slice P2: the obligation model ([`model`]) and `.obl.json` codecs
//!   ([`codec`]) for the C1–C3 subtype-range sites;
//! - PLAN-VERIFY-3 P1: the canonical statement encoder ([`stmt`]) — the
//!   hash-bound object (`tyu.stmt/1.0`) a developer's proof binds to, plus
//!   the `.obl.json` v2 growth (intent, assumption edges, cycles, target/
//!   model relativity; see `devdocs/plans/developer-proof-pipeline.md`).
//! - PLAN-VERIFY-3 P2: the parameterized reference semantics — verifier-local
//!   target identity ([`target`]), the memory-model boundary ([`mem`]), the
//!   promoted program generator ([`gen`]), and the width-relative
//!   `(TargetSpec, MemModel)` threading through [`interp`].
//! - PLAN-VERIFY-3 P3: the port exporter ([`export`]) — renders the Lean 4
//!   port's generated data layer (op enum, semantics table, target records,
//!   memory-model interfaces) with an exhaustive row registry, byte-drift-
//!   locked against the committed `verification/ports/lean/` files.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod codec;
pub mod export;
pub mod gen;
pub mod interp;
pub mod interval;
pub mod mem;
pub mod model;
pub mod report;
pub mod semantics;
pub mod stmt;
pub mod target;
/// Test-fixture builders (`test-util` feature only; `#[doc(hidden)]`).
#[cfg(feature = "test-util")]
#[doc(hidden)]
pub mod testutil;
pub mod verdict;

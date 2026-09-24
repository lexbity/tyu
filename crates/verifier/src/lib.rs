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
//!   ([`codec`]) for the C1–C3 subtype-range sites.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod codec;
pub mod model;
pub mod report;
pub mod semantics;
pub mod verdict;

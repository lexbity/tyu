#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Semantic analysis, typechecking, and effect checking for Tyu.
//!
//! The crate exports the `typecheck` and `types` modules used by the compiler
//! pipeline.
// Compiler IR-generation functions inherently pass cur, stack, sp, span,
// slice, lex, observer, and more through deep call chains.  Allowing this
// at the crate level avoids dozens of per-function annotations.
#![allow(clippy::too_many_arguments)]

extern crate alloc;

pub mod typecheck;
pub mod types;

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Front-end lexer and parser for Tyu source files.
//!
//! This crate owns the syntax-facing modules used by the compiler pipeline:
//! `fixed`, `lex`, `parse`, `span`, and `token`.

pub mod fixed;
pub mod lex;
pub mod parse;
pub mod span;
pub mod token;

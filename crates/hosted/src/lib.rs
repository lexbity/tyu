#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Host-side runtime glue for Tyu binaries and tests.
//!
//! The crate wraps the loader/runtime boundary used by `langc`, `tyu`, and
//! the execution-test harnesses. Optional encryption support is exposed via
//! the `encryption` feature.

pub mod args;
pub mod c;
pub mod cstr;
pub mod cstrbuf;
pub mod diag;
pub mod env;
pub mod errno;
pub mod fs;
pub mod io;
pub mod mem;
pub mod process;

pub mod hmac_sha256;
pub mod loader;

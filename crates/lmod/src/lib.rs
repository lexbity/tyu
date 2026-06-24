#![no_std]
//! Core `.lmod` module format crate.
//!
//! The public modules cover ABI hashing, headers, modinfo, relocation,
//! signatures, encryption, debug sections, and validation.

extern crate alloc;

pub mod abi_hash;
pub mod debugsec;
pub mod enc;
pub mod hash;
pub mod header;
pub mod modinfo;
pub mod reloc;
pub mod sig;
pub mod validate;

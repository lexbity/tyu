#![no_std]
//! Core loader implementation for Tyu `.lmod` modules.
//!
//! Public modules include `error`, `load`, `modpack`, `platform`, `rederive`,
//! and target-specific relocation helpers. The optional `crypto` module is
//! compiled when the `encryption` feature is enabled.

extern crate alloc;

pub mod error;
pub mod load;
pub mod modpack;
pub mod platform;
pub mod rederive;
pub mod reloc_arm;
pub mod reloc_riscv;
pub mod reloc_x86_64;
pub mod symbols;
pub mod apertures;

#[cfg(feature = "device-loader")]
pub mod boot;

#[cfg(feature = "encryption")]
pub mod crypto;

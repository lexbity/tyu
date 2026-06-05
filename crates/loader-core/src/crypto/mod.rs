//! Cryptographic primitives for authenticated decryption and key unwrapping.
//!
//! All functions are `#![no_std]`-compatible and operate on caller-provided
//! buffers (no heap allocation).  Gated behind `feature = "encryption"`.

pub mod chacha20poly1305;

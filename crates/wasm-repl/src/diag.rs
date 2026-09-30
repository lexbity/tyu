//! The diagnostic envelope (PLAN-WASM-1 §7.5): `{code, span, message}`.
//!
//! Reuses the runtime-diagnostic-protocol-v1 record shape. Codes: compile
//! diagnostics = native codes verbatim (parse/typecheck), traps =
//! `trap_code_u32` values, REPL-layer = the E9150+ registry (FR-11).

use alloc::vec::Vec;
use ir::Span;

/// One diagnostic record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diag {
    pub code: u32,
    pub span: Span,
    pub message: Vec<u8>,
}

impl Diag {
    pub const fn new(code: u32, span: Span, message: Vec<u8>) -> Diag {
        Diag {
            code,
            span,
            message,
        }
    }

    pub fn from_bytes(code: u32, span: Span, message: &[u8]) -> Diag {
        Diag {
            code,
            span,
            message: message.to_vec(),
        }
    }
}

/// REPL-layer diagnostic codes (E9150–E9159, PLAN-WASM-1 §7.5). The table in
/// the plan document is the registry; this module mirrors it as consts.
pub mod code {
    /// Fuel exhausted (E9150).
    pub const E9150_FUEL: u32 = 9150;
    /// Unlimited fuel rejected on wasm (E9151).
    pub const E9151_UNLIMITED_FUEL: u32 = 9151;
    /// Peripheral aperture unbound — no device registered (E9152).
    pub const E9152_PERIPH_UNBOUND: u32 = 9152;
    /// Sysroot blob invalid / version mismatch (E9153).
    pub const E9153_SYSROOT: u32 = 9153;
    /// Session source exceeds the 64 KiB cap (E9154).
    pub const E9154_SOURCE_CAP: u32 = 9154;
    /// Phrase required but absent (E9155).
    pub const E9155_PHRASE: u32 = 9155;
    /// Task/interrupt construct unsupported in the REPL (E9156).
    pub const E9156_TASKS: u32 = 9156;
    /// Session word-cap reached (E9157).
    pub const E9157_WORD_CAP: u32 = 9157;
    /// Internal engine invariant violated — always a bug (E9158).
    pub const E9158_INTERNAL: u32 = 9158;
    /// Reserved (E9159).
    pub const E9159_RESERVED: u32 = 9159;
}

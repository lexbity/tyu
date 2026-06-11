//! Per-architecture GDB register number tables.
//!
//! These numbers follow the GDB remote protocol register numbering as
//! implemented by QEMU's built-in gdbstub.  The data-stack pointer
//! register for each architecture is marked with `DS_PTR`.
//!
//! The tables are not exhaustive — they list only the registers needed
//! by the escalation debugger (A-side) and the hang classifier.

// ---------------------------------------------------------------------------
// x86_64 (qemu-system-x86_64)
// See GDB's `i386-64bit.xml` / QEMU's `gdbstub-x86_64.c`
// ---------------------------------------------------------------------------
pub mod x86_64 {
    /// Data-stack pointer (grows upward).
    pub const R15: u8 = 15;
    pub const R14: u8 = 14;
    pub const RIP: u8 = 16;
    pub const RSP: u8 = 7;
    pub const RAX: u8 = 0;
    pub const RBX: u8 = 1;
    pub const RCX: u8 = 2;
    pub const RDX: u8 = 3;
    pub const RSI: u8 = 4;
    pub const RDI: u8 = 5;
    pub const RBP: u8 = 6;

    /// The data-stack pointer — used for `ds_depth` computation.
    pub const DS_PTR: u8 = R15;
    /// All registers we need for a snapshot.
    pub const SNAPSHOT_REGS: &[u8] = &[RAX, RBX, RCX, RDX, RSI, RDI, RBP, RSP, R14, R15, RIP];
}

// ---------------------------------------------------------------------------
// ARM Cortex-M (armv7m, qemu-system-arm -machine lm3s6965evb)
// GDB register numbering follows the ARM architectural order.
// ---------------------------------------------------------------------------
pub mod arm {
    /// Data-stack pointer.
    pub const R4: u8 = 4;
    pub const R5: u8 = 5;
    pub const PC: u8 = 15;
    pub const LR: u8 = 14;

    /// __lang_trap_loc register contract (ARM AAPCS):
    ///   r0 = trap_code, r1 = valid, r2 = line, r3 = word_hash lo, r12 = word_hash hi
    pub const TRAP_CODE: u8 = 0;
    pub const VALID: u8 = 1;
    pub const LINE: u8 = 2;
    pub const WORD_HASH_LO: u8 = 3;
    pub const WORD_HASH_HI: u8 = 12;

    /// The data-stack pointer.
    pub const DS_PTR: u8 = R4;
    pub const SNAPSHOT_REGS: &[u8] = &[0, 1, 2, 3, R4, R5, 6, 7, 8, 9, 10, 11, 12, 13, LR, PC];
}

pub mod riscv {
    /// Data-stack pointer (s2 = x18).
    pub const S2: u8 = 18;
    /// Data-stack limit (s3 = x19).
    pub const S3: u8 = 19;
    pub const PC: u8 = 32;

    /// __lang_trap_loc register contract (RISC-V):
    ///   a0 = trap_code, a1 = valid, a2 = line, a3 = word_hash lo, a4 = word_hash hi
    pub const TRAP_CODE: u8 = 10;
    pub const VALID: u8 = 11;
    pub const LINE: u8 = 12;
    pub const WORD_HASH_LO: u8 = 13;
    pub const WORD_HASH_HI: u8 = 14;

    pub const DS_PTR: u8 = S2;
    pub const SNAPSHOT_REGS: &[u8] = &[10, 11, 12, 13, 14, 15, S2, S3, PC];
}

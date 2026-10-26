//! MMIO semantics (PLAN-WASM-1 P4): the aperture table, `WriteKind` RMW bit
//! math, and field mask/shift extraction — the ops the engine executes
//! concretely and the harness pins against the mmio fixtures.
//!
//! `WriteKind` (`W1c`/`W1s`/`Xor`) applies to the *store* the engine performs
//! against the device: the engine reads the register, computes the new value
//! (section P4 of the plan), and writes it. The bit math is unit-tested
//! exhaustively over 8/16/32-bit widths (`tests/mmio.rs`).

use ir::WriteKind;

/// The deterministic interpreter base for an emulated (link-time-symbol)
/// aperture — the numeric address space is internal to the interpreter (the
/// plan resolves MMIO via the session's aperture table, not the linker), so
/// emulated bases live far above the region heap (which tops out at
/// `crate::heap::MAX_REGION_BYTES` from 4 KiB).
pub const EMULATED_APERTURE_BASE: u64 = 0x4000_0000;
/// The per-aperture stride for consecutive emulated apertures (each ≤ 64 KiB
/// of registers, so a 1 MiB stride cannot collide).
pub const APERTURE_STRIDE: u64 = 0x10_0000;

/// The register-cell width mask for a byte width.
pub fn width_mask(width: u8) -> u64 {
    match width {
        1 => 0xff,
        2 => 0xffff,
        4 => 0xffff_ffff,
        8 => u64::MAX,
        _ => 0,
    }
}

/// The aperture table entry the engine resolves addresses against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApertureEntry {
    pub id: u16,
    pub base: u64,
    pub size: u32,
}

/// The deterministic base for a descriptor aperture: its absolute `base`
/// when bus-bound, or the emulated constant indexed by aperture id.
pub fn aperture_base(id: u16, base: Option<u64>) -> u64 {
    match base {
        Some(b) => b,
        None => EMULATED_APERTURE_BASE.wrapping_add(u64::from(id) * APERTURE_STRIDE),
    }
}

/// The `WriteKind` read-modify-write result at the register width (P4): the
/// width-masked `val` applied to the current register value `cur`.
pub fn apply_write_kind(cur: u64, val: u64, width: u8, kind: WriteKind) -> u64 {
    let m = width_mask(width);
    let v = val & m;
    let c = cur & m;
    match kind {
        WriteKind::Plain => v,
        // Write-1-to-clear: a 1 clears its bit; a 0 leaves it.
        WriteKind::W1c => c & !v,
        // Write-1-to-set: a 1 sets its bit; a 0 leaves it.
        WriteKind::W1s => c | v,
        // Write-1-to-invert: a 1 toggles its bit; a 0 leaves it.
        WriteKind::Xor => c ^ v,
    }
}

/// Extract a bit field from a register value.
pub fn extract_field(reg: u64, mask: u64, shift: u8) -> u64 {
    (reg & mask) >> shift
}

/// Insert a field value into a register value (the field RMW result).
pub fn insert_field(reg: u64, mask: u64, shift: u8, field: u64) -> u64 {
    (reg & !mask) | ((field.wrapping_shl(u32::from(shift))) & mask)
}

//! Platform descriptor v2 — the machine-checked artifact for all per-platform
//! memory and MMIO behavior.
//!
//! See `devdocs/plans/platform-layer.md`
//! Part 1 §5.2 (descriptor v2 schema) and §5.3 (canonical serialization).
//!
//! The descriptor is a *data* model, never code: the trusted lowering kernel
//! stays inside the toolchain (decision D-2). Descriptors are parsed
//! (`parse`), validated as a whole model (`validate`), canonically
//! serialized and hashed (`canonical`), and rendered for review (`report`).

pub mod canonical;
pub mod compile;
pub mod parse;
pub mod report;
pub mod validate;

pub use compile::{descriptor_file_path, ensure_compiled_descriptor, COMPILED_DESC_FILE};

use crate::error::TyuError;
use std::path::Path;

/// Descriptor schema version. Folds into `platform_hash` (canonical §5.3).
///
/// Bump only when the *shape* of the descriptor model changes such that a
/// descriptor written for an older schema must not load on a newer toolchain.
pub const DESCRIPTOR_SCHEMA: u32 = 2;

/// MMIO semantics version — the compiler-side strategy/access-semantics model
/// (decision D-2/D-3). Folds into `platform_hash` so that a *semantics*
/// evolution can never masquerade as a board match.
pub const MMIO_SEM_VER: u32 = 1;

/// Unknown write/read kind or strategy name referenced by a descriptor
/// (decision D-11 / E3646).
pub const E_DESC_UNKNOWN_KIND: u16 = 3646;

/// Descriptor schema/validation failure with a specific reason (D-11 / E3647).
pub const E_DESC_INVALID: u16 = 3647;

/// A single descriptor error: a diagnostic code (E3646/E3647) and a
/// human-actionable reason. The validator returns one per violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorError {
    pub code: u16,
    pub detail: String,
}

impl DescriptorError {
    pub fn new(code: u16, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// The validated, default-resolved platform descriptor model.
///
/// All optional fields carry their *effective* values (defaults resolved at
/// parse time) so that "explicit default" and "omitted default" hash
/// identically (canonical §5.3 invariance rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub schema: u32,
    pub name: String,
    pub family: String,
    pub description: Option<String>,
    pub apertures: Vec<MmioAperture>,
    pub devices: Vec<DeviceMap>,
    pub allocator: Option<AllocatorSpec>,
    pub scoped: Option<ScopedSpec>,
    pub metal_trust: Vec<String>,
    pub memory: MemoryModel,
}

impl Descriptor {
    /// Look up a aperture by module-local id.
    pub fn aperture(&self, id: u16) -> Option<&MmioAperture> {
        self.apertures.iter().find(|w| w.id == id)
    }

    /// Look up a `[memory]` region by name.
    pub fn region(&self, name: &str) -> Option<&MemoryRegionSpec> {
        self.memory.regions.iter().find(|r| r.name == name)
    }
}

/// A memory-mapped aperture a board exposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MmioAperture {
    pub id: u16,
    pub name: String,
    pub kind: ApertureKind,
    /// Absolute base address. `None` means the base is a link-time symbol
    /// (the hosted/emulated aperture, decision D-7) rather than a bus address.
    pub base: Option<u64>,
    pub size: u32,
    /// Binding-time relocation ISA for this aperture's base (P6). `None` for
    /// emulated apertures (runtime-dynamic addressing).
    pub reloc_isa: Option<codegen_core::RelocIsa>,
}

/// How a aperture is backed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApertureKind {
    /// Real device address space.
    Bus,
    /// RAM the runtime owns (the hosted `__mmio_mem` fiction, D-7).
    Emulated,
}

impl ApertureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ApertureKind::Bus => "bus",
            ApertureKind::Emulated => "emulated",
        }
    }
}

/// A board-exposed register map instance, referenced from source as
/// `board.<instance>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceMap {
    pub map: String,
    pub instance: String,
    pub aperture: u16,
    pub base_offset: u32,
    pub registers: Vec<RegisterRow>,
}

impl DeviceMap {
    /// Furthest byte extent of the map within its aperture
    /// (`max(offset + width/8)`, 0 when the map declares no registers).
    pub fn extent(&self) -> u32 {
        self.registers
            .iter()
            .map(|r| r.offset.saturating_add((r.width as u32) / 8))
            .max()
            .unwrap_or(0)
    }
}

/// A single register row with its declared access semantics (decision D-3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterRow {
    pub offset: u32,
    pub name: String,
    pub width: u8,
    pub access: AccessKind,
    pub write_kind: WriteKind,
    pub read_kind: ReadKind,
    /// Widest single undeclared access width. Defaults to `width`.
    pub atomic_max: u8,
    /// Implementable bits. Defaults to the full-width mask.
    pub mask: u64,
    /// Documentation + future contract use. Defaults to 0.
    pub reset: u64,
    pub barrier: BarrierKind,
    /// The NVIC IRQ number an `@interrupt(VECTOR)` binding to this register's
    /// device resolves to (P8, datasheet-derived). `None` when the register
    /// has no interrupt vector.
    pub interrupt: Option<u16>,
    /// The interrupt request number associated with this register (the
    /// datasheet IRQ a read/clear of this register acknowledges). `None` when
    /// the register does not participate in an IRQ.
    pub irq: Option<u16>,
}

impl RegisterRow {
    /// Access width in bytes.
    pub fn width_bytes(&self) -> u32 {
        (self.width as u32) / 8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    Ro,
    Wo,
    Rw,
}

impl AccessKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessKind::Ro => "ro",
            AccessKind::Wo => "wo",
            AccessKind::Rw => "rw",
        }
    }
}

/// What a *store* does to the register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Plain,
    W1s,
    W1c,
}

impl WriteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteKind::Plain => "plain",
            WriteKind::W1s => "w1s",
            WriteKind::W1c => "w1c",
        }
    }
}

/// Whether a *load* has side effects (decision D-3 rule R1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    Plain,
    Effectful,
}

impl ReadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadKind::Plain => "plain",
            ReadKind::Effectful => "effectful",
        }
    }
}

/// Ordering requirement of the access vs code around it (decision D-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarrierKind {
    None,
    Before,
    After,
    Both,
}

impl BarrierKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BarrierKind::None => "none",
            BarrierKind::Before => "before",
            BarrierKind::After => "after",
            BarrierKind::Both => "both",
        }
    }
}

/// The platform-implemented allocation region (`platform.region.*`, D-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocatorSpec {
    pub region: String,
    pub offset: u64,
    pub length: u64,
    pub impl_path: String,
    pub policy: String,
}

/// Scoped-metadata diagnostic bound (per-word body count must be ≤).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopedSpec {
    pub metadata_slots_max: u32,
}

/// The `[memory]` regions a descriptor's allocator may name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryModel {
    pub regions: Vec<MemoryRegionSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRegionSpec {
    pub name: String,
    pub origin: u64,
    pub length: u64,
    /// Data-stack size when this region is the `ds_region` (None otherwise).
    pub ds_size: Option<u64>,
}

/// The full-width mask for a register width in bits.
pub fn full_mask(width: u8) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// Load and parse the descriptor of a named platform pack.
///
/// Returns `Ok(None)` when the pack's manifest carries no descriptor v2
/// content (a legacy pack). Load errors and parse errors surface as
/// `TyuError` so the CLI can print either cleanly.
pub fn load_descriptor(root: &Path, name: &str) -> Result<Option<Descriptor>, TyuError> {
    let manifest_path = super::config::find_pack_manifest_path(root, name)
        .ok_or_else(|| TyuError::Platform(format!("platform pack '{}' not found", name)))?;
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        TyuError::Platform(format!("reading '{}': {}", manifest_path.display(), e))
    })?;
    parse::parse_descriptor(&text).map_err(|e| {
        TyuError::Platform(format!("E{} descriptor for '{}': {}", e.code, name, e.detail))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mask_covers_every_declared_width() {
        assert_eq!(full_mask(8), 0xff);
        assert_eq!(full_mask(16), 0xffff);
        assert_eq!(full_mask(32), 0xffff_ffff);
        assert_eq!(full_mask(64), u64::MAX);
    }

    #[test]
    fn descriptor_error_roundtrips_code_and_detail() {
        let e = DescriptorError::new(E_DESC_INVALID, "bad");
        assert_eq!(e.code, E_DESC_INVALID);
        assert_eq!(e.detail, "bad");
    }

    #[test]
    fn constants_match_design_doc() {
        assert_eq!(DESCRIPTOR_SCHEMA, 2);
        assert_eq!(MMIO_SEM_VER, 1);
        assert_eq!(E_DESC_UNKNOWN_KIND, 3646);
        assert_eq!(E_DESC_INVALID, 3647);
    }
}
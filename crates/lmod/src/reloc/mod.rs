//! Import relocation table codec.
//!
//! Canonical definition: module-format-and-loading.md §3.1.
//!
//! After pre-resolving internal relocations, the remaining fixups (imports)
//! are serialised into the `.lmod` import reloc table.  Each entry describes
//! one patch site that the loader must fix up at load time.
//!
//! P6 binding: a bus window's base is *not* a baked absolute constant. The
//! code carries a relocatable site (a 32-bit little-endian word in `.text`)
//! that the pack binds to the window base and the on-device loader re-derives
//! from its descriptor. The per-ISA site patterns live in [`arm`] and
//! [`riscv`]; the apply/read mechanics are shared here.

pub mod arm;
pub mod riscv;

/// Size of one on-wire relocation entry in bytes.
///
/// Layout: `{ u32 site_off, u64 sym_hash, u8 kind, u8[3] _pad }` = 16 bytes.
pub const RELOC_ENTRY_SIZE: u32 = 16;

/// Size of a window-base reloc site in bytes (P6): one 32-bit LE word.
pub const WINDOW_BASE_SITE_SIZE: usize = 4;

/// Per-target relocation subset kinds (module-format §3.1).
///
/// These are the values carried in `RelocEntry.kind`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelocKind {
    // x86_64
    #[allow(non_camel_case_types)]
    X86_64_64 = 1,
    #[allow(non_camel_case_types)]
    X86_64_PC32 = 2,
    #[allow(non_camel_case_types)]
    X86_64_PLT32 = 3,
    // ARM Thumb
    ArmAbs32 = 4,
    ArmThmCall = 5,
    ArmThmJump24 = 6,
    ArmRel32 = 7,
    // RISC-V
    RiscV32 = 8,
    RiscVCall = 9,
    /// Window-base fixup (P6): a code site holding `window_base + offset`
    /// that the loader patches with the bound window's base. Carries the
    /// window id in the entry's symbol-hash field.
    MmioWindowBase = 10,
}

impl RelocKind {
    /// Convert a raw u8 from the on-wire encoding to a `RelocKind`.
    /// Returns `None` for reserved / unsupported values.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::X86_64_64),
            2 => Some(Self::X86_64_PC32),
            3 => Some(Self::X86_64_PLT32),
            4 => Some(Self::ArmAbs32),
            5 => Some(Self::ArmThmCall),
            6 => Some(Self::ArmThmJump24),
            7 => Some(Self::ArmRel32),
            8 => Some(Self::RiscV32),
            9 => Some(Self::RiscVCall),
            10 => Some(Self::MmioWindowBase),
            _ => None,
        }
    }

    /// The window-base relocation kind for a binding-time relocation ISA
    /// (P6). Every reloc-capable ISA binds the base via `MmioWindowBase`.
    pub fn kind_for_isa(isa: ir::RelocIsa) -> Self {
        match isa {
            ir::RelocIsa::ArmThumbLdrLiteral | ir::RelocIsa::RiscVHi20Lo12 => Self::MmioWindowBase,
        }
    }

    /// The linker symbol a module's window-base site references before the
    /// pack binds it (P6): `__lang_window_{id}_base`. The firmware build
    /// defines these symbols from the descriptor; the loader re-derives the
    /// same bases from them.
    pub fn window_base_symbol(id: u16) -> alloc::string::String {
        alloc::format!("__lang_window_{}_base", id)
    }

    /// Write the window base into a reloc site (P6). The site is one 32-bit
    /// little-endian word; the raw value is written unchanged (the ARM lone
    /// literal and RISC-V literal-load patterns both carry the *address* in
    /// the pool word).
    pub fn apply_base(site: &mut [u8], site_off: usize, base: u32) -> Option<()> {
        let end = site_off.checked_add(WINDOW_BASE_SITE_SIZE)?;
        if end > site.len() {
            return None;
        }
        site[site_off..end].copy_from_slice(&base.to_le_bytes());
        Some(())
    }

    /// Read the window base back from a reloc site (P6). Used by tests and by
    /// the loader's `check_window_base` (the bound-window validation).
    pub fn read_site_base(site: &[u8], site_off: usize) -> Option<u32> {
        let end = site_off.checked_add(WINDOW_BASE_SITE_SIZE)?;
        if end > site.len() {
            return None;
        }
        let mut le = [0u8; 4];
        le.copy_from_slice(&site[site_off..end]);
        Some(u32::from_le_bytes(le))
    }
}

/// A single import relocation entry ready for serialization.
#[derive(Clone, Copy, Debug)]
pub struct RelocEntry {
    /// Byte offset within the `.text` segment where the fixup should be
    /// applied.  Relative to `code_off` in the container.
    pub site_off: u32,
    /// FNV-1a 64-bit hash of the imported symbol name.
    pub sym_hash: u64,
    /// Relocation kind (one of `RelocKind`).
    pub kind: u8,
}

/// Serialise a slice of `RelocEntry` into `buf`.
///
/// Returns `None` if `buf` is too small.
pub fn encode_relocs(buf: &mut [u8], entries: &[RelocEntry]) -> Option<usize> {
    let n_bytes = entries.len() * RELOC_ENTRY_SIZE as usize;
    if n_bytes > buf.len() {
        return None;
    }
    for (i, e) in entries.iter().enumerate() {
        let off = i * RELOC_ENTRY_SIZE as usize;
        buf[off..off + 4].copy_from_slice(&e.site_off.to_le_bytes());
        buf[off + 4..off + 12].copy_from_slice(&e.sym_hash.to_le_bytes());
        buf[off + 12] = e.kind;
        // bytes [13..16] are padding (zero-initialised)
    }
    Some(n_bytes)
}

/// Parse a single `RelocEntry` from raw bytes at offset `off`.
///
/// Returns `None` if the bytes are too short.
pub fn decode_entry(data: &[u8], off: usize) -> Option<RelocEntry> {
    if off + RELOC_ENTRY_SIZE as usize > data.len() {
        return None;
    }
    Some(RelocEntry {
        site_off: u32::from_le_bytes(data[off..off + 4].try_into().ok()?),
        sym_hash: u64::from_le_bytes(data[off + 4..off + 12].try_into().ok()?),
        kind: data[off + 12],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let entries = [
            RelocEntry {
                site_off: 0,
                sym_hash: 0x1234,
                kind: RelocKind::X86_64_PC32 as u8,
            },
            RelocEntry {
                site_off: 42,
                sym_hash: 0xdeadbeef,
                kind: RelocKind::X86_64_64 as u8,
            },
        ];
        let mut buf = [0u8; 64];
        let n = encode_relocs(&mut buf, &entries).unwrap();
        assert_eq!(n as u32, RELOC_ENTRY_SIZE * 2);

        let e0 = decode_entry(&buf, 0).unwrap();
        assert_eq!(e0.site_off, 0);
        assert_eq!(e0.sym_hash, 0x1234);
        assert_eq!(e0.kind, RelocKind::X86_64_PC32 as u8);

        let e1 = decode_entry(&buf, RELOC_ENTRY_SIZE as usize).unwrap();
        assert_eq!(e1.site_off, 42);
        assert_eq!(e1.sym_hash, 0xdeadbeef);
        assert_eq!(e1.kind, RelocKind::X86_64_64 as u8);
    }

    #[test]
    fn encode_small_buffer_returns_none() {
        let entries = [RelocEntry {
            site_off: 0,
            sym_hash: 0,
            kind: 0,
        }];
        let mut buf = [0u8; 4];
        assert!(encode_relocs(&mut buf, &entries).is_none());
    }

    #[test]
    fn decode_out_of_range_returns_none() {
        let buf = [0u8; 8];
        assert!(decode_entry(&buf, 0).is_none());
    }

    #[test]
    fn reloc_kind_from_u8() {
        assert_eq!(RelocKind::from_u8(1), Some(RelocKind::X86_64_64));
        assert_eq!(RelocKind::from_u8(7), Some(RelocKind::ArmRel32));
        assert_eq!(RelocKind::from_u8(0), None);
        assert_eq!(RelocKind::from_u8(99), None);
    }
}

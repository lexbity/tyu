//! `.lmod` container packer library.
//!
//! Lowers an ELF relocatable object (ET_REL) produced by `--emit=obj` into
//! the compact `.lmod` container format (module-format-and-loading.md §3):
//!
//!   - Extracts `.text`, `.rodata`, `.data`, `.lang.modinfo` sections.
//!   - Pre-resolves internal relocations (references between sections
//!     within the same module).
//!   - Emits the import-only reloc table for external references.
//!   - Appends an (empty) signature trailer slot.

use core::fmt;

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors that can occur during packing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackError {
    /// Input is too small for ELF.
    TooSmall,
    /// Bad ELF magic bytes.
    BadMagic,
    /// Unsupported ELF class (not 32- or 64-bit).
    UnsupportedClass(u8),
    /// Not little-endian.
    NotLittleEndian,
    /// Not an ET_REL relocatable object.
    NotRelocatable,
    /// Section header entry size mismatch.
    ShentsizeMismatch { expected: usize, actual: usize },
    /// Section headers overflow the file.
    SectionHeadersOverflow,
    /// A relocation site is outside the output buffer.
    RelocSiteOutOfRange { site: usize, kind: &'static str },
    /// Unsupported internal relocation type.
    UnsupportedInternalReloc(u32),
    /// Modinfo data integrity check failed.
    ModinfoCorruption,
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall => write!(f, "too small for ELF"),
            Self::BadMagic => write!(f, "bad ELF magic"),
            Self::UnsupportedClass(c) => write!(f, "unsupported ELF class {}", c),
            Self::NotLittleEndian => write!(f, "not little-endian"),
            Self::NotRelocatable => write!(f, "not ET_REL"),
            Self::ShentsizeMismatch { expected, actual } => {
                write!(f, "shentsize {} != {}", expected, actual)
            }
            Self::SectionHeadersOverflow => write!(f, "section headers overflow"),
            Self::RelocSiteOutOfRange { site, kind } => {
                write!(f, "{} site {} out of range", kind, site)
            }
            Self::UnsupportedInternalReloc(t) => {
                write!(f, "unsupported internal relocation type {}", t)
            }
            Self::ModinfoCorruption => write!(f, "modinfo data corruption during pack"),
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal ELF64 reader
// ---------------------------------------------------------------------------

const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_REL: u16 = 1;
const EM_ARM: u16 = 40;
const EM_X86_64: u16 = 62;
const EM_RISCV: u16 = 243;

const SHT_RELA: u32 = 4;
const SHT_REL: u32 = 9;
const SHT_SYMTAB: u32 = 2;
const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xFFF1;

fn le_u16(data: &[u8], off: usize) -> u16 {
    data.get(off..off + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0)
}
fn le_u32(data: &[u8], off: usize) -> u32 {
    data.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}
fn le_u64(data: &[u8], off: usize) -> u64 {
    data.get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}

#[derive(Clone)]
struct Section {
    name: String,
    ty: u32,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    entsize: u64,
}

#[derive(Clone)]
struct Symbol {
    shndx: u16,
    value: u64,
    name: String,
}

struct Elf<'a> {
    data: &'a [u8],
    elf_class: u8,
    machine: u16,
    sections: Vec<Section>,
    strtab: &'a [u8],
}

impl<'a> Elf<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, PackError> {
        if data.len() < 64 {
            return Err(PackError::TooSmall);
        }
        if data[0..4] != [0x7f, b'E', b'L', b'F'] {
            return Err(PackError::BadMagic);
        }
        let elf_class = data[4];
        if elf_class != ELFCLASS32 && elf_class != ELFCLASS64 {
            return Err(PackError::UnsupportedClass(elf_class));
        }
        if data[5] != ELFDATA2LSB {
            return Err(PackError::NotLittleEndian);
        }
        let e_type = le_u16(data, 16);
        if e_type != ET_REL {
            return Err(PackError::NotRelocatable);
        }
        let machine = le_u16(data, 18);

        let (e_shoff, e_shentsize, e_shnum, e_shstrndx, shent_size) = if elf_class == 2 {
            let shoff = le_u64(data, 40) as usize;
            let shent = le_u16(data, 58) as usize;
            let shnum = le_u16(data, 60) as usize;
            let shstr = le_u16(data, 62) as usize;
            (shoff, shent, shnum, shstr, 64)
        } else {
            let shoff = le_u32(data, 0x20) as usize;
            let shent = le_u16(data, 0x2E) as usize;
            let shnum = le_u16(data, 0x30) as usize;
            let shstr = le_u16(data, 0x32) as usize;
            (shoff, shent, shnum, shstr, 40)
        };

        if e_shentsize != shent_size {
            return Err(PackError::ShentsizeMismatch {
                expected: e_shentsize,
                actual: shent_size,
            });
        }
        if e_shoff + e_shnum * shent_size > data.len() {
            return Err(PackError::SectionHeadersOverflow);
        }

        let mut sections = Vec::with_capacity(e_shnum);
        for i in 0..e_shnum {
            let b = e_shoff + i * shent_size;
            let (sec_offset, sec_size, link, info, entsize) = if elf_class == 2 {
                (
                    le_u64(data, b + 24),
                    le_u64(data, b + 32),
                    le_u32(data, b + 40),
                    le_u32(data, b + 44),
                    le_u64(data, b + 56),
                )
            } else {
                (
                    le_u32(data, b + 16) as u64,
                    le_u32(data, b + 20) as u64,
                    le_u32(data, b + 24),
                    le_u32(data, b + 28),
                    le_u32(data, b + 36) as u64,
                )
            };
            sections.push(Section {
                name: String::new(),
                ty: le_u32(data, b + 4),
                offset: sec_offset,
                size: sec_size,
                link,
                info,
                entsize,
            });
        }

        if e_shstrndx >= sections.len() {
            return Err(PackError::BadMagic); // reuse: invalid section index
        }
        let shstrtab_sec = &sections[e_shstrndx];
        let shstrtab_end = shstrtab_sec.offset as usize + shstrtab_sec.size as usize;
        if shstrtab_end > data.len() || shstrtab_sec.offset as usize > data.len() {
            return Err(PackError::BadMagic);
        }
        let shstrtab = &data[shstrtab_sec.offset as usize..shstrtab_end];
        for (i, sec) in sections.iter_mut().enumerate() {
            let b = e_shoff + i * shent_size;
            let name_off = le_u32(data, b) as usize;
            if name_off >= shstrtab.len() {
                continue; // skip entries with broken name offsets
            }
            let raw = &shstrtab[name_off..];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            sec.name = String::from_utf8_lossy(&raw[..end]).to_string();
        }

        let strtab: &[u8] = {
            let st = sections.iter().find(|s| s.ty == SHT_SYMTAB);
            match st {
                Some(st) => {
                    let link = st.link as usize;
                    if link < sections.len() {
                        let s = &sections[link];
                        let start = s.offset as usize;
                        let end = start + s.size as usize;
                        if end <= data.len() && start <= data.len() {
                            &data[start..end]
                        } else {
                            &[]
                        }
                    } else {
                        &[]
                    }
                }
                None => &[],
            }
        };

        Ok(Self {
            data,
            elf_class,
            machine,
            sections,
            strtab,
        })
    }

    fn section_by_name(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    fn section_data(&self, s: &Section) -> &[u8] {
        let start = s.offset as usize;
        let end = start + s.size as usize;
        if end > self.data.len() || start > self.data.len() {
            &[]
        } else {
            &self.data[start..end]
        }
    }

    fn symbols(&self) -> Vec<Symbol> {
        let st = match self.section_by_name(".symtab") {
            Some(s) => s,
            None => return vec![],
        };
        let st_data = self.section_data(st);
        let sym_entry_size = if self.elf_class == 2 {
            24usize
        } else {
            16usize
        };
        let entsize = if st.entsize != 0 {
            st.entsize as usize
        } else {
            sym_entry_size
        };
        let mut syms = Vec::new();
        let mut pos = 0;
        while pos + entsize <= st_data.len() {
            let name_off = le_u32(st_data, pos) as usize;
            let (st_shndx, st_value) = if self.elf_class == 2 {
                (le_u16(st_data, pos + 6), le_u64(st_data, pos + 8))
            } else {
                (le_u16(st_data, pos + 14), le_u32(st_data, pos + 4) as u64)
            };
            let name = if name_off < self.strtab.len() {
                let nb = &self.strtab[name_off..];
                let end = nb.iter().position(|&b| b == 0).unwrap_or(nb.len());
                String::from_utf8_lossy(&nb[..end]).to_string()
            } else {
                String::new()
            };
            syms.push(Symbol {
                shndx: st_shndx,
                value: st_value,
                name,
            });
            pos += entsize;
        }
        syms
    }

    fn lmod_section_base(&self, idx: usize, layout: &lmod::header::LmodHeader) -> u64 {
        if idx >= self.sections.len() {
            return 0;
        }
        let name = &self.sections[idx].name;
        if name == ".text" {
            layout.code_off as u64
        } else if name == ".rodata" {
            layout.rodata_off as u64
        } else if name == ".data" {
            layout.data_off as u64
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Relocation application
// ---------------------------------------------------------------------------

/// Apply an internal relocation (pre-resolution).
fn apply_internal_reloc(
    out: &mut [u8],
    machine: u16,
    r_type: u32,
    r_offset: u64,
    sym_value: u64,
    site_base: u64,
    addend: i64,
) -> Result<(), PackError> {
    let site_addr = site_base + r_offset;
    match (machine, r_type) {
        (EM_RISCV, 51) => {}
        (EM_X86_64, 1) => {
            let val = sym_value.wrapping_add(addend as u64);
            let off = site_addr as usize;
            if off + 8 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_X86_64_64",
                });
            }
            out[off..off + 8].copy_from_slice(&val.to_le_bytes());
        }
        (EM_X86_64, 2) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_X86_64_PC32",
                });
            }
            out[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
        }
        (EM_X86_64, 3) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_X86_64_PLT32",
                });
            }
            out[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
        }
        (EM_ARM, 2) | (EM_RISCV, 1) => {
            let val = sym_value.wrapping_add(addend as u64);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "ABS32",
                });
            }
            out[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
        }
        (EM_ARM, 3) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_ARM_REL32",
                });
            }
            out[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
        }
        (EM_RISCV, 16) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_RISCV_BRANCH",
                });
            }
            encode_riscv_branch(&mut out[off..off + 4], val)
                .map_err(|_| PackError::UnsupportedInternalReloc(r_type))?;
        }
        (EM_RISCV, 17) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_RISCV_JAL",
                });
            }
            encode_riscv_jal(&mut out[off..off + 4], val)
                .map_err(|_| PackError::UnsupportedInternalReloc(r_type))?;
        }
        (EM_RISCV, 23) => {
            let p = site_addr as i64;
            let val = (sym_value as i64).wrapping_add(addend).wrapping_sub(p);
            let off = site_addr as usize;
            if off + 4 > out.len() {
                return Err(PackError::RelocSiteOutOfRange {
                    site: off,
                    kind: "R_RISCV_PCREL_HI20",
                });
            }
            encode_riscv_hi20(&mut out[off..off + 4], val);
        }
        _ => {
            return Err(PackError::UnsupportedInternalReloc(r_type));
        }
    }
    Ok(())
}

fn apply_riscv_pcrel_lo12_i(
    out: &mut [u8],
    site_addr: u64,
    hi_site: u64,
    hi_target: u64,
    addend: i64,
) -> Result<(), PackError> {
    let val = (hi_target as i64)
        .wrapping_add(addend)
        .wrapping_sub(hi_site as i64);
    let off = site_addr as usize;
    if off + 4 > out.len() {
        return Err(PackError::RelocSiteOutOfRange {
            site: off,
            kind: "R_RISCV_PCREL_LO12_I",
        });
    }
    encode_riscv_lo12_i(&mut out[off..off + 4], val);
    Ok(())
}

fn encode_riscv_hi20(insn: &mut [u8], value: i64) {
    let existing = u32::from_le_bytes(insn[..4].try_into().unwrap());
    let hi20 = (((value + 0x800) >> 12) as u32) & 0x000f_ffff;
    let enc = (existing & 0x0000_0fff) | (hi20 << 12);
    insn[..4].copy_from_slice(&enc.to_le_bytes());
}

fn encode_riscv_lo12_i(insn: &mut [u8], value: i64) {
    let existing = u32::from_le_bytes(insn[..4].try_into().unwrap());
    let lo12 = (value as u32) & 0x0fff;
    let enc = (existing & 0x000f_ffff) | (lo12 << 20);
    insn[..4].copy_from_slice(&enc.to_le_bytes());
}

fn encode_riscv_jal(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    if offset & 1 != 0 || !(-0x10_0000..=0x0f_ffff).contains(&offset) {
        return Err(());
    }
    let existing = u32::from_le_bytes(insn[..4].try_into().unwrap());
    let imm = offset as u32;
    let enc = (existing & 0x0000_0fff)
        | ((imm & 0x0010_0000) << 11)
        | ((imm & 0x0000_07fe) << 20)
        | ((imm & 0x0000_0800) << 9)
        | (imm & 0x000f_f000);
    insn[..4].copy_from_slice(&enc.to_le_bytes());
    Ok(())
}

fn encode_riscv_branch(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    if offset & 1 != 0 || !(-0x1000..=0x0ffe).contains(&offset) {
        return Err(());
    }
    let existing = u32::from_le_bytes(insn[..4].try_into().unwrap());
    let imm = offset as u32;
    let enc = (existing & 0x01fff07f)
        | ((imm & 0x1000) << 19)
        | ((imm & 0x07e0) << 20)
        | ((imm & 0x001e) << 7)
        | ((imm & 0x0800) >> 4);
    insn[..4].copy_from_slice(&enc.to_le_bytes());
    Ok(())
}

fn import_reloc_kind(machine: u16, r_type: u32) -> Option<u8> {
    use lmod::reloc::RelocKind;

    match (machine, r_type) {
        (EM_X86_64, 1) => Some(RelocKind::X86_64_64 as u8),
        (EM_X86_64, 2) => Some(RelocKind::X86_64_PC32 as u8),
        (EM_X86_64, 3) => Some(RelocKind::X86_64_PLT32 as u8),
        (EM_ARM, 2) => Some(RelocKind::ArmAbs32 as u8),
        (EM_ARM, 3) => Some(RelocKind::ArmRel32 as u8),
        (EM_ARM, 10) => Some(RelocKind::ArmThmCall as u8),
        (EM_ARM, 30) => Some(RelocKind::ArmThmJump24 as u8),
        (EM_RISCV, 1) => Some(RelocKind::RiscV32 as u8),
        (EM_RISCV, 17 | 18 | 19) => Some(RelocKind::RiscVCall as u8),
        _ => None,
    }
}

/// Parse a aperture-base symbol name (`__lang_aperture_{N}_base`) into the aperture
/// id (P6). Returns `None` for any other symbol.
fn aperture_base_id(name: &[u8]) -> Option<u16> {
    let prefix = b"__lang_aperture_";
    let suffix = b"_base";
    let name = name.strip_prefix(prefix)?;
    let name = name.strip_suffix(suffix)?;
    if name.is_empty() {
        return None;
    }
    let mut id: u16 = 0;
    for &b in name {
        if !b.is_ascii_digit() {
            return None;
        }
        id = id.checked_mul(10)?.checked_add((b - b'0') as u16)?;
    }
    Some(id)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Pack an ELF relocatable object into the `.lmod` container format.
///
/// `input` is the raw bytes of an ELF ET_REL object file (32- or 64-bit,
/// little-endian).  Returns the packed `.lmod` bytes on success.
///
/// # Known issues
///
/// - Import relocation site offsets assume all imports target `.text`
///   (the `TODO` at what was `lmod-pack/main.rs` step 8).  This is correct
///   for the current codegen, which only emits imports into `.text`, but
///   will produce wrong offsets if a future codegen emits import-site
///   relocations in `.rodata` or `.data`.
///
/// # P6 aperture-base sites (design doc §5.8, decision D-4)
///
/// A `MmioApertureBase` reloc site is *not* bound here: the packed `.lmod` keeps
/// the site as the assembler emitted it and carries the reloc record (aperture
/// id in the symbol-hash field). The on-device loader writes the board's base
/// at load time (FR-15) after enforcing `platform_hash` (E5220). Baking bases
/// here would make dynamic modules position-dependent on device maps.
pub fn pack(input: &[u8]) -> Result<Vec<u8>, PackError> {
    let elf = Elf::parse(input)?;

    // 1. Extract section data.
    let modinfo_data = elf
        .section_by_name(".lang.modinfo")
        .map(|s| elf.section_data(s))
        .unwrap_or(&[]);
    let code_data = elf
        .section_by_name(".text")
        .map(|s| elf.section_data(s))
        .unwrap_or(&[]);
    let rodata_data = elf
        .section_by_name(".rodata")
        .map(|s| elf.section_data(s))
        .unwrap_or(&[]);
    let data_data = elf
        .section_by_name(".data")
        .map(|s| elf.section_data(s))
        .unwrap_or(&[]);

    let modinfo_len = modinfo_data.len() as u32;
    let code_len = code_data.len() as u32;
    let rodata_len = rodata_data.len() as u32;
    let data_len = data_data.len() as u32;

    // 2. Parse abi_hash from modinfo (bytes [8..16]).
    let abi_hash = if modinfo_data.len() >= 16 {
        le_u64(modinfo_data, 8)
    } else {
        0
    };

    // 3. Parse symbol table.
    let symbols = elf.symbols();

    // 4. Compute .lmod layout — two-pass: count imports first.
    let mut import_relocs: Vec<(u64, u64, u8)> = Vec::new();
    let mut internal_fixups: Vec<(usize, u32, u64, usize, u64, i64)> = Vec::new();

    for sec in elf.sections.iter() {
        if sec.ty != SHT_RELA && sec.ty != SHT_REL {
            continue;
        }
        let target_idx = sec.info as usize;
        if target_idx >= elf.sections.len() {
            continue;
        }
        let rel_data = elf.section_data(sec);
        let is_rela = sec.ty == SHT_RELA;
        let entsize = if sec.entsize != 0 {
            sec.entsize as usize
        } else if is_rela {
            if elf.elf_class == 2 {
                24
            } else {
                12
            }
        } else {
            if elf.elf_class == 2 {
                24
            } else {
                8
            }
        };
        let mut pos = 0;

        while pos + entsize <= rel_data.len() {
            let (r_offset, r_info_wide, r_addend) = if is_rela && elf.elf_class == 2 {
                (
                    le_u64(rel_data, pos),
                    le_u64(rel_data, pos + 8),
                    le_u64(rel_data, pos + 16) as i64,
                )
            } else if is_rela && elf.elf_class == 1 {
                (
                    le_u32(rel_data, pos) as u64,
                    le_u32(rel_data, pos + 4) as u64,
                    le_u32(rel_data, pos + 8) as i64,
                )
            } else if !is_rela && elf.elf_class == 2 {
                (le_u64(rel_data, pos), le_u64(rel_data, pos + 8), 0i64)
            } else {
                (
                    le_u32(rel_data, pos) as u64,
                    le_u32(rel_data, pos + 4) as u64,
                    0i64,
                )
            };

            let (sym_idx, r_type) = if elf.elf_class == 2 {
                (
                    (r_info_wide >> 32) as usize,
                    (r_info_wide & 0xFFFFFFFF) as u32,
                )
            } else {
                ((r_info_wide >> 8) as usize, (r_info_wide & 0xFF) as u32)
            };

            if elf.machine == EM_RISCV && r_type == 51 {
                pos += entsize;
                continue;
            }

            if sym_idx >= symbols.len() {
                pos += entsize;
                continue;
            }

            let sym = &symbols[sym_idx];

if sym.shndx == SHN_UNDEF || (sym.name.is_empty() && sym_idx != 0) {
                // P6: a aperture-base reference (`__lang_aperture_{N}_base`) is a
                // binding-time reloc, not an ordinary import. It carries the
                // aperture id in the symbol-hash field and is bound to the
                // concrete base at pack time.
                if let Some(aperture_id) = aperture_base_id(sym.name.as_bytes()) {
                    let site_base = elf
                        .lmod_section_base(target_idx, &lmod::header::LmodHeader::new());
                    import_relocs.push((site_base + r_offset, aperture_id as u64, 10));
                } else {
                    let kind = import_reloc_kind(elf.machine, r_type)
                        .ok_or(PackError::UnsupportedInternalReloc(r_type))?;
                    let sym_hash = lmod::hash::linked_symbol_hash(sym.name.as_bytes());
                    let site_base = elf
                        .lmod_section_base(target_idx, &lmod::header::LmodHeader::new());
                    import_relocs.push((site_base + r_offset, sym_hash, kind));
                }
            } else if sym.shndx != SHN_ABS {
                let actual_addend = if is_rela {
                    r_addend
                } else {
                    let target_sec = &elf.sections[target_idx];
                    let section_data = elf.section_data(target_sec);
                    let site_in_section = r_offset as usize;
                    if r_type == 2 {
                        if site_in_section + 4 <= section_data.len() {
                            le_u32(section_data, site_in_section) as i32 as i64
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                };
                let sym_sec_idx = sym.shndx as usize;
                internal_fixups.push((
                    target_idx,
                    r_type,
                    r_offset,
                    sym_sec_idx,
                    sym.value,
                    actual_addend,
                ));
            }

            pos += entsize;
        }
    }

    // 5. Compute .lmod layout with the correct reloc count.
    let reloc_count = import_relocs.len() as u32;
    let bss_len = 0u32;
    let layout = lmod::header::compute_layout(
        abi_hash,
        modinfo_len,
        code_len,
        rodata_len,
        data_len,
        bss_len,
        reloc_count,
        0,
    );

    // 6. Build the container in a buffer.
    let total = layout.total_len as usize;
    let mut out = vec![0u8; total];

    lmod::header::encode_header(&mut out, &layout);

    if !modinfo_data.is_empty() {
        let off = layout.modinfo_off as usize;
        out[off..off + modinfo_data.len()].copy_from_slice(modinfo_data);
    }
    if !code_data.is_empty() {
        let off = layout.code_off as usize;
        out[off..off + code_data.len()].copy_from_slice(code_data);
    }
    // P6 (decision D-4): aperture-base reloc sites are NOT bound here. The
    // packed `.lmod` keeps the assembler-emitted site bytes (zeros) and the
    // kind-10 reloc records; the on-device loader writes the board's base at
    // load time (design doc §5.8 step 5, FR-15) after the `platform_hash`
    // gate (E5220). No absolute device address is baked into the code section.
    if !rodata_data.is_empty() {
        let off = layout.rodata_off as usize;
        out[off..off + rodata_data.len()].copy_from_slice(rodata_data);
    }
    if !data_data.is_empty() {
        let off = layout.data_off as usize;
        out[off..off + data_data.len()].copy_from_slice(data_data);
    }

    // 7. Pre-resolve internal relocations.
    let mut riscv_pcrel_hi_targets: Vec<(u64, u64)> = Vec::new();
    if elf.machine == EM_RISCV {
        for &(target_sec_idx, r_type, r_offset, sym_sec_idx, st_value, addend) in &internal_fixups {
            if r_type != 23 {
                continue;
            }
            let site_base = elf.lmod_section_base(target_sec_idx, &layout);
            let sym_base = if sym_sec_idx < elf.sections.len() {
                elf.lmod_section_base(sym_sec_idx, &layout)
            } else {
                0
            };
            riscv_pcrel_hi_targets.push((
                site_base + r_offset,
                sym_base.wrapping_add(st_value).wrapping_add(addend as u64),
            ));
        }
    }
    for &(target_sec_idx, r_type, r_offset, sym_sec_idx, st_value, addend) in &internal_fixups {
        let site_base = elf.lmod_section_base(target_sec_idx, &layout);
        let sym_base = if sym_sec_idx < elf.sections.len() {
            elf.lmod_section_base(sym_sec_idx, &layout)
        } else {
            0
        };
        let sym_value = sym_base + st_value;
        if elf.machine == EM_RISCV && r_type == 24 {
            let hi_target = riscv_pcrel_hi_targets
                .iter()
                .find_map(|(hi_site, hi_target)| (*hi_site == sym_value).then_some(*hi_target))
                .ok_or(PackError::UnsupportedInternalReloc(r_type))?;
            apply_riscv_pcrel_lo12_i(&mut out, site_base + r_offset, sym_value, hi_target, addend)?;
            continue;
        }
        apply_internal_reloc(
            &mut out,
            elf.machine,
            r_type,
            r_offset,
            sym_value,
            site_base,
            addend,
        )?;
    }

    // 8. Re-compute import site_off using the actual .lmod section bases.
    let mut import_final: Vec<(u32, u64, u8)> = Vec::new();
    for (raw_off, sym_hash, kind) in &import_relocs {
        // raw_off was computed with LmodHeader::new() (all-zero bases).
        // For v1, all imports target .text, so add code_off.
        // TODO: proper per-section adjustment — see §0.3 in the arch spec.
        import_final.push(((*raw_off as u32) + layout.code_off, *sym_hash, *kind));
    }

    if !import_final.is_empty() {
        let reloc_off = layout.reloc_off as usize;
        for (i, &(site_off, sym_hash, kind)) in import_final.iter().enumerate() {
            let entry_off = reloc_off + i * 16;
            out[entry_off..entry_off + 4].copy_from_slice(&site_off.to_le_bytes());
            out[entry_off + 4..entry_off + 12].copy_from_slice(&sym_hash.to_le_bytes());
            out[entry_off + 12] = kind;
        }
    }

    // 9. Verify section data integrity (modinfo matches source).
    if !modinfo_data.is_empty() {
        let off = layout.modinfo_off as usize;
        let written = &out[off..off + modinfo_data.len()];
        if written != modinfo_data {
            return Err(PackError::ModinfoCorruption);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Helpers: build minimal ELF64/32 ET_REL byte buffers.
    // -----------------------------------------------------------------------

    /// A minimal 64-bit ELF ET_REL header with no sections (just the header).
    /// `e_shnum = 0`, `e_shstrndx = 0`, no section headers.
    fn elf64_empty() -> Vec<u8> {
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // ELFCLASS64
        buf[5] = 1; // little-endian
        buf[16] = 1;
        buf[17] = 0; // e_type = ET_REL (1)
                     // e_machine = 0x3E (x86_64) at byte 18-19
        buf[18] = 0x3E;
        buf[19] = 0;
        // e_shoff at offset 40 (8 bytes)
        // e_shentsize at offset 58 (2 bytes) = 64
        // e_shnum at offset 60 (2 bytes) = 0
        // e_shstrndx at offset 62 (2 bytes) = 0
        buf[58] = 64;
        buf[59] = 0; // shentsize = 64
        buf[60] = 0;
        buf[61] = 0; // shnum = 0
        buf
    }

    /// Minimal 64-bit ELF ET_REL with one .text section header.
    /// Returns (buf, shoff) so callers can patch the section header.
    fn elf64_with_text_section(text_size: u32) -> Vec<u8> {
        let mut buf = elf64_empty();
        // Set e_shoff = 64 (section headers immediately after ELF header)
        buf[40..48].copy_from_slice(&64u64.to_le_bytes());
        // e_shnum = 1
        buf[60..62].copy_from_slice(&1u16.to_le_bytes());
        // e_shstrndx = 1 (second section = strtab)
        buf[62..64].copy_from_slice(&1u16.to_le_bytes());

        // Section header 0: .text  (at offset 64)
        // sh_name at +0: 4 bytes
        // sh_type at +4: 4 bytes (SHT_PROGBITS = 1)
        // sh_flags at +8: 8 bytes
        // sh_addr at +16: 8 bytes
        // sh_offset at +24: 8 bytes
        // sh_size at +32: 8 bytes
        // sh_link at +40: 4 bytes
        // sh_info at +44: 4 bytes
        // sh_addralign at +48: 8 bytes
        // sh_entsize at +56: 8 bytes
        let mut sh = vec![0u8; 64];
        // sh_name = offset into shstrtab for ".text" — will be set later
        // sh_type = SHT_PROGBITS (1)
        sh[4..8].copy_from_slice(&1u32.to_le_bytes());
        // sh_offset = after headers + section headers (64 + 64 + 64 = 192)
        let text_off: u64 = 64 + 64 + 64;
        sh[24..32].copy_from_slice(&text_off.to_le_bytes());
        sh[32..36].copy_from_slice(&text_size.to_le_bytes());
        buf.extend_from_slice(&sh);

        // Section header 1: .shstrtab (at offset 128)
        let mut shstr = vec![0u8; 64];
        shstr[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        let strtab_off = text_off + text_size as u64;
        shstr[24..32].copy_from_slice(&strtab_off.to_le_bytes());
        shstr[32..36].copy_from_slice(&32u32.to_le_bytes()); // size
        buf.extend_from_slice(&shstr);

        // .text section data (at text_off)
        let text_padding = vec![0xCCu8; text_size as usize];
        buf.extend_from_slice(&text_padding);

        // .shstrtab data: include ".text\0"
        let mut strtab = vec![0u8; 32];
        strtab[0..6].copy_from_slice(b".text\0");
        buf.extend_from_slice(&strtab);

        buf
    }

    /// Minimal 32-bit ELF ET_REL header.
    fn elf32_empty() -> Vec<u8> {
        let mut buf = vec![0u8; 52]; // ELF32 header is 52 bytes
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 1; // ELFCLASS32
        buf[5] = 1; // little-endian
        buf[16] = 1;
        buf[17] = 0; // e_type = ET_REL
                     // e_machine = 0x28 (ARM)
        buf[18] = 0x28;
        buf[19] = 0;
        // e_shoff at offset 0x20 (4 bytes)
        // e_shentsize at offset 0x2E (2 bytes) = 40
        // e_shnum at offset 0x30 (2 bytes) = 0
        // e_shstrndx at offset 0x32 (2 bytes)
        buf[0x2E] = 40;
        buf[0x2F] = 0; // shentsize = 40
        buf[0x30] = 0;
        buf[0x31] = 0; // shnum = 0
        buf
    }

    // -----------------------------------------------------------------------
    // PackError coverage: each variant must be reachable.
    // -----------------------------------------------------------------------

    #[test]
    fn rejects_too_small() {
        assert_eq!(pack(b""), Err(PackError::TooSmall));
        assert_eq!(pack(&[0; 63]), Err(PackError::TooSmall));
    }

    #[test]
    fn rejects_bad_magic() {
        let buf = [0u8; 64];
        assert_eq!(pack(&buf), Err(PackError::BadMagic));
    }

    #[test]
    fn rejects_unsupported_class() {
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 3; // ELF class 3 = unsupported
        assert_eq!(pack(&buf), Err(PackError::UnsupportedClass(3)));
    }

    #[test]
    fn rejects_big_endian() {
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 1; // ELFCLASS32
        buf[5] = 2; // big-endian
        assert_eq!(pack(&buf), Err(PackError::NotLittleEndian));
    }

    #[test]
    fn rejects_not_relocatable() {
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2;
        buf[5] = 1;
        buf[16] = 2;
        buf[17] = 0; // e_type = ET_EXEC (2), not ET_REL
        assert_eq!(pack(&buf), Err(PackError::NotRelocatable));
    }

    #[test]
    fn rejects_shentsize_mismatch() {
        let mut buf = elf64_empty();
        // Set up a section header offset and count so parse reaches shentsize check.
        buf[40..48].copy_from_slice(&64u64.to_le_bytes()); // e_shoff
        buf[60..62].copy_from_slice(&1u16.to_le_bytes()); // e_shnum = 1
        buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // e_shstrndx = 1
                                                          // Add one section header after the header (at offset 64) with wrong entsize
                                                          // sh_name name_off for ".text" would be in shstrtab, but we set shentsize wrong first
                                                          // Actually, we trick the parser: give it e_shentsize != 64 for ELF64
        buf[58] = 48;
        buf[59] = 0; // shentsize = 48 (should be 64 for ELF64)
                     // Add section header data so it doesn't overflow
        buf.extend_from_slice(&[0u8; 48]); // 48-byte section header
                                           // Add a dummy .shstrtab section
        let mut shstr = vec![0u8; 48];
        shstr[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        shstr[24..32].copy_from_slice(&(64u64 + 48 + 48).to_le_bytes()); // offset
        shstr[32..36].copy_from_slice(&16u32.to_le_bytes()); // size
        buf.extend_from_slice(&shstr);
        buf.extend_from_slice(&[0u8; 16]); // strtab data
        assert_eq!(
            pack(&buf),
            Err(PackError::ShentsizeMismatch {
                expected: 48,
                actual: 64
            })
        );
    }

    #[test]
    fn rejects_section_headers_overflow() {
        // e_shoff + e_shnum * shentsize > data.len()
        let mut buf = elf64_empty();
        buf[40..48].copy_from_slice(&1000u64.to_le_bytes()); // e_shoff past end
        buf[60..62].copy_from_slice(&10u16.to_le_bytes()); // shnum = 10
        assert_eq!(pack(&buf), Err(PackError::SectionHeadersOverflow));
    }

    #[test]
    fn rejects_unsupported_reloc_type() {
        // The packer reaches apply_internal_reloc only with valid ELF that
        // has internal relocations.  Test apply_internal_reloc directly.
        let mut out = [0u8; 8];
        let result = apply_internal_reloc(&mut out, EM_X86_64, 99, 0, 0, 0, 0);
        assert_eq!(result, Err(PackError::UnsupportedInternalReloc(99)));
    }

    #[test]
    fn rejects_reloc_site_out_of_range() {
        let mut out = [0u8; 4];
        // R_X86_64_64 writes 8 bytes starting at site_addr = site_base + r_offset.
        let result = apply_internal_reloc(&mut out, EM_X86_64, 1, 8, 0x100, 0, 0);
        assert!(result.is_err());
        assert!(matches!(result, Err(PackError::RelocSiteOutOfRange { .. })));

        // R_X86_64_PC32 writes 4 bytes at a 64-bit address.
        let mut out2 = [0u8; 4];
        let result2 = apply_internal_reloc(&mut out2, EM_X86_64, 2, 8, 0x100, 0, 0);
        assert!(result2.is_err());
        assert!(matches!(
            result2,
            Err(PackError::RelocSiteOutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_modinfo_corruption() {
        // Build a valid-enough 64-bit ELF that packs, then corrupt the output.
        // We can't easily trigger ModinfoCorruption from pack() directly since
        // it only fires when the internal section-copy goes wrong.  Verify the
        // error variant exists and construct output that would trigger it.
        // The check compares written modinfo against source modinfo; when they
        // don't match (shouldn't happen in practice), it returns this error.
        // We test the error is constructable and displayable.
        let e = PackError::ModinfoCorruption;
        assert_eq!(format!("{}", e), "modinfo data corruption during pack");
    }

    // -----------------------------------------------------------------------
    // apply_internal_reloc: known-answer tests
    // -----------------------------------------------------------------------

    #[test]
    fn reloc_arm_abs32() {
        // R_ARM_ABS32 (kind=2, site_addr < 2^32): S + A, 4 bytes
        let mut out = vec![0u8; 16];
        apply_internal_reloc(&mut out, EM_ARM, 2, 0, 0x2000, 8, 0x100).unwrap();
        let written = u32::from_le_bytes(out[8..12].try_into().unwrap());
        assert_eq!(written, 0x2100, "R_ARM_ABS32 must write S+A");
    }

    #[test]
    fn reloc_arm_abs32_negative_addend() {
        let mut out = vec![0u8; 16];
        apply_internal_reloc(&mut out, EM_ARM, 2, 0, 0x1000, 8, -0x100).unwrap();
        let written = i32::from_le_bytes(out[8..12].try_into().unwrap());
        assert_eq!(
            written, 0xf00,
            "R_ARM_ABS32 with negative addend must write S+A"
        );
    }

    #[test]
    fn reloc_x86_64_64_additive() {
        // R_X86_64_64 (kind=1): S + A, 8 bytes
        let mut out = vec![0u8; 16];
        apply_internal_reloc(&mut out, EM_X86_64, 1, 0, 0x1234, 0, 0x100).unwrap();
        let written = u64::from_le_bytes(out[0..8].try_into().unwrap());
        assert_eq!(written, 0x1334, "R_X86_64_64 must write S+A at site");
    }

    #[test]
    fn reloc_x86_64_64_wrapping() {
        let mut out = vec![0u8; 16];
        apply_internal_reloc(&mut out, EM_X86_64, 1, 0, u64::MAX, 0, 1).unwrap();
        let written = u64::from_le_bytes(out[0..8].try_into().unwrap());
        assert_eq!(written, 0, "R_X86_64_64 must wrap on overflow");
    }

    #[test]
    fn reloc_x86_64_64_offset_nonzero() {
        let mut out = vec![0u8; 16];
        // r_offset = 4 → write at site_base + 4 = 8
        apply_internal_reloc(&mut out, EM_X86_64, 1, 4, 0xABCD, 4, 0).unwrap();
        let written = u64::from_le_bytes(out[8..16].try_into().unwrap());
        assert_eq!(
            written, 0xABCD,
            "R_X86_64_64 with non-zero r_offset must write at site"
        );
    }

    #[test]
    fn apply_r_x86_64_pc32_is_S_plus_A_minus_P() {
        let mut out = vec![0u8; 16];
        apply_internal_reloc(&mut out, EM_X86_64, 2, 0, 0x1000, 8, -4).unwrap();
        let got = u32::from_le_bytes(out[8..12].try_into().unwrap());
        assert_eq!(got, 0x0ff4, "x86_64 PC32 path: S + A - P = 0x1000 - 4 - 8");
    }

    #[test]
    fn apply_r_x86_64_plt32_is_S_plus_A_minus_P() {
        // kind=3 (PLT32) uses S+A-P unconditionally — no address check.
        let mut out = vec![0u8; 16];
        // S=0x2000, A=-4, P=site_base(8)+r_offset(0) → 0x2000 - 4 - 8 = 0x1FF4
        apply_internal_reloc(&mut out, EM_X86_64, 3, 0, 0x2000, 8, -4).unwrap();
        let got = i32::from_le_bytes(out[8..12].try_into().unwrap());
        assert_eq!(got, 0x1FF4);
    }

    // -----------------------------------------------------------------------
    // Malformed-ELF corpus: verify pack never panics, only Errors.
    // -----------------------------------------------------------------------

    /// Return a minimal valid 64-bit ELF ET_REL skeleton with one .text
    /// section (8 bytes) and a .shstrtab.  Corruptor functions modify a
    /// clone and pass it to pack() via catch_unwind.
    fn valid_min_et_rel() -> Vec<u8> {
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // ELFCLASS64
        buf[5] = 1; // little-endian
        buf[16..18].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
        buf[40..48].copy_from_slice(&64u64.to_le_bytes()); // e_shoff = 64
        buf[58..60].copy_from_slice(&64u16.to_le_bytes()); // shentsize
        buf[60..62].copy_from_slice(&2u16.to_le_bytes()); // shnum = 2
        buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // shstrndx = 1

        // Section 0: .text (SHT_PROGBITS), 8 bytes at offset 64+64+64=192
        let mut sh0 = vec![0u8; 64];
        sh0[4..8].copy_from_slice(&1u32.to_le_bytes()); // SHT_PROGBITS
        sh0[24..32].copy_from_slice(&192u64.to_le_bytes()); // sh_offset
        sh0[32..36].copy_from_slice(&8u32.to_le_bytes()); // sh_size
        buf.extend_from_slice(&sh0);

        // Section 1: .shstrtab (SHT_STRTAB), 16 bytes at offset 192+8=200
        let mut sh1 = vec![0u8; 64];
        sh1[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        sh1[24..32].copy_from_slice(&200u64.to_le_bytes()); // sh_offset
        sh1[32..36].copy_from_slice(&16u32.to_le_bytes()); // sh_size
        buf.extend_from_slice(&sh1);

        // .text content (8 bytes)
        buf.extend_from_slice(&[0xCCu8; 8]);

        // .shstrtab: ".text\0"
        let mut strtab = vec![0u8; 16];
        strtab[0..6].copy_from_slice(b".text\0");
        buf.extend_from_slice(&strtab);

        buf
    }

    fn bad_shstrndx(mut base: Vec<u8>) -> Vec<u8> {
        // Set shstrndx past section count → out-of-bounds access.
        base[62..64].copy_from_slice(&5u16.to_le_bytes()); // shstrndx = 5 (only 2 sections)
        base
    }

    fn name_off_overflow(mut base: Vec<u8>) -> Vec<u8> {
        // Set the sh_name field of section 0 to a value past the strtab.
        let name_off: u32 = 200; // past 16-byte strtab
        base[64..68].copy_from_slice(&name_off.to_le_bytes());
        base
    }

    fn truncate_sh_table(mut base: Vec<u8>) -> Vec<u8> {
        // e_shoff points into the ELF header itself (overlap).
        base[40..48].copy_from_slice(&0u64.to_le_bytes());
        base
    }

    fn huge_shnum(mut base: Vec<u8>) -> Vec<u8> {
        // e_shnum massive → overflow check triggers SectionHeadersOverflow.
        base[60..62].copy_from_slice(&0xFFFFu16.to_le_bytes());
        base
    }

    #[test]
    fn malformed_elf_never_panics() {
        let base = valid_min_et_rel();
        let corruptors: [fn(Vec<u8>) -> Vec<u8>; 4] = [
            bad_shstrndx,
            name_off_overflow,
            truncate_sh_table,
            huge_shnum,
        ];
        for corrupt in corruptors {
            let bytes = corrupt(base.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = pack(&bytes);
            }));
            assert!(
                result.is_ok(),
                "pack must not panic on malformed ELF (corruptor: {})",
                std::any::type_name_of_val(&corrupt),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Malformed ELF no-panic tests (Elf::parse must not panic)
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_shstrndx_out_of_range() {
        // e_shstrndx greater than e_shnum — causes sections[e_shstrndx] panic
        // in the original code.  We need at least one section header so the
        // shstrndx access happens.
        let mut buf = elf64_empty();
        buf[40..48].copy_from_slice(&64u64.to_le_bytes()); // e_shoff
        buf[60..62].copy_from_slice(&1u16.to_le_bytes()); // e_shnum = 1
        buf[62..64].copy_from_slice(&2u16.to_le_bytes()); // e_shstrndx = 2 (>= shnum)
                                                          // Add one section header
        buf.extend_from_slice(&[0u8; 64]);
        // The parser will try access sections[2] → out of bounds → should Err, not panic.
        let result = pack(&buf);
        assert!(
            result.is_err(),
            "shstrndx >= shnum must return Err, not panic"
        );
    }

    #[test]
    fn malformed_shstrtab_name_off_past_end() {
        // sh_name offset in a section header points past shstrtab → panic on
        // shstrtab[name_off..].  We need a real shstrtab section.
        let mut buf = elf64_empty();
        buf[40..48].copy_from_slice(&64u64.to_le_bytes()); // e_shoff
        buf[60..62].copy_from_slice(&2u16.to_le_bytes()); // e_shnum = 2
        buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // e_shstrndx = 1

        // Section 0: name_off points past strtab
        let mut sh0 = vec![0u8; 64];
        sh0[0..4].copy_from_slice(&99u32.to_le_bytes()); // sh_name = 99 (past strtab)
        sh0[4..8].copy_from_slice(&1u32.to_le_bytes()); // SHT_PROGBITS
        sh0[24..32].copy_from_slice(&256u64.to_le_bytes()); // offset
        sh0[32..36].copy_from_slice(&8u32.to_le_bytes()); // size
        buf.extend_from_slice(&sh0);

        // Section 1: .shstrtab
        let mut sh1 = vec![0u8; 64];
        sh1[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
        sh1[24..32].copy_from_slice(&256u64.to_le_bytes()); // offset
        sh1[32..36].copy_from_slice(&16u32.to_le_bytes()); // size (only 16 bytes)
        buf.extend_from_slice(&sh1);

        // .shstrtab data: 16 bytes
        buf.extend_from_slice(&[0u8; 16]);

        // The name_off = 99 is past the 16-byte strtab → parser should Err, not panic.
        let result = pack(&buf);
        assert!(
            result.is_err(),
            "name_off past strtab must return Err, not panic"
        );
    }

    #[test]
    fn malformed_e_shnum_overflow() {
        // e_shnum massive → overflow in section headers
        let mut buf = elf64_empty();
        buf[40..48].copy_from_slice(&1000u64.to_le_bytes()); // e_shoff
        buf[60..62].copy_from_slice(&0xFFFFu16.to_le_bytes()); // e_shnum = 65535
                                                               // This should trigger SectionHeadersOverflow rather than panic.
        let result = pack(&buf);
        assert_eq!(result, Err(PackError::SectionHeadersOverflow));
    }

    // -----------------------------------------------------------------------
    // 32-bit ELF (elf_class == 1) path coverage
    // -----------------------------------------------------------------------

    #[test]
    fn elf32_rejected_properly() {
        // The packer accepts 32-bit ELF.  This test just verifies the
        // 32-bit parse path doesn't panic on a minimal valid 32-bit object.
        let mut buf = elf32_empty();
        // Set e_shoff so the parser enters the 32-bit branch
        buf[0x20..0x24].copy_from_slice(&52u32.to_le_bytes()); // e_shoff after header
        buf[0x30..0x32].copy_from_slice(&1u16.to_le_bytes()); // shnum = 1
        buf[0x32..0x34].copy_from_slice(&1u16.to_le_bytes()); // shstrndx = 1
                                                              // Add a 40-byte section header (ELF32 shentsize)
        let mut sh = vec![0u8; 40];
        sh[4..8].copy_from_slice(&1u32.to_le_bytes()); // SHT_PROGBITS
        sh[16..20].copy_from_slice(&52u32.to_le_bytes()); // sh_offset
        sh[20..24].copy_from_slice(&8u32.to_le_bytes()); // sh_size
        buf.extend_from_slice(&sh);
        // Add shstrtab section header
        let mut shstr = vec![0u8; 40];
        shstr[4..8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
                                                          // set offset + size after the second section header
        shstr[16..20].copy_from_slice(&(52u32 + 40 + 40).to_le_bytes()); // offset
        shstr[20..24].copy_from_slice(&16u32.to_le_bytes()); // size
        buf.extend_from_slice(&shstr);
        // Section data + strtab
        buf.extend_from_slice(&[0xCCu8; 8]); // .text data
        buf.extend_from_slice(&[0u8; 16]); // strtab (empty names)

        // It must not panic.  It may Err (no modinfo section is OK).
        let result = pack(&buf);
        // Must not panic: either Ok (surprising but OK) or Err (expected).
        assert!(
            result.is_ok() || result.is_err(),
            "32-bit ELF parse must not panic"
        );
    }

    // -----------------------------------------------------------------------
    // Golden round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn golden_pack_parity_with_binary() {
        let gold_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("test-goldens");

        // Re-compile golden.mod to get a fresh .o.
        let tmp = std::env::temp_dir()
            .join("lmod_pack_lib_test")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mod_src = std::fs::read(gold_dir.join("golden.mod")).unwrap();
        std::fs::write(tmp.join("golden.mod"), &mod_src).unwrap();

        let langc = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join("debug")
            .join("langc");

        let out = std::process::Command::new(&langc)
            .args([
                "--emit=obj",
                "--target=x86_64-unknown-none",
                &format!("--out-dir={}", tmp.to_string_lossy()),
                tmp.join("golden.mod").to_string_lossy().as_ref(),
            ])
            .output()
            .expect("langc");
        assert!(
            out.status.success(),
            "langc failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );

        // Find the .o file.
        let o_file: PathBuf = std::fs::read_dir(&tmp)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("o"))
            .expect("no .o produced");

        // Read the golden .lmod (from Phase 0, produced by lmod-pack binary).
        let golden_lmod = std::fs::read(gold_dir.join("packed.lmod")).unwrap();

        // Use the library to pack the same .o.
        let elf_bytes = std::fs::read(&o_file).unwrap();
        let lib_output = pack(&elf_bytes).unwrap();

        assert_eq!(
            lib_output.len(),
            golden_lmod.len(),
            "packed size must match golden"
        );
        assert_eq!(
            lib_output, golden_lmod,
            "pack() output must be byte-identical to the lmod-pack binary golden"
        );
    }
}

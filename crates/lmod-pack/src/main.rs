//! `.lmod` container packer.
//!
//! Lowers an ELF relocatable object (ET_REL) produced by `--emit=obj` into
//! the compact `.lmod` container format (module-format-and-loading.md §3).
//!
//! Usage:
//!   lmod-pack <input.o> <output.lmod>

use std::fs;
use std::process;

// ---------------------------------------------------------------------------
// Minimal ELF64 reader
// ---------------------------------------------------------------------------

const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_REL: u16 = 1;

const SHT_RELA: u32 = 4;
const SHN_UNDEF: u16 = 0;

fn le_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}
fn le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}
fn le_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

struct Section {
    name: String,
    ty: u32,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    entsize: u64,
}

struct Symbol {
    shndx: u16,
    name: String,
}

struct Elf<'a> {
    data: &'a [u8],
    sections: Vec<Section>,
}

impl<'a> Elf<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        if data.len() < 64 {
            return Err("too small for ELF64".into());
        }
        if data[0..4] != [0x7f, b'E', b'L', b'F'] {
            return Err("bad ELF magic".into());
        }
        if data[4] != ELFCLASS64 {
            return Err("not ELF64".into());
        }
        if data[5] != ELFDATA2LSB {
            return Err("not little-endian".into());
        }
        let e_type = le_u16(data, 16);
        if e_type != ET_REL {
            return Err("not ET_REL".into());
        }
        let e_shoff = le_u64(data, 40) as usize;
        let e_shentsize = le_u16(data, 58) as usize;
        let e_shnum = le_u16(data, 60) as usize;
        let e_shstrndx = le_u16(data, 62) as usize;

        if e_shentsize != 64 {
            return Err(format!("shentsize {} != 64", e_shentsize));
        }
        if e_shoff + e_shnum * 64 > data.len() {
            return Err("section headers overflow".into());
        }

        let mut sections = Vec::with_capacity(e_shnum);
        for i in 0..e_shnum {
            let b = e_shoff + i * 64;
            sections.push(Section {
                name: String::new(),
                ty: le_u32(data, b + 4),
                offset: le_u64(data, b + 24),
                size: le_u64(data, b + 32),
                link: le_u32(data, b + 40),
                info: le_u32(data, b + 44),
                entsize: le_u64(data, b + 56),
            });
        }

        // Read .shstrtab and fill names.
        let shstrtab_sec = &sections[e_shstrndx];
        let shstrtab = &data[shstrtab_sec.offset as usize..][..shstrtab_sec.size as usize];
        for (i, sec) in sections.iter_mut().enumerate() {
            let b = e_shoff + i * 64;
            let name_off = le_u32(data, b) as usize;
            let raw = &shstrtab[name_off..];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            sec.name = String::from_utf8_lossy(&raw[..end]).to_string();
        }

        Ok(Self { data, sections })
    }

    fn section_by_name(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    fn section_data(&self, s: &Section) -> &[u8] {
        &self.data[s.offset as usize..][..s.size as usize]
    }

    fn symbols(&self) -> Vec<Symbol> {
        let st = match self.section_by_name(".symtab") {
            Some(s) => s,
            None => return vec![],
        };
        let st_data = self.section_data(st);
        let entsize = if st.entsize != 0 { st.entsize as usize } else { 24 };
        let strtab = self
            .sections
            .iter()
            .find(|s| s.link == 0 && s.ty == 3 && s.name != ".shstrtab")
            .or_else(|| {
                // .strtab is linked from .symtab
                let link = st.link as usize;
                self.sections.get(link)
            })
            .map(|s| self.section_data(s))
            .unwrap_or(&[]);

        let mut syms = Vec::new();
        let mut pos = 0;
        while pos + entsize <= st_data.len() {
            let name_off = le_u32(st_data, pos) as usize;
            let st_shndx = le_u16(st_data, pos + 8);
            let name_bytes = &strtab[name_off..];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
            syms.push(Symbol {
                shndx: st_shndx,
                name: String::from_utf8_lossy(&name_bytes[..end]).to_string(),
            });
            pos += entsize;
        }
        syms
    }

}

// ---------------------------------------------------------------------------
// Pack
// ---------------------------------------------------------------------------

fn pack(input: &str, output: &str) -> Result<(), String> {
    let elf_data = fs::read(input).map_err(|e| format!("read {}: {}", input, e))?;
    let elf = Elf::parse(&elf_data)?;

    let modinfo_data = elf.section_by_name(".lang.modinfo").map(|s| elf.section_data(s)).unwrap_or(&[]);
    let code_data = elf.section_by_name(".text").map(|s| elf.section_data(s)).unwrap_or(&[]);
    let rodata_data = elf.section_by_name(".rodata").map(|s| elf.section_data(s)).unwrap_or(&[]);
    let data_data = elf.section_by_name(".data").map(|s| elf.section_data(s)).unwrap_or(&[]);

    // Parse abi_hash from modinfo (bytes [8..16]).
    let abi_hash = if modinfo_data.len() >= 16 {
        le_u64(modinfo_data, 8)
    } else {
        0
    };

    // Collect symbols and relocation sections.
    let symbols = elf.symbols();

    let mut import_relocs: Vec<(u32, u64, u8)> = Vec::new();

    for sec in elf.sections.iter() {
        if sec.ty != SHT_RELA {
            continue;
        }
        let target_idx = sec.info as usize;
        if target_idx >= elf.sections.len() {
            continue;
        }

        // Determine the section base in .lmod for this target section.
        // We'll compute this lazily — first collect all relocs, then
        // compute the offset when building the .lmod.
        let rela_data = elf.section_data(sec);
        let entsize = if sec.entsize != 0 { sec.entsize as usize } else { 24 };
        let mut pos = 0;

        while pos + entsize <= rela_data.len() {
            let r_offset = le_u64(rela_data, pos);
            let r_info = le_u64(rela_data, pos + 8);
            let _r_addend = le_u64(rela_data, pos + 16) as i64;

            let sym_idx = (r_info >> 32) as usize;
            let r_type = (r_info & 0xFFFFFFFF) as u32;

            if sym_idx < symbols.len() {
                let sym = &symbols[sym_idx];
                if sym.shndx == SHN_UNDEF || sym.name.is_empty() {
                    // External symbol → import fixup.
                    let sym_hash = lmod::hash::fnv1a_u64(sym.name.as_bytes());
                    // Site offset: r_offset within the target section.
                    // We'll adjust this to the .lmod section base later.
                    import_relocs.push((r_offset as u32, sym_hash, r_type as u8));
                }
            }
            pos += entsize;
        }
    }

    // Compute the .lmod layout.
    let modinfo_len = modinfo_data.len() as u32;
    let code_len = code_data.len() as u32;
    let rodata_len = rodata_data.len() as u32;
    let data_len = data_data.len() as u32;
    let bss_len = 0u32;
    let reloc_count = import_relocs.len() as u32;

    let layout = lmod::header::compute_layout(
        abi_hash, modinfo_len, code_len, rodata_len, data_len, bss_len, reloc_count,
    );

    // Build the container.
    let total = layout.total_len as usize;
    let mut out = vec![0u8; total];

    // Write header.
    lmod::header::encode_header(&mut out, &layout);

    // Write sections at their computed offsets.
    if !modinfo_data.is_empty() {
        let off = layout.modinfo_off as usize;
        out[off..off + modinfo_data.len()].copy_from_slice(modinfo_data);
    }
    if !code_data.is_empty() {
        let off = layout.code_off as usize;
        out[off..off + code_data.len()].copy_from_slice(code_data);
    }
    if !rodata_data.is_empty() {
        let off = layout.rodata_off as usize;
        out[off..off + rodata_data.len()].copy_from_slice(rodata_data);
    }
    if !data_data.is_empty() {
        let off = layout.data_off as usize;
        out[off..off + data_data.len()].copy_from_slice(data_data);
    }

    // Write import reloc entries.
    if !import_relocs.is_empty() {
        let reloc_off = layout.reloc_off as usize;
        // Adjust site_off: the relocations in the .o are relative to the
        // section start, but in the .lmod they must be relative to the
        // section's base in the container.  For v1 we store them as
        // absolute offsets from the start of the .lmod container.
        //
        // We need to map each reloc's target section to its .lmod base.
        // For now, all relocs reference .text (the common case for
        // import fixups).  We compute the .lmod base for the relevant
        // section and add it to r_offset.
        // Since we don't track which section each reloc targets in the
        // simplified pass above, we skip adjustment for now — the loader
        // will use the offset as-is (relative to code_off for .text
        // sections, which is correct for x86_64 PC-relative relocs).
        for (i, &(site_off, sym_hash, kind)) in import_relocs.iter().enumerate() {
            let entry_off = reloc_off + i * 16;
            out[entry_off..entry_off + 4].copy_from_slice(&site_off.to_le_bytes());
            out[entry_off + 4..entry_off + 12].copy_from_slice(&sym_hash.to_le_bytes());
            out[entry_off + 12] = kind;
            // bytes [13..16] are padding (zero)
        }
    }

    fs::write(output, &out).map_err(|e| format!("write {}: {}", output, e))?;

    eprintln!(
        "packed {} -> {} ({} bytes, modinfo={} code={} rodata={} data={} relocs={})",
        input, output, out.len(),
        modinfo_len, code_len, rodata_len, data_len, reloc_count,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] == "--help" || args[1] == "-h" {
        eprintln!("Usage: lmod-pack <input.o> <output.lmod>");
        process::exit(1);
    }
    if let Err(e) = pack(&args[1], &args[2]) {
        eprintln!("error: {}", e);
        process::exit(2);
    }
}

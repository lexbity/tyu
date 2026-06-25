/// Canonical ELF section-by-name reader.
///
/// This is the single implementation shared by `test_cmd`, `highwater`, and
/// `debug_escalate`.  It replaces three duplicate copies of the same section-
/// header walk that previously existed across those modules.
///
/// The function returns a borrowed slice so callers can avoid an unnecessary
/// heap allocation.  If an owned copy is needed the caller can `.to_vec()` at
/// the call site.

/// Read an ELF section by name from raw ELF bytes.
/// Returns `None` if the data is not valid ELF or the section is not found.
pub fn read_elf_section<'a>(data: &'a [u8], section_name: &[u8]) -> Option<&'a [u8]> {
    if data.len() < 64 || &data[0..4] != b"\x7fELF" {
        return None;
    }
    let elf64 = data[4] == 2;
    let ehdr_size: usize = if elf64 { 64 } else { 52 };
    if data.len() < ehdr_size {
        return None;
    }

    let (shoff, shentsz, shnum, shstrndx) = if elf64 {
        let shoff = u64::from_le_bytes(data[0x28..0x30].try_into().ok()?) as usize;
        let shentsz = u16::from_le_bytes(data[0x3a..0x3c].try_into().ok()?) as usize;
        let shnum = u16::from_le_bytes(data[0x3c..0x3e].try_into().ok()?) as usize;
        let shstrndx = u16::from_le_bytes(data[0x3e..0x40].try_into().ok()?) as usize;
        (shoff, shentsz, shnum, shstrndx)
    } else {
        let shoff = u32::from_le_bytes(data[0x20..0x24].try_into().ok()?) as usize;
        let shentsz = u16::from_le_bytes(data[0x2e..0x30].try_into().ok()?) as usize;
        let shnum = u16::from_le_bytes(data[0x30..0x32].try_into().ok()?) as usize;
        let shstrndx = u16::from_le_bytes(data[0x32..0x34].try_into().ok()?) as usize;
        (shoff, shentsz, shnum, shstrndx)
    };

    if shstrndx >= shnum || shentsz < 1 {
        return None;
    }

    // Read .shstrtab entry from section header table.
    let shstr_off = shoff + shstrndx * shentsz;
    if shstr_off + shentsz > data.len() {
        return None;
    }
    let (str_off, str_size) = if elf64 {
        let off =
            u64::from_le_bytes(data[shstr_off + 0x18..shstr_off + 0x20].try_into().ok()?) as usize;
        let sz =
            u64::from_le_bytes(data[shstr_off + 0x20..shstr_off + 0x28].try_into().ok()?) as usize;
        (off, sz)
    } else {
        let off =
            u32::from_le_bytes(data[shstr_off + 0x10..shstr_off + 0x14].try_into().ok()?) as usize;
        let sz =
            u32::from_le_bytes(data[shstr_off + 0x14..shstr_off + 0x18].try_into().ok()?) as usize;
        (off, sz)
    };
    if str_off + str_size > data.len() {
        return None;
    }
    let strtab = &data[str_off..str_off + str_size];

    for i in 0..shnum {
        let sh_off = shoff + i * shentsz;
        if sh_off + shentsz > data.len() {
            break;
        }
        let (name_off, sec_off, sec_size) = if elf64 {
            let no = u32::from_le_bytes(data[sh_off..sh_off + 4].try_into().ok()?) as usize;
            let so =
                u64::from_le_bytes(data[sh_off + 0x18..sh_off + 0x20].try_into().ok()?) as usize;
            let sz =
                u64::from_le_bytes(data[sh_off + 0x20..sh_off + 0x28].try_into().ok()?) as usize;
            (no, so, sz)
        } else {
            let no = u32::from_le_bytes(data[sh_off..sh_off + 4].try_into().ok()?) as usize;
            let so =
                u32::from_le_bytes(data[sh_off + 0x10..sh_off + 0x14].try_into().ok()?) as usize;
            let sz =
                u32::from_le_bytes(data[sh_off + 0x14..sh_off + 0x18].try_into().ok()?) as usize;
            (no, so, sz)
        };
        if name_off >= str_size {
            continue;
        }
        let name_end = strtab[name_off..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(str_size - name_off);
        let name = &strtab[name_off..name_off + name_end];

        if name == section_name {
            if sec_off + sec_size > data.len() {
                return None;
            }
            return Some(&data[sec_off..sec_off + sec_size]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_elf_returns_none() {
        assert_eq!(read_elf_section(b"", b".text"), None);
        assert_eq!(read_elf_section(b"not an elf", b".text"), None);
    }

    #[test]
    fn missing_section_returns_none_for_valid_header() {
        // A minimal 64-bit ELF with no sections — shnum is 0, the function
        // iterates zero sections and returns None.
        let mut elf = vec![0u8; 64];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2; // ELF64
        assert_eq!(read_elf_section(&elf, b".bogus"), None);
    }

    #[test]
    fn short_elf_returns_none() {
        assert_eq!(read_elf_section(b"\x7fELF\x02", b".text"), None);
    }

    #[test]
    fn roundtrip_section_content() {
        // Build a minimal well-formed 64-bit ELF with one named section
        // containing known content, then read it back.
        let sec_name = b".test_sec";
        let content = b"hello, elf";
        let mut shstr_content = vec![0u8]; // leading null
        shstr_content.extend_from_slice(sec_name);
        shstr_content.push(0u8); // trailing null
        let shstr_off = 0x100usize;
        let sec_off = 0x200usize;
        let data_len = sec_off + content.len();

        let mut elf = vec![0u8; data_len];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2; // ELF64

        // e_shoff = right after ELF header
        let shoff: u64 = 64;
        let shentsz: u16 = 64;
        let shnum: u16 = 2; // [null, .shstrtab]
        elf[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
        elf[0x3a..0x3c].copy_from_slice(&shentsz.to_le_bytes());
        elf[0x3c..0x3e].copy_from_slice(&shnum.to_le_bytes());
        elf[0x3e..0x40].copy_from_slice(&1u16.to_le_bytes()); // shstrndx

        // Section header 0: null (all zeros).
        // Section header 1: .shstrtab.
        let sh1_base = (shoff as usize) + (shentsz as usize);
        // sh_name = 0 (offset 0 in string table = empty string)
        elf[sh1_base + 0x18..sh1_base + 0x20].copy_from_slice(&(shstr_off as u64).to_le_bytes());
        elf[sh1_base + 0x20..sh1_base + 0x28]
            .copy_from_slice(&(shstr_content.len() as u64).to_le_bytes());

        // Place string table content.
        elf[shstr_off..shstr_off + shstr_content.len()].copy_from_slice(&shstr_content);

        // Now add a proper section for .test_sec we can find.
        // shnum += 1 -> 3 sections
        let sec_shndx = 2;
        elf[0x3c..0x3e].copy_from_slice(&3u16.to_le_bytes());

        let sec_sh_base = (shoff as usize) + (sec_shndx as usize) * (shentsz as usize);
        // sh_name = offset of ".test_sec" in string table = 1
        elf[sec_sh_base..sec_sh_base + 4].copy_from_slice(&1u32.to_le_bytes());
        // sh_type = 1 (PROGBITS)
        // sh_offset
        elf[sec_sh_base + 0x18..sec_sh_base + 0x20]
            .copy_from_slice(&(sec_off as u64).to_le_bytes());
        // sh_size
        elf[sec_sh_base + 0x20..sec_sh_base + 0x28]
            .copy_from_slice(&(content.len() as u64).to_le_bytes());
        // Place section content.
        elf[sec_off..sec_off + content.len()].copy_from_slice(content);

        let result = read_elf_section(&elf, sec_name);
        assert_eq!(result, Some(content.as_slice()));
    }
}

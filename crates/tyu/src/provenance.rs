//! Link-time symbol provenance verification for the flat word-symbol
//! namespace.
//!
//! Word symbols mangle to `w_<fnv1a64(word-name)>` — the module named in
//! `import M { w }` is not part of the symbol, so a bind is correct only as
//! long as every linked module's exports stay unambiguous. Before an image
//! links, this module cross-checks every attributed object (one carrying a
//! `.lang.modinfo`) against the others:
//!
//! 1. **Duplicate export** — two modules exporting the same word hash. This
//!    is the precondition for every silent wrong-module bind (and today
//!    surfaces as ld's duplicate-symbol error, which names neither module's
//!    provenance).
//! 2. **Undeclared reference** — an undefined `w_*` symbol that the object's
//!    modinfo declares as neither import nor export. langc emits `extrn`
//!    only for words named in an `import` declaration, so this fires on
//!    stale or mixed artifacts (e.g. an object reused from an older build)
//!    binding symbols nobody asked for.
//!
//! Known residual (accepted, documented in the technical manual): `import
//! A { x }` where the link set contains no module `A` but exactly one other
//! module exporting `x` still binds silently — the from-module of an import
//! is not recorded in modinfo v4. Closing that requires the module-qualified
//! symbol ABI (or an import-table format change), which is deliberately
//! deferred.
//!
//! Objects without a parsable `.lang.modinfo` (runtime asm, generated
//! aperture/symtab objects) are unattributed: their symbols are
//! runtime-provided and pass without a definer.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::elf_reader;
use crate::error::TyuError;

/// Verify the provenance of every word symbol about to be linked.
///
/// Called from `link_image` with the full input object set. Unreadable or
/// unattributed inputs are skipped — a genuinely broken object fails the
/// link itself, which this check is not meant to replace.
pub fn verify_link_provenance(objs: &[PathBuf]) -> Result<(), TyuError> {
    let mut scans: Vec<(String, ObjectScan)> = Vec::new();
    for path in objs {
        let Ok(data) = fs::read(path) else {
            continue;
        };
        if let Some(scan) = scan_object(&data) {
            scans.push((path.display().to_string(), scan));
        }
    }
    check(&scans)
}

/// The provenance-relevant scan of one attributed object.
struct ObjectScan {
    module: String,
    /// `(sym_hash, word_name)` as declared in `.lang.modinfo`.
    exports: Vec<(u64, String)>,
    /// Import hashes as declared in `.lang.modinfo`.
    imports: Vec<u64>,
    /// Undefined `w_*` references from the ELF symbol table.
    undefined: Vec<u64>,
}

fn scan_object(data: &[u8]) -> Option<ObjectScan> {
    let minfo = elf_reader::read_elf_section(data, b".lang.modinfo")?;
    let hdr = lmod::modinfo::decode(minfo)?;
    let module = String::from_utf8_lossy(hdr.module_name).into_owned();

    let mut exports = Vec::with_capacity(hdr.export_count as usize);
    for i in 0..hdr.export_count {
        let e = lmod::modinfo::read_export(minfo, i)?;
        exports.push((e.sym_hash, String::from_utf8_lossy(e.name).into_owned()));
    }

    // Import entries follow the export entries (12 bytes each: u64 hash +
    // u32 name offset). lmod exposes no read_import, so parse them here on
    // top of the public offset helper.
    let import_entries_off =
        lmod::modinfo::export_entries_offset(minfo)? + hdr.export_count as usize * 16;
    let mut imports = Vec::with_capacity(hdr.import_count as usize);
    for i in 0..hdr.import_count {
        let off = import_entries_off + i as usize * 12;
        if off + 12 > minfo.len() {
            return None;
        }
        imports.push(u64::from_le_bytes(minfo[off..off + 8].try_into().ok()?));
    }

    let undefined = elf_reader::undefined_word_hashes(data);
    Some(ObjectScan {
        module,
        exports,
        imports,
        undefined,
    })
}

/// Apply the provenance rules over an already-scanned object set.
fn check(scans: &[(String, ObjectScan)]) -> Result<(), TyuError> {
    // sym_hash -> (module, word, object path) for every declared export.
    let mut defined: HashMap<u64, (String, String, &str)> = HashMap::new();
    for (path, scan) in scans {
        for (hash, word) in &scan.exports {
            if let Some((prev_module, prev_word, prev_path)) = defined.get(hash) {
                return Err(TyuError::Build(format!(
                    "duplicate word export: modules '{prev_module}' ('{prev_path}') and '{}' \
                     ('{path}') both export '{prev_word}' — word symbols are \
                     w_<fnv1a64(word-name)>, so these are one linker symbol; rename the word \
                     in one of the modules",
                    scan.module
                )));
            }
            defined.insert(*hash, (scan.module.clone(), word.clone(), path));
        }
    }

    for (path, scan) in scans {
        for hash in &scan.undefined {
            let declared =
                scan.imports.contains(hash) || scan.exports.iter().any(|(h, _)| h == hash);
            if declared {
                continue;
            }
            let described = match defined.get(hash) {
                Some((def_module, word, def_path)) => {
                    format!("'{word}' (exported by module '{def_module}', '{def_path}')")
                }
                None => format!("<unnamed> w_{hash:016x}"),
            };
            return Err(TyuError::Build(format!(
                "undeclared word reference: module '{}' ('{path}') references {} but its \
                 .lang.modinfo declares it as neither import nor export — the object is stale \
                 or was not produced for this module set; rebuild it",
                scan.module, described
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_X: u64 = 0x1111111111111111;
    const HASH_Y: u64 = 0x2222222222222222;

    /// Build a minimal ELF64 object with `.lang.modinfo` and `.symtab` /
    /// `.strtab` sections, following the `elf_reader` test construction.
    /// `undefined_syms` become SHN_UNDEF `w_<hex>` entries.
    fn object_elf(module: &[u8], exports: &[u64], imports: &[u64], undefined_syms: &[u64]) -> Vec<u8> {
        // Real modinfo bytes via the lmod encoder.
        let export_names: Vec<String> = exports.iter().map(|&h| name_of(h)).collect();
        let import_names: Vec<String> = imports.iter().map(|&h| name_of(h)).collect();
        let export_entries: Vec<lmod::modinfo::ExportEntry> = exports
            .iter()
            .zip(&export_names)
            .map(|(&h, name)| lmod::modinfo::ExportEntry {
                sym_hash: h,
                name: name.as_bytes(),
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            })
            .collect();
        let import_entries: Vec<lmod::modinfo::ImportEntry> = imports
            .iter()
            .zip(&import_names)
            .map(|(&h, name)| lmod::modinfo::ImportEntry {
                sym_hash: h,
                name: name.as_bytes(),
            })
            .collect();
        let mut minfo = vec![0u8; 1024];
        let written = lmod::modinfo::encode_into(
            &mut minfo,
            module,
            &export_entries,
            &import_entries,
            0,
            0u16,
            &[],
            0u64,
            &[],
        )
        .expect("encode modinfo");
        minfo.truncate(written as usize);

        // Symbol table: null entry + one entry per undefined w_ symbol.
        let mut symtab = vec![0u8; 24];
        let mut strtab = vec![0u8];
        for &h in undefined_syms {
            let name = name_of(h);
            let st_name = strtab.len() as u32;
            strtab.extend_from_slice(name.as_bytes());
            strtab.push(0);
            let mut sym = [0u8; 24];
            sym[0..4].copy_from_slice(&st_name.to_le_bytes());
            // st_shndx stays 0 (SHN_UNDEF); st_info stays 0 (LOCAL/NOTYPE) —
            // the scan only reads name and shndx.
            symtab.extend_from_slice(&sym);
        }

        let sections: Vec<(&[u8], u32, &[u8], usize)> = vec![
            (b".lang.modinfo", 1, &minfo, 0),
            (b".symtab", 2, &symtab, 3), // sh_link -> .strtab section index
            (b".strtab", 3, &strtab, 0),
        ];
        minimal_elf64(&sections)
    }

    fn name_of(hash: u64) -> String {
        format!("w_{hash:016x}")
    }

    /// Assemble a minimal ELF64 with the given named sections, laid out
    /// after the section-header table.
    fn minimal_elf64(sections: &[(&[u8], u32, &[u8], usize)]) -> Vec<u8> {
        let shentsz = 64usize;
        let shnum = sections.len() + 1; // + null section
        let shstrndx = shnum; // .shstrtab appended last

        let mut shstr = vec![0u8];
        let mut names = Vec::new();
        for (name, _, _, _) in sections {
            names.push(shstr.len() as u32);
            shstr.extend_from_slice(name);
            shstr.push(0);
        }
        let shtab_name_off = shstr.len() as u32;
        shstr.extend_from_slice(b".shstrtab");
        shstr.push(0);

        let shoff = 64usize;
        let shtab_size = shnum * shentsz + shentsz; // + .shstrtab's own header
        let mut off = shoff + shtab_size;
        let mut placed: Vec<(usize, usize, usize)> = Vec::new(); // (offset, size, link)
        for (_, _, content, _) in sections {
            let pad = (4 - off % 4) % 4;
            off += pad;
            placed.push((off, content.len(), 0));
            off += content.len();
        }
        let shstr_off = off;
        let data_len = shstr_off + shstr.len();

        let mut elf = vec![0u8; data_len];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2; // ELF64
        elf[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        elf[0x3a..0x3c].copy_from_slice(&(shentsz as u16).to_le_bytes());
        elf[0x3c..0x3e].copy_from_slice(&((shnum + 1) as u16).to_le_bytes());
        elf[0x3e..0x40].copy_from_slice(&(shstrndx as u16).to_le_bytes());

        let hdr = |i: usize| shoff + i * shentsz;
        // .shstrtab header (last).
        let s = hdr(shnum);
        elf[s + 0..s + 4].copy_from_slice(&shtab_name_off.to_le_bytes());
        elf[s + 0x18..s + 0x20].copy_from_slice(&(shstr_off as u64).to_le_bytes());
        elf[s + 0x20..s + 0x28].copy_from_slice(&(shstr.len() as u64).to_le_bytes());

        for (i, ((_name, sec_type, content, link), (soff, ssize, _))) in
            sections.iter().zip(&placed).enumerate()
        {
            let s = hdr(i + 1); // index 0 is the null section
            elf[s..s + 4].copy_from_slice(&names[i].to_le_bytes());
            elf[s + 4..s + 8].copy_from_slice(&sec_type.to_le_bytes());
            elf[s + 0x18..s + 0x20].copy_from_slice(&(*soff as u64).to_le_bytes());
            elf[s + 0x20..s + 0x28].copy_from_slice(&(*ssize as u64).to_le_bytes());
            elf[s + 40..s + 44].copy_from_slice(&(*link as u32).to_le_bytes());
            elf[*soff..*soff + *ssize].copy_from_slice(content);
        }
        elf[shstr_off..shstr_off + shstr.len()].copy_from_slice(&shstr);
        elf
    }

    fn run(objs: &[(&str, Vec<u8>)]) -> Result<(), TyuError> {
        // Tests run in parallel in one process: give every call its own
        // directory or the obj_N.o names collide.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("tyu_provenance_{}_{seq}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut paths = Vec::new();
        for (i, (_module, data)) in objs.iter().enumerate() {
            let p = dir.join(format!("obj_{i}.o"));
            fs::write(&p, data).unwrap();
            paths.push(p);
        }
        let result = verify_link_provenance(&paths);
        for p in &paths {
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_dir(&dir);
        result
    }

    #[test]
    fn declared_cross_module_import_passes() {
        // The runner→fixture shape: TestRunner imports w_x, Fixture exports it.
        let a = object_elf(b"TestRunner", &[HASH_Y], &[HASH_X], &[HASH_X]);
        let b = object_elf(b"Fixture", &[HASH_X], &[], &[]);
        run(&[("runner", a), ("fixture", b)]).expect("legitimate import binds cleanly");
    }

    #[test]
    fn duplicate_export_is_rejected_with_both_modules() {
        let a = object_elf(b"ModuleA", &[HASH_X], &[], &[]);
        let b = object_elf(b"ModuleB", &[HASH_X], &[], &[]);
        let err = run(&[("a", a), ("b", b)]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate word export"), "{msg}");
        assert!(msg.contains("ModuleA"), "{msg}");
        assert!(msg.contains("ModuleB"), "{msg}");
    }

    #[test]
    fn undeclared_reference_is_rejected() {
        // References w_x but declares neither the import nor the export.
        let a = object_elf(b"Stale", &[], &[], &[HASH_X]);
        let b = object_elf(b"Other", &[HASH_X], &[], &[]);
        let err = run(&[("stale", a), ("other", b)]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undeclared word reference"), "{msg}");
        assert!(msg.contains("Stale"), "{msg}");
    }

    #[test]
    fn runtime_provided_symbol_passes() {
        // Declared import with no attributed definer in the set: the symbol
        // comes from the runtime asm, which carries no modinfo.
        let a = object_elf(b"Module", &[], &[HASH_X], &[HASH_X]);
        run(&[("m", a)]).expect("runtime-provided symbols are unattributed");
    }

    #[test]
    fn unattributed_objects_are_skipped() {
        // No .lang.modinfo: the object never participates, even with
        // undefined references.
        let bare = object_elf(b"", &[], &[], &[HASH_X]);
        let stripped = {
            // Drop the modinfo section by rebuilding with symtab only.
            let mut symtab = vec![0u8; 24];
            let mut strtab = vec![0u8];
            strtab.extend_from_slice(b"w_1111111111111111");
            strtab.push(0);
            let mut sym = [0u8; 24];
            sym[0..4].copy_from_slice(&1u32.to_le_bytes());
            symtab.extend_from_slice(&sym);
            minimal_elf64(&[
                (b".symtab".as_slice(), 2, symtab.as_slice(), 2usize),
                (b".strtab".as_slice(), 3, strtab.as_slice(), 0usize),
            ])
        };
        let _ = bare;
        run(&[("bare", stripped)]).expect("objects without modinfo are unattributed");
    }

}

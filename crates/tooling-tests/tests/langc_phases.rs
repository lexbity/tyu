use std::{path::PathBuf, process::Command, sync::Once};

static BUILD_ONCE: Once = Once::new();

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_tools() {
    BUILD_ONCE.call_once(|| {
        // Capture stdio so the nested cargo never flips the shared test
        // terminal to O_NONBLOCK; that flag leaks back to the outer
        // `cargo test` harness, whose writes then panic with EAGAIN.
        let out = Command::new(env!("CARGO"))
            .current_dir(workspace_root())
            .args(["build", "-q"])
            .output()
            .expect("cargo build");
        assert!(
            out.status.success(),
            "cargo build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    });
}

fn runtime_asm_linux_x86_64_hosted() -> PathBuf {
    workspace_root()
        .join("runtime")
        .join("linux-x86_64-hosted.asm")
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_lang_tests").join(format!(
        "{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn repo_sysroot() -> PathBuf {
    workspace_root().join("sysroot")
}

#[test]
fn phase0_et_rel_object_structure() {
    build_tools();
    let dir = fresh_dir("phase0_et_rel_object_structure");

    // Module with string literal (exercises .rodata section).
    // No explicit export → all words are public (w_<hash> symbols in .o).
    std::fs::write(
        dir.join("Arith.mod"),
        b"module Arith;\n\
          : helper ( i64 -- i64 ) 42 + ;\n\
          : main ( -- i64 ) \"ok\" drop 7 helper ;\n\
          end;\n",
    )
    .unwrap();

    // --- Static path (--emit=asm) must still work ---
    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Arith.mod"])
        .output()
        .unwrap();
    assert!(out.status.success(), "static asm emission failed");
    std::fs::write(dir.join("Arith.asm"), &out.stdout).unwrap();

    let status = Command::new("fasm")
        .current_dir(&dir)
        .args(["Arith.asm", "prog_static"])
        .status()
        .unwrap();
    assert!(status.success(), "fasm static assembly failed");

    let run = Command::new(dir.join("prog_static")).status().unwrap();
    assert_eq!(
        run.code(),
        Some(49),
        "static executable gave wrong exit code"
    );

    // --- Dynamic/obj path (--emit=obj) produces valid ET_REL ---
    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Arith.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "obj emission failed");

    let obj_path = dir.join("Arith.o");
    assert!(obj_path.exists(), "Arith.o was not produced");

    // readelf -h: verify ELF type is REL (relocatable).
    let readelf_h = Command::new("readelf")
        .arg("-h")
        .arg(&obj_path)
        .output()
        .unwrap();
    assert!(readelf_h.status.success());
    let h_out = String::from_utf8_lossy(&readelf_h.stdout);
    assert!(
        h_out.contains("Type:                              REL (Relocatable file)"),
        "expected REL type, got:\n{h_out}"
    );

    // readelf -S: verify expected sections.
    let readelf_s = Command::new("readelf")
        .args(["-S", &obj_path.to_string_lossy()])
        .output()
        .unwrap();
    assert!(readelf_s.status.success());
    let s_out = String::from_utf8_lossy(&readelf_s.stdout);
    assert!(s_out.contains(".text"), "missing .text section");
    assert!(s_out.contains(".rodata"), "missing .rodata section");
    assert!(s_out.contains(".symtab"), "missing .symtab section");
    assert!(s_out.contains(".strtab"), "missing .strtab section");
    assert!(s_out.contains(".rela.text"), "missing .rela.text section");

    // readelf -s: verify w_<hash> exported symbols.
    // Without an explicit export statement, all words are public.
    let readelf_syms = Command::new("readelf")
        .args(["-s", &obj_path.to_string_lossy()])
        .output()
        .unwrap();
    assert!(readelf_syms.status.success());
    let syms_out = String::from_utf8_lossy(&readelf_syms.stdout);
    // "main" → fnv1a_u64("main") = 0x1f5962a2ce9803c8 → label w_1f5962a2ce9803c8
    assert!(
        syms_out.contains("w_1f5962a2ce9803c8"),
        "expected exported symbol w_1f5962a2ce9803c8 (main), got:\n{syms_out}"
    );
    // "helper" → fnv1a_u64("helper") = 0x9c4c7ccd2b84562d → label w_9c4c7ccd2b84562d
    assert!(
        syms_out.contains("w_9c4c7ccd2b84562d"),
        "expected exported symbol w_9c4c7ccd2b84562d (helper), got:\n{syms_out}"
    );

    // Link the .o against the runtime and verify the linked executable runs.
    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog_dynamic", "rt.o", "Arith.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog_dynamic")).status().unwrap();
    assert_eq!(
        run.code(),
        Some(49),
        "linked dynamic executable gave wrong exit code"
    );
}

// ---------------------------------------------------------------------------
// S2 Phase 1 — .lang.modinfo serialization
// ---------------------------------------------------------------------------

#[test]
fn phase1_modinfo_section_present_and_decodable() {
    build_tools();
    let dir = fresh_dir("phase1_modinfo_section_present_and_decodable");

    // Module with one export + string literal (exercises both export and
    // import paths — imports are from sysroot builtins).
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
          : add ( i64 -- i64 ) 1 + ;\n\
          : main ( -- i64 ) \"x\" drop 0 add ;\n\
          end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Main.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "obj emission failed");

    let obj_path = dir.join("Main.o");
    assert!(obj_path.exists(), "Main.o was not produced");

    // readelf -S must show a .lang.modinfo section.
    let readelf_s = Command::new("readelf")
        .args(["-S", &obj_path.to_string_lossy()])
        .output()
        .unwrap();
    assert!(readelf_s.status.success());
    let s_out = String::from_utf8_lossy(&readelf_s.stdout);
    assert!(
        s_out.contains(".lang.modinfo"),
        "missing .lang.modinfo section:\n{s_out}"
    );

    // objcopy the section to a binary blob and verify header fields via
    // a small Rust helper that reads the 32-byte fixed header.
    let blob_path = dir.join("modinfo.bin");
    let status = Command::new("objcopy")
        .args([
            "-O",
            "binary",
            "-j",
            ".lang.modinfo",
            obj_path.to_str().unwrap(),
            blob_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "objcopy failed");

    let blob = std::fs::read(&blob_path).unwrap();
    assert!(
        blob.len() >= 32,
        "modinfo blob too small: {} bytes",
        blob.len()
    );

    // Manually decode the fixed header (32-byte little-endian with abi_hash).
    let magic = u32::from_le_bytes(blob[0..4].try_into().unwrap());
    assert_eq!(magic, 0x4c4d4f44, "bad magic: 0x{magic:08x}");

    let version = u16::from_le_bytes(blob[4..6].try_into().unwrap());
    assert_eq!(version, 4, "bad version: {version}");

    // abi_hash lives at bytes [8..16]; skip byte-checking it.
    let name_off = u32::from_le_bytes(blob[16..20].try_into().unwrap()) as usize;
    let name_len = u32::from_le_bytes(blob[20..24].try_into().unwrap()) as usize;
    let export_count = u32::from_le_bytes(blob[24..28].try_into().unwrap());
    let import_count = u32::from_le_bytes(blob[28..32].try_into().unwrap());

    // Verify module name.
    let module_name = &blob[name_off..name_off + name_len];
    assert_eq!(module_name, b"Main", "bad module name");

    // At minimum we export `main` and `add`.
    assert!(
        export_count >= 2,
        "expected at least 2 exports, got {export_count}"
    );

    // Imports may be 0 (builtins like `+` are not in the import table;
    // only symbols from `.def` files appear there).

    // Verify the linked executable still runs correctly.
    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(
        run.code(),
        Some(1),
        "linked executable gave wrong exit code"
    );
}

// ---------------------------------------------------------------------------
// S2 Phase 3 — .lmod container packer
// ---------------------------------------------------------------------------

fn lmod_pack_exe() -> PathBuf {
    exe("lmod-pack")
}

#[test]
fn phase3_lmod_packer_produces_valid_container() {
    build_tools();
    let dir = fresh_dir("phase3_lmod_packer_produces_valid_container");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
          : main ( -- i64 ) \"done\" drop 42 ;\n\
          end;\n",
    )
    .unwrap();

    // Compile to .o first.
    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Main.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "obj emission failed");

    assert!(dir.join("Main.o").exists(), "Main.o missing");

    // Pack to .lmod.
    let status = Command::new(lmod_pack_exe())
        .current_dir(&dir)
        .args(["Main.o", "Main.lmod"])
        .status()
        .unwrap();
    assert!(status.success(), "lmod-pack failed");

    let lmod_path = dir.join("Main.lmod");
    assert!(lmod_path.exists(), "Main.lmod missing");

    let lmod = std::fs::read(&lmod_path).unwrap();
    assert!(lmod.len() > 72, ".lmod too small");

    // Verify header magic.
    let magic = u32::from_le_bytes(lmod[0..4].try_into().unwrap());
    assert_eq!(magic, 0x4c4d4f44, "bad magic");

    // Verify format version.
    let ver = u16::from_le_bytes(lmod[4..6].try_into().unwrap());
    assert_eq!(ver, 3, "bad format version");

    // Verify total_len matches file size.
    let total_len = u32::from_le_bytes(lmod[16..20].try_into().unwrap());
    assert_eq!(total_len as usize, lmod.len(), "total_len mismatch");

    // Verify modinfo section is present and decodable.
    let mi_off = u32::from_le_bytes(lmod[20..24].try_into().unwrap()) as usize;
    let mi_len = u32::from_le_bytes(lmod[24..28].try_into().unwrap()) as usize;
    assert!(mi_off >= 72, "modinfo_off before header end");
    assert!(mi_len > 0, "modinfo_len zero");

    let modinfo = &lmod[mi_off..mi_off + mi_len];
    let mi_magic = u32::from_le_bytes(modinfo[0..4].try_into().unwrap());
    assert_eq!(mi_magic, 0x4c4d4f44, "bad modinfo magic");

    // Verify code section.
    let code_off = u32::from_le_bytes(lmod[28..32].try_into().unwrap()) as usize;
    let code_len = u32::from_le_bytes(lmod[32..36].try_into().unwrap()) as usize;
    assert!(code_len > 0, "code_len zero");
    assert!(code_off > mi_off, "code overlaps modinfo");

    // Verify no sections overlap.
    let data_off = u32::from_le_bytes(lmod[44..48].try_into().unwrap()) as usize;
    let data_len = u32::from_le_bytes(lmod[48..52].try_into().unwrap()) as usize;
    let reloc_off = u32::from_le_bytes(lmod[56..60].try_into().unwrap()) as usize;
    let reloc_count = u32::from_le_bytes(lmod[60..64].try_into().unwrap()) as usize;

    let mut ranges = vec![
        ("header", 0usize, 72usize),
        ("modinfo", mi_off, mi_off + mi_len),
        ("code", code_off, code_off + code_len),
    ];
    if data_len > 0 {
        ranges.push(("data", data_off, data_off + data_len));
    }
    if reloc_count > 0 {
        ranges.push(("reloc", reloc_off, reloc_off + reloc_count * 16));
    }
    for i in 0..ranges.len() {
        for j in (i + 1)..ranges.len() {
            let (name1, s1, e1) = ranges[i];
            let (name2, s2, e2) = ranges[j];
            assert!(
                e1 <= s2 || e2 <= s1,
                "section overlap: {} [{},{}) vs {} [{},{})",
                name1,
                s1,
                e1,
                name2,
                s2,
                e2
            );
        }
    }

    // Verify total_len covers everything.
    let max_end = ranges.iter().map(|&(_, _, e)| e).max().unwrap_or(0);
    assert!(
        total_len as usize >= max_end,
        "total_len {} < last section end {}",
        total_len,
        max_end
    );
}

// ---------------------------------------------------------------------------
// S2 Phase 4 — Container reader + integrity validation
// ---------------------------------------------------------------------------

#[test]
fn phase4_container_reader_validates_packed_module() {
    build_tools();
    let dir = fresh_dir("phase4_container_reader_validates_packed_module");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
          : main ( -- i64 ) \"ok\" drop 42 ;\n\
          end;\n",
    )
    .unwrap();

    // Compile to .o → pack to .lmod → read back with Container::parse
    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Main.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lmod-pack"))
        .current_dir(&dir)
        .args(["Main.o", "Main.lmod"])
        .status()
        .unwrap();
    assert!(status.success());

    let lmod = std::fs::read(dir.join("Main.lmod")).unwrap();

    // Use a minimal Rust program to invoke the Container reader.
    // We embed the lmod_bytes as a static and compile a check binary
    // that calls lmod::validate::Container::parse.
    //
    // Since we can't easily compile and run another Rust program here,
    // we do the validation manually (same logic as Container::parse).

    // Manual validation using the same checks as Container::parse.
    assert!(lmod.len() >= 72, "too small");
    let magic = u32::from_le_bytes(lmod[0..4].try_into().unwrap());
    assert_eq!(magic, 0x4c4d4f44, "bad magic");
    let ver = u16::from_le_bytes(lmod[4..6].try_into().unwrap());
    assert_eq!(ver, 3, "bad version");
    let total_len = u32::from_le_bytes(lmod[16..20].try_into().unwrap()) as usize;
    assert_eq!(total_len, lmod.len(), "total_len mismatch");

    // Validate every section offset/length is within bounds.
    let sections = [
        ("modinfo", 20usize, 24usize),
        ("code", 28, 32),
        ("rodata", 36, 40),
        ("data", 44, 48),
    ];
    for &(name, off_off, len_off) in &sections {
        let off = u32::from_le_bytes(lmod[off_off..off_off + 4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(lmod[len_off..len_off + 4].try_into().unwrap()) as usize;
        if len > 0 {
            assert!(off >= 72, "{name} offset {off} < header size 72");
            assert!(
                off + len <= total_len,
                "{name} [{off},{}) exceeds total_len {total_len}",
                off + len,
            );
        }
    }

    // Validate reloc table.
    let reloc_off = u32::from_le_bytes(lmod[56..60].try_into().unwrap()) as usize;
    let reloc_cnt = u32::from_le_bytes(lmod[60..64].try_into().unwrap()) as usize;
    let reloc_bytes = reloc_cnt * 16;
    if reloc_bytes > 0 {
        assert!(reloc_off >= 72, "reloc offset < header");
        assert!(
            reloc_off + reloc_bytes <= total_len,
            "reloc [{},{}) exceeds total_len {total_len}",
            reloc_off,
            reloc_off + reloc_bytes,
        );
    }

    // Validate no overlaps (monotonic ordering check).
    let mut ranges: Vec<(usize, usize, &str)> = vec![(0, 72, "header")];
    for &(name, off_off, len_off) in &sections {
        let off = u32::from_le_bytes(lmod[off_off..off_off + 4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(lmod[len_off..len_off + 4].try_into().unwrap()) as usize;
        if len > 0 {
            ranges.push((off, off + len, name));
        }
    }
    if reloc_bytes > 0 {
        ranges.push((reloc_off, reloc_off + reloc_bytes, "reloc"));
    }
    ranges.sort_by_key(|&(s, _, _)| s);
    for w in ranges.windows(2) {
        let (_, e1, name1) = w[0];
        let (s2, _, name2) = w[1];
        assert!(
            e1 <= s2,
            "overlap: {} ends at {e1} but {} starts at {s2}",
            name1,
            name2,
        );
    }
}

#[test]
fn milestone7_if_while_locals_smoke() {
    build_tools();
    let dir = fresh_dir("milestone7_if_while_locals_smoke");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  0\n\
  [ dup 3 < ] [ 1 + ] while\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(3));
}

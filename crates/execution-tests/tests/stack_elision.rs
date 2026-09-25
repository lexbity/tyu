//! Slice P7 — stack-guard elision, behavioral (static-verification.md §4
//! Q5/FR-11, §12 AC-8).
//!
//! The x86_64 per-push data-stack overflow guard (C8) is elidable only for
//! an image whose `stack-budget(main)` obligation is discharged: `high(main)`
//! finite and ≤ the *derived* `N_main` (from the runtime binary's own DS
//! geometry — bounds are computed, never hand-declared). The `__lang_ds_high`
//! high-water observability update is NOT elided — the harness's
//! `measured ≤ declared` channel runs against elided builds too (§10).
//!
//! - Fixture A-level codegen: the same module with/without `--elide-ds-guards`
//!   — guards gone, high-water kept, text strictly smaller (NFR-6).
//! - The *image* story: `tyu build --elide-stack-guards` on the geometry-
//!   exporting metal runtime (static) produces an elided image with no
//!   `__stack_overflow` references in the module object, still runs correctly
//!   (the §10 falsification channel), and a non-tail-recursive `main`
//!   (`high = ⊤`) refuses elision.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

fn langc_dir(extra: &[&str], source: &str) -> PathBuf {
    let dir = common::temp_dir(if extra.is_empty() { "s7_guarded" } else { "s7_elided" });
    let mod_path = dir.join("M.mod");
    std::fs::write(&mod_path, source).unwrap();
    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        "--target=x86_64-unknown-linux-gnu".into(),
        format!("--out-dir={}", dir.display()),
        mod_path.to_str().unwrap().into(),
    ];
    for a in extra {
        args.push(a.to_string());
    }
    let status = Command::new(common::langc_exe())
        .args(&args)
        .status()
        .expect("langc invocation");
    assert!(status.success(), "langc failed");
    dir
}

const FINITE_MOD: &str = "\
module M;
: main ( -- i64 )
  1 2 + 3 + 4 + 5 + 6 + ;
export { main } ;
end;
";

#[test]
fn elided_codegen_omits_guards_keeps_high_water_and_is_smaller() {
    build_langc();
    let guarded = langc_dir(&[], FINITE_MOD);
    let elided = langc_dir(&["--elide-ds-guards"], FINITE_MOD);

    let g_asm = std::fs::read_to_string(guarded.join("M.asm")).unwrap();
    let e_asm = std::fs::read_to_string(elided.join("M.asm")).unwrap();

    // C8 guards present in the guarded build, absent in the elided one.
    assert!(
        g_asm.matches("ja __stack_overflow").count() > 0,
        "guarded build must carry per-push guards"
    );
    assert!(
        !e_asm.contains("ja __stack_overflow"),
        "elided build must carry no per-push guards:\n{e_asm}"
    );
    // The __lang_ds_high update is observability, not a check — kept.
    assert!(
        e_asm.contains("cmp r15, [__lang_ds_high]"),
        "high-water observability must survive elision"
    );
    // NFR-6: the elided build is strictly smaller (guards are real bytes).
    assert!(
        e_asm.len() < g_asm.len(),
        "elided assembly must be strictly smaller: {} vs {}",
        e_asm.len(),
        g_asm.len()
    );
    // Text section gate on the objects (matches the plan's "text section
    // strictly smaller" wording at the shape level).
    let g_text = text_section_size(&guarded);
    let e_text = text_section_size(&elided);
    assert!(
        e_text < g_text,
        "elided .text must be strictly smaller: {e_text} vs {g_text}"
    );
    let _ = std::fs::remove_dir_all(&guarded);
    let _ = std::fs::remove_dir_all(&elided);
}

fn text_section_size(dir: &Path) -> u64 {
    let obj = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .expect("object present");
    let out = Command::new("objdump")
        .args(["-h", obj.to_str().unwrap()])
        .output()
        .expect("objdump");
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // `objdump -h` layouts vary (some print an Idx column): find the line
        // whose tokens contain `.text`; the size is the token after it.
        let pos = parts.iter().position(|t| *t == ".text");
        if let Some(pos) = pos {
            if let Some(size) = parts.get(pos + 1) {
                let v = size.trim_start_matches("0x");
                if let Ok(n) = u64::from_str_radix(v, 16) {
                    return n;
                }
            }
        }
    }
    panic!("no .text section found:\n{text}")
}

// ---------------------------------------------------------------------------
// The image story: `tyu build --elide-stack-guards` on the metal runtime.
// ---------------------------------------------------------------------------

fn tyu_metal_build(tag: &str, source: &str, elide: bool) -> std::path::PathBuf {
    build_langc();
    let dir = common::temp_dir(tag);
    let mod_path = dir.join("M.mod");
    std::fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(common::tyu_exe());
    cmd.arg("build")
        .arg("--target=x86_64-unknown-none")
        .arg("--platform=x86_64-unknown-none")
        .arg("--mode=static")
        .arg(format!("--sysroot={}", common::sysroot_dir().display()))
        .arg(format!("--out-dir={}", out_dir.display()));
    if elide {
        cmd.arg("--elide-stack-guards");
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().expect("tyu invocation");
    assert!(
        out.status.success(),
        "tyu build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = out;
    out_dir
}

const DONE_FIXTURE: &str = "\
module M;
import platform/testio { testio.write-byte };

: done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;

: main ( -- i64 )
  1 2 + done 3 + ;
export { main } ;
end;
";

#[test]
fn elided_image_runs_correctly_under_qemu() {
    if !common::require_tools(&["langc", "tyu", "fasm", "ld", "qemu-system-x86_64", "objdump"]) {
        return;
    }
    // Gate: the geometry requirement — the metal runtime exports the DS
    // symbols (N_main derived), so the elided image must be produced.
    let out_elided = tyu_metal_build("s7_elided_image", DONE_FIXTURE, true);

    // The module object carries no `__stack_overflow` references.
    let module_obj: PathBuf = std::fs::read_dir(&out_elided)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.starts_with("M-") && n.ends_with(".o")
        })
        .expect("module object");
    let dis = Command::new("objdump")
        .args(["-dr", module_obj.to_str().unwrap()])
        .output()
        .expect("objdump");
    let dis = String::from_utf8_lossy(&dis.stdout);
    assert!(
        !dis.contains("__stack_overflow"),
        "elided module object must carry no __stack_overflow:\n{dis}"
    );

    // The image runs correctly: completes with the `S\n` marker (the §10
    // falsification channel — the harness's `measured ≤ declared` check runs
    // against elided builds too, because __lang_ds_high tracking is kept).
    let image = out_elided.join("image.elf");
    let outcome = common::run_with_product_runner(
        codegen_core::Target::X86_64UnknownNone,
        &image,
        Duration::from_secs(8),
    );
    assert!(
        !outcome.timed_out,
        "elided image must complete, not hang (stdout: {:02x?})",
        &outcome.stdout
    );
    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(
        summary.completed,
        "elided image must emit the completion marker (stdout: {:02x?})",
        &outcome.stdout
    );

    // The guarded build of the same source runs identically (control).
    let out_guarded = tyu_metal_build("s7_guarded_image", DONE_FIXTURE, false);
    let gimage = out_guarded.join("image.elf");
    let gin = common::run_with_product_runner(
        codegen_core::Target::X86_64UnknownNone,
        &gimage,
        Duration::from_secs(8),
    );
    assert!(!gin.timed_out, "guarded image must complete");
    let gsum = harness_core::parse_output(&gin.stdout);
    assert!(gsum.completed);
    let _ = std::fs::remove_dir_all(&out_elided);
    let _ = std::fs::remove_dir_all(&out_guarded);
}

#[test]
fn recursive_main_refuses_elision() {
    if !common::require_tools(&["langc", "tyu", "fasm", "ld"]) {
        return;
    }
    let rec: &str = "\
module M;
: main ( -- i64 )
  main ;
export { main } ;
end;
";
    let out = tyu_metal_build("s7_refused", rec, true);
    // The report records the refusal honestly: top = ⊤, verdict open, guards
    // retained (never a silent elision of an unproven image).
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(out.join("verify-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["contexts"]["stack"]["guards"], "retained");
    assert_eq!(report["contexts"]["stack"]["main"]["top"], true);
    assert_eq!(report["emitted_checks"]["data_stack_guards"], true);
    // The module object still carries its guard references.
    let module_obj: PathBuf = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.starts_with("M-") && n.ends_with(".o")
        })
        .expect("module object");
    let dis = Command::new("objdump")
        .args(["-dr", module_obj.to_str().unwrap()])
        .output()
        .expect("objdump");
    let dis = String::from_utf8_lossy(&dis.stdout);
    assert!(
        dis.contains("__stack_overflow"),
        "refused elision must keep the guards in the object"
    );
    let _ = std::fs::remove_dir_all(&out);
}
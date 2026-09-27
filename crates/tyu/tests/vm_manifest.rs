//! The `tyu.vm/1` producer (PLAN-VERIFY-3 P11.1): the verdicts-v2 →
//! manifest-summary converter is AUTOMATIC and its values are the real,
//! non-human-producible ones — the `id_hash` (fnv1a64 of the artifact) and
//! `statement_hash` (the canonical-encoder SHA-256) recompute from the
//! obligation artifact, never hand-authored constants.
//!
//! This test builds a module whose obligations carry real statement hashes,
//! then asserts the emitted `<Module>.vm.json` obligations match the
//! artifact's computed values exactly.

use std::process::Command;

use tyu::test_helpers::*;
use verifier::stmt::{sha256_hex16, StatementContext};

const OBL_MOD: &str = "\
module Bank;\nsubtype Percent = i64 range 0..100;\nsubtype Counter = i64 range 0..1000000;\n\
: clamp ( i64 -- Percent )\n  dup 100 > [ drop 100 ] [ ] if\n  dup 0 < [ drop 0 ] [ ] if\n  as Percent ;\n\
: bounded_inc ( Percent -- Percent ) 1 + as Percent ;\n\
: main ( -- Counter ) 50 as Percent bounded_inc as Counter ;\nend;\n";

fn ensure_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn vm_summary_derives_real_hashes() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }
    ensure_tools();
    let dir = temp_dir("vm_summary");
    let main_mod = dir.join("Bank.mod");
    std::fs::write(&main_mod, OBL_MOD).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let build = Command::new(tyu_exe())
        .current_dir(&dir)
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
        ])
        .arg(&main_mod)
        .output()
        .expect("tyu build");
    assert!(
        build.status.success(),
        "tyu build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    // The derived summary exists and its obligations carry REAL hashes.
    let vm_path = out_dir.join(".tyu-verify/Bank.vm.json");
    let vm_text = std::fs::read_to_string(&vm_path).expect("Bank.vm.json emitted");
    let artifacts: Vec<_> = std::fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with(".obl.json"))
        })
        .map(|e| e.path())
        .collect();
    assert!(
        !artifacts.is_empty(),
        "the build must emit obligation artifacts"
    );
    let bytes = std::fs::read(&artifacts[0]).unwrap();
    let set = verifier::codec::read_obl(&bytes).expect("artifact parses");
    assert!(
        !set.obligations.is_empty(),
        "the fixture must have obligations"
    );

    for o in &set.obligations {
        let word_ir_hash = set
            .facts
            .words
            .iter()
            .find(|w| w.name == o.site.word)
            .map(|w| sha256_hex16(w.ir.as_bytes()))
            .unwrap_or_default();
        let ctx = StatementContext::for_obligation(
            &set.module,
            &set.target,
            &set.model_semantics,
            &word_ir_hash,
            o,
        );
        let expected = ctx.statement_hash_hex(&o.formula);
        let idh = u64::from_str_radix(&o.id_hash, 16).unwrap_or(0);
        // The summary must carry the exact statement_hash for this obligation's id.
        assert!(
            vm_text.contains(&format!("\"id\":\"{}\"", o.id)),
            "summary must contain the obligation id:\n{vm_text}"
        );
        // The statement_hash must appear as the REAL 64-hex (never a
        // hand-authored placeholder).
        assert!(
            vm_text.contains(&format!("\"statement_hash\":\"{}\"", expected)),
            "summary statement_hash must equal the canonical encoder's for {}:\n{vm_text}",
            o.id
        );
        assert!(
            vm_text.contains(&format!("\"id_hash\":{}", idh)),
            "summary id_hash must be the artifact's numeric fnv1a64 for {}:\n{vm_text}",
            o.id
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

//! `tyu proof fill <input.mod>` end to end (PLAN-VERIFY-3 P10.2): the
//! standalone extraction form — passing the entry `.mod` runs the
//! `langc --emit=obligations` graph extraction (no prior build needed), the
//! port's `gen` renderer, and the `fill` exe, writing marker-headed
//! candidates under `proofs/candidates/`. Tier-A: needs the Lean toolchain +
//! langc (`TYU_PROOF_E2E=1` forces a loud failure when absent; `ci/port.sh`
//! runs it with the pinned toolchain).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn toolchain_present() -> bool {
    let lean = Command::new("lean").arg("--version").output();
    let lake = Command::new("lake").arg("--version").output();
    match (lean, lake) {
        (Ok(l), Ok(k)) => l.status.success() && k.status.success(),
        _ => false,
    }
}

fn fresh_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_proof_fill_e2e")
        .join(format!("{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn proof_fill_standalone_input_mod_writes_candidates() {
    if !toolchain_present() {
        if std::env::var("TYU_PROOF_E2E").is_ok() {
            panic!("TYU_PROOF_E2E=1 requires `lean`/`lake` and the port built");
        }
        eprintln!("skipping the proof-fill e2e (no Lean toolchain; ci/port.sh runs it)");
        return;
    }
    let root = workspace_root();
    // The port exes must be built (the gate does `lake build automation_rate
    // fill`).
    let fill_bin = root.join("verification/ports/lean/.lake/build/bin/fill");
    assert!(
        fill_bin.is_file(),
        "the port's fill exe must be built (lake build fill) before this test"
    );
    let langc = root.join("target/debug/langc");
    assert!(
        langc.is_file(),
        "langc must be built (cargo build -p langc)"
    );
    let tyu = root.join("target/debug/tyu");

    let dir = fresh_dir();
    fs::copy(
        root.join("verification/ports/lean/goldens/obl-src/bank.mod"),
        dir.join("Bank.mod"),
    )
    .unwrap();

    let out = Command::new(&tyu)
        .current_dir(&dir)
        .args([
            "proof",
            "fill",
            "--dir=.",
            "--target=x86_64-unknown-none",
            "Bank.mod",
        ])
        .output()
        .expect("tyu proof fill");
    assert!(
        out.status.success(),
        "proof fill (standalone input) failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Candidates land under proofs/candidates/ with the marker.
    let cand_dir = dir.join("proofs/candidates");
    let files: Vec<String> = fs::read_dir(&cand_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".lean"))
        .collect();
    assert!(
        files.len() >= 2,
        "expected candidate files for the Bank obligations: {files:?}"
    );
    let sample = fs::read_to_string(cand_dir.join(&files[0])).unwrap();
    assert!(
        sample.contains("-- tyu:candidate obligation=Bank::"),
        "candidates must carry the attribution marker:\n{sample}"
    );
    assert!(
        sample.contains("import Gen.Bank"),
        "candidates import the generated statements:\n{sample}"
    );

    let _ = fs::remove_dir_all(&dir);
}

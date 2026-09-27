//! `tyu proof init` (PLAN-VERIFY-3 P6.1, FR-6): scaffolds the developer-owned
//! `proofs/` directory — the root import, a per-module template for each
//! module of the entry graph, a README, and `.gitignore` entries. Idempotent:
//! re-runs never clobber developer files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-proof-init-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const BANK_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;

: main ( -- i64 )
  50 as Percent as i64
;
end;
";

fn run_init(dir: &Path, args: &[&str]) -> bool {
    let status = Command::new(tyu_exe())
        .current_dir(dir)
        .arg("proof")
        .arg("init")
        .arg("--dir=.")
        .args(args)
        .status()
        .expect("tyu proof init");
    status.success()
}

#[test]
fn proof_init_scaffolds_the_developer_surface() {
    let dir = fresh_dir("scaffold");
    fs::write(dir.join("Bank.mod"), BANK_MOD).unwrap();
    assert!(run_init(&dir, &["Bank.mod"]), "proof init must succeed");

    let root = dir.join("proofs").join("proofs.lean");
    let root_text = fs::read_to_string(&root).unwrap();
    // Imports lead the file (Lean forbids commands before imports) and
    // enumerate the entry graph's modules.
    assert!(
        root_text.starts_with("import Bank\n"),
        "root must import the module first: {root_text}"
    );
    assert!(root_text.contains("Developer proofs root"));

    let bank = dir.join("proofs").join("Bank.lean");
    let bank_text = fs::read_to_string(&bank).unwrap();
    assert!(
        bank_text.starts_with("import Gen.Bank\n"),
        "per-module template must import the generated statements: {bank_text}"
    );
    assert!(bank_text.contains("Tyu.Gen.Corpus.Bank"));

    assert!(dir.join("proofs").join("README.md").is_file());
    let ignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.contains(".tyu-verify/"), "gitignore: {ignore}");
    assert!(ignore.contains("target/"), "gitignore: {ignore}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn proof_init_is_idempotent_and_never_clobbers() {
    let dir = fresh_dir("idem");
    fs::write(dir.join("Bank.mod"), BANK_MOD).unwrap();
    assert!(run_init(&dir, &["Bank.mod"]));
    let bank = dir.join("proofs").join("Bank.lean");
    // The developer edits their proof file; a re-init must not touch it.
    fs::write(
        &bank,
        "import Gen.Bank\n\ntheorem mine : True := by trivial\n",
    )
    .unwrap();
    assert!(run_init(&dir, &["Bank.mod"]));
    assert_eq!(
        fs::read_to_string(&bank).unwrap(),
        "import Gen.Bank\n\ntheorem mine : True := by trivial\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn proof_init_without_input_scaffolds_the_bare_root() {
    let dir = fresh_dir("bare");
    assert!(run_init(&dir, &[]));
    assert!(dir.join("proofs").join("proofs.lean").is_file());
    assert!(dir.join("proofs").join("README.md").is_file());
    // No modules → the root carries no imports.
    let root = fs::read_to_string(dir.join("proofs").join("proofs.lean")).unwrap();
    assert!(
        !root.starts_with("import "),
        "bare root must not import anything: {root}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn proof_fill_fails_closed_without_extracted_artifacts() {
    let dir = fresh_dir("fill");
    // Without a prior `tyu build --verify-tool=lean` extraction (and, in the
    // hermetic tier, possibly without the port toolchain), `proof fill` must
    // fail closed with the honest E6416 that names the missing artifact
    // source — never a fabricated candidate.
    let out = Command::new(tyu_exe())
        .current_dir(&dir)
        .args(["proof", "fill"])
        .output()
        .expect("tyu proof fill");
    assert!(
        !out.status.success(),
        "fill must fail closed without artifacts"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E6416"), "fail-closed code: {stderr}");
    let _ = fs::remove_dir_all(&dir);
}

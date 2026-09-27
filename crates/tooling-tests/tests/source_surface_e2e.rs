//! Source-surface worked example, end to end (PLAN-VERIFY-3 P9.2, §11
//! acceptance item 1's source-variant): the `Sum` fixture's
//! `subtype-range` obligation is PROVEN at the SOURCE surface and harvested
//! with `surface: "source"` + `relies: ["T-S"]` provenance (§Q2).
//!
//! The fixture (`verification/ports/lean/tests/source-fixture/`) is an
//! authored `tyu.obl/v2` artifact whose word IR is entirely fragment ops.
//! This test drives the REAL port drifts:
//!
//!   1. the port's `gen` renderer produces `Gen/Sum.lean` (with the
//!      `src_stmt_Sum_answer_subtype_range_0` source-surface statement) +
//!      `Sum.gen.json` (with the `src_def` row);
//!   2. a temp lake package assembles the fixture proof (`SumFix.lean` — the
//!      kernel-checked theorem of the SOURCE statement) with the port library;
//!   3. the harvest binds the certificate and emits `tyu.verdicts/v2`; the
//!      record MUST carry `surface: "source"` and `proof.relies: ["T-S"]`
//!      (fail-closed: a `surface: "ir"` or a missing reliance is a bug).
//!
//! The test is tier-A style: it needs a Lean toolchain. Without one it
//! SKIPS with a message; set `TYU_SOURCE_SURFACE_E2E=1` to force the run
//! (and fail loudly when the toolchain is absent). `ci/port.sh` runs it
//! unconditionally (the port gate always has the toolchain).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = "verification/ports/lean/tests/source-fixture";
const PORT: &str = "verification/ports/lean";

fn workspace_root() -> PathBuf {
    common::workspace_root()
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
        .join("tyu_source_e2e")
        .join(format!("{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Produce the generated statements via the port's `gen` renderer.
fn render_fixture(root: &Path, dir: &Path) {
    let gen_exe = root.join(PORT).join(".lake/build/bin/gen");
    let status = if gen_exe.is_file() {
        Command::new(&gen_exe)
            .current_dir(dir)
            .arg("--render")
            .arg("--obl")
            .arg(dir.join("Sum.obl.json"))
            .arg("--out")
            .arg(dir.join("Gen"))
            .status()
            .unwrap()
    } else {
        // Build the exe (the toolchain tier has lake).
        Command::new("lake")
            .current_dir(root.join(PORT))
            .args(["build", "gen"])
            .status()
            .expect("lake build gen");
        Command::new(&gen_exe)
            .current_dir(dir)
            .arg("--render")
            .arg("--obl")
            .arg(dir.join("Sum.obl.json"))
            .arg("--out")
            .arg(dir.join("Gen"))
            .status()
            .unwrap()
    };
    assert!(status.success(), "gen --render failed for the Sum fixture");
}

fn assemble_package(root: &Path, dir: &Path) {
    // lean-toolchain pin (the package resolves the same toolchain).
    let pin = fs::read(root.join(PORT).join("lean-toolchain")).unwrap();
    fs::write(dir.join("lean-toolchain"), pin).unwrap();
    // lakefile: the port library + the Gen output + the developer proofs.
    let lakefile = format!(
        "name = \"tyu-source-e2e\"\nversion = \"0.1.0\"\n\n[[lean_lib]]\nname = \"Tyu\"\nsrcDir = \"{}\"\nroots = [\"Tyu\"]\n\n[[lean_lib]]\nname = \"Gen\"\nsrcDir = \".\"\nroots = [\"Gen\"]\n\n[[lean_lib]]\nname = \"SumFix\"\nsrcDir = \".\"\nroots = [\"SumFix\"]\n",
        root.join(PORT).display()
    );
    fs::write(dir.join("lakefile.toml"), lakefile).unwrap();
    fs::copy(
        root.join(FIXTURE).join("SumFix.lean"),
        dir.join("SumFix.lean"),
    )
    .unwrap();
    fs::write(
        dir.join("hv.lean"),
        "import SumFix\nimport Tyu.Verdicts.Harvest\n\n#eval! Tyu.Verdicts.Harvest.run\n",
    )
    .unwrap();
}

fn run_harvest(dir: &Path) -> String {
    let build = Command::new("lake")
        .current_dir(dir)
        .args(["build", "SumFix", "Tyu.Verdicts.Harvest"])
        .status()
        .unwrap();
    assert!(build.success(), "lake build of the fixture package failed");
    let out = dir.join("out.v2.json");
    let status = Command::new("lake")
        .current_dir(dir)
        .env("TYU_HARVEST_GEN_DIR", dir.join("Gen"))
        .env("TYU_HARVEST_OBL", dir.join("Sum.obl.json"))
        .env("TYU_HARVEST_OUT", &out)
        .args(["env", "lean", "hv.lean"])
        .status()
        .unwrap();
    assert!(status.success(), "harvest exited nonzero");
    fs::read_to_string(&out).unwrap()
}

#[test]
fn source_surface_worked_example_harvests_with_source_provenance() {
    if !toolchain_present() {
        if std::env::var("TYU_SOURCE_SURFACE_E2E").is_ok() {
            panic!("TYU_SOURCE_SURFACE_E2E=1 requires `lean` and `lake` on PATH");
        }
        eprintln!(
            "skipping the source-surface e2e (no Lean toolchain on PATH; \
             ci/port.sh runs it with the pinned toolchain)"
        );
        return;
    }
    let root = workspace_root();
    let dir = fresh_dir();
    fs::copy(
        root.join(FIXTURE).join("Sum.obl.json"),
        dir.join("Sum.obl.json"),
    )
    .unwrap();
    render_fixture(&root, &dir);

    // The renderer MUST produce the source-surface statement + the src_def
    // metadata row parent carries it (P9.1's Gen source forms).
    let lean_text = fs::read_to_string(dir.join("Gen/Sum.lean")).unwrap();
    assert!(
        lean_text.contains("def src_stmt_Sum_answer_subtype_range_0 : Prop"),
        "generated Gen/Sum.lean must contain the source-surface statement:\n{lean_text}"
    );
    assert!(
        lean_text.contains("Tyu.Src.outInRange"),
        "the source statement must run the pure-fragment semantics (Tyu.Src)"
    );
    let meta = fs::read_to_string(dir.join("Gen/Sum.gen.json")).unwrap();
    assert!(
        meta.contains("\"src_def\": \"src_stmt_Sum_answer_subtype_range_0\""),
        "gen.json must carry the src_def row:\n{meta}"
    );

    assemble_package(&root, &dir);
    let verdicts = run_harvest(&dir);

    // The certificate IS a source-surface certificate with the T-S reliance.
    assert!(
        verdicts.contains("\"surface\":\"source\""),
        "verdict must record surface source:\n{verdicts}"
    );
    assert!(
        verdicts.contains("\"relies\":[\"T-S\"]"),
        "proof must carry relies [\"T-S\"]:\n{verdicts}"
    );
    assert!(
        verdicts.contains("\"trust\":\"proof\"") && verdicts.contains("\"method\":\"certificate\""),
        "the source certificate remains proof-class (kernel + statement binding):\n{verdicts}"
    );
    // The same obligation's IR statement (stmt_Sum_…) is generated but NOT
    // proven — the source theorem is the only certificate, and it binds.
    assert!(
        verdicts.contains("obl_Sum_answer_subtype_range_0"),
        "the harvested theorem is the source-surface one:\n{verdicts}"
    );
    // Fail-closed negative: an IR-typed theorem would be surface: "ir"; the
    // source-typed one must NOT carry "ir".
    assert!(
        !verdicts.contains("\"surface\":\"ir\""),
        "the source certificate must not be recorded as IR-surface:\n{verdicts}"
    );
}

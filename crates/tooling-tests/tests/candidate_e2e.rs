//! Candidate attribution, end to end (PLAN-VERIFY-3 P10.2, §Q10): a
//! fill-generated (unreviewed) proof candidate is kernel-checked —
//! `trust: proof` — and the harvest records `authored: "candidate"` when the
//! obligation id is in the candidate set (`TYU_HARVEST_CANDIDATES`, derived
//! by `tyu proof fill` from the `-- tyu:candidate obligation=` markers).
//!
//! The fixture is a tiny `tyu.obl/v2` artifact with ONE `mmio-bounds`
//! obligation (`OffsetLE` geometry). Its rendered statement
//! (`offsetWithin … 0 4 16`) is closed by the automation's dispatcher
//! (`tyu_auto` → the geometry closure), so the full candidate path is real:
//! the candidate's theorem elaborates, the kernel gates it, and the harvest
//! attributes it. A negative variant (no candidate id in the env) must record
//! `authored: "developer"`.
//!
//! Tier-A style: needs a Lean toolchain (`TYU_CANDIDATE_E2E=1` forces a loud
//! failure when absent; `ci/port.sh` runs it with the pinned toolchain).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        .join("tyu_cand_e2e")
        .join(format!("{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A minimal `tyu.obl/v2` artifact: one `mmio-bounds` obligation whose
/// geometry (`0 + 4 ≤ 16`) the automation closes.
const TINY_OBL: &str = r#"{
  "schema": "tyu.obl/v2", "semantics": "tyu.ir-sem/1.0", "stmt": "tyu.stmt/1.0",
  "module": "Tiny", "target": "x86_64-unknown-none", "platform": "x86_64-unknown-none",
  "model_semantics": "unmodeled", "abi_contract_version": 2,
  "facts": { "words": [ { "name": "probe", "net": 1, "high": 4, "top": false,
    "diverge_free": true, "blocks": 0, "ir": "", "ir_hash": "abc1234567890def" } ],
    "subtypes": [], "predicates": [] },
  "obligations": [
    { "id": "Tiny::probe::mmio-bounds::0", "id_hash": "1111111111111111",
      "kind": "mmio-bounds",
      "site": { "word": "probe", "occurrence": 0, "span": { "line": 1, "col": 1 } },
      "formula": { "op": "OffsetLE", "off": 0, "width": 4, "size": 16 },
      "assumptions": [], "cycles": [] }
  ]
}"#;

fn render_fixture(root: &Path, dir: &Path) {
    let gen_exe = root.join(PORT).join(".lake/build/bin/gen");
    Command::new(&gen_exe)
        .current_dir(dir)
        .arg("--render")
        .arg("--obl")
        .arg(dir.join("Tiny.obl.json"))
        .arg("--out")
        .arg(dir.join("Gen"))
        .status()
        .unwrap();
}

fn assemble_package(root: &Path, dir: &Path, candidate: &str) {
    let pin = fs::read(root.join(PORT).join("lean-toolchain")).unwrap();
    fs::write(dir.join("lean-toolchain"), pin).unwrap();
    let lakefile = format!(
        "name = \"tyu-cand-e2e\"\nversion = \"0.1.0\"\n\n[[lean_lib]]\nname = \"Tyu\"\nsrcDir = \"{}\"\nroots = [\"Tyu\"]\n\n[[lean_lib]]\nname = \"Gen\"\nsrcDir = \".\"\nroots = [\"Gen\"]\n\n[[lean_lib]]\nname = \"Cand\"\nsrcDir = \".\"\nroots = [\"Cand\"]\n",
        root.join(PORT).display()
    );
    fs::write(dir.join("lakefile.toml"), lakefile).unwrap();
    fs::write(dir.join("Cand.lean"), candidate).unwrap();
    fs::write(
        dir.join("hv.lean"),
        "import Cand\nimport Tyu.Verdicts.Harvest\n\n#eval! Tyu.Verdicts.Harvest.run\n",
    )
    .unwrap();
}

/// The candidate file `tyu proof fill` would write for the Tiny obligation
/// (marker-headed, automation-closed).
fn candidate_text() -> String {
    "-- tyu:candidate obligation=Tiny::probe::mmio-bounds::0\n\
     import Gen.Tiny\n\
     import Tyu.Automation.Auto\n\
     import Tyu.Automation.Cycle\n\n\
     theorem obl_Tiny_probe_mmio_bounds_0 :\n\
     \x20 (Tyu.Gen.Corpus.Tiny.stmt_Tiny_probe_mmio_bounds_0) := by\n\
     \x20 tyu_auto\n"
        .to_string()
}

fn run_harvest(dir: &Path, candidates: &str) -> String {
    let build = Command::new("lake")
        .current_dir(dir)
        .args(["build", "Cand", "Tyu.Verdicts.Harvest"])
        .status()
        .unwrap();
    assert!(
        build.success(),
        "lake build of the candidate package failed"
    );
    let out = dir.join("out.v2.json");
    let status = Command::new("lake")
        .current_dir(dir)
        .env("TYU_HARVEST_GEN_DIR", dir.join("Gen"))
        .env("TYU_HARVEST_OBL", dir.join("Tiny.obl.json"))
        .env("TYU_HARVEST_OUT", &out)
        .env("TYU_HARVEST_CANDIDATES", candidates)
        .args(["env", "lean", "hv.lean"])
        .status()
        .unwrap();
    assert!(status.success(), "harvest exited nonzero");
    fs::read_to_string(&out).unwrap()
}

#[test]
fn candidate_verdict_is_kernel_checked_and_attributed() {
    if !toolchain_present() {
        if std::env::var("TYU_CANDIDATE_E2E").is_ok() {
            panic!("TYU_CANDIDATE_E2E=1 requires `lean` and `lake` on PATH");
        }
        eprintln!(
            "skipping the candidate e2e (no Lean toolchain on PATH; ci/port.sh \
             runs it with the pinned toolchain)"
        );
        return;
    }
    let root = workspace_root();
    let dir = fresh_dir();
    fs::write(dir.join("Tiny.obl.json"), TINY_OBL).unwrap();
    render_fixture(&root, &dir);

    // The geometry statement is rendered and `tyu_auto`-closable (it stays
    // *open* until a theorem exists — the kernel is the only gate).
    let lean_text = fs::read_to_string(dir.join("Gen/Tiny.lean")).unwrap();
    assert!(
        lean_text.contains("def stmt_Tiny_probe_mmio_bounds_0 : Prop"),
        "gen must render the mmio-bounds statement:\n{lean_text}"
    );

    assemble_package(&root, &dir, &candidate_text());

    // Positive: the candidate id is in the harvest attribution set.
    let with_candidate = run_harvest(&dir, "Tiny::probe::mmio-bounds::0");
    assert!(
        with_candidate.contains("\"authored\":\"candidate\""),
        "a candidate verdict must be attributed candidate:\n{with_candidate}"
    );
    assert!(
        with_candidate.contains("\"trust\":\"proof\""),
        "the candidate remains kernel-checked proof-class (the marker never \
         licenses an unchecked label):\n{with_candidate}"
    );
    assert!(
        with_candidate.contains("\"status\":\"discharged\""),
        "the mmio-bounds obligation must be discharged by the candidate theorem:\n{with_candidate}"
    );

    // Negative: without the id in the set, the same theorem is a developer
    // certificate (`authored: "developer"`).
    let with_developer = run_harvest(&dir, "");
    assert!(
        with_developer.contains("\"authored\":\"developer\""),
        "without the candidate marker id the attribution defaults to developer:\n{with_developer}"
    );

    let _ = fs::remove_dir_all(&dir);
}

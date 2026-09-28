//! The refined-proof worked example, end to end (PLAN-VERIFY-3 P13.2,
//! §11 acceptance item 1's refined-variant): the rp2350 bundle's
//! `[refinements]` device (`UARTFR` → `rp2350.uart-fr`) relativizes a
//! developer certificate — the statement context carries the refinement, the
//! kernel-checked theorem is harvested at the SOURCE surface (`T-S`), and
//! langc's FR-5 consumption BINDS the certificate only when the consuming
//! build carries the same refinement context (§Q3/P13.1 — without it the
//! recompute is E6421-stale, fail-closed to open).
//!
//! The fixture (`verification/ports/lean/tests/refined-fixture/`) is an
//! authored `tyu.obl/v2` artifact (module `Uart7`, model
//! `tyu.model/rp2350/1`) whose word `read-tx-idle` reads the refined UARTFR
//! register via the **pure-fragment** `vol_load u32 uart.UARTFR; drop u32;
//! const_i64 1; ret` — every op is in `Tyu.Src.Op`, so the word also renders
//! a SOURCE-surface statement (`src_stmt_…` over `Tyu.Src`), which the
//! worked proof `Uart7Fix.lean` proves (the `Sum`/P9.2 pattern). This test
//! drives the REAL port machinery:
//!
//!   1. the `gen` renderer WITH the bundle's refinement context
//!      (`TYU_GEN_REFINEMENTS` = the fixture's `tyu.refinements/1`
//!      document) renders `Gen/Uart7.lean`; the statement hash binds
//!      `refinement: "rp2350.uart-fr"` — byte-equal to the Rust encoder's
//!      `for_obligation_with_refinement` (the renderer↔encoder drift lock,
//!      extended to the refinement dimension);
//!   2. the render WITHOUT the context REFUSES the modeled bundle's
//!      MMIO-word statements (`omitted: true, reason: refined-read-unbound`
//!      — the "mismatch ⇒ render refuses" rule, P13.1): never a silent
//!      re-binding to the unrefined default;
//!   3. a temp lake package assembles the fixture proof — the theorem of the
//!      SOURCE-surface statement — with the port library; the harvest binds
//!      it with `surface: "source"`, `proof.relies: ["T-S"]`, the refined
//!      statement hash, and `model_semantics: "tyu.model/rp2350/1"`;
//!   4. langc consumes the verdicts through `--refinements=<the same
//!      context>` + `--bind-obl` and the site discharges; the run-once
//!      negative (consumption without the context) stays healthy — the
//!      recompute is E6421-stale, the site stays open, the check retained
//!      (never a wrong discharge).
//!
//! The test is tier-A style: it needs a Lean toolchain. Without one it
//! SKIPS with a message; `TYU_REFINED_E2E=1` forces the run (and fails
//! loudly when the toolchain is absent). `ci/port.sh` runs it
//! unconditionally (the port gate always has the toolchain).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = "verification/ports/lean/tests/refined-fixture";
const PORT: &str = "verification/ports/lean";
const OBL_ID: &str = "Uart7::read-tx-idle::subtype-range::1";

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

fn fresh_dir(label: &str) -> PathBuf {
    common::fresh_dir(&format!("refined_{label}"))
}

/// Run the port's `gen` renderer with (or without) the refinement context.
fn render_fixture(root: &Path, dir: &Path, with_refinements: bool) {
    let gen_exe = root.join(PORT).join(".lake/build/bin/gen");
    if !gen_exe.is_file() {
        Command::new("lake")
            .current_dir(root.join(PORT))
            .args(["build", "gen"])
            .status()
            .expect("lake build gen");
    }
    let mut cmd = Command::new(&gen_exe);
    if with_refinements {
        cmd.env(
            "TYU_GEN_REFINEMENTS",
            root.join(FIXTURE).join("refinements.json"),
        );
    }
    let status = cmd
        .current_dir(dir)
        .arg("--render")
        .arg("--obl")
        .arg(dir.join("Uart7.obl.json"))
        .arg("--out")
        .arg(dir.join("Gen"))
        .status()
        .unwrap();
    assert!(
        status.success(),
        "gen --render failed for the Uart7 fixture"
    );
}

/// The fixture's declared refinements (the Rust side of the drift lock —
/// the same document the renderer consumes).
fn rust_decls(root: &Path) -> Vec<verifier::refinements::Refinement> {
    let text = fs::read_to_string(root.join(FIXTURE).join("refinements.json")).unwrap();
    verifier::refinements::parse_manifest(&text).expect("fixture manifest parses")
}

/// The Rust encoder's statement hash for the fixture's `::1` obligation
/// under a refinement context (`Some(decls)` from the manifest, or `None`
/// for the §Q13 default) — the drift-lock expectations the Lean renderer
/// must byte-match.
fn rust_hash(root: &Path, with_refinement: bool) -> String {
    let bytes = fs::read(root.join(FIXTURE).join("Uart7.obl.json")).unwrap();
    let set = verifier::codec::read_obl(&bytes).unwrap();
    let o = set
        .obligations
        .iter()
        .find(|o| o.id == OBL_ID)
        .expect("renderable obligation");
    let word_ir = set
        .facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| w.ir.as_str())
        .unwrap_or("");
    let word_ir_hash = verifier::stmt::sha256_hex16(word_ir.as_bytes());
    let candidate = if with_refinement {
        verifier::refinements::word_refinement(word_ir, &rust_decls(root))
            .map(|r| r.refinement.clone())
    } else {
        None
    };
    let ctx = verifier::stmt::StatementContext::for_obligation_with_refinement(
        &set.module,
        &set.target,
        &set.model_semantics,
        &word_ir_hash,
        candidate.as_deref(),
        o,
    );
    ctx.statement_hash_hex(&o.formula)
}

fn assemble_package(root: &Path, dir: &Path) {
    let pin = fs::read(root.join(PORT).join("lean-toolchain")).unwrap();
    fs::write(dir.join("lean-toolchain"), pin).unwrap();
    let lakefile = format!(
        "name = \"tyu-refined-e2e\"\nversion = \"0.1.0\"\n\n[[lean_lib]]\nname = \"Tyu\"\nsrcDir = \"{}\"\nroots = [\"Tyu\"]\n\n[[lean_lib]]\nname = \"Gen\"\nsrcDir = \".\"\nroots = [\"Gen\"]\n\n[[lean_lib]]\nname = \"Uart7Fix\"\nsrcDir = \".\"\nroots = [\"Uart7Fix\"]\n",
        root.join(PORT).display()
    );
    fs::write(dir.join("lakefile.toml"), lakefile).unwrap();
    fs::copy(
        root.join(FIXTURE).join("Uart7Fix.lean"),
        dir.join("Uart7Fix.lean"),
    )
    .unwrap();
    fs::write(
        dir.join("hv.lean"),
        "import Uart7Fix\nimport Tyu.Verdicts.Harvest\n\n#eval! Tyu.Verdicts.Harvest.run\n",
    )
    .unwrap();
}

fn run_harvest(dir: &Path) -> String {
    let build = Command::new("lake")
        .current_dir(dir)
        .args(["build", "Uart7Fix", "Tyu.Verdicts.Harvest"])
        .status()
        .unwrap();
    assert!(build.success(), "lake build of the fixture package failed");
    let out = dir.join("out.v2.json");
    let status = Command::new("lake")
        .current_dir(dir)
        .env("TYU_HARVEST_GEN_DIR", dir.join("Gen"))
        .env("TYU_HARVEST_OBL", dir.join("Uart7.obl.json"))
        .env("TYU_HARVEST_OUT", &out)
        .args(["env", "lean", "hv.lean"])
        .status()
        .unwrap();
    assert!(status.success(), "harvest exited nonzero");
    fs::read_to_string(&out).unwrap()
}

/// Compile the real source with langc under the rp2350 pack; returns the
/// object-emission success + stderr (the FR-5 consumption leg).
fn compile_with_refinements(root: &Path, dir: &Path, refinements: bool) -> (bool, String) {
    let out_dir = dir.join(if refinements { "out" } else { "outnr" });
    fs::create_dir_all(&out_dir).unwrap();
    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        "--target=armv7m-unknown-none".into(),
        format!("--out-dir={}", out_dir.display()),
        "--write-obl".into(),
        "--checks=undischarged".into(),
        format!("--platform={}", root.join("platforms/rp2350").display()),
        "--model-semantics=tyu.model/rp2350/1".into(),
    ];
    let verdict_file = dir.join("out.v2.json");
    args.push(format!("--verdicts={}", verdict_file.display()));
    args.push(format!(
        "--bind-obl={}",
        root.join(FIXTURE).join("Uart7.obl.json").display()
    ));
    if refinements {
        args.push(format!(
            "--refinements={}",
            root.join(FIXTURE).join("refinements.json").display()
        ));
    }
    args.push(root.join(FIXTURE).join("Uart7.mod").display().to_string());
    let out = Command::new(common::bin::resolve("langc"))
        .args(&args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The `statement_hash` hex recorded in a `<Module>.gen.json` document for
/// the obligation with the exact id `id` (the row key).
fn meta_statement_hash(meta: &str, id: &str) -> Option<String> {
    let key = format!("\"id\": \"{id}\"");
    let from = meta.find(&key)?;
    let window = &meta[from..(from + 512).min(meta.len())];
    let sh = "\"statement_hash\": \"";
    let start = window.find(sh)? + sh.len();
    let end = window[start..].find('"')?;
    Some(window[start..start + end].to_string())
}

#[test]
fn refined_worked_example_harvests_and_binds_with_refinement_context() {
    if !toolchain_present() {
        if std::env::var("TYU_REFINED_E2E").is_ok() {
            panic!("TYU_REFINED_E2E=1 requires `lean` and `lake` on PATH");
        }
        eprintln!("skipping the refined-proof e2e (no Lean toolchain on PATH; ci/port.sh runs it)");
        return;
    }
    let root = workspace_root();
    let dir = fresh_dir("main");
    fs::copy(
        root.join(FIXTURE).join("Uart7.obl.json"),
        dir.join("Uart7.obl.json"),
    )
    .unwrap();

    // 1. The refusal ("mismatch ⇒ render refuses", P13.1): rendered WITHOUT
    // the refinement context, the MODELED bundle's MMIO-word statements are
    // OMITTED with `refined-read-unbound` — never silently re-bound to the
    // unrefined default. (A certificate under the refinement therefore
    // cannot consume a refinement-less build: E6421 / E6418 fail-closed.)
    let dir_nr = fresh_dir("neg");
    fs::copy(
        root.join(FIXTURE).join("Uart7.obl.json"),
        dir_nr.join("Uart7.obl.json"),
    )
    .unwrap();
    render_fixture(&root, &dir_nr, false);
    let meta_nr = fs::read_to_string(dir_nr.join("Gen/Uart7.gen.json")).unwrap();
    assert!(
        meta_nr.contains("\"omitted\": true, \"reason\": \"refined-read-unbound\""),
        "the refinement-less render must refuse the modeled MMIO word's statements:\n{meta_nr}"
    );
    let lean_nr = fs::read_to_string(dir_nr.join("Gen/Uart7.lean")).unwrap();
    assert!(
        !lean_nr.contains("def stmt_Uart7_") && !lean_nr.contains("def src_stmt_Uart7_"),
        "no statements may render without the refinement context:\n{lean_nr}"
    );

    // 2. Render WITH the refinement context; the drift lock: the Lean
    // renderer's statement hash must byte-equal the Rust encoder's refined
    // hash, and gen.json must carry the refinement + the source-surface
    // statement (the pure-fragment word renders `src_stmt_…` too).
    render_fixture(&root, &dir, true);
    let lean_text = fs::read_to_string(dir.join("Gen/Uart7.lean")).unwrap();
    assert!(
        lean_text.contains("def stmt_Uart7_read_tx_idle_subtype_range_1 : Prop")
            && lean_text.contains("def src_stmt_Uart7_read_tx_idle_subtype_range_1 : Prop"),
        "generated Gen/Uart7.lean must contain both surface statements:\n{lean_text}"
    );
    assert!(
        lean_text.contains("Tyu.Src.outInRange"),
        "the source statement must run the pure-fragment semantics (Tyu.Src)"
    );
    let meta = fs::read_to_string(dir.join("Gen/Uart7.gen.json")).unwrap();
    assert!(
        meta.contains("\"refinement\": \"rp2350.uart-fr\"")
            && meta.contains("\"src_def\": \"src_stmt_Uart7_read_tx_idle_subtype_range_1\""),
        "gen.json must carry the refinement and the source def:\n{meta}"
    );
    let refined_expected = rust_hash(&root, true);
    let meta_hash = meta_statement_hash(&meta, OBL_ID).expect("refined render records the hash");
    assert_eq!(
        meta_hash, refined_expected,
        "renderer↔encoder drift lock: the Lean renderer must bind the refined Rust hash"
    );
    // The refinement dimension changes the statement hash (§Q13).
    assert_ne!(
        rust_hash(&root, false),
        refined_expected,
        "the refinement must relativize the statement hash"
    );

    // 3. Assemble the lake package, build the SOURCE-surface proof, harvest.
    assemble_package(&root, &dir);
    let verdicts = run_harvest(&dir);
    assert!(
        verdicts.contains("\"trust\":\"proof\"") && verdicts.contains("\"method\":\"certificate\""),
        "the refined certificate is proof-class (kernel + statement binding):\n{verdicts}"
    );
    assert!(
        verdicts.contains("\"surface\":\"source\"") && verdicts.contains("\"relies\":[\"T-S\"]"),
        "the source-surface certificate must carry surface source + T-S reliance:\n{verdicts}"
    );
    assert!(
        verdicts.contains(&format!("\"statement_hash\":\"{refined_expected}\"")),
        "the harvested certificate binds the REFINED statement hash:\n{verdicts}"
    );
    assert!(
        verdicts.contains("\"model_semantics\":\"tyu.model/rp2350/1\""),
        "verdict carries the bundle model identity:\n{verdicts}"
    );

    // 4. langc FR-5 consumption: with the refinement context the record
    // binds and the module compiles; the run-once negative without it still
    // compiles (the site stays open, check retained — never a wrong
    // discharge, and never a crash: the E6421 path).
    let (ok, _) = compile_with_refinements(&root, &dir, true);
    assert!(
        ok,
        "langc must bind the refined verdict with --refinements in context"
    );
    let (ok_neg, _) = compile_with_refinements(&root, &dir, false);
    assert!(
        ok_neg,
        "langc must stay healthy without the refinement in context (E6421-stale → open, check retained)"
    );
}

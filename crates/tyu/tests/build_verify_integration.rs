//! P6.2 — `tyu build --verify-tool=lean`: the developer-visible loop.
//!
//! The build runs gen → digest (E6418) → lake build and reports the per-module
//! statement accounting; before the P7 harvest exists every rendered statement
//! is unproven, and the report says so (`proof.harvest: "not-built"`) — no
//! path claims a proof verdict.
//!
//! Two tiers:
//!   - tier B (always run): `TYU_SKIP_PORT_BUILD=1` — the port build is
//!     skipped, the report records `lean-skipped`, the build proceeds.
//!   - tier A (env-gated `TYU_PROOF_E2E=1`, toolchain required): the full
//!     real pipeline runs — package generation, Gen digest verification, and
//!     the elaborating `lake build` in a cold package.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-build-verify-{tag}-{}-{}",
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

fn ensure_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

const BANK_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;

: bounded_inc ( Percent -- Percent )
  1 + as Percent
;

: main ( -- i64 )
  50 as Percent bounded_inc as i64
;
end;
";

/// Build the fixture project with `--verify-tool=lean` under the given env,
/// returning (success, report JSON value, stderr).
fn build(tag: &str, envs: &[(&str, &str)]) -> (bool, serde_json::Value, String) {
    let dir = fresh_dir(tag);
    fs::write(dir.join("Bank.mod"), BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(tyu_exe());
    cmd.current_dir(&dir)
        .arg("build")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg("--verify-tool=lean")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("Bank.mod");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("tyu build");
    let ok = out.status.success();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let report = if ok {
        fs::read(out_dir.join("verify-report.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };
    (ok, report, stderr)
}

// ---------------------------------------------------------------------------
// Tier B: the always-on, toolchain-free integration tier.
// ---------------------------------------------------------------------------

#[test]
fn tier_b_skip_mode_builds_and_reports_unproven_statements() {
    ensure_langc();
    let (ok, report, stderr) = build("tierb", &[("TYU_SKIP_PORT_BUILD", "1")]);
    assert!(
        ok,
        "the build must proceed under TYU_SKIP_PORT_BUILD:\n{stderr}"
    );
    assert_eq!(report["schema"], "tyu.verify-report/v2");
    let proof = &report["proof"];
    assert_eq!(proof["tool"], "lean-skipped");
    assert_eq!(proof["harvest"], "not-built");
    assert_eq!(proof["gen_digest"], "skipped");
    assert_eq!(proof["statements"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Tier A: the full real pipeline — env-gated so CI runners without the Lean
// toolchain keep green on tier B while the toolchain-present runs exercise
// package generation, E6418, and the elaborating lake build.
// ---------------------------------------------------------------------------

fn toolchain_available() -> bool {
    let lean = std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("lean"))
                .find(|c| c.is_file())
        })
        .is_some();
    let lake = std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("lake"))
                .find(|c| c.is_file())
        })
        .is_some();
    lean && lake
}

#[test]
fn proven_policy_fails_closed_without_proof_class_discharges() {
    // `--verify-policy=proven` (§Q12): interval-legacy `checked` discharges
    // are NOT admitted — the langc trust gate forces them open, the report
    // sees only opens, and the build fails E6410. Runs without the Lean
    // toolchain (the in-tree pipeline alone exercises the gate).
    let dir = fresh_dir("proven");
    fs::write(dir.join("Bank.mod"), BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let out = Command::new(tyu_exe())
        .current_dir(&dir)
        .arg("build")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg("--verify-policy=proven")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("Bank.mod")
        .output()
        .expect("tyu build");
    assert!(
        !out.status.success(),
        "proven must fail when nothing is proven"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E6410"), "stderr: {stderr}");
    // The report is still written with policy: proven and the opens listed.
    let report = serde_json::from_slice::<serde_json::Value>(
        &fs::read(out_dir.join("verify-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["policy"], "proven");
    assert!(!report["open"].as_array().unwrap().is_empty());
    // The langc trust gate records the not-admitted reason per open site.
    let reasons: Vec<&str> = report["open"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["reason"].as_str())
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("policy-proven")),
        "interval-only checked must be forced open under proven: {reasons:?}"
    );
}

#[test]
fn tier_a_full_pipeline_builds_and_lists_unproven_statements_per_module() {
    if std::env::var("TYU_PROOF_E2E")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!("skipping tier A (set TYU_PROOF_E2E=1 with a Lean toolchain to run)");
        return;
    }
    assert!(
        toolchain_available(),
        "TYU_PROOF_E2E=1 requires `lean` and `lake` on PATH"
    );
    ensure_langc();
    let (ok, report, stderr) = build("tiera", &[]);
    assert!(ok, "the full pipeline must build:\n{stderr}");
    let proof = &report["proof"];
    assert_eq!(proof["tool"], "lean");
    // P7: the harvest consumed the kernel environment (the two-pass flow).
    assert_eq!(proof["harvest"], "ok");
    assert_eq!(proof["gen_digest"], "verified");
    let statements = proof["statements"].as_array().unwrap();
    assert!(
        !statements.is_empty(),
        "per-module statement accounting expected"
    );
    let bank = statements
        .iter()
        .find(|s| s["module"] == "Bank")
        .expect("Bank accounting present");
    assert!(bank["rendered"].as_u64().unwrap() >= 1);
    // P7.1: `proven` counts come from the harvested v2 verdicts; the rest of
    // the rendered statements stay `unproven` (a state, not a fault).
    assert!(bank["proven"].as_u64().is_some());
    assert_eq!(
        bank["unproven"].as_u64().unwrap() + bank["proven"].as_u64().unwrap(),
        bank["rendered"].as_u64().unwrap()
    );
    assert!(stderr.contains("unproven"), "stderr: {stderr}");
}

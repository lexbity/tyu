//! Slice 8 — per-suite `verify_policy` (static-verification.md §14 P8; the
//! plan's §14: "test manifests MAY declare `verify_policy = 'no-open'` per
//! suite; default remains `open-ok`").
//!
//! A fixture that declares a policy compiles under the verification pipeline
//! (`--checks=undischarged --write-obl --verdicts=<empty>` — the same default
//! argv `tyu build` uses, so only the in-tree dischargers decide) and is then
//! *held* to the policy: `no-open` fails the suite with an E6410-class error
//! if any obligation stays open; `no-open-no-assumptions` additionally fails
//! on assumed verdicts. Absent policy = the legacy `--checks=all` compile —
//! zero behavior change for existing suites (adoption is per-suite,
//! explicit).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-verify-policy-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ensure_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

#[test]
fn manifest_parses_verify_policy_per_fixture() {
    let dir = fresh_dir("parse");
    let manifest = dir.join("manifest.toml");
    fs::write(
        &manifest,
        "\
[[fixture]]
name = \"ok\"
file = \"ok.mod\"
axes = [\"arith\"]
verify_policy = \"no-open\"

[[fixture]]
name = \"strict\"
file = \"strict.mod\"
axes = [\"stack\"]
verify_policy = \"no-open-no-assumptions\"

[[fixture]]
name = \"legacy\"
file = \"legacy.mod\"
axes = [\"trap\"]
",
    )
    .unwrap();
    let m = tyu::manifest::parse_manifest(&manifest).expect("manifest parses");
    assert_eq!(m.fixtures.len(), 3);
    assert_eq!(
        m.fixtures[0].verify_policy,
        Some(tyu::args::VerifyPolicy::NoOpen)
    );
    assert_eq!(
        m.fixtures[1].verify_policy,
        Some(tyu::args::VerifyPolicy::NoOpenNoAssumptions)
    );
    assert_eq!(m.fixtures[2].verify_policy, None, "default stays open-ok");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn manifest_rejects_unknown_verify_policy() {
    let dir = fresh_dir("parsebad");
    let manifest = dir.join("manifest.toml");
    fs::write(
        &manifest,
        "\
[[fixture]]
name = \"x\"
file = \"x.mod\"
axes = [\"arith\"]
verify_policy = \"sometimes\"
",
    )
    .unwrap();
    let err = tyu::manifest::parse_manifest(&manifest).expect_err("must reject unknown policy");
    assert!(
        err.to_string().contains("verify_policy"),
        "error should name the bad field: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// `tyu test` integration — hosted (native runner, no qemu)
// ---------------------------------------------------------------------------

/// A fixture whose only subtype casts are *provably in range* (constants) —
/// every obligation discharges, so `no-open` is satisfied.
const OK_FIXTURE: &str = "\
module NoOpenOk;
subtype Percent = i64 range 0..100;
import platform/testio { testio.write-byte };

: done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;

: no-open-ok-run ( -- )
  50 as Percent drop
  100 as Percent drop
  done ;
export { no-open-ok-run };
end;
";

/// A fixture with an *input-derived* narrowing cast — the helper word's
/// `as Percent` on its ⊤ input keeps a C1 obligation open, so `no-open`
/// must reject the suite (the entry word stays `( -- )` so the generated
/// runner's stub interface matches).
const BAD_FIXTURE: &str = "\
module NoOpenBad;
subtype Percent = i64 range 0..100;

: bad-helper ( i64 -- )
  as Percent drop ;

: no-open-bad-run ( -- )
  42 bad-helper ;
export { no-open-bad-run };
end;
";

fn run_test_suite(tag: &str, manifest: &str, files: &[(&str, &str)]) -> (bool, String) {
    let dir = fresh_dir(tag);
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
    let manifest_path = dir.join("manifest.toml");
    fs::write(&manifest_path, manifest).unwrap();
    let out = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-linux-gnu",
            &format!("--manifest={}", manifest_path.display()),
        ])
        .output()
        .expect("tyu test");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn no_open_suite_passes_when_all_obligations_discharge() {
    ensure_langc();
    let (ok, stderr) = run_test_suite(
        "ok_suite",
        "\
[[fixture]]
name = \"ok\"
file = \"ok.mod\"
axes = [\"arith\"]
verify_policy = \"no-open\"
",
        &[("ok.mod", OK_FIXTURE)],
    );
    assert!(ok, "fully-discharged no-open suite must pass:\n{stderr}");
}

#[test]
fn no_open_suite_fails_with_e6410_on_open_obligations() {
    ensure_langc();
    let (ok, stderr) = run_test_suite(
        "bad_suite",
        "\
[[fixture]]
name = \"bad\"
file = \"bad.mod\"
axes = [\"arith\"]
verify_policy = \"no-open\"
",
        &[("bad.mod", BAD_FIXTURE)],
    );
    assert!(!ok, "an input-derived cast must keep an open obligation under no-open");
    assert!(
        stderr.contains("E6410") && stderr.contains("NoOpenBad"),
        "failure must surface E6410 naming the fixture:\n{stderr}"
    );
}

#[test]
fn legacy_fixture_without_policy_compiles_unconditionally() {
    // Absent policy = the legacy compile; the suite proceeds to run the
    // fixture (which completes with the completion marker), unchanged.
    ensure_langc();
    let (ok, stderr) = run_test_suite(
        "legacy_suite",
        "\
[[fixture]]
name = \"ok\"
file = \"ok.mod\"
axes = [\"arith\"]
",
        &[("ok.mod", OK_FIXTURE)],
    );
    assert!(ok, "legacy (no policy) suite must keep its behavior:\n{stderr}");
}
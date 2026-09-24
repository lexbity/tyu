//! tyu verification-pipeline tests (static-verification.md slice P4):
//!
//! - the `.tyu-verify` verdicts cache round-trips (§7.4): a second identical
//!   build hits the cache and produces a byte-identical report (FR-17);
//! - a corrupt cache entry self-heals (treated as a miss, build stays
//!   correct — R10);
//! - `--verify=off` reproduces the legacy path in the report (`policy: "off"`,
//!   every check emitted — FR-22);
//! - `--verify-policy=no-open` fails the build with E6410 listing open
//!   obligations (FR-18);
//! - the hosted running image traps (exit code 21) only when an open subtype
//!   site actually violates — the AC-4 behavioral pair.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-verify-pipeline-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
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

fn build(tag: &str, extra: &[&str], source: &str) -> (PathBuf, String, bool) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(tyu_exe());
    cmd.arg("build")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap());
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.output().expect("tyu invocation");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out_dir, stderr, out.status.success())
}

#[test]
fn default_build_populates_cache_and_second_build_hits() {
    // First build: the verdicts cache slot is created and filled from langc's
    // echo. Second build over the same tree: object + verdicts cache hit.
    let dir = fresh_dir("cache");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let run = |extra: &[&str]| {
        let mut cmd = Command::new(tyu_exe());
        cmd.arg("build")
            .arg(format!("--out-dir={}", out_dir.display()))
            .arg(mod_path.to_str().unwrap());
        for a in extra {
            cmd.arg(a);
        }
        let out = cmd.output().expect("tyu invocation");
        assert!(
            out.status.success(),
            "build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    };

    let first = run(&[]);
    assert!(first.contains("verify:"), "one-line summary (NFR-9): {first}");
    let cache_dir = out_dir.join(".tyu-verify");
    let entries: Vec<String> = fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries.len(), 1, "one module → one verdicts cache slot: {entries:?}");
    let echo = fs::read(cache_dir.join(&entries[0])).unwrap();
    let parsed = verifier::verdict::read_echo(&echo).expect("echo is a valid verdicts/echo file");
    assert_eq!(parsed.verdicts.records.len(), 0, "no site discharged in P4 (no in-tree subtype discharge)");

    let report1 = fs::read(out_dir.join("verify-report.json")).unwrap();

    let second = run(&[]);
    assert!(second.contains("cache hit"), "second build must hit the cache: {second}");
    let report2 = fs::read(out_dir.join("verify-report.json")).unwrap();
    assert_eq!(
        report1, report2,
        "identical tree → byte-identical report (FR-17)"
    );
}

#[test]
fn corrupt_cache_slot_self_heals_as_a_miss() {
    let dir = fresh_dir("corrupt");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let build_once = || {
        let out = Command::new(tyu_exe())
            .arg("build")
            .arg(format!("--out-dir={}", out_dir.display()))
            .arg(mod_path.to_str().unwrap())
            .output()
            .expect("tyu invocation");
        assert!(
            out.status.success(),
            "build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    };

    build_once();
    // Corrupt the verdicts slot AND the re-homed object's obl sibling is
    // fine; only the verdicts echo is attacked.
    let cache_dir = out_dir.join(".tyu-verify");
    let slot = fs::read_dir(&cache_dir).unwrap().filter_map(|e| e.ok()).next().unwrap().path();
    fs::write(&slot, b"not a verdicts file at all {").unwrap();

    let third = build_once();
    assert!(
        third.contains("cache miss"),
        "corrupt verdicts slot must invalidate the cache entry (self-heal): {third}"
    );
    // The build still succeeds and the report is still honest.
    let report = fs::read(out_dir.join("verify-report.json")).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(v["policy"], "open-ok");
    assert_eq!(v["modules"][0]["classes"]["subtype-range"]["open"], 4);
    // The slot has healed back into a valid file.
    let healed = fs::read(&slot).unwrap();
    verifier::verdict::read_echo(&healed).expect("slot healed into a valid echo");
}

#[test]
fn verify_off_reports_legacy_policy_with_all_checks() {
    let (out_dir, stderr, ok) = build("off", &["--verify=off"], BANK_MOD);
    assert!(ok, "verify=off must build: {stderr}");
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(out_dir.join("verify-report.json")).unwrap()).unwrap();
    assert_eq!(v["policy"], "off", "legacy all-checks mode is documented");
    assert_eq!(
        v["emitted_checks"]["subtype_range"],
        4,
        "every subtype site emitted under verify=off"
    );
    assert_eq!(v["emitted_checks"]["data_stack_guards"], true);
}

#[test]
fn no_open_policy_fails_listing_open_obligations() {
    // `150 as Percent` is a constant the language traps on at runtime (the
    // book's own example — ch03:226): in P4 there is no in-tree subtype
    // discharger, so the site is open. open-ok lets it build (the image
    // traps); no-open rejects it with E6410 (AC-4 behavior under no-open).
    const TRAP150: &str = "\
module T;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  150 as Percent as i64
;
end;
";
    let (out_dir, stderr, ok) = build("noopen", &["--verify-policy=no-open"], TRAP150);
    assert!(!ok, "no-open must fail: {stderr}");
    assert!(stderr.contains("E6410"), "E6410 diagnostics: {stderr}");
    assert!(
        stderr.contains("subtype-range") && stderr.contains("T::main"),
        "open obligation listed: {stderr}"
    );
    // The report is still written (diagnostics for the failed gate).
    let report = fs::read(out_dir.join("verify-report.json")).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(v["policy"], "no-open");
}

#[test]
fn hosted_run_traps_only_on_out_of_range_cast() {
    // AC-4: open-ok builds both programs; running the out-of-range constant
    // traps with exit code 21 (the runtime maps trap 21 → exit 21), the
    // in-range constant runs clean (no 21).
    const TRAP150: &str = "\
module T;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  150 as Percent as i64
;
end;
";
    const CLEAN: &str = "\
module K;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  50 as Percent as i64
;
end;
";

    for (tag, src, expect_trap) in [
        ("clean", CLEAN, false),
        ("trap150", TRAP150, true),
    ] {
        let dir = fresh_dir(tag);
        let mod_path = dir.join("M.mod");
        fs::write(&mod_path, src).unwrap();
        let out_dir = dir.join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let out = Command::new(tyu_exe())
            .arg("run")
            .arg(format!("--out-dir={}", out_dir.display()))
            .arg(mod_path.to_str().unwrap())
            .output()
            .expect("tyu run invocation");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if expect_trap {
            assert!(
                stderr.contains("code 21"),
                "{tag}: out-of-range must trap 21, got: {stderr}"
            );
        } else {
            assert!(
                !stderr.contains("code 21"),
                "{tag}: in-range must not trap, got: {stderr}"
            );
        }
    }
}
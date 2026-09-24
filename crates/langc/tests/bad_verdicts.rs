//! Fail-closed robustness of verdicts input (static-verification.md §7.5,
//! R10, slice P4): every malformed verdicts file MUST abort the compile with
//! E6402 before codegen (a failed build never produces a less-checked
//! image — FR-13), and valid-but-mismatched files (unknown ids, hash
//! disagreement) MUST fail closed to *more* checking (the site stays open).
//!
//! The plan's negative-control suite is hand-rolled here as an exhaustive
//! table: each entry is a verdicts file body and the expected outcome.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn langc_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_langc"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-badverdicts-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
: f ( Percent -- i64 )
  drop 0
;
: main ( -- i64 )
  0
;
end;
";

/// Compile the fixture with the given verdicts file; returns (success, stderr).
fn compile_with(tag: &str, verdicts_body: &[u8]) -> (bool, String, PathBuf) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let vf = dir.join("v.json");
    fs::write(&vf, verdicts_body).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg("--checks=undischarged")
        .arg(format!("--verdicts={}", vf.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .expect("langc invocation");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        out_dir,
    )
}

const HEADER: &str = "{\"schema\":\"tyu.verdicts/v1\",\"tool\":{\"name\":\"t\",\"version\":\"0\"},\"semantics\":\"tyu.ir-sem/1.0\",\"verdicts\":[]}";

/// Fail-closed: the build MUST fail with E6402 and produce no object.
fn expect_malformed(tag: &str, body: &[u8]) {
    let (ok, err, out_dir) = compile_with(tag, body);
    assert!(!ok, "{tag}: malformed verdicts must fail the compile");
    assert!(err.contains("6402"), "{tag}: E6402 diagnostic, got: {err}");
    assert!(
        !out_dir.join("Bank.o").exists() && !out_dir.join("Bank.asm").exists(),
        "{tag}: fail-closed — no object/asm may be produced"
    );
}

/// Valid-but-mismatched: must FAIL CLOSED to more checking — build succeeds,
/// every check is retained. (An empty file is the degenerate valid case.)
fn expect_keep_all_checks(tag: &str, body: &[u8], asm_traps: usize) {
    let (ok, err, out_dir) = compile_with(tag, body);
    assert!(ok, "{tag}: valid-but-mismatched must still build: {err}");
    let asm = fs::read_to_string(out_dir.join("Bank.asm")).unwrap();
    let traps = asm
        .lines()
        .filter(|l| l.trim() == "jmp __lang_trap")
        .count();
    assert_eq!(
        traps, asm_traps,
        "{tag}: fail-closed — checks retained ({traps} != {asm_traps})"
    );
}

#[test]
fn malformed_verdicts_fail_closed_e6402() {
    // 1. Not JSON at all.
    expect_malformed("not-json", b"this is not json");
    // 2. Truncated document.
    expect_malformed("truncated", HEADER[..HEADER.len() - 3].as_bytes());
    // 3. Wrong schema version.
    expect_malformed(
        "wrong-schema",
        HEADER.replacen("tyu.verdicts/v1", "tyu.verdicts/v999", 1).as_bytes(),
    );
    // 4. Wrong semantics version.
    expect_malformed(
        "wrong-semantics",
        HEADER.replacen("tyu.ir-sem/1.0", "tyu.ir-sem/999.0", 1).as_bytes(),
    );
    // 5. Unknown record status.
    expect_malformed(
        "unknown-status",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"proven"}]}"#,
    );
    // 6. Missing required record field (no id).
    expect_malformed(
        "missing-id",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id_hash":"0000000000000000","status":"discharged"}]}"#,
    );
    // 7. Missing records array entirely.
    expect_malformed(
        "missing-verdicts",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0"}"#,
    );
    // 8. Record is an array, not an object.
    expect_malformed(
        "record-is-array",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[[1,2,3]]}"#,
    );
    // 9. Root is an array, not an object.
    expect_malformed("root-is-array", b"[1,2,3]");
    // 10. id is a number, not a string.
    expect_malformed(
        "numeric-id",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":4,"id_hash":"0000000000000000","status":"discharged"}]}"#,
    );
    // 11. id exceeding the §7.5 length cap.
    {
        let mut long = String::new();
        long.push_str("{\"schema\":\"tyu.verdicts/v1\",\"tool\":{\"name\":\"t\",\"version\":\"0\"},\"semantics\":\"tyu.ir-sem/1.0\",\"verdicts\":[{\"id\":\"");
        long.push_str(&"x".repeat(600));
        long.push_str("\",\"id_hash\":\"0000000000000000\",\"status\":\"discharged\"}]}");
        expect_malformed("oversized-id", long.as_bytes());
    }
    // 12. File exceeding the 4 MiB cap.
    {
        let mut big = Vec::from(HEADER.as_bytes());
        big.extend_from_slice(&vec![b' '; 4 * 1024 * 1024 + 1]);
        expect_malformed("oversized-file", &big);
    }
    // 13. Unbalanced braces.
    expect_malformed("unbalanced", b"{\"schema\":\"tyu.verdicts/v1\"");
    // 14. Control char inside a string.
    expect_malformed(
        "control-char",
        b"{\"schema\":\"tyu.verdicts/v1\",\"tool\":{\"name\":\"t\",\"version\":\"0\"},\"semantics\":\"tyu.ir-sem/1.0\",\"verdicts\":[{\"id\":\"a\x01b\",\"id_hash\":\"0000000000000000\",\"status\":\"discharged\"}]}",
    );
    // 15. Missing status value (empty string).
    expect_malformed(
        "empty-status",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":""}]}"#,
    );
    // 16. id_hash exceeding the §7.5 length cap.
    {
        let mut long = String::new();
        long.push_str("{\"schema\":\"tyu.verdicts/v1\",\"tool\":{\"name\":\"t\",\"version\":\"0\"},\"semantics\":\"tyu.ir-sem/1.0\",\"verdicts\":[{\"id\":\"Bank::f::subtype-range::0\",\"id_hash\":\"");
        long.push_str(&"y".repeat(600));
        long.push_str("\",\"status\":\"discharged\"}]}");
        expect_malformed("oversized-id-hash", long.as_bytes());
    }
}

/// Valid-but-mismatched files: unknown ids and hash-disagreements must FAIL
/// CLOSED — the build succeeds and every check is retained (Q3/§7.5: a
/// hostile file can only cause more checking, never less).
#[test]
fn mismatched_verdicts_fail_closed_to_more_checking() {
    // Baseline trap count under an empty (valid) verdicts file: the fixture's
    // only subtype site is f's C1 param check (a ⊤ caller input — the engine
    // cannot discharge it), 1 site × 2 TrapIfFalse = 2 trap sites.
    expect_keep_all_checks("empty", HEADER.as_bytes(), 2);

    // Unknown id (matches no obligation).
    expect_keep_all_checks(
        "unknown-id",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":"Bank::nope::subtype-range::9","id_hash":"1234567890abcdef","status":"discharged"}]}"#,
        2,
    );

    // Correct id, wrong hash — stale, fail-closed.
    expect_keep_all_checks(
        "wrong-hash",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged"}]}"#,
        2,
    );

    // Duplicated records: deterministic first-match wins; both remain matched
    // (no staleness), neither discharges a site with a wrong hash on the
    // correct record.
    expect_keep_all_checks(
        "duplicate-wrong-hash",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged"},{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"assumed"}]}"#,
        2,
    );

    // Additive/unknown top-level keys are tolerated (schema grows additively).
    expect_keep_all_checks(
        "additive-keys",
        br#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","future_extra":{"a":1},"verdicts":[],"stale_verdicts":0}"#,
        2,
    );
}
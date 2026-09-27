//! Fail-closed robustness of verdicts input (`tyu.verdicts/v2`,
//! static-verification.md §7.5, PLAN-VERIFY-3 §6.3): every malformed
//! verdicts file MUST abort the compile with E6402 (or E6417 for closed-
//! registry violations) before codegen — a failed build never produces a
//! less-checked image — and valid-but-mismatched files (unknown ids, hash
//! disagreement, stale statement/identity) MUST fail closed to *more*
//! checking (the site stays open).

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

/// The v2 document head REQUIRED fields: schema, (absent certifier), the
/// build identity its records bind against (target/model — v2 REQUIRES them
/// and a mismatch is FR-5-stale), semantics/stmt, verdicts.
const HEADER: &str = "{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[]}";

/// Fail-closed: the build MUST fail with `code` and produce no object.
fn expect_error(tag: &str, body: &[u8], code: &str) {
    let (ok, err, out_dir) = compile_with(tag, body);
    assert!(!ok, "{tag}: malformed verdicts must fail the compile");
    assert!(err.contains(code), "{tag}: {code} diagnostic, got: {err}");
    assert!(
        !out_dir.join("Bank.o").exists() && !out_dir.join("Bank.asm").exists(),
        "{tag}: fail-closed — no object/asm may be produced"
    );
}

/// Valid-but-mismatched: must FAIL CLOSED to more checking — build succeeds,
/// every check is retained.
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
    expect_error("not-json", b"this is not json", "6402");
    // 2. Truncated document.
    expect_error("truncated", &HEADER.as_bytes()[..HEADER.len() - 3], "6402");
    // 3. Wrong schema version.
    expect_error(
        "wrong-schema",
        HEADER
            .replacen("tyu.verdicts/v2", "tyu.verdicts/v999", 1)
            .as_bytes(),
        "6402",
    );
    // 4. Wrong semantics version.
    expect_error(
        "wrong-semantics",
        HEADER
            .replacen("tyu.ir-sem/1.0", "tyu.ir-sem/999.0", 1)
            .as_bytes(),
        "6402",
    );
    // 5. Unknown record status (closed per-surface set).
    expect_error(
        "unknown-status",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"proven","trust":"checked"}]}"#,
        "6402",
    );
    // 6. Missing required record field (no id).
    expect_error(
        "missing-id",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id_hash":"0000000000000000","status":"discharged","trust":"checked"}]}"#,
        "6402",
    );
    // 7. Missing records array entirely.
    expect_error(
        "missing-verdicts",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled"}"#,
        "6402",
    );
    // 8. Record is an array, not an object.
    expect_error(
        "record-is-array",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[[1,2,3]]}"#,
        "6402",
    );
    // 9. Root is an array, not an object.
    expect_error("root-is-array", b"[1,2,3]", "6402");
    // 10. id is a number, not a string.
    expect_error(
        "numeric-id",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":4,"id_hash":"0000000000000000","status":"discharged","trust":"checked"}]}"#,
        "6402",
    );
    // 11. id exceeding the §7.5 length cap.
    {
        let mut long = String::new();
        long.push_str("{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{\"id\":\"");
        long.push_str(&"x".repeat(600));
        long.push_str("\",\"id_hash\":\"0000000000000000\",\"status\":\"discharged\",\"trust\":\"checked\"}]}");
        expect_error("oversized-id", long.as_bytes(), "6402");
    }
    // 12. File exceeding the 4 MiB cap.
    {
        let mut big = Vec::from(HEADER.as_bytes());
        big.extend_from_slice(&vec![b' '; 4 * 1024 * 1024 + 1]);
        expect_error("oversized-file", &big, "6402");
    }
    // 13. Unbalanced braces.
    expect_error("unbalanced", b"{\"schema\":\"tyu.verdicts/v2\"", "6402");
    // 14. Control char inside a string.
    expect_error(
        "control-char",
        b"{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{\"id\":\"a\x01b\",\"id_hash\":\"0000000000000000\",\"status\":\"discharged\",\"trust\":\"checked\"}]}",
        "6402",
    );
    // 15. Missing status value (empty string).
    expect_error(
        "empty-status",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"","trust":"checked"}]}"#,
        "6402",
    );
    // 16. id_hash exceeding the §7.5 length cap.
    {
        let mut long = String::new();
        long.push_str("{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{\"id\":\"Bank::f::subtype-range::0\",\"id_hash\":\"");
        long.push_str(&"y".repeat(600));
        long.push_str("\",\"status\":\"discharged\",\"trust\":\"checked\"}]}");
        expect_error("oversized-id-hash", long.as_bytes(), "6402");
    }
    // 17. Discharged without `trust` (§6.3: trust REQUIRED on discharged).
    expect_error(
        "discharged-without-trust",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged"}]}"#,
        "6402",
    );
    // 18. trust=proof without the `proof` object (§6.3: REQUIRED for proof).
    expect_error(
        "proof-without-proof-object",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"proof","method":"certificate","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
        "6402",
    );
    // 19. certificate method without `statement_hash` (§6.3: REQUIRED).
    expect_error(
        "certificate-without-statement-hash",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"proof","method":"certificate","proof":{"kind":"certificate","statement":"tyu.stmt/1.0","theorem":"obl_a","kernel_check":"lean-kernel"}}]}"#,
        "6402",
    );
}

/// Closed-registry violations: an unknown `method` or `proof.kind` is E6417
/// (fail-closed, §6.3) — never silently tolerated.
#[test]
fn unknown_registry_members_are_e6417() {
    expect_error(
        "unknown-method",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"proof","method":"quantum","proof":{"kind":"certificate","statement":"tyu.stmt/1.0","theorem":"obl_a","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]}"#,
        "6417",
    );
    expect_error(
        "unknown-proof-kind",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"proof","method":"certificate","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","proof":{"kind":"wizardry","statement":"tyu.stmt/1.0"}}]}"#,
        "6417",
    );
    expect_error(
        "unknown-trust",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"hyperproof"}]}"#,
        "6402",
    );
}

/// Valid-but-mismatched files: unknown ids, hash-disagreements, and
/// FR-5-stale identities must FAIL CLOSED — the build succeeds and every
/// check is retained.
#[test]
fn mismatched_verdicts_fail_closed_to_more_checking() {
    // Baseline trap count under an empty (valid) verdicts file: the fixture's
    // only subtype site is f's C1 param check (a ⊤ caller input — the engine
    // cannot discharge it), 1 site × 2 TrapIfFalse = 2 trap sites.
    expect_keep_all_checks("empty-v2", HEADER.as_bytes(), 2);

    // Unknown id (matches no obligation).
    expect_keep_all_checks(
        "unknown-id",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::nope::subtype-range::9","id_hash":"1234567890abcdef","status":"discharged","trust":"checked","method":"interval"}]}"#,
        2,
    );

    // Correct id, wrong hash — stale, fail-closed.
    expect_keep_all_checks(
        "wrong-hash",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"checked","method":"interval"}]}"#,
        2,
    );

    // FR-5 (P7.3): the file's identity pair disagrees with the build —
    // every record is stale, checks retained.
    expect_keep_all_checks(
        "wrong-target-identity",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"armv7m-unknown-none","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"checked","method":"interval"}]}"#,
        2,
    );

    // Duplicated records: deterministic first-match wins; both remain matched
    // (no staleness), neither discharges a site with a wrong hash on the
    // correct record.
    expect_keep_all_checks(
        "duplicate-wrong-hash",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"checked","method":"interval"},{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"assumed","trust":"assumed","justification":"x"}]}"#,
        2,
    );

    // Additive/unknown top-level keys are tolerated (schema grows additively).
    expect_keep_all_checks(
        "additive-keys",
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","future_extra":{"a":1},"verdicts":[],"stale_verdicts":0}"#,
        2,
    );
}

/// An *unrecognized producer* (P2 Q7 / §Q6) cannot mint `proof` trust: the
/// records downgrade to `assumed` (checks retained), and the memory remains
/// closed (id+hash) so the site stays open.
#[test]
fn unrecognized_producer_downgrades_proof_to_assumed() {
    expect_keep_all_checks(
        "unrecognized-producer",
        br#"{"schema":"tyu.verdicts/v2","certifier":{"class":"port","name":"mystery","recognition":"tyu-port/unknown/9","tool":{"name":"m","version":"0"},"toolchain":"lean4:9.9.9"},"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-linux-gnu","model_semantics":"unmodeled","verdicts":[{"id":"Bank::f::subtype-range::0","id_hash":"0000000000000000","status":"discharged","trust":"proof","method":"certificate","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","proof":{"kind":"certificate","statement":"tyu.stmt/1.0","theorem":"obl_a","kernel_check":"lean-kernel+lean4checker","file":"proofs/Bank.lean"}}]}"#,
        2,
    );
}

/// P8.2: an explicitly-`open` file record's `witness.reason` (the harvest
/// closure's `assumption-unresolved: <dep>` flip) survives consumption —
/// the site stays open (checks retained, fail-closed) AND the echo's
/// `open_reasons` carries the witness, so the report row says WHY.
#[test]
fn open_record_witness_survives_to_the_echo() {
    let dir = fresh_dir("open-witness");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, MOD).unwrap();
    // The site's real `(id, id_hash)` (a mismatched hash is fail-closed
    // stale — the record would never be consulted and the witness lost).
    let obl_dir = dir.join("obl");
    fs::create_dir_all(&obl_dir).unwrap();
    let obl = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", obl_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .expect("obl pass");
    assert!(obl.status.success());
    let set = verifier::codec::read_obl(&fs::read(obl_dir.join("Bank.obl.json")).unwrap()).unwrap();
    let site = set
        .obligations
        .iter()
        .find(|o| o.id == "Bank::f::subtype-range::0")
        .expect("C1 site present");
    let doc = format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{{\"id\":\"{}\",\"id_hash\":\"{}\",\"status\":\"open\",\"trust\":\"open\",\"witness\":{{\"reason\":\"assumption-unresolved: Math::op::contract-pre::0\"}}}}]}}",
        site.id, site.id_hash
    );
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let vf = dir.join("v.json");
    fs::write(&vf, doc).unwrap();
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
    assert!(
        output.status.success(),
        "an open record must fail closed to more checking, not fail the build"
    );
    // The echo carries the witness in open_reasons (drives the report row).
    let echo =
        verifier::verdict::read_echo(&fs::read(out_dir.join("Bank.verdicts.inTree.json")).unwrap())
            .unwrap();
    assert!(
        echo.open_reasons
            .iter()
            .any(|r| r.reason == "assumption-unresolved: Math::op::contract-pre::0"),
        "the open-record witness must ride the echo's open_reasons: {:?}",
        echo.open_reasons
    );
    // No record closes the site: the checks are retained.
    assert!(echo.verdicts.records.is_empty());
}

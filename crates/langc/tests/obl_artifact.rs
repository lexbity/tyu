//! Obligation-artifact behavior tests (static-verification.md slice P2).
//!
//! - `--emit=obligations` produces `<Module>.obl.json`, byte-identical to the
//!   committed goldens in `test-goldens/obl/` (one per site class: param,
//!   cast, return, and the full Bank fixture);
//! - the write is deterministic (FR-17): two runs are byte-identical;
//! - `--emit=obj --write-obl` writes the identical artifact beside the
//!   object, while the default `--emit=obj` writes nothing (FR-22: the fast
//!   path is unchanged);
//! - artifacts are suppressed on typecheck failure (no partial outputs).
//!
//! Bless a deliberate schema change with TYU_BLESS_OBL_GOLDEN=1 and review
//! the diff; a structural change MUST bump `OBL_SCHEMA` (owner doc
//! `verification-obligations.md` §6).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use verifier::codec::read_obl;
use verifier::model::{Formula, Kind, Oel, Provenance};

fn langc_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_langc"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-obl-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Compile `source` with `--emit=obligations` into a fresh dir; returns the
/// artifact filename (`<Module>.obl.json`) and bytes on success.
fn emit_obligations(tag: &str, source: &str) -> (String, Vec<u8>) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("in.mod");
    fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .expect("langc invocation");
    assert!(
        output.status.success(),
        "langc --emit=obligations failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Module name = the declared module, not the file stem.
    let module = if source.contains("module Bank;") {
        "Bank"
    } else if source.contains("module P;") {
        "P"
    } else if source.contains("module C;") {
        "C"
    } else {
        "R"
    };
    let name = format!("{module}.obl.json");
    let bytes = fs::read(out_dir.join(&name)).unwrap_or_else(|e| {
        panic!("langc did not produce {name}: {e}");
    });
    let _ = fs::remove_dir_all(&dir);
    (name, bytes)
}

fn golden_bytes(name: &str) -> Vec<u8> {
    fs::read(workspace_root().join("test-goldens/obl").join(name)).unwrap_or_else(|e| {
        panic!(
            "missing OBL golden test-goldens/obl/{name}: {e}; \
             rerun with TYU_BLESS_OBL_GOLDEN=1",
        )
    })
}

fn assert_matches_golden(tag: &str, source: &str, golden: &str) {
    let (_name, bytes) = emit_obligations(tag, source);
    if std::env::var_os("TYU_BLESS_OBL_GOLDEN").is_some() {
        fs::write(workspace_root().join("test-goldens/obl").join(golden), &bytes).unwrap();
        return;
    }
    assert_eq!(
        golden_bytes(golden),
        bytes,
        "obl artifact for {tag} drifted from test-goldens/obl/{golden}"
    );
}

const BANK_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
subtype Counter = i64 range 0..1000000;

: clamp ( i64 -- Percent )
  dup 100 > [ drop 100 ] [ ] if
  dup 0 < [ drop 0 ] [ ] if
  as Percent
;

: bounded_inc ( Percent -- Percent )
  1 + as Percent
;

: main ( -- Counter )
  50 as Percent bounded_inc as Counter
;
end;
";

const PARAM_MOD: &str = "\
module P;
subtype Percent = i64 range 0..100;
: f ( Percent -- i64 )
  drop 0
;
end;
";

const CAST_MOD: &str = "\
module C;
subtype Percent = i64 range 0..100;
: f ( i64 -- i64 )
  as Percent drop 0
;
end;
";

const RETURN_MOD: &str = "\
module R;
subtype Percent = i64 range 0..100;
: f ( Percent -- Percent )
;
end;
";

// ---------------------------------------------------------------------------
// Golden conformance (exact bytes)
// ---------------------------------------------------------------------------

#[test]
fn bank_artifact_matches_golden_exact_bytes() {
    assert_matches_golden("bank", BANK_MOD, "bank.obl.json");
}

#[test]
fn param_site_artifact_matches_golden() {
    assert_matches_golden("param", PARAM_MOD, "param.obl.json");
}

#[test]
fn cast_site_artifact_matches_golden() {
    assert_matches_golden("cast", CAST_MOD, "cast.obl.json");
}

#[test]
fn return_site_artifact_matches_golden() {
    assert_matches_golden("return", RETURN_MOD, "return.obl.json");
}

// ---------------------------------------------------------------------------
// Site-class semantics (parsed), on top of the exact-bytes goldens
// ---------------------------------------------------------------------------

#[test]
fn site_classes_and_provenance_semantics() {
    let (_n, bytes) = emit_obligations("bank-sem", BANK_MOD);
    let set = read_obl(&bytes).expect("artifact must round-trip through the codec");
    assert_eq!(set.module, "Bank");
    assert_eq!(set.obligations.len(), 8);
    for o in &set.obligations {
        assert_eq!(o.kind, Kind::SubtypeRange);
    }

    let by_id = |needle: &str| {
        set.obligations
            .iter()
            .find(|o| o.id == needle)
            .unwrap_or_else(|| panic!("missing obligation {needle}"))
    };

    // C1: bounded_inc takes a Percent — callee-entry param site, direct.
    let c1 = by_id("Bank::bounded_inc::subtype-range::0");
    assert_eq!(c1.provenance, Provenance::Direct);
    assert_eq!(c1.site.span, verifier::model::SpanInfo { line: 0, col: 0 });
    assert_eq!(
        c1.formula,
        Formula::InRange {
            value: Oel::Var {
                name: "in.0".to_string(),
            },
            lo: 0,
            hi: 100,
        }
    );

    // C2: main returns a Counter — epilogue return site, direct.
    let c2 = by_id("Bank::main::subtype-range::2");
    assert_eq!(c2.provenance, Provenance::Direct);
    assert_eq!(
        c2.formula,
        Formula::InRange {
            value: Oel::Var {
                name: "out.0".to_string(),
            },
            lo: 0,
            hi: 1000000,
        }
    );

    // C3: `as Percent` casts carry Cast provenance with an opaque $top operand
    // and a real source span (v1 — P5's interval engine replaces $top).
    let c3 = by_id("Bank::clamp::subtype-range::0");
    assert_eq!(c3.provenance, Provenance::Opaque);
    assert!(c3.site.span.line >= 7, "cast site carries a real line");
    assert_eq!(
        c3.formula,
        Formula::InRange {
            value: Oel::Cast {
                from: "i64".to_string(),
                to: "Percent".to_string(),
                arg: Box::new(Oel::Var {
                    name: "$top".to_string(),
                }),
            },
            lo: 0,
            hi: 100,
        }
    );

    // q3: ids never derive from spans — the clamps' occurrences are in IR
    // order (cast then return), each with a distinct canonical id.
    assert_eq!(by_id("Bank::clamp::subtype-range::0").site.occurrence, 0);
    assert_eq!(by_id("Bank::clamp::subtype-range::1").site.occurrence, 1);
    assert_eq!(
        by_id("Bank::clamp::subtype-range::0").site.word, "clamp",
        "site.word is the canonical word name"
    );
}

// ---------------------------------------------------------------------------
// Determinism (FR-17) and fast-path preservation (FR-22)
// ---------------------------------------------------------------------------

#[test]
fn two_runs_are_byte_identical() {
    let dir = fresh_dir("det");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let a = dir.join("a");
    let b = dir.join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    for out in [&a, &b] {
        let st = Command::new(langc_exe())
            .arg("--emit=obligations")
            .arg(format!("--out-dir={}", out.display()))
            .arg(mod_path.to_str().unwrap())
            .status()
            .unwrap();
        assert!(st.success());
    }
    let bytes_a = fs::read(a.join("Bank.obl.json")).unwrap();
    let bytes_b = fs::read(b.join("Bank.obl.json")).unwrap();
    assert_eq!(bytes_a, bytes_b, "obl.json must be byte-identical (FR-17)");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_obl_with_obj_matches_obligations_mode() {
    let dir = fresh_dir("writeobl");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let st = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg(mod_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(st.success(), "--emit=obj --write-obl must succeed");
    let artifact = fs::read(out_dir.join("Bank.obl.json"))
        .expect("--write-obl must write Bank.obl.json");
    let (_n, obl_mode) = emit_obligations("writeobl-ref", BANK_MOD);
    assert_eq!(obl_mode, artifact, "--write-obl artifact must match --emit=obligations");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn default_obj_writes_no_artifact() {
    let dir = fresh_dir("fastpath");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let st = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(st.success());
    assert!(
        !out_dir.join("Bank.obl.json").exists(),
        "default --emit=obj must NOT write an artifact (fast path, FR-22)"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn typecheck_failure_writes_no_artifact() {
    // P.mod declares a subtype param but the body is invalid: the artifact
    // must not appear (no partial outputs on failure).
    let dir = fresh_dir("fail");
    let mod_path = dir.join("P.mod");
    fs::write(
        &mod_path,
        "module P;\nsubtype Percent = i64 range 0..100;\n: f ( Percent -- i64 )\n  undefined_word\n;\nend;\n",
    )
    .unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !out_dir.join("P.obl.json").exists(),
        "failed compile must leave no artifact behind"
    );
    let _ = fs::remove_dir_all(&dir);
}
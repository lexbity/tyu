//! Verification-report composition tests (static-verification.md slice P3).
//!
//! Every `tyu build` writes `<out_dir>/verify-report.json` (§6.5, P3 v1
//! fields): per-module class accounting, per-context stack-budget verdicts
//! (§7.3), the open list, and the trusted descriptor assumptions. Two builds
//! over an unchanged tree are byte-identical (FR-17).

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
    // CARGO_BIN_EXE_ is set for integration tests of the same package.
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-verify-report-{tag}-{}-{}",
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

fn build(
    tag: &str,
    extra: &[&str],
    source: &str,
) -> (PathBuf, String) {
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
    let output = cmd.output().expect("tyu invocation");
    assert!(
        output.status.success(),
        "tyu build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read(out_dir.join("verify-report.json"))
        .expect("verify-report.json must exist for a produced image");
    let _ = fs::remove_dir_all(&dir);
    (out_dir, String::from_utf8(report).expect("report is UTF-8"))
}

#[test]
fn metal_platform_build_reports_discharged_main_context() {
    // A real platform build: N_main is *derived* from the metal runtime's own
    // data-stack geometry (131072 bytes / 8 slot_bytes = 16384 slots —
    // amended §6.4: declared nowhere). Main's peak (3) fits → the context
    // verdict is "discharged" and the derived geometry is listed in
    // `assumptions_trusted` with kind "runtime" (checked, abi-hash-covered).
    let (_, report) = build(
        "metal",
        &[
            "--target=x86_64-unknown-none",
            "--platform=x86_64-unknown-none",
        ],
        BANK_MOD,
    );
    let v: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(v["schema"], "tyu.verify-report/v1");
    assert_eq!(v["policy"], "open-ok");
    let main = &v["contexts"]["stack"]["main"];
    assert_eq!(main["verdict"], "discharged");
    assert_eq!(main["budget"], 16384);
    assert_eq!(v["contexts"]["stack"]["guards"], "retained");

    // Per-module class accounting: Bank carries 4 subtype-range obligations —
    // bounded_inc C1+C3+C2, main C3. P5's interval engine discharges
    // bounded_inc's C2 return (the body cast narrows its value) and main's
    // C3 cast (operand [50,50]); the two ⊤-operand sites stay open.
    let classes = &v["modules"][0]["classes"];
    assert_eq!(classes["subtype-range"]["total"], 4);
    assert_eq!(classes["subtype-range"]["open"], 2);
    assert_eq!(classes["subtype-range"]["discharged"], 2);
    assert_eq!(classes["mmio-bounds"]["total"], 0);

    // The assumptions list carries the derived geometry that drove the
    // discharge (kind "runtime", not a declared descriptor grant).
    let entries = v["assumptions_trusted"].as_array().unwrap();
    assert!(
        entries.iter().any(|a| {
            a["kind"] == "runtime"
                && a["what"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("derived N_main=16384")
        }),
        "derived geometry must be listed: {entries:?}"
    );

    // Open obligations all land in the open list with ids.
    let open = v["open"].as_array().unwrap();
    assert!(!open.is_empty());
    assert!(
        open.iter().any(|o| o["kind"] == "subtype-range")
    );
}

#[test]
fn platformless_hosted_build_degrades_to_open_main_context() {
    // The plain hosted runtime (no platform pack) defines no
    // `__lang_ds_base`/`__lang_ds_limit` symbols in its runtime object, so
    // N_main cannot be derived → `stack-budget(main)` is open (fail-closed:
    // absence can only ever cause more checking, never less), and the report
    // is still written. Contrast the metal test: a runtime that exports its
    // geometry gets the discharge with no declaration anywhere.
    let (_, report) = build("noplatform", &["--target=x86_64-unknown-linux-gnu"], BANK_MOD);
    let v: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(v["contexts"]["stack"]["main"]["budget"], 0);
    assert_eq!(v["contexts"]["stack"]["main"]["verdict"], "open");
    // The declared ISR grant defaults to 32 with no pack (FR-10).
    assert_eq!(v["contexts"]["stack"]["isr"]["budget"], 32);
    // No descriptor facts were used → no trusted assumptions.
    assert!(v["assumptions_trusted"].as_array().unwrap().is_empty());
}

#[test]
fn contract_sites_are_reported_retained_under_module_loading() {
    // FR-21 (report leg): the default tyu build resolves every feature
    // (all-features-on fallback), so `module-loading` is on. langc then
    // force-opens every contract site — a dynamic export is a runtime
    // surface no build-time discharge may remove — and the report names
    // those sites `retained` with the reason, so "why is this open" has a
    // policy answer.
    const CONTRACT_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;
: bounded_inc ( Percent -- Percent )
  needs [ pct-in-range ]
  1 + as Percent ;
: main ( -- i64 )
  50 as Percent bounded_inc as i64 ;
end;
";
    let (_, report) = build("retained", &["--target=x86_64-unknown-linux-gnu"], CONTRACT_MOD);
    let v: serde_json::Value = serde_json::from_str(&report).unwrap();
    let retained = v["retained"].as_array().unwrap();
    assert!(!retained.is_empty(), "module-loading build must retain contract sites");
    for r in retained {
        assert_eq!(r["reason"], "dynamic export (module-loading)");
        assert!(r["id"].as_str().unwrap().contains("contract-"));
    }
    // The contract obligations are open (kept), and the report's honesty
    // block counts the emitted contract traps.
    assert!(
        v["emitted_checks"]["contract"].as_u64().unwrap() >= 1,
        "the retained contract check is present in the object"
    );
}

#[test]
fn report_is_byte_deterministic_across_builds() {
    // FR-17: identical tree → identical report bytes (no timestamps, no
    // iteration-order output).
    let dir = fresh_dir("det");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let mut reports = Vec::new();
    for i in 0..2 {
        let out_dir = dir.join(format!("out{i}"));
        fs::create_dir_all(&out_dir).unwrap();
        let status = Command::new(tyu_exe())
            .arg("build")
            .arg("--target=x86_64-unknown-none")
            .arg("--platform=x86_64-unknown-none")
            .arg(format!("--out-dir={}", out_dir.display()))
            .arg(mod_path.to_str().unwrap())
            .status()
            .unwrap();
        assert!(status.success());
        reports.push(fs::read(out_dir.join("verify-report.json")).unwrap());
    }
    assert_eq!(reports[0], reports[1], "verify-report.json must be byte-identical (FR-17)");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn oboligation_artifact_rides_beside_each_object() {
    // P3: every module compile writes its `<Module>-<fp>.obl.json` artifact
    // (the source of the report's accounting), source-keyed like the object.
    // (the source of the report's accounting), source-keyed like the object.
    let dir = fresh_dir("obl");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let status = Command::new(tyu_exe())
        .arg("build")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    let obl_files: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(".obl.json")
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(obl_files.len(), 1, "one module → one artifact: {obl_files:?}");
    assert!(obl_files[0].starts_with("Bank-"));
    let _ = fs::remove_dir_all(&dir);
}

/// An image with an ISR handler reports the ISR context: the handler count,
/// the max peak over handlers, the N_isr grant, and the discharged verdict
/// (E5030 already proved every handler fits its grant against the same
/// file — §7.3 records the exact check). Both grants are listed as trusted.
#[test]
fn isr_handler_image_reports_isr_context_and_both_grants() {
    const ISR_MOD: &str = "\
module App;
@interrupt(TIMER0) : isr ( -- )\n\
  0\n\
  dup dup dup dup dup dup dup dup dup dup\n\
  dup dup dup dup dup dup dup dup dup dup\n\
  drop drop drop drop drop drop drop drop drop drop\n\
  drop drop drop drop drop drop drop drop drop drop\n\
  drop\n\
;\n\
: main ( -- i64 ) 0 ;\n\
end;\n";
    let (_, report) = build(
        "isr",
        &[
            "--target=x86_64-unknown-none",
            "--platform=x86_64-unknown-none",
        ],
        ISR_MOD,
    );
    let v: serde_json::Value = serde_json::from_str(&report).unwrap();
    let isr = &v["contexts"]["stack"]["isr"];
    assert_eq!(isr["handlers"], 1);
    assert!(isr["max_high"].as_u64().unwrap() <= 32);
    assert_eq!(isr["budget"], 32);
    assert_eq!(isr["verdict"], "discharged");
    // Both budget facts used in discharges are listed: the derived main
    // geometry (kind "runtime") and the declared ISR grant (kind
    // "descriptor", trusted — T2).
    let entries = v["assumptions_trusted"].as_array().unwrap();
    assert!(
        entries.iter().any(|a| {
            a["kind"] == "runtime"
                && a["what"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("derived N_main=16384")
        }),
        "derived main geometry must be listed: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|a| a["kind"] == "descriptor" && a["what"] == "isr_stack_slots=32"),
        "isr grant must be listed as trusted once used in a discharge: {entries:?}"
    );
}
//! Slice P7 — the two-pass guard-elision flow (`tyu build --elide-stack-guards`).
//!
//! The story is: extraction (pass 1, `--emit=obligations`, no codegen) →
//! image verdict (the *same* `main_context` accounting the report runs,
//! against the runtime binary's own DS geometry — `N_main` derived, never
//! declared) → one flag for the whole image (pass 2 forwards
//! `--elide-ds-guards` to every module or none — FR-11 per-image atomicity,
//! never per-word) → a durable, validated image-verdicts record (E6415 on a
//! corrupt record) → a report whose `contexts.stack.guards` and
//! `emitted_checks.data_stack_guards` name what the object actually
//! contains (FR-16).
//!
//! Geometry note (§6.4 amendment): only runtimes that export the DS
//! geometry symbols (`__lang_ds_base`/`__lang_ds_limit`) can discharge
//! `stack-budget(main)`. The *plain hosted* runtime does not (keeps guards
//! — the fail-closed path); the metal x86_64 runtime does (its 131072-byte
//! DS reservation, `N_main = 16384`). Elision is static-only (a dynamic
//! image has no single derived geometry).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-elide-two-pass-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const FINITE_MOD: &str = "\
module M;
: main ( -- i64 )
  1 2 + 3 + 4 + ;
export { main } ;
end;
";

const RECURSIVE_MOD: &str = "\
module M;
: main ( -- i64 )
  main ;
export { main } ;
end;
";

/// The canonical embedded event loop: *diverging* (a `loop` — DIVERGE is
/// set) but *finite* high (`loop` bodies are net-zero, §3.2). §3.2's
/// correction: DIVERGE is NOT the elision criterion.
const EVENT_LOOP_MOD: &str = "\
module M;
: poll ( -- )
  1 drop ;
: event-loop ( -- )
  [ poll ] loop ;
: main ( -- i64 )
  event-loop 0 ;
export { main } ;
end;
";

/// Metal-static build args (the geometry-exporting runtime).
const METAL: &[&str] = &[
    "--target=x86_64-unknown-none",
    "--platform=x86_64-unknown-none",
    "--mode=static",
];

fn build(tag: &str, extra: &[&str], source: &str) -> (PathBuf, String, bool) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("M.mod");
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

fn report(out_dir: &Path) -> Value {
    let bytes = fs::read(out_dir.join("verify-report.json")).expect("verify-report.json present");
    serde_json::from_slice(&bytes).expect("report is JSON")
}

/// The module's re-homed object (`<Module>-<fp>.o`) in `out_dir`.
fn module_object(out_dir: &Path) -> PathBuf {
    let mut hits: Vec<PathBuf> = fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            name.starts_with("M-") && name.ends_with(".o")
        })
        .collect();
    hits.sort();
    assert!(!hits.is_empty(), "no module object in {}", out_dir.display());
    hits.remove(0)
}

/// `__stack_overflow` link-time references in the module object — the
/// compiler-emitted guard branches (`ja __stack_overflow`), which appear as
/// relocations until the final link.
fn stack_overflow_refs(obj: &Path) -> usize {
    let out = Command::new("objdump")
        .args(["-dr", obj.to_str().unwrap()])
        .output()
        .expect("objdump");
    let text = String::from_utf8_lossy(&out.stdout);
    text.matches("__stack_overflow").count()
}

#[test]
fn finite_main_elides_guards_on_metal() {
    let (out_dir, stderr, ok) = build("finite_elided", &[METAL, &["--elide-stack-guards"]].concat(), FINITE_MOD);
    assert!(ok, "elided metal build must succeed:\n{stderr}");
    let r = report(&out_dir);
    assert_eq!(r["contexts"]["stack"]["guards"], "elided");
    assert_eq!(r["emitted_checks"]["data_stack_guards"], false);
    assert_eq!(r["contexts"]["stack"]["main"]["verdict"], "discharged");
    assert_eq!(r["contexts"]["stack"]["main"]["high"], 2);
    // No guard relocations in the module object (the disassembly gate).
    assert_eq!(
        stack_overflow_refs(&module_object(&out_dir)),
        0,
        "elided module object must carry no __stack_overflow references"
    );
    // The durable decision record agrees with the report.
    let record = fs::read_to_string(out_dir.join(".tyu-verify/image-verdicts.json")).unwrap();
    assert!(record.contains("\"guards\":\"elided\""));
    // Pass-1 scratch dir is discarded (the scratch+rehome discipline).
    assert_no_pass1_scratch(&out_dir);
    // The guarded build of the same source keeps guards (FR-11 control).
    let (gout, _, gok) = build("finite_guarded", METAL, FINITE_MOD);
    assert!(gok);
    assert_eq!(report(&gout)["contexts"]["stack"]["guards"], "retained");
    assert!(stack_overflow_refs(&module_object(&gout)) > 0);
}

#[test]
fn recursive_main_keeps_guards_with_open_reason() {
    let (out_dir, stderr, ok) = build("recursive", &[METAL, &["--elide-stack-guards"]].concat(), RECURSIVE_MOD);
    assert!(ok, "top-main build must still succeed (guards retained, not an error):\n{stderr}");
    let r = report(&out_dir);
    assert_eq!(r["contexts"]["stack"]["guards"], "retained");
    assert_eq!(r["emitted_checks"]["data_stack_guards"], true);
    assert_eq!(r["contexts"]["stack"]["main"]["top"], true, "recursive main is high = ⊤");
    assert_eq!(r["contexts"]["stack"]["main"]["verdict"], "open");
    // Guards present in the object despite the request — elision refused.
    assert!(stack_overflow_refs(&module_object(&out_dir)) > 0);
}

#[test]
fn event_loop_diverges_but_elides() {
    // §3.2 correction pinned: a diverging-but-finite event loop (`[ poll ]
    // loop`) has finite `high`, so the image verdict discharges and elision
    // is allowed — DIVERGE is not the elision criterion.
    let (out_dir, stderr, ok) = build("evloop", &[METAL, &["--elide-stack-guards"]].concat(), EVENT_LOOP_MOD);
    assert!(ok, "event-loop elided build must succeed:\n{stderr}");
    let r = report(&out_dir);
    assert_eq!(r["contexts"]["stack"]["guards"], "elided");
    assert_eq!(r["contexts"]["stack"]["main"]["verdict"], "discharged");
    assert_eq!(r["contexts"]["stack"]["main"]["top"], false);
    assert_eq!(stack_overflow_refs(&module_object(&out_dir)), 0);
}

#[test]
fn hosted_runtime_keeps_guards_fail_closed() {
    // The plain hosted runtime exports no DS geometry symbols → `N_main`
    // unavailable → `stack-budget(main)` open → guards retained (absence can
    // only cause MORE checking; §7.5). This is the *documented* fail-closed
    // path, not a bug (runtimes exposing the symbols is the §6.4 migration).
    let (out_dir, _stderr, ok) = build("hosted", &["--elide-stack-guards"], FINITE_MOD);
    assert!(ok);
    let r = report(&out_dir);
    assert_eq!(r["contexts"]["stack"]["guards"], "retained");
    assert_eq!(r["contexts"]["stack"]["main"]["budget"], 0);
    assert_eq!(r["contexts"]["stack"]["main"]["verdict"], "open");
}

#[test]
fn elide_static_only_and_requires_verify_on() {
    // Dynamic images have no single derived geometry — refused loudly.
    let (_, stderr, ok) = build("dyn", &["--target=x86_64-unknown-none", "--platform=x86_64-unknown-none", "--elide-stack-guards"], FINITE_MOD);
    assert!(!ok);
    assert!(stderr.contains("static"), "dynamic elision refused: {stderr}");
    // `--verify=off` is the legacy all-checks path — elision has no verdict
    // to discharge against; refused loudly, never a silent guarded build.
    let (_, stderr2, ok2) = build("off", &["--verify=off", "--elide-stack-guards"], FINITE_MOD);
    assert!(!ok2);
    assert!(stderr2.contains("--verify=on"), "verify=off elision refused: {stderr2}");
}

#[test]
fn pass1_failure_leaves_no_partial_outputs() {
    // A deliberate syntax error in pass 1 must fail the build with no
    // `.tyu-elide-pass1-*` scratch and no partial module artifacts.
    let dir = fresh_dir("pass1fail");
    let mod_path = dir.join("M.mod");
    fs::write(&mod_path, "module M;\n: main ( -- i64 )\n @@@ not valid ;\nend;\n").unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let status = Command::new(tyu_exe())
        .arg("build")
        .arg("--target=x86_64-unknown-none")
        .arg("--platform=x86_64-unknown-none")
        .arg("--mode=static")
        .arg("--elide-stack-guards")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .status()
        .expect("tyu invocation");
    assert!(!status.success(), "a broken module must fail the build");
    assert_no_pass1_scratch(&out_dir);
    // Runtime units are assembled before pass 1 (for the geometry) and stay;
    // the failing *module* must leave no module artifact behind.
    let leftovers: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| (n.starts_with("M-") && n.ends_with(".o")) || n.ends_with(".obl.json"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "pass-1 failure must leave no module artifacts: {leftovers:?}"
    );
}

#[test]
fn elided_and_guarded_builds_have_distinct_cache_entries() {
    // The elide decision folds into the object cache key (like the verify
    // mode): an elided-build object must never satisfy a guarded lookup —
    // the object would then disagree with the report's guard field (FR-16).
    let dir = fresh_dir("cache");
    let mod_path = dir.join("M.mod");
    fs::write(&mod_path, FINITE_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let run = |extra: &[&str]| {
        let mut cmd = Command::new(tyu_exe());
        cmd.arg("build")
            .arg("--target=x86_64-unknown-none")
            .arg("--platform=x86_64-unknown-none")
            .arg("--mode=static")
            .arg(format!("--out-dir={}", out_dir.display()))
            .arg(mod_path.to_str().unwrap());
        for a in extra {
            cmd.arg(a);
        }
        let out = cmd.output().expect("tyu invocation");
        assert!(out.status.success(), "build failed: {}", String::from_utf8_lossy(&out.stderr));
    };

    run(&["--elide-stack-guards"]);
    let elided_objs: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("M-") && n.ends_with(".o"))
        .collect();
    assert_eq!(elided_objs.len(), 1);
    let elided_obj = out_dir.join(&elided_objs[0]);
    assert_eq!(stack_overflow_refs(&elided_obj), 0, "elided object has no guards");

    run(&["--elide-stack-guards"]);
    let elided_objs2: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("M-") && n.ends_with(".o"))
        .collect();
    assert_eq!(
        elided_objs, elided_objs2,
        "second identical elided build must cache-hit (same fingerprint)"
    );

    // A guarded build of the same source must NOT satisfy the elided
    // fingerprint (the elide decision folds into the cache key); a third
    // elided build therefore recompiles — and its object MUST again carry no
    // guards (FR-16: the object always agrees with the report).
    run(&[]);
    let guarded = module_object(&out_dir);
    assert!(stack_overflow_refs(&guarded) > 0, "guarded object has guards");
    let _ = fs::remove_dir_all(&dir);

    let (out_dir2, _stderr2, ok2) = build("cache_after_guarded", &[METAL, &["--elide-stack-guards"]].concat(), FINITE_MOD);
    assert!(ok2);
    let r2 = report(&out_dir2);
    assert_eq!(r2["contexts"]["stack"]["guards"], "elided");
    assert_eq!(
        stack_overflow_refs(&module_object(&out_dir2)),
        0,
        "a re-elided build after a guarded build must still elide (never a stale guarded object)"
    );
    let _ = fs::remove_dir_all(&out_dir2);
}

fn assert_no_pass1_scratch(out_dir: &Path) {
    let leftovers: Vec<String> = fs::read_dir(out_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.filter_map(|e| e.ok()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tyu-elide-pass1"))
        .collect();
    assert!(leftovers.is_empty(), "pass-1 scratch must be discarded: {leftovers:?}");
}
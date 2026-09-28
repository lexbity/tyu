//! The unmodeled-bundle pipeline (PLAN-VERIFY-3 P12.1, §Q15(c)/§11.9):
//! a bundle without model semantics is *loadable, runnable, testable — and
//! uncertifiable*. End to end:
//!
//! - the lint warns (desirable-but-optional) but never errors;
//! - `--verify-policy=proven` fails the build closed (E6510,
//!   `E_MODEL_UNMODELED`) — a proven claim is meaningless against a bundle
//!   without model semantics;
//! - `--verify-policy=no-open` succeeds and the whole chain records the
//!   §Q15 honest default: artifact, verdicts echo, `verify_manifest`
//!   record, and report all carry `model: "unmodeled"`;
//! - `tyu deploy --verify-policy=proven` rejects the unmodeled module at
//!   the deploy's model gate (E6510 — the root-manifest gate fires before
//!   the import-graph walk, which itself rejects unmodeled callees).
//!
//! The modeled-packs mirror image is `model_id_flow.rs` (§Q3: the declared
//! id reaches every evidence surface); this file pins the *absence* path —
//! identity is never invented.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tyu::platform::lint_pack;

use common::{big_stack_static_build, copy_dir, fresh_dir, workspace_root, X86_NONE_ABI_HASH};

/// A schema-3 pack with NO `[model]` section: the §Q15 unmodeled bundle
/// (real metal, honest tier). The unrelated-capable lint must warn but
/// never error.
fn write_unmodeled_pack(root: &Path) {
    let pack = root.join("platforms/unmodeled-demo");
    fs::create_dir_all(&pack).unwrap();
    let manifest = format!(
        r#"[platform]
name = "unmodeled-demo"
schema = 3
compiler-interface = 1
description = "synthetic unmodeled bundle (P12.1 §Q15)"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "0x{X86_NONE_ABI_HASH:016x}"

[metal]
path = "metal"
startup = "runtime.asm"
linker = "link.ld"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[test]
rung = "untested"
"#
    );
    fs::write(pack.join("platform.toml"), manifest).unwrap();
    copy_dir(
        &workspace_root().join("platforms/x86_64-unknown-none/metal"),
        &pack.join("metal"),
    );
}

/// Run a tyu build library-level (the big-stack convention).
struct BuildAttempt {
    ok: bool,
    err_text: String,
    out_dir: PathBuf,
}

fn attempt_build(root: &Path, policy: tyu::args::VerifyPolicy) -> BuildAttempt {
    let input = root.join("Main.mod");
    let attempt = big_stack_static_build(root, "unmodeled-demo", policy, &input);
    match attempt {
        Ok(_) => BuildAttempt {
            ok: true,
            err_text: String::new(),
            out_dir: root.join("out"),
        },
        Err(e) => BuildAttempt {
            ok: false,
            err_text: e.to_string(),
            out_dir: root.join("out"),
        },
    }
}

/// The unmodeled bundle lints with the §Q15 warning, never errors.
#[test]
fn unmodeled_bundle_lints_warn_only() {
    let root = fresh_dir("lint");
    write_unmodeled_pack(&root);

    let outcome = lint_pack(&root, "unmodeled-demo", false).unwrap();
    assert!(
        outcome.errors.is_empty(),
        "unmodeled bundle must lint clean: {}",
        tyu::platform::format_lint_outcome(&outcome)
    );
    assert!(
        outcome.warnings.iter().any(|w| w.contains("unmodeled")),
        "unmodeled bundle must warn (§Q15 desirable-but-optional): {:?}",
        outcome.warnings
    );
}

/// §11.9: `proven` against an unmodeled bundle fails closed at build
/// (E6510 — a proven claim is meaningless without model semantics).
#[test]
fn proven_build_fails_closed_against_unmodeled() {
    common::ensure_bins();
    let root = fresh_dir("proven");
    write_unmodeled_pack(&root);
    fs::write(root.join("Main.mod"), common::BUNDLE_MAIN_MOD).unwrap();

    let attempt = attempt_build(&root, tyu::args::VerifyPolicy::Proven);
    assert!(!attempt.ok, "proven × unmodeled build must fail (E6510)");
    assert!(
        attempt.err_text.contains("E6510"),
        "failure must name E6510, got: {}",
        attempt.err_text
    );
}

/// §11.9: `no-open` admits the unmodeled bundle and the whole chain records
/// `model: "unmodeled"` — artifact, verdicts echo, and the packed
/// `verify_manifest` record (§Q15's honest default; identity is never
/// invented).
#[test]
fn no_open_build_succeeds_and_records_unmodeled() {
    common::ensure_bins();
    let root = fresh_dir("noopen");
    write_unmodeled_pack(&root);
    fs::write(root.join("Main.mod"), common::BUNDLE_MAIN_MOD).unwrap();

    let attempt = attempt_build(&root, tyu::args::VerifyPolicy::NoOpen);
    assert!(
        attempt.ok,
        "no-open × unmodeled build must succeed: {}",
        attempt.err_text
    );
    let out = &attempt.out_dir;

    // Artifact leg.
    let obl_path = fs::read_dir(out)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            let s = p.to_string_lossy();
            s.ends_with(".obl.json") && s.contains("Main")
        })
        .expect("re-homed Main.obl.json present");
    let set = verifier::codec::read_obl(&fs::read(&obl_path).unwrap()).unwrap();
    assert_eq!(
        set.model_semantics,
        verifier::model::MODEL_UNMODELED,
        "artifact leg"
    );

    // Verdicts leg.
    let slots: Vec<_> = fs::read_dir(out.join(".tyu-verify"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    let echo_path = slots
        .iter()
        .find(|p| p.to_string_lossy().ends_with(".verdicts.json"))
        .expect("verdicts echo slot present");
    let verdicts =
        verifier::verdict::read_verdicts(&fs::read(echo_path).unwrap()).expect("echo parses");
    assert_eq!(
        verdicts.model_semantics,
        verifier::model::MODEL_UNMODELED,
        "verdicts leg"
    );

    // Manifest leg: the tyu.vm/1 summary.
    let vm_path = out.join(".tyu-verify").join("Main.vm.json");
    let spec = lmod_pack::verify::verify_manifest_from_json(
        &fs::read_to_string(&vm_path).expect("vm summary present"),
    )
    .unwrap();
    assert_eq!(
        spec.model,
        verifier::model::MODEL_UNMODELED,
        "vm summary leg"
    );

    // Report leg.
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("verify-report.json")).unwrap()).unwrap();
    assert_eq!(
        report["modules"][0]["model"],
        verifier::model::MODEL_UNMODELED,
        "report leg"
    );
    assert_eq!(report["modules"][0]["target"], "x86_64-unknown-none");
}

/// §11.9: `tyu deploy --verify-policy=proven` rejects an unmodeled module
/// (E6510). This e2e deploy carries no platform, so the *deploy build's*
/// model gate fires first (`enforce_policy(proven)` E6510 on the unmodeled
/// module) — the root-manifest gate, not the image-level walk. The
/// import-graph walk's own E6510-against-unmodeled is covered by
/// `deploy_pairing::proven_rejects_unmodeled_callee_with_e6510`; this test
/// pins the e2e deploy path.
#[test]
fn proven_deploy_rejects_unmodeled() {
    common::ensure_bins();
    let root = fresh_dir("deploy");
    write_unmodeled_pack(&root);
    fs::write(root.join("Main.mod"), common::BUNDLE_MAIN_MOD).unwrap();

    // Host-rendered proven summary carrying the unmodeled model id (the
    // build refuses to produce one; deploy must refuse to consume one).
    let summary = r#"{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
       "target":"x86_64-unknown-none","model":"unmodeled",
       "policy":"proven","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1"},
       "candidate_ratio":0,"counts":{"proof":1,"checked":0,"assumed":0,"open":0},"obligations":[]}"#;
    let sump = root.join("summary.json");
    fs::write(&sump, summary).unwrap();
    let out_dir = root.join("out-deploy");
    fs::create_dir_all(&out_dir).unwrap();

    let output = Command::new(common::bin::resolve("tyu"))
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", workspace_root().join("sysroot").display()),
            &format!("--out-dir={}", out_dir.display()),
            "--verify-policy=proven",
            &format!("--verify-manifest={}", sump.display()),
        ])
        .arg(root.join("Main.mod"))
        .output()
        .expect("tyu deploy");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "proven deploy of an unmodeled module must fail: {text}"
    );
    assert!(
        text.contains("E6510"),
        "deploy failure must name E6510, got: {text}"
    );
}

//! Image-level deploy pairing (PLAN-VERIFY-3 P11.3, §Q7 rule 3 / FR-8):
//! `tyu deploy --verify-policy=…` walks the import graph (E6510,
//! compositional-challenger rule). `proven` rejects an unmodeled/unproven
//! callee; `no-open` admits a fully verified two-module image.
//!
//! These are the deploy-time gates exercised through the real `tyu deploy`
//! binary (tool-gated, same convention as the other deploy suites). The
//! hermetic module-level pairing logic is covered by `deploy.rs`'s unit
//! tests; this file pins the *end-to-end* behavior the plan's P11.3 gate
//! names (`deploy_pairing`).

use std::process::Command;

use tyu::test_helpers::*;

const MAIN_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\nimport Cal { cal };\n\
: main ( -- i64 ) cal drop 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn write_two_module_project(dir: &std::path::Path) {
    std::fs::write(
        dir.join("Cal.def"),
        "module Cal;\n: cal ( -- i64 ) ;\nexport { cal };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Cal.mod"),
        "module Cal;\n: cal ( -- i64 ) 5 ;\nexport { cal };\nend;\n",
    )
    .unwrap();
    std::fs::write(dir.join("Main.mod"), MAIN_MOD).unwrap();
}

fn ensure_tools() -> bool {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        return false;
    }
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "langc",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
    true
}

/// Deploy the two-module project under a policy. `root_summary` (optional)
/// supplies an explicit `--verify-manifest` for the root.
fn deploy_two_module(dir: &std::path::Path, policy: &str, root_summary: Option<&str>) -> bool {
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let mut args: Vec<String> = vec![
        "deploy".to_string(),
        "--target=x86_64-unknown-none".to_string(),
        format!("--sysroot={}", sysroot.display()),
        format!("--out-dir={}", out_dir.display()),
        format!("--verify-policy={policy}"),
    ];
    if let Some(s) = root_summary {
        let sump = dir.join("summary.json");
        std::fs::write(&sump, s).unwrap();
        args.push(format!("--verify-manifest={}", sump.display()));
    }
    args.push(dir.join("Main.mod").to_string_lossy().to_string());

    let output = Command::new(tyu_exe())
        .args(&args)
        .output()
        .expect("tyu deploy");
    if output.status.success() {
        return true;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("deploy failed:\n{text}");
    false
}

#[test]
fn no_open_admits_a_fully_verified_image() {
    if !ensure_tools() {
        return;
    }
    // Single-module e2e (a verified, manifest-carrying module deploys under
    // `no-open` and its certification package is produced). The two-module
    // PAIRING acceptance is the hermetic unit test in deploy.rs
    // (`check_image_pairing` accepts a proven+modeled callee); the full
    // two-module QEMU execution is an orthogonal runtime-path (the dynamic
    // image's harness completion is exercised by execution-tests, not by
    // this deploy gate).
    let dir = temp_dir("pair_no_open");
    std::fs::write(
        dir.join("Main.mod"),
        "module Main;\nimport platform/testio { testio.write-byte };\n\
         : main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\n\
         export { main };\nend;\n",
    )
    .unwrap();
    let root_summary = r#"{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
       "target":"x86_64-unknown-none","model":"unmodeled",
       "policy":"no-open","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1"},
       "candidate_ratio":0,"counts":{"proof":0,"checked":1,"assumed":0,"open":0},"obligations":[]}"#;
    let ok = deploy_two_module(&dir, "no-open", Some(root_summary));
    assert!(
        ok,
        "no-open deploy of a verified image must succeed (E6510 compositional rule)"
    );
    let pkg = dir.join("out/deploy/signed.lmod.tyucert");
    assert!(pkg.join("cert.json").is_file(), "cert package must exist");
    let idx_text = std::fs::read_to_string(pkg.join("cert.json")).unwrap();
    assert!(idx_text.contains("\"Main\""), "index names Main");
    assert!(
        idx_text.contains("\"no-open\""),
        "index records the no-open policy"
    );
}

#[test]
fn proven_rejects_unmodeled_callee_with_e6510() {
    if !ensure_tools() {
        return;
    }
    let dir = temp_dir("pair_proven_callee");
    write_two_module_project(&dir);
    // Root packed manifest is proven + modeled; the derived CALLEE summary
    // carries `model: unmodeled` until P12 supplies bundle model semantics —
    // the image-level walk must reject it (E6510, E_MODEL_UNMODELED).
    let root_summary = r#"{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
       "target":"x86_64-unknown-none","model":"tyu.model/x86_64-unknown-none/1",
       "policy":"proven","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1"},
       "candidate_ratio":0,"counts":{"proof":1,"checked":0,"assumed":0,"open":0},"obligations":[]}"#;
    let ok = deploy_two_module(&dir, "proven", Some(root_summary));
    assert!(
        !ok,
        "proven deploy with an unmodeled callee must fail (E6510 walking the import graph)"
    );
}

#[test]
fn proven_rejects_unproven_callee_by_name() {
    if !ensure_tools() {
        return;
    }
    let dir = temp_dir("pair_proven_callee_unproven");
    write_two_module_project(&dir);
    let root_summary = r#"{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
       "target":"x86_64-unknown-none","model":"tyu.model/x86_64-unknown-none/1",
       "policy":"proven","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1"},
       "candidate_ratio":0,"counts":{"proof":1,"checked":0,"assumed":0,"open":0},"obligations":[]}"#;
    let ok = deploy_two_module(&dir, "proven", Some(root_summary));
    assert!(
        !ok,
        "a proven caller with a callee that is not proven-and-modeled must fail (E6510)"
    );
}

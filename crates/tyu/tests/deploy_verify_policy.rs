//! Deploy-time verify-policy pairing (PLAN-VERIFY-3 P11.3, §Q7 rule 3 / FR-8).
//!
//! `tyu deploy --verify-policy=…` gates the *shipped artifact* on its
//! `verify_manifest` (E6510): a proven-required deploy rejects a module
//! without a manifest or with a below-bar declared policy, and accepts one
//! packed with a proven, modeled manifest (assembled from a `tyu.vm/1`
//! summary via `--verify-manifest`).

use std::process::Command;

use lmod::validate::Container;
use sha2::{Digest as _, Sha256};
use tyu::test_helpers::*;

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn proven_summary(model: &str, policy: &str) -> String {
    proven_summary_ratio(model, policy, 0)
}

fn proven_summary_ratio(model: &str, policy: &str, candidate_ratio: u16) -> String {
    format!(
        r#"{{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
           "target":"x86_64-unknown-none","model":"{}","policy":"{}",
           "certifier":{{"class":"port","name":"lean","recognition":"tyu-port/lean/1"}},
           "candidate_ratio":{},
           "counts":{{"proof":1,"checked":0,"assumed":0,"open":0}},
           "obligations":[{{"id":"Main::main::subtype-range::0","id_hash":7,
             "status":"discharged","trust":"proof",
             "statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]}}"#,
        model, policy, candidate_ratio
    )
}

/// Deploy `Main.mod` with the given policy/manif: returns the exit status +
/// the combined output.
fn ensure_tools() {
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
}

fn deploy_with(dir: &std::path::Path, verify_policy: Option<&str>, summary: Option<&str>) -> bool {
    deploy_with_extra(dir, verify_policy, summary, None)
}

fn deploy_with_extra(
    dir: &std::path::Path,
    verify_policy: Option<&str>,
    summary: Option<&str>,
    extra_flag: Option<&str>,
) -> bool {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        // Tools absent in this environment — skip (same convention as the
        // other deploy suites).
        return true;
    }
    ensure_tools();
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let mut args: Vec<String> = vec![
        "deploy".to_string(),
        "--target=x86_64-unknown-none".to_string(),
        format!("--sysroot={}", sysroot.display()),
        format!("--out-dir={}", out_dir.display()),
        main_mod.to_string_lossy().to_string(),
    ];
    if let Some(p) = verify_policy {
        args.insert(1, format!("--verify-policy={p}"));
    }
    if let Some(s) = summary {
        let sum = dir.join("summary.json");
        std::fs::write(&sum, s).unwrap();
        args.insert(1, format!("--verify-manifest={}", sum.display()));
    }
    if let Some(f) = extra_flag {
        args.insert(1, f.to_string());
    }

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
fn proven_requires_a_manifest() {
    let dir = temp_dir("vm_proven_missing");
    let ok = deploy_with(&dir, Some("proven"), None);
    assert!(
        !ok,
        "proven-required deploy without a manifest must fail (E6510)"
    );
}

#[test]
fn proven_with_proven_modeled_manifest_succeeds() {
    let dir = temp_dir("vm_proven_ok");
    let ok = deploy_with(
        &dir,
        Some("proven"),
        Some(&proven_summary("tyu.model/x86_64-unknown-none/1", "proven")),
    );
    assert!(
        ok,
        "proven deploy with a proven, modeled manifest must succeed"
    );

    // The packaged artifact carries the record; digest recomputes.
    let lmod_path = dir.join("out/deploy/Main.lmod");
    let bytes = std::fs::read(&lmod_path).expect("packed lmod");
    let container = Container::parse(&bytes).expect("container parses");
    let vm = lmod::verify_manifest::scan_verify_manifest(container.modinfo())
        .expect("scan")
        .expect("manifest present");
    assert_eq!(vm.policy, lmod::verify_manifest::VM_POLICY_PROVEN);
    let region = &container.modinfo()[vm.obligations_start..vm.obligations_end];
    let digest: [u8; 32] = Sha256::digest(region).into();
    assert_eq!(digest, vm.digest, "manifest digest must recompute");
}

#[test]
fn proven_rejects_below_bar_and_unmodeled() {
    // open-ok manifest under a proven requirement.
    let dir = temp_dir("vm_proven_openok");
    let ok = deploy_with(
        &dir,
        Some("proven"),
        Some(&proven_summary(
            "tyu.model/x86_64-unknown-none/1",
            "open-ok",
        )),
    );
    assert!(
        !ok,
        "open-ok manifest under proven-required must fail (E6510)"
    );

    // proven but unmodeled.
    let dir2 = temp_dir("vm_proven_unmodeled");
    let ok2 = deploy_with(
        &dir2,
        Some("proven"),
        Some(&proven_summary("unmodeled", "proven")),
    );
    assert!(
        !ok2,
        "proven manifest against an unmodeled bundle must fail (E6510)"
    );
}

#[test]
fn default_open_ok_admits_manifestless() {
    let dir = temp_dir("vm_default");
    let ok = deploy_with(&dir, None, None);
    assert!(
        ok,
        "default deploy (open-ok) must admit a manifest-less module"
    );
}

#[test]
fn proven_deploy_walks_the_import_graph() {
    // The compositional rule (§Q7 rule 3 / FR-8): a proven-required deploy
    // walks the import graph, so a two-module image is rejected when the
    // CALLEE is not proven-and-modeled — even though the root's own packed
    // manifest is proven+modeled. (Until P12 supplies bundle model
    // semantics, the derived callee summaries carry `model: unmodeled` and
    // the walk rejects them with E_MODEL_UNMODELED.)
    let dir = temp_dir("vm_pair");
    // A callee module (+ its `.def` interface, the boundary the caller's
    // subword import requires) and a caller importing it.
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
    std::fs::write(
        dir.join("Main.mod"),
        "module Main;\nimport platform/testio { testio.write-byte };\nimport Cal { cal };\n: main ( -- i64 ) cal 83 testio.write-byte drop 0 ;\nexport { main };\nend;\n",
    )
    .unwrap();
    let ok = deploy_with_pair(
        &dir,
        Some(&proven_summary("tyu.model/x86_64-unknown-none/1", "proven")),
    );
    assert!(
        !ok,
        "proven deploy of a two-module image must fail when the callee is not \
         proven-and-modeled (E6510 walking the import graph)"
    );
}

/// Like `deploy_with` but for the two-module layout (Cal.mod + Main.mod in one
/// dir); returns false on failure.
fn deploy_with_pair(dir: &std::path::Path, summary: Option<&str>) -> bool {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        return true;
    }
    ensure_tools();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let mut args: Vec<String> = vec![
        "deploy".to_string(),
        "--target=x86_64-unknown-none".to_string(),
        "--verify-policy=proven".to_string(),
        format!("--sysroot={}", sysroot.display()),
        format!("--out-dir={}", out_dir.display()),
    ];
    if let Some(s) = summary {
        let sump = dir.join("summary.json");
        std::fs::write(&sump, s).unwrap();
        args.insert(1, format!("--verify-manifest={}", sump.display()));
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
    assert!(
        text.contains("callee") || text.contains("E6510"),
        "the pairing failure must name the callee/walk:\n{text}"
    );
    false
}

#[test]
fn loader_policy_agrees_with_deploy_gate() {
    // The loader's RequireProven policy must reject the same below-bar
    // manifests the deploy gate rejects — both read the same record.
    let record = {
        let spec = lmod_pack::verify::verify_manifest_from_json(&proven_summary(
            "tyu.model/x86_64-unknown-none/1",
            "open-ok",
        ))
        .unwrap();
        lmod_pack::verify::encode_verify_manifest(&spec).unwrap()
    };
    let mut payload = b"modinfo-prefix".to_vec();
    payload.extend_from_slice(&record);
    // The payload's tail is the record; the loader's scan must parse it and
    // its policy (open-ok) must be below the RequireProven bar.
    let vm = lmod::verify_manifest::scan_verify_manifest(&payload)
        .unwrap()
        .expect("present");
    assert_eq!(vm.policy, lmod::verify_manifest::VM_POLICY_OPEN_OK);
    assert!(vm.policy != lmod::verify_manifest::VM_POLICY_PROVEN);
}

#[test]
fn proven_no_candidates_knob_gates_the_ratio() {
    // §Q10: by default a proven deploy ADMITS candidate-authored
    // certificates (kernel-checked); with --proven-no-candidates any
    // nonzero shipped ratio fails (E6510).
    let ratio_summary = proven_summary_ratio("tyu.model/x86_64-unknown-none/1", "proven", 5000);

    let dir = temp_dir("vm_ratio_default");
    assert!(
        deploy_with(&dir, Some("proven"), Some(&ratio_summary)),
        "default proven deploy must admit a candidate-authored ratio"
    );

    let dir2 = temp_dir("vm_ratio_knob");
    let ok = deploy_with_extra(
        &dir2,
        Some("proven"),
        Some(&ratio_summary),
        Some("--proven-no-candidates"),
    );
    assert!(
        !ok,
        "--proven-no-candidates must fail a proven deploy with a nonzero ratio"
    );

    // A zero ratio deploys under the knob too (nothing to object to).
    let dir3 = temp_dir("vm_ratio_knob_zero");
    let ok3 = deploy_with_extra(
        &dir3,
        Some("proven"),
        Some(&proven_summary("tyu.model/x86_64-unknown-none/1", "proven")),
        Some("--proven-no-candidates"),
    );
    assert!(ok3, "--proven-no-candidates must admit a zero ratio");
}

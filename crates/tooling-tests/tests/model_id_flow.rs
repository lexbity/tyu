//! Model-identity flow (PLAN-VERIFY-3 P12.1, §Q3/§Q15/FR-10): the
//! `[model] model_semantics` id declared by a platform pack must arrive
//! **unchanged** at every evidence-chain surface:
//!
//! pack → obligation artifact → verdicts (echo) → `verify_manifest` record →
//! certification package (cert.json), with the report carrying the same
//! per-module `(target, model)` identity (§7.3).
//!
//! The pack is synthetic (schema 3, a real model id) but its metal is the
//! real x86_64-unknown-none runtime, so the build exercises the full static
//! path. A direct langc run with no `--model-semantics` is also pinned to
//! the §Q15 honest default (`unmodeled`) — identity is never invented.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tyu::platform::lint_pack;

use common::{big_stack_static_build, copy_dir, fresh_dir, workspace_root, X86_NONE_ABI_HASH};

const MODEL_ID: &str = "tyu.model/demo-model/1";

/// The synthetic schema-3 modeled pack: real metal (so the static build
/// links), declared `[model]`, the matching `model/model.toml` artifact, and
/// the `evidence/` vector corpus the lint requires iff modeled.
fn write_modeled_pack(root: &Path) {
    let pack = root.join("platforms").join("demo-model");
    fs::create_dir_all(&pack).unwrap();
    let manifest = format!(
        r#"[platform]
name = "demo-model"
schema = 3
compiler-interface = 1
description = "synthetic modeled bundle for the identity-flow test"

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

[model]
# P12.1 (§6.7): the bundle's model-semantics identity — this exact string
# must reach the artifact, verdicts, manifest, and package unchanged.
model_semantics = "{MODEL_ID}"
mmio = "nondeterministic"
concurrency = "unmodeled"
"#
    );
    fs::write(pack.join("platform.toml"), manifest).unwrap();
    copy_dir(
        &workspace_root().join("platforms/x86_64-unknown-none/metal"),
        &pack.join("metal"),
    );
    fs::create_dir_all(pack.join("model")).unwrap();
    fs::write(
        pack.join("model/model.toml"),
        format!("[model]\nid = \"{MODEL_ID}\"\n"),
    )
    .unwrap();
    fs::create_dir_all(pack.join("evidence")).unwrap();
    fs::write(
        pack.join("evidence/vectors.json"),
        "{\"schema\":\"tyu.vec/1\",\"vectors\":[]}\n",
    )
    .unwrap();
}

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

/// Direct-langc legs: the flag is the only identity source for a direct run,
/// and the absence of a bundle keeps the §Q15 `unmodeled` default.
#[test]
fn langc_defaults_to_unmodeled_and_passes_the_flag_through() {
    common::ensure_bins();
    let root = fresh_dir("langc");
    write_modeled_pack(&root);
    let main = root.join("Main.mod");
    fs::write(&main, common::BUNDLE_MAIN_MOD).unwrap();

    // langc consumes the *compiled* descriptor (`platform.desc`, E3647
    // otherwise); tyu compiles it for pack-backed runs. Mirror the manifest
    // path so this synthetic pack gets its compiled board identity.
    tyu::platform::ensure_compiled_descriptor(
        &root.join("platforms/demo-model/platform.toml"),
        &root.join("platforms/demo-model"),
    )
    .expect("synthetic pack descriptor compiles");

    for (args, expected) in [
        (vec![], "unmodeled"),
        (vec![format!("--model-semantics={MODEL_ID}")], MODEL_ID),
    ] {
        let out_dir = root.join(format!("out{}", expected));
        fs::create_dir_all(&out_dir).unwrap();
        let mut cmd = Command::new(langc_exe());
        cmd.arg("--emit=obligations")
            .arg("--target=x86_64-unknown-none")
            .arg(format!(
                "--sysroot={}",
                workspace_root().join("sysroot").display()
            ))
            .arg(format!(
                "--platform={}",
                root.join("platforms/demo-model").display()
            ))
            .arg(format!("--out-dir={}", out_dir.display()))
            .args(&args)
            .arg(&main);
        let st = cmd.status().expect("langc run");
        assert!(st.success(), "pass-1 failed for {expected:?}");

        let obl_path = out_dir.join("Main.obl.json");
        let set = verifier::codec::read_obl(&fs::read(&obl_path).unwrap()).unwrap();
        assert_eq!(
            set.model_semantics, expected,
            "artifact model identity must be {:?}",
            expected
        );
    }
}

/// The full chain: a `tyu build` under the modeled pack stamps the pack's id
/// into the artifact, the verdicts echo, the `tyu.vm/1` summary, the packed
/// module's `verify_manifest` record, the report, and the certification
/// package — all equal to the declared id.
#[test]
fn model_id_flows_pack_to_artifact_verdicts_manifest_and_package() {
    common::ensure_bins();
    let root = fresh_dir("chain");
    write_modeled_pack(&root);
    fs::write(root.join("Main.mod"), common::BUNDLE_MAIN_MOD).unwrap();

    // The synthetic pack must lint clean: the flow asserts identity, not a
    // lint failure masking it.
    let lint = lint_pack(&root, "demo-model", false).unwrap();
    assert!(
        lint.errors.is_empty(),
        "modeled fixture pack must lint clean: {}",
        tyu::platform::format_lint_outcome(&lint)
    );

    let input = root.join("Main.mod");

    // The graph resolver parses with the language frontend, whose deep parse
    // frames exceed the 2 MiB default test-thread stack — the shared 64 MiB
    // convention (common::big_stack_static_build).
    let outcome =
        big_stack_static_build(&root, "demo-model", tyu::args::VerifyPolicy::NoOpen, &input)
            .expect("build must succeed");
    let _ = outcome;

    let out = root.join("out");

    // --- artifact leg -----------------------------------------------------
    // The obligation artifact is re-homed to `<Module>-<inputs_fp:016x>` in
    // the out dir (P6); locate it by the re-homed naming convention.
    let obl_path = fs::read_dir(&out)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            let s = p.to_string_lossy();
            s.ends_with(".obl.json") && s.contains("Main")
        })
        .expect("re-homed Main.obl.json present");
    let set = verifier::codec::read_obl(&fs::read(&obl_path).unwrap()).unwrap();
    assert_eq!(set.target, "x86_64-unknown-none");
    assert_eq!(set.model_semantics, MODEL_ID, "artifact leg");

    // --- verdicts leg (the re-homed echo slot) ----------------------------
    let slots: Vec<_> = fs::read_dir(out.join(".tyu-verify"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    let echo_path = slots
        .iter()
        .find(|p| p.to_string_lossy().contains(".verdicts.json"))
        .expect("verdicts echo slot present");
    let verdicts =
        verifier::verdict::read_verdicts(&fs::read(echo_path).unwrap()).expect("echo parses");
    assert_eq!(verdicts.model_semantics, MODEL_ID, "verdicts leg");
    assert_eq!(verdicts.target, "x86_64-unknown-none");

    // --- manifest leg: the tyu.vm/1 summary AND the packed record ---------
    let vm_path = out.join(".tyu-verify").join("Main.vm.json");
    let vm_text = fs::read_to_string(&vm_path).unwrap();
    let spec = lmod_pack::verify::verify_manifest_from_json(&vm_text).unwrap();
    assert_eq!(spec.model, MODEL_ID, "vm summary leg");

    // The packed image carries the record (no-open build embeds the root
    // summary); its `model` bytes are the declared id.
    let image = out.join("Main.lmod");
    assert!(image.exists(), "packed image present");
    let image_bytes = fs::read(&image).unwrap();
    let manifest_model: Vec<u8> = {
        let container = lmod::validate::Container::parse(&image_bytes).unwrap();
        let vm = lmod::verify_manifest::scan_verify_manifest(container.modinfo())
            .unwrap()
            .expect("verify_manifest record present in the packed module");
        assert_eq!(vm.model, MODEL_ID.as_bytes(), "manifest record leg");
        assert_eq!(vm.target, b"x86_64-unknown-none");
        vm.model.to_vec()
    };

    // --- report leg: per-module (target, model) identity (§7.3) -----------
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("verify-report.json")).unwrap()).unwrap();
    let module = &report["modules"][0];
    assert_eq!(module["model"], MODEL_ID, "report leg");
    assert_eq!(module["target"], "x86_64-unknown-none");

    // --- package leg: the certification index records the same id ---------
    let sysroot = workspace_root().join("sysroot");
    let module_bytes = image_bytes.clone();
    let image_ref = image.clone();
    let out_ref = out.clone();
    let root_ref = root.clone();
    let input_ref = input.clone();
    let sysroot_ref = sysroot.clone();
    let pkg = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tyu::cert::assemble_for_deploy(&tyu::cert::AssembleInput {
                image: &image_ref,
                image_bytes: &image_bytes,
                module_bytes: &module_bytes,
                out_dir: &out_ref,
                input: &input_ref,
                include_dirs: std::slice::from_ref(&root_ref),
                sysroot: Some(sysroot_ref.as_path()),
                policy: tyu::args::DeployVerifyPolicy::NoOpen,
                sign_key: None,
            })
            .expect("assembly succeeds")
            .expect("package produced")
        })
        .expect("spawn big-stack assembly thread")
        .join()
        .expect("assembly thread");
    let index = tyu::cert::parse_index(&fs::read(pkg.join("cert.json")).unwrap()).unwrap();
    assert_eq!(index.modules.len(), 1);
    assert_eq!(index.modules[0].model, MODEL_ID, "package leg");
    assert_eq!(index.modules[0].target, "x86_64-unknown-none");

    // The whole chain, stated once: five surfaces, one id.
    assert_eq!(
        [
            set.model_semantics.as_str(),
            verdicts.model_semantics.as_str(),
            spec.model.as_str(),
            core::str::from_utf8(&manifest_model).unwrap(),
            index.modules[0].model.as_str(),
        ],
        [MODEL_ID; 5],
        "pack → artifact → verdicts → manifest → package must carry one id"
    );
}

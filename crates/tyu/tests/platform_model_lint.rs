//! The `[model]` lint matrix (PLAN-VERIFY-3 P12.1, §6.7 / §Q15 / FR-10).
//!
//! Bundles declare model identity via `platform.toml [model]` under the
//! schema-3 stamp; the lint enforces the declared↔artifact pairing, the
//! closed value sets, and the evidence-corpus requirement (iff modeled),
//! and *warns* (never errors) on a bundle that declares neither — an
//! unmodeled bundle is honest, not wrong (§Q15). Library-level, over
//! synthetic packs, one negative per rule plus the accept paths.

use std::fs;
use std::path::Path;

use tyu::platform::{discover_platforms_in, format_lint_outcome, lint_pack};

const X86_ABI_HASH: u64 = 0x50FB_AC4F_4E87_016C; // compute_abi_hash(x86_64, MODINFO_VER=4)

const MODEL_ID: &str = "tyu.model/demo/1";

fn tyu_workspace_root() -> std::path::PathBuf {
    tyu::test_helpers::workspace_root()
}

/// P12.2/P13.2 gate: the modeled workspace bundles lint clean — zero errors
/// AND zero warnings (the `[model]` sections, the `model/model.toml`
/// artifacts — the rp2350 board pack's P13.2 refinement manifest included —
/// and the `evidence/` corpora all pair correctly per §6.7).
#[test]
fn modeled_workspace_packs_lint_clean() {
    let root = tyu_workspace_root();
    for name in [
        "x86_64-unknown-none",
        "armv7m-unknown-none",
        "riscv32-unknown-none",
        "rp2350",
    ] {
        let outcome = lint_pack(&root, name, false).unwrap();
        assert!(
            outcome.errors.is_empty(),
            "modeled pack {} must lint clean: {}",
            name,
            format_lint_outcome(&outcome)
        );
        assert!(
            outcome.warnings.is_empty(),
            "modeled pack {} must not warn: {:?}",
            name,
            outcome.warnings
        );
    }
}

/// P12.2/P13.2 gate: the unmodeled workspace packs carry the §Q15 model
/// warning and **no model-pairing errors** (E5413–E5417). (rp2350 joined the
/// modeled set in P13.2 — it carries a `[model]` section, a model artifact
/// with a `[refinements]` manifest, and an evidence corpus.) The hosted
/// runtime additionally carries a pre-existing, P12-orthogonal lint gap (the
/// `[capabilities.gpio] glue` path `platform/gpio.def` does not exist in the
/// hosted layout — a latent hosted-surface issue, tracked outside this
/// phase) — the model surface is asserted independently of it.
#[test]
fn unmodeled_workspace_packs_warn_but_pass() {
    let root = tyu_workspace_root();
    // The hosted runtime carries P12-orthogonal, pre-existing lint gaps (the
    // DS-geometry symbols are deliberately not exported — the documented
    // fail-closed design pinned by `elide_two_pass::hosted_runtime_keeps_guards_fail_closed`
    // — and the `[capabilities.gpio] glue` path is absent from the hosted
    // layout), so its model surface is asserted under `all` (lint everything) —
    // no model-pairing errors (E5413–E5417), §Q15 warning present.
    let hosted = lint_pack(&root, "linux-x86_64-hosted", true).unwrap();
    assert!(
        !hosted
            .errors
            .iter()
            .any(|e| (5413..=5417).contains(&e.code)),
        "hosted must have no model-pairing errors: {}",
        format_lint_outcome(&hosted)
    );
    assert!(
        hosted.warnings.iter().any(|w| w.contains("unmodeled")),
        "hosted must carry the §Q15 warning: {:?}",
        hosted.warnings
    );
}

fn x86_abi_hash_literal() -> String {
    format!("0x{:016x}", X86_ABI_HASH)
}

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// The minimal `model/model.toml` artifact (P12.1: identity only; P12.2
/// extends it with memory refs and the refinement manifest).
fn write_model_artifact(root: &Path, id: &str) {
    write_file(
        &root.join("platforms/demo/model/model.toml"),
        &format!("[model]\nid = \"{id}\"\n"),
    );
}

fn write_evidence(root: &Path) {
    write_file(
        &root.join("platforms/demo/evidence/vectors.json"),
        "{\"schema\":\"tyu.vec/1\"}\n",
    );
}

fn write_pack(root: &Path, manifest: &str) {
    write_file(&root.join("platforms/demo/platform.toml"), manifest);
    write_file(&root.join("platforms/demo/runtime.asm"), base_startup());
    write_file(
        &root.join("crates/tyu/tests/run_qemu_x86.rs"),
        "#[test] fn run_qemu_pass() {}",
    );
    write_file(&root.join("platforms/demo/tests/demo.rs"), "demo evidence");
    write_file(
        &root.join("platforms/demo/concurrency.asm"),
        "; feature unit",
    );
}

fn base_manifest() -> String {
    r#"
[platform]
name = "demo"
schema = 3
compiler-interface = 1
description = "demo pack"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "__ABI_HASH__"

[metal]
path = "."
startup = "runtime.asm"
linker = ""

[features.concurrency]
unit = "concurrency.asm"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[test]
rung = "qemu"
target = "crates/tyu/tests/run_qemu_x86.rs"
evidence = "tests/demo.rs"
"#
    .replace("__ABI_HASH__", &x86_abi_hash_literal())
}

fn manifest_with_model(model_section: &str) -> String {
    format!(
        "{base}\n[model]\n{model_section}\n",
        base = base_manifest(),
        model_section = model_section
    )
}

fn base_startup() -> &'static str {
    "\
public __lang_start\n\
public __lang_trap\n\
public __lang_ds_base\n\
public __lang_ds_limit\n\
public __lang_ds_high\n\
public __lang_expected_abi_hash\n"
}

fn fresh_root(tag: &str) -> std::path::PathBuf {
    tyu::test_helpers::temp_dir(tag)
}

fn lint(root: &Path) -> tyu::platform::LintOutcome {
    lint_pack(root, "demo", false).unwrap()
}

// ---------------------------------------------------------------------------
// Accept paths
// ---------------------------------------------------------------------------

/// A fully-declared modeled bundle: id + matching artifact + evidence corpus
/// lints clean with no warnings.
#[test]
fn modeled_pack_with_artifact_and_evidence_is_clean() {
    let root = fresh_root("clean");
    let manifest = manifest_with_model(&format!(
        "model_semantics = \"{}\"\nmmio = \"nondeterministic\"\nconcurrency = \"unmodeled\"",
        MODEL_ID
    ));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let outcome = lint(&root);
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
    assert!(
        outcome.warnings.is_empty(),
        "a declared modeled bundle must not warn: {:?}",
        outcome.warnings
    );
}

/// The optional keys default: `[model]` with only `model_semantics` declared
/// is well-formed (mmio/concurrency carry their v1 defaults).
#[test]
fn model_section_with_only_the_id_is_clean() {
    let root = fresh_root("id_only");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let outcome = lint(&root);
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
}

/// Declared `"unmodeled"`: honest, needs no artifact and no evidence, and
/// does not warn (§Q15 — the declaration makes the tier explicit).
#[test]
fn declared_unmodeled_needs_no_artifact_or_evidence() {
    let root = fresh_root("declared_unmodeled");
    let manifest = manifest_with_model("model_semantics = \"unmodeled\"");
    write_pack(&root, &manifest);

    let outcome = lint(&root);
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

/// The `[model]` section is optional: a schema-3 pack without one lints with
/// an error-free outcome plus the §Q15 warning (the desirable-but-optional
/// rule). The accessor reports the `unmodeled` default.
#[test]
fn neither_declared_warns_but_passes() {
    let root = fresh_root("undeclared");
    write_pack(&root, &base_manifest());

    let outcome = lint(&root);
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    assert!(
        outcome.warnings[0].contains("unmodeled"),
        "{:?}",
        outcome.warnings
    );

    let pack = discover_platforms_in(root.join("platforms").parent().unwrap())
        .unwrap()
        .into_iter()
        .find(|p| p.name() == "demo")
        .unwrap();
    assert!(pack.model().is_none());
    assert_eq!(pack.model_semantics(), "unmodeled");
}

/// The pack-sourced accessor returns the declared id verbatim — the string
/// that must flow into the artifact, verdicts, manifest, and package.
#[test]
fn accessor_returns_the_declared_id_verbatim() {
    let root = fresh_root("accessor");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let packs = discover_platforms_in(&root).unwrap();
    let pack = packs.iter().find(|p| p.name() == "demo").unwrap();
    assert_eq!(pack.model_semantics(), MODEL_ID);
    let model = pack.model().unwrap();
    assert_eq!(model.mmio_str(), "nondeterministic");
    assert_eq!(model.concurrency_str(), "unmodeled");
}

// ---------------------------------------------------------------------------
// Reject paths — one per §6.7 rule
// ---------------------------------------------------------------------------

/// Declared modeled id without a `model/` artifact ⇒ E5413.
#[test]
fn declared_id_without_artifact_is_5413() {
    let root = fresh_root("no_artifact");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_evidence(&root);

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5413, "{:?}", outcome.errors);
    assert!(
        outcome.errors[0].detail.contains(MODEL_ID),
        "{:?}",
        outcome.errors[0]
    );
}

/// The artifact exists but declares a different id ⇒ E5413 (never a silent
/// half-pairing).
#[test]
fn mismatched_artifact_id_is_5413() {
    let root = fresh_root("mismatch");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_model_artifact(&root, "tyu.model/other-bundle/9");
    write_evidence(&root);

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5413, "{:?}", outcome.errors);
    assert!(
        outcome.errors[0]
            .detail
            .contains("tyu.model/other-bundle/9"),
        "{:?}",
        outcome.errors[0]
    );
}

/// A `model/model.toml` artifact with no declared `[model]` ⇒ E5414 (the
/// artifact would silently never be used).
#[test]
fn artifact_without_declaration_is_5414() {
    let root = fresh_root("undeclared_artifact");
    write_pack(&root, &base_manifest());
    write_model_artifact(&root, MODEL_ID);

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5414, "{:?}", outcome.errors);
    // The undeclared-artifact case is an error, not the §Q15 warning.
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

/// `mmio` outside its closed set ⇒ E5415 naming the supported set.
#[test]
fn bad_mmio_enum_is_5415() {
    let root = fresh_root("bad_mmio");
    let manifest = manifest_with_model(&format!(
        "model_semantics = \"{}\"\nmmio = \"strong\"",
        MODEL_ID
    ));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let outcome = lint(&root);
    assert!(
        outcome.errors.iter().any(|e| e.code == 5415),
        "{:?}",
        outcome.errors
    );
    let err = outcome.errors.iter().find(|e| e.code == 5415).unwrap();
    assert!(err.detail.contains("nondeterministic"), "{:?}", err);
}

/// `concurrency` outside its closed set ⇒ E5415.
#[test]
fn bad_concurrency_enum_is_5415() {
    let root = fresh_root("bad_conc");
    let manifest = manifest_with_model(&format!(
        "model_semantics = \"{}\"\nconcurrency = \"preemptive\"",
        MODEL_ID
    ));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let outcome = lint(&root);
    assert!(
        outcome.errors.iter().any(|e| e.code == 5415),
        "{:?}",
        outcome.errors
    );
}

/// A `[model]` section under the schema-2 stamp ⇒ manifest-invalid (E5400):
/// model identity requires the schema-3 declaration (§13).
#[test]
fn model_under_schema_two_is_rejected() {
    let root = fresh_root("schema2");
    let manifest = base_manifest().replace("schema = 3", "schema = 2")
        + "\n[model]\n"
        + &format!("model_semantics = \"{}\"\n", MODEL_ID);
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    write_evidence(&root);

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5400, "{:?}", outcome.errors);
    assert!(
        outcome.errors[0].detail.contains("schema"),
        "{:?}",
        outcome.errors[0]
    );
}

/// A modeled bundle without an `evidence/` vector corpus ⇒ E5416 (required
/// iff modeled, §6.7).
#[test]
fn modeled_without_evidence_is_5416() {
    let root = fresh_root("no_evidence");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5416, "{:?}", outcome.errors);

    // An empty evidence/ directory is equally absent (presence of ≥ 1 file).
    fs::create_dir_all(root.join("platforms/demo/evidence")).unwrap();
    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5416, "{:?}", outcome.errors);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Warnings render as `warning: pack=… detail=…` and never fail the lint.
#[test]
fn format_renders_warnings_without_failing() {
    let root = fresh_root("render");
    write_pack(&root, &base_manifest());

    let outcome = lint(&root);
    let text = format_lint_outcome(&outcome);
    assert!(text.starts_with("warning: pack=demo"), "{}", text);
    assert!(text.contains("platform demo: ok"), "{}", text);
}

// ---------------------------------------------------------------------------
// P12 review finding 4 — §6.7's letter
// ---------------------------------------------------------------------------

/// P12 finding 4a: `evidence/*.md` docs alone must NOT satisfy the
/// `evidence/vectors.json` corpus requirement (E5416).
#[test]
fn md_only_evidence_does_not_satisfy_e5416() {
    let root = fresh_root("md_only");
    let manifest = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID);
    // Docs only — no vectors.json.
    fs::create_dir_all(root.join("platforms/demo/evidence")).unwrap();
    fs::write(
        root.join("platforms/demo/evidence/run_qemu.md"),
        "harness note",
    )
    .unwrap();
    fs::write(root.join("platforms/demo/evidence/debug.md"), "debug note").unwrap();

    let outcome = lint(&root);
    assert_eq!(
        outcome.errors[0].code, 5416,
        "docs-only evidence must lint as missing the vector corpus: {:?}",
        outcome.errors
    );
    assert!(
        outcome.errors[0].detail.contains("vectors.json"),
        "{:?}",
        outcome.errors[0]
    );
}

/// P12 finding 4b: a pack declaring `model_semantics = "unmodeled"` with a
/// stray artifact declaring a *real* id is a contradiction — E5413, never a
/// silent pass.
#[test]
fn declared_unmodeled_with_real_id_artifact_is_5413() {
    let root = fresh_root("unmodeled_real_id");
    let manifest = manifest_with_model("model_semantics = \"unmodeled\"");
    write_pack(&root, &manifest);
    write_model_artifact(&root, MODEL_ID); // real id under an unmodeled declaration

    let outcome = lint(&root);
    assert_eq!(outcome.errors[0].code, 5413, "{:?}", outcome.errors);
    assert!(
        outcome.errors[0].detail.contains("unmodeled"),
        "{:?}",
        outcome.errors[0]
    );
}

/// A declared-unmodeled artifact declaring `id = "unmodeled"` is consistent
/// (no contradiction).
#[test]
fn declared_unmodeled_with_unmodeled_artifact_is_clean() {
    let root = fresh_root("unmodeled_consistent");
    let manifest = manifest_with_model("model_semantics = \"unmodeled\"");
    write_pack(&root, &manifest);
    write_model_artifact(&root, "unmodeled");

    let outcome = lint(&root);
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
}

/// P12 finding 4c: the build-time pairing gate — a modeled pack with a
/// missing artifact blocks the build path (third-party structural
/// protection), while the in-tree modeled packs pass it.
#[test]
fn ensure_model_pairing_gates_build_path() {
    let root = fresh_root("build_gate");

    // Modeled declaration, NO artifact → must fail.
    let bad = manifest_with_model(&format!("model_semantics = \"{}\"", MODEL_ID));
    write_pack(&root, &bad);
    write_evidence(&root);
    let packs = discover_platforms_in(root.join("platforms").parent().unwrap()).unwrap();
    let pack = packs.iter().find(|p| p.name() == "demo").unwrap();
    let err = tyu::platform::ensure_model_pairing(pack)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("5413"),
        "must name the artifact-pairing violation: {err}"
    );

    // Unmodeled declaration (no artifact): honest — passes.
    let _ = fs::remove_dir_all(&root);
    let root2 = fresh_root("build_gate_ok");
    write_pack(&root2, &base_manifest());
    let packs = discover_platforms_in(root2.join("platforms").parent().unwrap()).unwrap();
    let pack = packs.iter().find(|p| p.name() == "demo").unwrap();
    tyu::platform::ensure_model_pairing(pack).expect("an unmodeled pack passes the build gate");
}

/// The in-tree modeled workspace packs pass the build-time pairing gate.
#[test]
fn modeled_workspace_packs_pass_build_gate() {
    let root = tyu_workspace_root();
    for triple in [
        "x86_64-unknown-none",
        "armv7m-unknown-none",
        "riscv32-unknown-none",
    ] {
        let packs = discover_platforms_in(&root).unwrap();
        let pack = packs
            .iter()
            .find(|p| p.name() == triple)
            .unwrap_or_else(|| panic!("{triple} discovered"));
        tyu::platform::ensure_model_pairing(pack)
            .unwrap_or_else(|e| panic!("{triple} must pass the build gate: {e}"));
    }
}

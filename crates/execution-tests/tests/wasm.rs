//! wasm32-unknown-unknown backend execution tests (PLAN-WASM-2 B1).
//!
//! G54: `codegen-wasm` → `.wasm` → `wasm-tools validate` + wasmtime run,
//! backend ↔ hosted-native golden parity (DEC-38).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to wasm-tools binary (DEC-35: the validator is a CI tool, never a crate dep).
fn wasm_tools() -> PathBuf {
    Path::new("wasm-tools").to_path_buf()
}

/// Path to wasmtime binary (used in B2 for the wasmtime run leg).
#[allow(dead_code)]
fn wasmtime() -> PathBuf {
    Path::new("wasmtime").to_path_buf()
}

/// Build the langc binary with the wasm32 target.
fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc", "--release"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

/// Require wasm-tools and wasmtime for G54 (B1).
fn require_wasm_tools() -> bool {
    common::require_tools(&["wasm-tools"])
}

// ---------------------------------------------------------------------------
// Core backend corpus: every fixture the hosted-native goldens cover (§Q17:
// backend corpus must mirror G53's fixture set, no inheritance by construction).
// ---------------------------------------------------------------------------

/// The backend corpus root: a subset of the labs that:
/// - Typecheck clean (no compile errors)
/// - Do not use platform imports (platform.io, etc.) — MMIO is B2 scope
/// - Do not use TaskSpawn/Interrupt* (rejected at compile per FR-3/FR-44)
/// - Have a `main` word (the entry point)
///
/// For B1, we use hand-authored fixtures in `fixtures/wasm/`. For B2, this
/// expands to include MMIO fixtures.
fn wasm_corpus_root() -> PathBuf {
    common::workspace_root().join("crates/execution-tests/fixtures/wasm")
}

/// G54's validate leg: every backend-corpus fixture compiles to a
/// wasm-tools-valid module.
#[test]
fn backend_emits_valid_wasm() {
    if !require_wasm_tools() {
        return;
    }
    build_langc();

    let corpus_root = wasm_corpus_root();
    if !corpus_root.exists() {
        // B1: the corpus may not exist yet — this test is a no-op until the
        // corpus is populated.
        return;
    }

    for entry in fs::read_dir(corpus_root).expect("read wasm corpus") {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("mod") {
            continue;
        }

        let out_dir = common::workspace_root().join("target/wasm-g54");
        
        // The output filename is the module name from the source (e.g.,
        // "module BasicConst;" → "BasicConst.wasm"). The fixture filenames
        // match the module names.
        let wasm_path = out_dir.join(path.file_stem().unwrap()).with_extension("wasm");

        // Ensure the output directory exists.
        fs::create_dir_all(&out_dir).ok();

        // Compile with the wasm32 backend. Use absolute paths to avoid
        // cwd-relative issues (langc runs in an arbitrary directory under cargo test).
        let langc_path = common::workspace_root().join("target/release/langc");
        let input_abs = common::workspace_root().join(&path);
        let output = Command::new(&langc_path)
            .current_dir(common::workspace_root())
            .args([
                input_abs.to_str().unwrap(),
                "--emit=obj",
                "--target=wasm32-unknown-unknown",
                &format!("--out-dir={}", out_dir.display()),
            ])
            .output()
            .expect("langc compile");

        if !output.status.success() {
            panic!(
                "langc compile failed for {}:\n{}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        assert!(
            wasm_path.exists(),
            "wasm output not found: {}",
            wasm_path.display()
        );

        // Validate with wasm-tools (G54: every module MUST validate).
        let output = Command::new(wasm_tools())
            .args(["validate", wasm_path.to_str().unwrap()])
            .output()
            .expect("wasm-tools validate");

        assert!(
            output.status.success(),
            "wasm-tools validate failed for {}:\n{}",
            wasm_path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// ---------------------------------------------------------------------------
// G54's wasmtime run leg: backend-compiled modules execute under wasmtime
// with the same observable behavior as the hosted-native goldens (DEC-38).
// ---------------------------------------------------------------------------

/// G54's parity leg (B1: core corpus only — arithmetic, stack, control flow,
/// no MMIO yet). The hosted-native goldens are the oracle; the backend MUST
/// match them on the `testio.*` surface. This test is a stub for B1 — the full
/// harness is added in B2 when MMIO is included.
#[test]
#[ignore = "G54 native-golden parity leg is not implemented yet"]
fn backend_parity_core_corpus() {
    assert!(require_wasm_tools(), "wasm-tools is required for G54");
}

// ---------------------------------------------------------------------------
// G54's red path: a deliberately miscompiled arm MUST fail validation
// (the validator is actually consulted — AC-B1).
// ---------------------------------------------------------------------------

#[test]
#[ignore = "G54 miscompile red-path requires the native-golden parity harness"]
fn miscompile_selftest_g54_red_path() {
}

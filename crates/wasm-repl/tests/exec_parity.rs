//! G53(a) — the interpreter ↔ hosted-native differential (PLAN-WASM-1 §7.7).
//!
//! For every in-scope fixture: the native leg (hosted x86_64 binary via the
//! `tyu` runner, merging stdout+stderr onto the `testio` channel) and the
//! interpreter leg (compile → run) must produce **byte-identical**
//! diagnostic-channel output and equal exit codes (FR-8, I-4). Trap fixtures
//! pin the interpreter's frozen trap code (10/StackOverflow) — hosted native
//! hangs there by manifest, so the interpreter is checked against the pinned
//! contract instead (I-3).
//!
//! Also pinned here: the D-1/R-2 byte-identity of `build_ir`+`render_ir`
//! against `langc --emit=ir` for the same fixtures (DEC-13/G53-b wiring —
//! the pane's fidelity claim, mechanized).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::sysroot_search;
use wasm_repl::{compile, Engine, ExecState, Outcome, X86_64_LINUX_SPEC};

mod common;

// ---------------------------------------------------------------------------
// Paths / tools
// ---------------------------------------------------------------------------

fn workspace_root() -> std::path::PathBuf {
    common::workspace_root()
}

fn fixtures_dir() -> PathBuf {
    common::fixtures_dir()
}

fn sysroot_dir() -> PathBuf {
    common::sysroot_dir()
}

fn langc_exe() -> PathBuf {
    common::workspace_root()
        .join("target")
        .join("debug")
        .join("langc")
}

fn tyu_exe() -> PathBuf {
    common::workspace_root()
        .join("target")
        .join("debug")
        .join("tyu")
}

fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_tools() -> bool {
    // `langc`/`tyu` are built by the test itself (`build_bins`); the *link*
    // tools must already be present or the native leg cannot assemble.
    let missing: Vec<String> = ["fasm", "ld"]
        .iter()
        .filter(|t| !tool_available(t))
        .map(|t| t.to_string())
        .collect();
    if missing.is_empty() {
        true
    } else if std::env::var("CI").is_ok() {
        panic!(
            "required tools not available under CI: {}",
            missing.join(", ")
        );
    } else {
        eprintln!(
            "SKIP: required tools not available ({})",
            missing.join(", ")
        );
        false
    }
}

// ---------------------------------------------------------------------------
// Fixture scope (NFR-1 in-scope hosted fixtures for P2)
// ---------------------------------------------------------------------------

/// (stem, entry word): hosted-comparable fixtures.
const PARITY: &[(&str, &str)] = &[
    ("arithmetic", "arithmetic-run"),
    ("deep_stack", "deep-stack-run"),
    ("locals_slices", "locals-slices-run"),
    ("controlflow", "controlflow-run"),
    ("call_abi", "call-abi-run"),
    ("ds_guard_deep", "ds-guard-run"),
    ("aliasing_probe1", "aliasing-probe1"),
    ("aliasing_probe2", "aliasing-probe2"),
    ("aliasing_probe3", "aliasing-probe3"),
];

/// Trap fixtures: the interpreter pins trap code 10 (stack overflow), which
/// is the arm/riscv-native contract (I-3); hosted native may hang (manifest).
const TRAP_ZERO: &[(&str, &str)] = &[
    ("trap_overflow", "trap-overflow-run"),
    ("trap_10_arm", "trap-10-arm-run"),
    ("trap_10_riscv", "trap-10-riscv-run"),
];

fn fixture_path(stem: &str) -> PathBuf {
    fixtures_dir().join(format!("{stem}.mod"))
}

// ---------------------------------------------------------------------------
// Native leg
// ---------------------------------------------------------------------------

fn build_bins() {
    for pkg in ["langc", "tyu"] {
        let s = Command::new(env!("CARGO"))
            .current_dir(workspace_root())
            .args(["build", "-q", "-p", pkg])
            .status()
            .expect("cargo build");
        assert!(s.success(), "cargo build -p {pkg} failed");
    }
}

fn native_run(fixture: &Path) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!("tyu-repl-parity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out_dir = dir.join("out");

    // `tyu build` requires a runnable, *exported* `main ( -- i64 )` (the exit
    // code); the parity fixtures export run words. Wrap the source with a
    // `main` that calls the exported run word and returns 0 — the bytes under
    // test are otherwise unchanged.
    let raw = std::fs::read(fixture).expect("fixture source");
    let mut src = String::from_utf8_lossy(&raw).into_owned();
    // 1. Add `main` to the export list (or introduce the statement).
    if let Some(open) = src.find("export {") {
        let close = src[open..].find('}').expect("export closing brace") + open;
        src.insert_str(close, " main");
    } else {
        src = format!("export {{ main }};\n{src}");
    }
    // 2. Define `main` just before `end;`.
    let main_def = format!(": main ( -- i64 )\n  {}\n  0\n;\n", run_word_for(fixture));
    src = src.replacen("end;", &format!("{main_def}end;"), 1);
    let wrapped = dir.join("wrapped.mod");
    std::fs::write(&wrapped, src).expect("write wrapped fixture");

    let build = Command::new(tyu_exe())
        .args([
            "build",
            &format!("--out-dir={}", out_dir.display()),
            &format!("--sysroot={}", sysroot_dir().display()),
        ])
        .arg(&wrapped)
        .output()
        .expect("tyu build");
    assert!(
        build.status.success(),
        "tyu build failed for {}:\n{}",
        fixture.display(),
        String::from_utf8_lossy(&build.stderr)
    );
    let image = out_dir.join("image.elf");
    assert!(image.is_file(), "no hosted image for {}", fixture.display());
    let outcome = tyu::runner::Runner::for_target(codegen_core::Target::X86_64UnknownLinuxGnu)
        .run(&image, Duration::from_secs(10))
        .expect("native run failed");
    assert!(
        !outcome.timed_out,
        "native run timed out for {}",
        fixture.display()
    );
    (outcome.stdout, outcome.exit_code)
}

/// The exported run word for a fixture stem (the wrapped `main` calls it).
fn run_word_for(fixture: &Path) -> String {
    let stem = fixture.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    match stem {
        "arithmetic" => "arithmetic-run",
        "deep_stack" => "deep-stack-run",
        "locals_slices" => "locals-slices-run",
        "controlflow" => "controlflow-run",
        "call_abi" => "call-abi-run",
        "ds_guard_deep" => "ds-guard-run",
        "aliasing_probe1" => "aliasing-probe1",
        "aliasing_probe2" => "aliasing-probe2",
        "aliasing_probe3" => "aliasing-probe3",
        other => panic!("no run word known for fixture {other}"),
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Interpreter leg
// ---------------------------------------------------------------------------

fn interp_run(src: &[u8], entry: &[u8], search: &[std::sync::Arc<common::MemTable>]) -> Outcome {
    let refs = common::search_refs(search);
    let compiled = compile(src, &refs, X86_64_LINUX_SPEC, None).expect("fixture compiles");
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    let mut periph = wasm_repl::periph::Disabled;
    engine.run(entry, &mut state, &mut periph)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn interpreter_matches_hosted_native_byte_for_byte() {
    if !require_tools() {
        return;
    }
    build_bins();
    let search = sysroot_search();

    for (stem, entry) in PARITY {
        let src = std::fs::read(fixture_path(stem)).expect("fixture present");
        let outcome = interp_run(&src, entry.as_bytes(), &search);
        let Outcome::Ok { out, .. } = outcome else {
            panic!("{stem}: interpreter must complete the run (got {outcome:?})");
        };

        let (native_stdout, native_code) = native_run(&fixture_path(stem));
        assert_eq!(
            native_code, 0,
            "{stem}: native binary must exit 0 (an 'F' byte would mean a check failed)"
        );
        assert_eq!(
            out, native_stdout,
            "{stem}: interpreter output diverged from the hosted native binary"
        );
    }
}

#[test]
fn trap_fixtures_pin_trap_code_10() {
    let search = sysroot_search();
    for (stem, entry) in TRAP_ZERO {
        let src = std::fs::read(fixture_path(stem)).expect("fixture present");
        match interp_run(&src, entry.as_bytes(), &search) {
            Outcome::Trap { code, out, .. } => {
                assert_eq!(
                    code,
                    ir::TrapCode::StackOverflow,
                    "{stem}: expected stack-overflow trap (code 10)"
                );
                assert!(
                    out.is_empty(),
                    "{stem}: the trap must fire before any output"
                );
            }
            other => panic!("{stem}: expected Trap(StackOverflow), got {other:?}"),
        }
    }
}

#[test]
fn ir_render_matches_langc_emit_ir_byte_for_byte() {
    if !require_tools() {
        return;
    }
    build_bins();
    for (stem, _entry) in PARITY {
        let fixture = fixture_path(stem);
        let src = std::fs::read(&fixture).expect("fixture present");

        // langc --emit=ir — the native text, the expected bytes (I-2/R-2).
        let langc_ir = Command::new(langc_exe())
            .args([
                "--emit=ir",
                &format!("--sysroot={}", sysroot_dir().display()),
            ])
            .arg(&fixture)
            .output()
            .expect("langc --emit=ir");
        assert!(
            langc_ir.status.success(),
            "{stem}: langc --emit=ir failed:\n{}",
            String::from_utf8_lossy(&langc_ir.stderr)
        );

        // The REPL render path (build_ir → render_ir) — the pane's bytes.
        let search = sysroot_search();
        let refs = common::search_refs(&search);
        let compiled = compile(&src, &refs, X86_64_LINUX_SPEC, None).expect("fixture compiles");
        let mut rendered = Vec::new();
        wasm_repl::render_ir(&mut VecSink(&mut rendered), &compiled.module);

        assert_eq!(
            rendered, langc_ir.stdout,
            "{stem}: `--emit=ir` text drifted between langc and the REPL renderer \
             (build_ir/render_ir must be byte-identical; D-1/R-2/DEC-13)"
        );
    }
}

struct VecSink<'a>(&'a mut Vec<u8>);
impl frontend::parse::Output for VecSink<'_> {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

// ---------------------------------------------------------------------------
// G53 (P4 scope): MMIO fixtures — compile parity + scripted-device runs
// ---------------------------------------------------------------------------

/// The in-scope MMIO fixtures (P4): the `_x86` pack driven through the
/// x86_64-unknown-none descriptor (OQ-3 — the pack the existing MMIO tests
/// already drive). The aperture/device pinning lives in `tests/mmio.rs`
/// (device-log == IR projection); here G53 asserts (a) the REPL render is
/// byte-equal to `langc --emit=ir` for the same descriptor, and (b) the
/// engine runs each fixture self-check-clean (the fixture writes 'F' on any
/// wrong MMIO value) against the emulated-aperture RAM device.
const MMIO_PARITY: &[(&str, &str)] = &[
    ("mmio_smoke_x86", "mmio-smoke-x86-run"),
    ("mmio_strategies_x86", "mmio-strategies-x86-run"),
];

fn x86_desc() -> codegen_core::compiled_desc::CompiledDescriptor {
    let bytes =
        std::fs::read(common::workspace_root().join("platforms/x86_64-unknown-none/platform.desc"))
            .expect("x86_64-unknown-none platform.desc committed");
    let cd =
        codegen_core::compiled_desc::decode_compiled_desc(&bytes).expect("decode platform.desc");
    codegen_core::compiled_desc::validate_compiled_desc(&cd).expect("validate platform.desc");
    cd
}

#[test]
fn mmio_fixtures_render_byte_equal_to_langc_and_self_check() {
    if !require_tools() {
        return;
    }
    build_bins();
    let search = sysroot_search();
    let refs = common::search_refs(&search);
    let desc = x86_desc();

    for (stem, entry) in MMIO_PARITY {
        let fixture = fixture_path(stem);
        let src = std::fs::read(&fixture).expect("fixture present");

        // Native text with the descriptor — the expected bytes (I-2/R-2 +
        // P4 descriptor wiring; `langc` needs the bare-metal sysroot include).
        let langc_ir = Command::new(langc_exe())
            .args([
                "--emit=ir",
                &format!("--sysroot={}", sysroot_dir().display()),
                "-I",
                &format!("{}/x86_64-unknown-none", sysroot_dir().display()),
                &format!(
                    "--platform={}",
                    common::workspace_root()
                        .join("platforms/x86_64-unknown-none")
                        .display()
                ),
            ])
            .arg(&fixture)
            .output()
            .expect("langc --emit=ir (mmio)");
        assert!(
            langc_ir.status.success(),
            "{stem}: langc --emit=ir failed:\n{}",
            String::from_utf8_lossy(&langc_ir.stderr)
        );

        // The REPL render path with the same descriptor — byte-identical.
        let compiled =
            compile(&src, &refs, X86_64_LINUX_SPEC, Some(&desc)).expect("fixture compiles");
        let mut rendered = Vec::new();
        wasm_repl::render_ir(&mut VecSink(&mut rendered), &compiled.module);
        assert_eq!(
            rendered, langc_ir.stdout,
            "{stem}: MMIO `--emit=ir` text drifted between langc and the REPL \
             (build_ir/render_ir must be byte-identical; D-1/R-2/DEC-13)"
        );

        // Run through the interpreter against the emulated-aperture device:
        // every self-check in the fixture must pass (a wrong MMIO value
        // writes an 'F' through `testio.write-byte`).
        let engine = Engine::new(&compiled);
        let mut state = ExecState::new(&compiled);
        let mut dev = wasm_repl::RamDevice::new(&[0]);
        let out = engine.run(entry.as_bytes(), &mut state, &mut dev);
        let Outcome::Ok { out, .. } = out else {
            panic!("{stem}: MMIO run must complete: {out:?}");
        };
        assert!(
            out.is_empty(),
            "{stem}: MMIO fixture self-checks must pass (no 'F' byte): {out:?}"
        );
    }
}

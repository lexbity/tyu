//! Hosted (x86_64-unknown-linux-gnu) codegen tests.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// BUG-004: the x86_64 backend must emit the resource symbol address for
/// `&!Res` inside `lock` instead of returning `UnsupportedAddrOf` (E8008),
/// and must declare the resource globals in the module object.
#[test]
fn resource_lock_addr_of_emitted() {
    if !common::require_tools(&["langc", "fasm"]) {
        return;
    }
    let s = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let dir = common::temp_dir("resource_lock_hosted");
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("resource_lock_hosted.mod");

    common::langc_compile(
        codegen_core::Target::X86_64UnknownLinuxGnu,
        &src_path,
        &dir,
        true, // --lib: runner provides main; we only inspect asm shape
    );

    let asm_path = dir.join("ResourceLockHosted.asm");
    let asm = std::fs::read_to_string(&asm_path).expect("x86_64 assembly must be generated");
    assert!(
        asm.contains("mov rax, r_"),
        "resource AddrOf must load the resource symbol address;\n{asm}"
    );
    assert!(
        asm.contains("section '.bss'") && asm.contains("r_") && asm.contains("dq 0"),
        "module must declare its resource globals;\n{asm}"
    );
}

/// PLAN-VERIFY-3 P15.2 — the service-conformance runtime anchor: the
/// concurrency template (`fixtures/conc_roundtrip.mod`, the §Q14
/// channel round-trip) compiles for the hosted bundle, links, and RUNS — the
/// native leg exits 0 exactly when the received value equals the sent payload
/// (42) i.e. when the hosted runtime's channel IPC + task scheduler deliver
/// FIFO semantics on the wire. This is the runtime side of the `tyu.svcvec/1`
/// corpus consensus (Lean model + Rust mirror + hosted runtime — zero
/// divergence, R9 detection).
#[test]
fn conc_roundtrip_runs_fifo_on_the_hosted_runtime() {
    if !common::require_tools(&["langc", "fasm", "ld"]) {
        return;
    }
    for pkg in ["langc", "tyu"] {
        let s = Command::new(env!("CARGO"))
            .current_dir(common::workspace_root())
            .args(["build", "-q", "-p", pkg])
            .status()
            .expect("cargo build");
        assert!(s.success(), "cargo build -p {pkg} failed");
    }

    let dir = common::temp_dir("conc_native");
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("conc_roundtrip.mod");
    let out_dir = dir.join("out");

    let build = Command::new(common::tyu_exe())
        .args([
            "build",
            &format!("--out-dir={}", out_dir.display()),
            &format!(
                "--sysroot={}",
                common::workspace_root().join("sysroot").display()
            ),
        ])
        .arg(&src)
        .output()
        .expect("tyu build");
    assert!(
        build.status.success(),
        "tyu build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let image = out_dir.join("image.elf");
    assert!(image.is_file(), "no hosted image produced");
    let outcome = tyu::runner::Runner::for_target(codegen_core::Target::X86_64UnknownLinuxGnu)
        .run(&image, Duration::from_secs(10))
        .expect("native run failed");
    assert!(
        !outcome.timed_out,
        "the round-trip must not hang (bounded wait queues)"
    );
    assert_eq!(
        outcome.exit_code,
        0,
        "the FIFO round-trip must deliver the payload (exit 0); stdout: {}",
        String::from_utf8_lossy(&outcome.stdout)
    );
}

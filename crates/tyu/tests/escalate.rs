//! Phase 14 — A-side escalation runner integration tests.
//!
//! Tests:
//! - E-1: trap fixture escalates hard on x86/ARM/RISC-V
//! - E-2: clean pass does NOT trigger escalation (no needless second run)

use std::process::Command;

use codegen_core::FeatureSet;
use tyu::build;
use tyu::runner::QemuDebugStart;
use tyu::test_helpers::*;

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

// ---------------------------------------------------------------------------
// E-1: trap fixture escalates hard
// ---------------------------------------------------------------------------

#[test]
fn escalate_trap_no_d_record() {
    assert_escalates(
        codegen_core::Target::X86_64UnknownNone,
        &["langc", "fasm", "ld", "qemu-system-x86_64", "nm"],
    );
    assert_escalates(
        codegen_core::Target::ArmV7MUnknownNone,
        &[
            "langc",
            "arm-none-eabi-as",
            "arm-none-eabi-ld",
            "qemu-system-arm",
            "nm",
        ],
    );
    assert_escalates(
        codegen_core::Target::RiscV32UnknownNone,
        &[
            "langc",
            "riscv32-elf-as",
            "riscv32-elf-ld",
            "qemu-system-riscv32",
            "nm",
        ],
    );
}

// ---------------------------------------------------------------------------
// E-2: clean pass does NOT trigger escalation
// ---------------------------------------------------------------------------

#[test]
fn escalate_not_invoked_on_clean_pass() {
    // When `tyu test` runs a clean fixture, the escalation should NOT be
    // invoked.  We verify this by checking that the test process succeeds
    // (clean pass) without any escalation attempt showing in the output.
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("escalate_clean");
    std::fs::write(
        &dir.join("clean.mod"),
        "\
module Clean;
import platform/testio { testio.write-byte };
: clean-run ( -- ) ;
export { clean-run };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"clean\"
file = \"clean.mod\"
axes = [\"arith\"]
requires = []
",
    )
    .unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", dir.join("manifest.toml").display()),
        ])
        .output()
        .expect("tyu test");
    assert!(
        output.status.success(),
        "clean fixture must pass:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("escalat"),
        "clean pass must not invoke escalation:\n{}",
        stderr,
    );
}

fn assert_escalates(target: codegen_core::Target, tools: &[&str]) {
    if !require_tools(tools) {
        return;
    }
    ensure_langc();

    let dir = temp_dir(&format!("escalate_{:?}", target));
    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  100 as Small drop
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs =
        build::assemble_runtime(target, &dir, FeatureSet::empty(), None).expect("assemble runtime");
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = build::link_image(target, &objs, &dir, None).expect("link image");

    let outcome = tyu::debug_escalate::escalate(&image, target, None);

    assert_eq!(outcome.target, target);
    assert_eq!(outcome.mode, QemuDebugStart::FrozenAtReset);
    assert!(
        outcome.port != 0,
        "escalation should allocate a real gdbstub port: {:?}",
        outcome,
    );
    assert!(
        outcome.phase.is_none(),
        "escalation must complete the full product path: {:?}",
        outcome,
    );
    assert!(
        outcome.error.is_none(),
        "escalation must succeed on {:?}: {:?}",
        target,
        outcome,
    );

    let diag = outcome
        .diagnostic_string
        .as_ref()
        .expect("escalation must return a diagnostic");
    assert!(
        diag.contains("SUBTYPE_FAIL") || diag.contains("21"),
        "escalation diagnostic should contain claim text or trap code: {}",
        diag,
    );
}

fn langc_compile(
    target: codegen_core::Target,
    src: &std::path::Path,
    out_dir: &std::path::Path,
    _is_lib: bool,
) -> std::path::PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let before: std::collections::HashSet<std::path::PathBuf> = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        format!("--target={triple}"),
        format!("--sysroot={}", workspace_root().join("sysroot").display()),
        format!("--out-dir={}", out_dir.display()),
    ];
    args.push(src.to_str().unwrap().into());

    let status = Command::new(langc_exe())
        .args(&args)
        .status()
        .expect("langc invocation failed");
    assert!(status.success(), "langc failed on {}", src.display());

    std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .expect("langc produced no .o file")
}

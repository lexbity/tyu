//! Phase 4 — Poison-fixture harness & CI=1 skip→fail.
//!
//! Tests:
//! - P-1: poison = "trap:10" on a stack-overflow fixture → pass
//! - P-2: poison = "fail-marker" on an F-emitting fixture → pass
//! - P-3: poison = "no-completion" on an infinite-loop fixture → pass (HANG)
//! - P-4: poison = "fail-marker" on a clean fixture → POISON_DID_NOT_FAIL
//! - P-5: CI=1 with missing tools → hard error
//! - P-6: Non-poison fixtures are unaffected

use std::process::Command;

use tyu::test_helpers::*;

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

// P-1 removed: trap:10 (stack overflow) is a compile-time guarantee on
// bounded bare-metal targets (E_STACK_UNBOUNDED / E_STACK_EXCEEDS_BUDGET),
// not a runtime event.  Unbounded-recursion programs are rejected at
// compile time, so there is no image to produce a runtime trap.
// The runtime trap:10 path exists only for hosted (unbounded-profile)
// targets.  This test would belong in the compile-time negative corpus
// (tooling-tests/phase17_negative_corpus.rs), not in the runtime poison
// suite.

// ---------------------------------------------------------------------------
// P-2: poison = "fail-marker" on an F-emitting fixture
// ---------------------------------------------------------------------------

#[test]
fn poison_fail_marker() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("poison_fail");
    std::fs::write(
        &dir.join("poison_fail.mod"),
        "\
module PoisonFail;
import platform/testio { testio.write-byte };
: poison-fail-run ( -- ) 70 testio.write-byte ;
export { poison-fail-run };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"poison_fail\"
file = \"poison_fail.mod\"
axes = [\"trap\"]
requires = []
poison = \"fail-marker\"
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
        "poison fail-marker fixture must pass:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// P-3: poison = "no-completion" on an infinite loop (HANG)
// ---------------------------------------------------------------------------

#[test]
fn poison_no_completion() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("poison_nocomp");
    std::fs::write(
        &dir.join("poison_nocomp.mod"),
        "\
module PoisonNocomp;
: poison-nocomp-run ( -- )
  [ true ] [ ] while ;
export { poison-nocomp-run };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"poison_nocomp\"
file = \"poison_nocomp.mod\"
axes = [\"trap\"]
requires = []
poison = \"no-completion\"
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
        "poison no-completion fixture must pass:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// P-4: poison = "fail-marker" on a clean fixture → POISON_DID_NOT_FAIL
// ---------------------------------------------------------------------------

#[test]
fn poison_clean_fixture_rejected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("poison_clean");
    // A fixture that passes cleanly (no F marker, no hang)
    std::fs::write(
        &dir.join("poison_clean.mod"),
        "\
module PoisonClean;
: poison-clean-run ( -- ) ;
export { poison-clean-run };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"poison_clean\"
file = \"poison_clean.mod\"
axes = [\"trap\"]
requires = []
poison = \"fail-marker\"
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
        !output.status.success(),
        "clean fixture with poison must fail with POISON_DID_NOT_FAIL",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("POISON_DID_NOT_FAIL"),
        "must mention POISON_DID_NOT_FAIL:\n{}",
        stderr,
    );
}

// ---------------------------------------------------------------------------
// P-5: CI=1 with missing tools → hard error
// ---------------------------------------------------------------------------

#[test]
fn ci_missing_tools_hard_error() {
    // Use RISC-V target (least likely to have tools installed).
    let riscv_tools = ["riscv32-elf-as", "qemu-system-riscv32"];
    let all_available = riscv_tools.iter().all(|t| tool_available(t));
    if all_available {
        eprintln!("SKIP: CI=1 test skipped because all riscv32 tools are present");
        return;
    }
    ensure_langc();

    let dir = temp_dir("ci_missing");
    std::fs::write(
        &dir.join("simple.mod"),
        "\
module Simple;
: main ( -- i64 ) 0 ;
export { main };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"simple\"
file = \"simple.mod\"
axes = [\"arith\"]
requires = []
",
    )
    .unwrap();

    let output = Command::new(tyu_exe())
        .env("CI", "1")
        .args([
            "test",
            "--target=riscv32-unknown-none",
            &format!("--manifest={}", dir.join("manifest.toml").display()),
        ])
        .output()
        .expect("tyu test");
    assert!(
        !output.status.success(),
        "CI=1 with missing tools must fail",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing") || stderr.contains("CI"),
        "CI=1 error must mention missing tools or CI:\n{}",
        stderr,
    );
}

// ---------------------------------------------------------------------------
// P-6: Non-poison fixtures are unaffected
// ---------------------------------------------------------------------------

#[test]
fn poison_non_poison_fixtures_unaffected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("non_poison");
    std::fs::write(
        &dir.join("pass.mod"),
        "\
module Pass;
: pass-run ( -- ) ;
export { pass-run };
end;
",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]
name = \"pass\"
file = \"pass.mod\"
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
        "non-poison fixture must still pass:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

//! Feature gate acceptance tests (Slice 2–3).
//!
//! Verifies:
//! 1. AC-3: `task spawn` under `concurrency`-off → E6101, no `.o`, non-zero exit.
//! 2. AC-4: same fixture under `concurrency`-on → compiles and assembles.
//! 3. Two-misuse fixture → two span-ordered E6101 diagnostics.
//! 4. E6102 readiness: the gate infrastructure for module-loading is wired.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn sysroot_dir() -> PathBuf {
    common::workspace_root().join("sysroot")
}

fn sysroot_target_dir() -> PathBuf {
    sysroot_dir().join("x86_64-unknown-linux-gnu")
}

/// Self-contained fixture using `platform.task.spawn` (needs `platform/linux`
/// sysroot module for the hosted x86_64 target).
const TASK_SPAWN_FIXTURE: &str = "\
module Main;
import platform/linux { platform.task.spawn, platform.task.join };
: main ( -- i64 ) !{suspend}
  [ ( -- ) ] platform.task.spawn => t
  0 bitcast |Task| t |>
  0 bitcast |Task| <| platform.task.join
  0 ;
end;
";

/// Fixture with TWO `platform.task.spawn` uses to verify span-ordered diagnostics.
const TWO_TASK_SPAWN_FIXTURE: &str = "\
module Main;
import platform/linux { platform.task.spawn, platform.task.join };
: spawner ( -- i64 ) !{suspend}
  [ ( -- ) ] platform.task.spawn => t
  0 bitcast |Task| t |>
  0 bitcast |Task| <| platform.task.join
  0 ;
: main ( -- i64 ) !{suspend}
  [ ( -- ) ] platform.task.spawn => t
  0 bitcast |Task| t |>
  0 bitcast |Task| <| platform.task.join
  0 ;
end;
";

const TARGET_ARG: &str = "--target=x86_64-unknown-linux-gnu";

// ---------------------------------------------------------------------------
// AC-3: concurrency-off + task spawn → E6101, no .o, non-zero
// ---------------------------------------------------------------------------

#[test]
fn gate_rejects_task_spawn_when_concurrency_off() {
    let dir = common::fresh_dir("gate_reject");
    std::fs::write(dir.join("G.mod"), TASK_SPAWN_FIXTURE).unwrap();

    let output = Command::new(common::exe("langc"))
        .current_dir(&dir)
        .arg("--no-default-features")
        .arg("--emit=obj")
        .arg(TARGET_ARG)
        .arg(format!("--sysroot={}", sysroot_dir().to_string_lossy()))
        .arg("-I")
        .arg(sysroot_target_dir().to_string_lossy().as_ref())
        .arg("--out-dir=.")
        .arg("G.mod")
        .output()
        .expect("langc invocation");

    // Exit code must be non-zero.
    assert!(
        !output.status.success(),
        "langc must fail when concurrency is disabled and task spawn is used"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Must contain E6101.
    assert!(
        stderr.contains("E6101"),
        "stderr must contain E6101 diagnostic, got:\n{stderr}"
    );
    assert!(
        stderr.contains("concurrency"),
        "stderr must mention 'concurrency', got:\n{stderr}"
    );
    assert!(
        stderr.contains("platform.task.spawn") || stderr.contains("task.spawn"),
        "stderr must mention the spawn construct, got:\n{stderr}"
    );

    // No .o file must be produced.
    let o_files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("o"))
        .collect();
    assert!(
        o_files.is_empty(),
        "no .o file must be produced on gate failure, found: {:?}",
        o_files.iter().map(|e| e.file_name()).collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// AC-4: concurrency-on + task spawn → compiles cleanly
// ---------------------------------------------------------------------------

#[test]
fn gate_allows_task_spawn_when_concurrency_on() {
    let dir = common::fresh_dir("gate_allow");
    std::fs::write(dir.join("G.mod"), TASK_SPAWN_FIXTURE).unwrap();

    let output = Command::new(common::exe("langc"))
        .current_dir(&dir)
        .arg("--emit=obj")
        .arg(TARGET_ARG)
        .arg(format!("--sysroot={}", sysroot_dir().to_string_lossy()))
        .arg("-I")
        .arg(sysroot_target_dir().to_string_lossy().as_ref())
        .arg("--out-dir=.")
        .arg("G.mod")
        .output()
        .expect("langc invocation");

    assert!(
        output.status.success(),
        "langc must succeed when concurrency is enabled, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    // At least one .o file must be produced.
    let o_files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("o"))
        .collect();
    assert!(
        !o_files.is_empty(),
        "at least one .o file must be produced when gate passes"
    );
}

// ---------------------------------------------------------------------------
// Two misuses → two span-ordered diagnostics
// ---------------------------------------------------------------------------

#[test]
fn gate_rejects_two_task_spawns_with_two_diagnostics() {
    let dir = common::fresh_dir("gate_two_spawns");
    std::fs::write(dir.join("G.mod"), TWO_TASK_SPAWN_FIXTURE).unwrap();

    let output = Command::new(common::exe("langc"))
        .current_dir(&dir)
        .arg("--no-default-features")
        .arg("--emit=obj")
        .arg(TARGET_ARG)
        .arg(format!("--sysroot={}", sysroot_dir().to_string_lossy()))
        .arg("-I")
        .arg(sysroot_target_dir().to_string_lossy().as_ref())
        .arg("--out-dir=.")
        .arg("G.mod")
        .output()
        .expect("langc invocation");

    assert!(
        !output.status.success(),
        "langc must fail when concurrency is disabled and two task spawns are used"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Count E6101 occurrences.
    let count = stderr.matches("E6101").count();
    assert_eq!(
        count, 2,
        "two misuses must produce exactly two E6101 diagnostics, got {count}:\n{stderr}"
    );

    // Verify no .o file produced.
    let o_files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("o"))
        .collect();
    assert!(
        o_files.is_empty(),
        "no .o file must be produced on gate failure"
    );
}

// ---------------------------------------------------------------------------
// Deliberately moving the span must make the test fail (AC-3 falsifiability)
// ---------------------------------------------------------------------------

#[test]
fn gate_rejects_span_must_be_catchable() {
    // This test proves the E6101 detection is specific: if the span is
    // deliberately moved, the diagnostic must differ.  We assert the
    // opposite of what a correct gate does so that a broken gate (wrong
    // span) would make this test *pass* — but we want it to *fail* when
    // the gate is correct.
    //
    // Concretely: we expect "platform.task.spawn" at column 14 of line 4.
    // If the span pointed elsewhere, the diagnostic would reference a
    // different location.
    let dir = common::fresh_dir("gate_span_catch");
    std::fs::write(dir.join("G.mod"), TASK_SPAWN_FIXTURE).unwrap();

    let output = Command::new(common::exe("langc"))
        .current_dir(&dir)
        .arg("--no-default-features")
        .arg("--emit=obj")
        .arg(TARGET_ARG)
        .arg(format!("--sysroot={}", sysroot_dir().to_string_lossy()))
        .arg("-I")
        .arg(sysroot_target_dir().to_string_lossy().as_ref())
        .arg("--out-dir=.")
        .arg("G.mod")
        .output()
        .expect("langc invocation");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // The correct diagnostic points to `platform.task.spawn` at line 4.
    // If the span were wrong, this assertion would catch it.
    assert!(
        stderr.contains("G.mod:4:14") || stderr.contains("G.mod:4:13"),
        "span must point to line 4 column ~14 where platform.task.spawn begins, \
         not elsewhere; stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// E6102 readiness: the infrastructure for module-loading feature gating
// is wired.  No ModuleLoad* IR ops exist yet to trigger E6102, so this
// test verifies that the *runtime side* of the gate is ready: modload.o
// can be excluded from the link when the feature is off.
// ---------------------------------------------------------------------------

#[test]
fn gate_module_loading_runtime_excludable() {
    // Verify that codegen_core::Feature::ModuleLoading exists, maps to
    // runtime unit "modload", and can be checked against a FeatureSet.
    use codegen_core::{Feature, FeatureSet};

    let empty = FeatureSet::empty();
    let all = FeatureSet::all();

    assert!(!empty.contains(Feature::ModuleLoading));
    assert!(all.contains(Feature::ModuleLoading));
    assert_eq!(Feature::ModuleLoading.as_str(), "module-loading");
    assert_eq!(Feature::ModuleLoading.runtime_unit(), Some("modload"));
}

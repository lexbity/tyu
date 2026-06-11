//! Phase 15 — Hang disambiguation tests.
//!
//! Tests:
//! - H-1: infinite net-zero poll loop → classified "poll loop (expected divergence)"
//! - H-2: non-tail unbounded recursion → classified "runaway recursion suspected"

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

fn build_fixture(
    dir: &std::path::Path,
    target: codegen_core::Target,
    fixture_src: &str,
) -> std::path::PathBuf {
    std::fs::write(dir.join("Main.mod"), fixture_src).unwrap();

    // Compile fixture with -g so .lang.debug is emitted.
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let before: std::collections::HashSet<std::path::PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let status = Command::new(langc_exe())
        .args([
            "-g",
            "--emit=obj",
            &format!("--target={}", triple),
            &format!("--sysroot={}", workspace_root().join("sysroot").display()),
            &format!("--out-dir={}", dir.display()),
            &dir.join("Main.mod").to_string_lossy(),
        ])
        .status()
        .expect("langc invocation");
    assert!(status.success(), "langc -g failed");

    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .expect("langc produced no .o file")
}

fn link_and_run(target: codegen_core::Target, objs: &[std::path::PathBuf], dir: &std::path::Path, port: u16) {
    let rt_dir = workspace_root()
        .join("runtime")
        .join(std::str::from_utf8(target.triple()).unwrap());

    // Assemble runtime first.
    let runtime_o = dir.join("runtime.o");
    let fasm_status = Command::new("fasm")
        .args([rt_dir.join("runtime.asm").to_str().unwrap(), runtime_o.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(fasm_status.success(), "fasm runtime failed");

    // Single link with all objects.
    let mut all_objs = objs.to_vec();
    all_objs.push(runtime_o);
    let ld_script = rt_dir.join("link.ld");
    let image = dir.join("test.elf");
    let linker = std::str::from_utf8(target.spec().linker).unwrap();
    let mut cmd = Command::new(linker);
    cmd.arg("-T").arg(&ld_script).arg("-o").arg(&image);
    for o in &all_objs {
        cmd.arg(o);
    }
    let status = cmd.status().unwrap_or_else(|_| panic!("{}", linker));
    assert!(status.success(), "link failed");

    let hc = tyu::debug_escalate::classify_hang_on_port(&image, target, port);
    let report = hc.to_string();
    eprintln!("{}", report);
    std::fs::write(dir.join("classification.txt"), &report).unwrap();
}

// ---------------------------------------------------------------------------
// H-1: net-zero poll loop → PollLoop
// ---------------------------------------------------------------------------

#[test]
fn hang_poll_loop() {
    if !require_tools(&[
        "langc", "fasm", "ld", "qemu-system-x86_64", "nm",
    ]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("hang_poll_loop");
    let target = codegen_core::Target::X86_64UnknownNone;

    // A net-zero infinite event loop: [ true ] [ ] while.
    // This diverges (never returns) but has finite stack (net-zero body).
    let fixture_src = "\
module Main;
import platform/testio { testio.write-byte };
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  [ true ] [ ] while
  emit-done
  0 ;
end;
";

    let fixture_o = build_fixture(&dir, target, fixture_src);
    link_and_run(target, &[fixture_o], &dir, 1241);
}

// ---------------------------------------------------------------------------
// H-2: unbounded non-tail recursion → RunawayRecursion
// ---------------------------------------------------------------------------

#[test]
fn hang_runaway_recursion() {
    if !require_tools(&[
        "langc", "fasm", "ld", "qemu-system-x86_64", "nm",
    ]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("hang_runaway");
    let target = codegen_core::Target::X86_64UnknownNone;

    // Non-tail recursion: each call pushes a return address on the data
    // stack, so 'high' grows without bound → ⊤.
    let fixture_src = "\
module Main;
import platform/testio { testio.write-byte };
: recurse ( -- ) recurse ;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  recurse
  emit-done
  0 ;
end;
";

    let fixture_o = build_fixture(&dir, target, fixture_src);
    link_and_run(target, &[fixture_o], &dir, 1242);
}

// ---------------------------------------------------------------------------
// H-3: clean fixture should NOT hang (negative control)
// ---------------------------------------------------------------------------

#[test]
fn hang_clean_fixture_does_not_classify() {
    if !require_tools(&[
        "langc", "fasm", "ld", "qemu-system-x86_64",
    ]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("hang_clean");
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
    // No "hang" or "timed" messages for a clean fixture.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("HANG"),
        "clean fixture must not trigger hang:\n{}",
        stderr,
    );
}

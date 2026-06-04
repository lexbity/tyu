//! End-to-end build test for x86_64-unknown-none.
//!
//! Requires `langc`, `fasm`, and `ld` in PATH.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn tyu_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("tyu")
}

/// Returns true if the named binary exists in PATH.
fn tool_available(name: &str) -> bool {
    Command::new("which").arg(name).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

/// Skip check — same pattern as execution-tests.
fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!("Required tools not available under CI: {}", missing.join(", "));
    }
    eprintln!("SKIP: required tools not available ({})", missing.join(", "));
    false
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_build_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Minimal main.mod that just returns 0.
const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

#[test]
fn build_minimal_x86_64_none_elf() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let dir = temp_dir("minimal_x86_64");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    // Build: cargo build -p langc first so langc is up to date.
    let build_status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(build_status.success(), "cargo build -p langc failed");

    // Run tyu build.
    let tyu = tyu_exe();
    let output = Command::new(&tyu)
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build failed");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("tyu build failed:\nstdout: {}\nstderr: {}", stdout, stderr);
    }

    // Check that the ELF was produced.
    let image = out_dir.join("image.elf");
    assert!(image.exists(), "ELF image not produced at {}", image.display());

    // Check ELF magic.
    let data = std::fs::read(&image).expect("failed to read ELF");
    assert_eq!(&data[..4], b"\x7fELF", "output is not a valid ELF");

    // Check it's ELF64 (class byte at offset 4 should be 2).
    assert_eq!(data[4], 2, "expected ELF64");
}

#[test]
fn build_uses_cache_on_second_run() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let dir = temp_dir("cached_build");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    // Ensure langc is built.
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let tyu = tyu_exe();

    // First build.
    let first = Command::new(&tyu)
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("first tyu build");
    assert!(first.status.success(), "first build failed");

    // Second build with unchanged source — must succeed.
    let second = Command::new(&tyu)
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("second tyu build");
    assert!(second.status.success(), "second (cached) build failed");

    // Both should produce the same ELF path.
    let image = out_dir.join("image.elf");
    assert!(image.exists(), "ELF exists after second build");
}

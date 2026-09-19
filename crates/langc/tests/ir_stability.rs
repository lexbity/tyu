//! `--emit=ir` stability (NFR-1 / FR-22): identical inputs produce
//! byte-identical IR, and the IR is independent of descriptor *formatting*
//! (the platform hash flows through, but the IR text reflects the resolved
//! windows, which are formatting-invariant). Also pins the `format_ver`
//! header (D-8 / FR-6).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn langc_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/langc")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-ir-stab-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const FIXTURE: &[u8] = include_bytes!("../../execution-tests/fixtures/mmio_smoke_x86.mod");

fn emit_ir(out_dir: &PathBuf) -> String {
    let out = Command::new(langc_exe())
        .current_dir(out_dir)
        .arg("--emit=ir")
        .arg("--target=x86_64-unknown-none")
        .arg(format!(
            "--sysroot={}",
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("sysroot")
                .display()
        ))
        .arg(format!(
            "--platform={}",
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("platforms/x86_64-unknown-none")
                .display()
        ))
        .arg("Main.mod")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=ir failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn ir_emit_is_byte_stable_and_versioned() {
    let a = temp_dir("a");
    let b = temp_dir("b");
    fs::write(a.join("Main.mod"), FIXTURE).unwrap();
    fs::write(b.join("Main.mod"), FIXTURE).unwrap();

    let ir_a = emit_ir(&a);
    let ir_b = emit_ir(&b);
    assert_eq!(
        ir_a, ir_b,
        "identical inputs must produce byte-identical --emit=ir (NFR-1/FR-22)"
    );

    // D-8 / FR-6: the first line is `format_ver 4` and the window section is
    // present; no absolute MMIO address may appear anywhere in the text.
    assert!(
        ir_a.starts_with("format_ver 6\n"),
        "IR must start with format_ver 6, got: {:?}",
        &ir_a[..ir_a.find('\n').unwrap_or(0)]
    );
    assert!(
        ir_a.contains("\nwindows 1\nwindow 0 mmio emulated bind=none link 0x10000\n"),
        "IR must carry the windows section, got:\n{ir_a}"
    );
    assert!(
        !ir_a.contains("addr="),
        "no absolute MMIO address may remain in IR text (P4), got:\n{ir_a}"
    );
    assert!(
        ir_a.contains("mmio_place scratch.A window=0 offset=0x0")
            || ir_a.contains("addr_of scratch.A window=0 offset=0x0"),
        "IR must carry window-relative symbolic places, got:\n{ir_a}"
    );

    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
}

#[test]
fn ir_emit_is_independent_of_descriptor_formatting() {
    // The descriptor's canonical content resolves the windows; reformatting
    // the TOML (comments, key order) must not change the emitted IR.
    let a = temp_dir("fa");
    let b = temp_dir("fb");
    fs::write(a.join("Main.mod"), FIXTURE).unwrap();
    fs::write(b.join("Main.mod"), FIXTURE).unwrap();
    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
}
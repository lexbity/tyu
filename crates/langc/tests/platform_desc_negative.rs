//! Negative: the `--platform` descriptor path rejects an invalid compiled
//! descriptor with E3647 (P3). A malformed `platform.desc` (window size 0, or
//! overlapping bus windows) must be a loud compile-stop — the descriptor is
//! the board claim, and a bad one is never silently compiled around.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use codegen_core::compiled_desc::{
    CompiledDescriptor, COMPILED_DESC_MAX_BYTES, encode_compiled_desc,
};
use codegen_core::{MmioWindowKind, MmioWindowSpec};

fn atom(bytes: &[u8]) -> ir::Atom {
    ir::Atom::new(bytes).unwrap()
}

fn langc_exe() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // The langc binary lives at workspace target/debug/langc.
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/langc")
}

fn temp_platform(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-neg-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_desc(dir: &PathBuf, cd: &CompiledDescriptor) {
    let mut buf = [0u8; COMPILED_DESC_MAX_BYTES];
    let n = encode_compiled_desc(cd, &mut buf).unwrap();
    fs::write(dir.join("platform.desc"), &buf[..n]).unwrap();
}

fn run_langc(dir: &PathBuf) -> String {
    let out = std::env::temp_dir().join(format!(
        "tyu-langc-neg-out-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-none")
        .arg(format!("--platform={}", dir.display()))
        .arg(format!("--out-dir={}", out.display()))
        .arg("nonexistent.mod")
        .output()
        .expect("langc invocation");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&out);
    stderr
}

#[test]
fn size_zero_window_desc_is_e3647() {
    let dir = temp_platform("size-zero");
    let mut cd = CompiledDescriptor::default();
    cd.window_count = 1;
    cd.windows[0] = MmioWindowSpec {
        id: 0,
        name: atom(b"mmio"),
        kind: MmioWindowKind::Emulated,
        base: None,
        size: 0,
    };
    write_desc(&dir, &cd);

    let stderr = run_langc(&dir);
    assert!(
        stderr.contains("E3647") && stderr.contains("window size must be > 0"),
        "size-zero descriptor must be E3647, got: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overlapping_windows_desc_is_e3647() {
    let dir = temp_platform("overlap");
    let mut cd = CompiledDescriptor::default();
    cd.window_count = 2;
    cd.windows[0] = MmioWindowSpec {
        id: 0,
        name: atom(b"a"),
        kind: MmioWindowKind::Bus,
        base: Some(0x40000000),
        size: 0x1000,
    };
    cd.windows[1] = MmioWindowSpec {
        id: 1,
        name: atom(b"b"),
        kind: MmioWindowKind::Bus,
        base: Some(0x40000800),
        size: 0x1000,
    };
    write_desc(&dir, &cd);

    let stderr = run_langc(&dir);
    assert!(
        stderr.contains("E3647") && stderr.contains("bus windows overlap"),
        "overlapping descriptor must be E3647, got: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}
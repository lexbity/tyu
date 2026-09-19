//! `--emit=ir` text-format committed golden and consumer check (P4, D-8).
//!
//! The format's in-tree consumer contract: the first line is checked with
//! `ir::check_format_ver` before anything else is parsed — the fail-fast the
//! D-8 compatibility policy requires — and the emitted text is pinned against
//! a committed artifact so structural drift cannot land unnoticed. Run-to-run
//! byte stability and descriptor-formatting invariance live in
//! `ir_stability.rs`.
//!
//! Bless a deliberate format change with TYU_BLESS_IR_GOLDEN=1 and review the
//! diff; a format_ver bump MUST accompany any structural change (D-8 table).

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

fn hosted_desc_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("runtime")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-ir-golden-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The same symbolic MMIO module `platform_resolution.rs` pins its negatives
/// with; compiled against the hosted runtime descriptor it exercises the
/// aperture-use table and `aperture=` place annotations in the emit.
const MMIO_MOD: &str = "module Main;\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
end;\n\
const gpio = GPIO @ board.gpio;\n\
: read ( -- u32 )\n\
  &gpio.DATA @u32\n\
;\n\
end;\n";

/// Compile `--emit=ir` and return stdout bytes; panics on compile failure.
fn compile_ir_bytes() -> Vec<u8> {
    let dir = temp_dir("run");
    fs::write(dir.join("Main.mod"), MMIO_MOD).unwrap();
    let out = Command::new(langc_exe())
        .current_dir(&dir)
        .arg("--emit=ir")
        .arg(format!("--platform={}", hosted_desc_dir().display()))
        .arg("Main.mod")
        .output()
        .expect("spawn langc");
    assert!(
        out.status.success(),
        "emit=ir compile failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
    out.stdout
}

fn first_line(bytes: &[u8]) -> &[u8] {
    bytes.split(|&b| b == b'\n').next().unwrap_or(&[])
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/ir_mmio.ir")
}

#[test]
fn ir_emit_header_passes_consumer_check() {
    let bytes = compile_ir_bytes();
    ir::check_format_ver(first_line(&bytes))
        .unwrap_or_else(|e| panic!("fresh --emit=ir output rejected by consumer check: {e}"));
}
// Run-to-run byte stability and descriptor-formatting invariance are owned by
// `ir_stability.rs`; this file owns the committed artifact and the consumer
// check against it.

#[test]
fn ir_golden_matches_committed() {
    let bytes = compile_ir_bytes();
    ir::check_format_ver(first_line(&bytes))
        .unwrap_or_else(|e| panic!("fresh --emit=ir output rejected by consumer check: {e}"));
    let golden = golden_path();
    if std::env::var_os("TYU_BLESS_IR_GOLDEN").is_some() {
        if let Some(parent) = golden.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&golden, &bytes)
            .unwrap_or_else(|err| panic!("failed to bless {}: {err}", golden.display()));
        return;
    }
    let expected = fs::read(&golden).unwrap_or_else(|err| {
        panic!(
            "missing IR golden {}: {err}; rerun with TYU_BLESS_IR_GOLDEN=1",
            golden.display()
        )
    });
    assert_eq!(
        expected, bytes,
        "--emit=ir golden drifted: every structural change MUST bump format_ver (D-8)"
    );
}

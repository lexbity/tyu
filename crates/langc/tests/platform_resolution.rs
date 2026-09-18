//! Negative resolution tests for P4 symbolic MMIO (D-1, FR-1/3/4):
//! E3640 (MMIO without --platform), E3641 (raw base under a descriptor),
//! E3644 (unknown `board.<instance>`), E3647 (source register-map rows
//! diverging from the descriptor device).

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
        "tyu-langc-res-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run langc `--emit=ir` on `src` with optional `--platform`; return stderr.
fn compile_ir(src: &str, platform: Option<&str>) -> String {
    let dir = temp_dir("ir");
    fs::write(dir.join("Main.mod"), src).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.current_dir(&dir).arg("--emit=ir").arg("Main.mod");
    if let Some(p) = platform {
        cmd.arg(format!("--platform={p}"));
    }
    let out = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = fs::remove_dir_all(&dir);
    stderr
}

const MMIO_MOD: &str = "module Main;\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
end;\n\
const gpio = GPIO @ board.gpio;\n\
: read ( -- u32 )\n\
  &gpio.DATA @u32\n\
;\n\
end;\n";

#[test]
fn e3640_mmio_without_platform() {
    let stderr = compile_ir(MMIO_MOD, None);
    assert!(
        stderr.contains("E3640"),
        "MMIO without --platform must be E3640, got: {stderr}"
    );
}

#[test]
fn e3641_raw_base_under_descriptor() {
    let src = MMIO_MOD.replace("board.gpio", "0x0");
    let stderr = compile_ir(&src, Some(hosted_desc_dir().to_str().unwrap()));
    assert!(
        stderr.contains("E3641"),
        "raw base under a descriptor must be E3641, got: {stderr}"
    );
}

#[test]
fn e3644_unknown_board_instance() {
    let src = MMIO_MOD.replace("board.gpio", "board.nosuch");
    let stderr = compile_ir(&src, Some(hosted_desc_dir().to_str().unwrap()));
    assert!(
        stderr.contains("E3644"),
        "unknown board instance must be E3644, got: {stderr}"
    );
}

#[test]
fn e3647_row_diverges_from_descriptor() {
    // The hosted descriptor declares DATA at 0x00 (gpio) and 0x10
    // (gpio_data10); a row at a truly-declared-nowhere offset diverges.
    let src = MMIO_MOD.replace("0x00 DATA", "0x08 DATA");
    let stderr = compile_ir(&src, Some(hosted_desc_dir().to_str().unwrap()));
    assert!(
        stderr.contains("E3647"),
        "diverging register row must be E3647, got: {stderr}"
    );
}

#[test]
fn e3647_missing_register_from_descriptor() {
    let src = "module Main;\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
  0x04 EXTRA u32 rw\n\
end;\n\
const gpio = GPIO @ board.gpio;\n\
: read ( -- u32 )\n\
  &gpio.EXTRA @u32\n\
;\n\
end;\n";
    let stderr = compile_ir(src, Some(hosted_desc_dir().to_str().unwrap()));
    assert!(
        stderr.contains("E3647"),
        "register absent from the descriptor must be E3647, got: {stderr}"
    );
}

#[test]
fn symbolic_compile_succeeds() {
    let stderr = compile_ir(MMIO_MOD, Some(hosted_desc_dir().to_str().unwrap()));
    assert!(
        stderr.is_empty(),
        "valid symbolic MMIO must compile, got: {stderr}"
    );
}
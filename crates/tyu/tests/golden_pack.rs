//! Golden test for `lmod-pack`: byte-exact output comparison.
//!
//! P0.1 — The `lmod-pack` binary must produce a byte-identical `.lmod` to the
//! committed golden file when given the same `.o` input.
//!
//! The golden file lives at `test-goldens/packed.lmod` and was generated from
//! `test-goldens/golden.mod` compiled with `langc --emit=obj`.
//!
//! This guards Phase 5 (library extraction) against accidental output changes.

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

fn golden_dir() -> PathBuf {
    workspace_root().join("test-goldens")
}

#[test]
fn golden_pack_matches_committed() {
    // Ensure lmod-pack is built.
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let gold = golden_dir();

    // Re-compile the golden source in a temp dir to get a fresh .o.
    let tmp = std::env::temp_dir()
        .join("tyu_golden_pack")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    std::fs::write(
        tmp.join("golden.mod"),
        std::fs::read(gold.join("golden.mod")).unwrap(),
    )
    .unwrap();

    let output = Command::new(tyu::test_helpers::bin("langc"))
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-none",
            &format!("--out-dir={}", tmp.to_string_lossy()),
            tmp.join("golden.mod").to_string_lossy().as_ref(),
        ])
        .output()
        .expect("langc");
    assert!(
        output.status.success(),
        "langc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Find the .o produced by langc.
    let o_files: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("o"))
        .collect();
    assert_eq!(o_files.len(), 1, "expected exactly one .o file");
    let o_path = &o_files[0];

    // Pack into a fresh .lmod in temp dir.
    let fresh_lmod = tmp.join("packed.lmod");
    let status = Command::new(tyu::test_helpers::bin("lmod-pack"))
        .args([
            o_path.to_string_lossy().as_ref(),
            fresh_lmod.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("lmod-pack");
    assert!(status.success(), "lmod-pack failed");

    // Compare against committed golden.
    let golden_bytes = std::fs::read(gold.join("packed.lmod")).unwrap();
    let fresh_bytes = std::fs::read(&fresh_lmod).unwrap();

    assert_eq!(
        fresh_bytes.len(),
        golden_bytes.len(),
        "packed.lmod size mismatch: got {} bytes, expected {} bytes",
        fresh_bytes.len(),
        golden_bytes.len(),
    );
    assert_eq!(
        fresh_bytes, golden_bytes,
        "packed.lmod byte mismatch — the packer output has changed from the committed golden.\n\
         If the change is intentional, regenerate the golden:\n  \
         cargo build -p langc -p lmod-pack && \\\n  \
         ./target/debug/lmod-pack <(./target/debug/langc ...) test-goldens/packed.lmod\n\
         Then commit the updated golden."
    );
}

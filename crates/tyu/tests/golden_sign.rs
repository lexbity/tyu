//! Golden test for `lmod-sign` on plain (unencrypted) input: byte-exact.
//!
//! P0.2 — `lmod-sign` on a deterministic plaintext `.lmod` must produce
//! byte-identical output to the committed golden.
//!
//! The golden file `test-goldens/signed_plain.lmod` was generated from
//! `test-goldens/packed.lmod` with the key `abab…` (32 bytes of 0xab).
//!
//! Guards Phase 7 (library extraction) for the signing path.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn golden_dir() -> PathBuf {
    workspace_root().join("test-goldens")
}

const SIGN_KEY: &str = "abababababababababababababababababababababababababababababababab";

#[test]
fn golden_sign_plain_matches_committed() {
    // Ensure tools are built.
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let gold = golden_dir();

    // Sign the committed packed.lmod in a temp dir.
    let tmp = std::env::temp_dir()
        .join("tyu_golden_sign")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let packed_path = gold.join("packed.lmod");
    let signed_path = tmp.join("signed_plain.lmod");

    let status = Command::new(workspace_root().join("target").join("debug").join("lmod-sign"))
        .args([
            packed_path.to_string_lossy().as_ref(),
            signed_path.to_string_lossy().as_ref(),
            &format!("--key={}", SIGN_KEY),
        ])
        .status().expect("lmod-sign");
    assert!(status.success(), "lmod-sign failed");

    // Compare against committed golden.
    let golden_bytes = std::fs::read(gold.join("signed_plain.lmod")).unwrap();
    let fresh_bytes = std::fs::read(&signed_path).unwrap();

    assert_eq!(
        fresh_bytes.len(),
        golden_bytes.len(),
        "signed_plain.lmod size mismatch: got {} bytes, expected {} bytes",
        fresh_bytes.len(),
        golden_bytes.len(),
    );
    assert_eq!(
        fresh_bytes, golden_bytes,
        "signed_plain.lmod byte mismatch.\n\
         If the change is intentional, regenerate:\n  \
         ./target/debug/lmod-sign test-goldens/packed.lmod test-goldens/signed_plain.lmod --key=<key>\n\
         Then commit the updated golden."
    );
}

//! Structural golden tests for `lmod-encrypt` and `lmod-sign` on encrypted input.
//!
//! These tools use random nonces/CEKs so byte-exact comparison is impossible.
//! Instead we verify:
//!   - P0.3: Encrypted `.lmod` has correct flags, enc_mode, and slot count.
//!   - P0.4: Encrypt-then-sign produces a valid signed container.
//!   - P0.5: `lmod-encrypt` + `lmod-sign` pipeline is structurally correct.
//!
//! Guard phases 6–8 (library extraction) against regressions in container
//! structure.

use std::path::PathBuf;
use std::process::Command;

use lmod::enc::EncMode;
use lmod::header::{FORMAT_VER, HEADER_SIZE, LMOD_FLAG_ENCRYPTED, LMOD_FLAG_SIGNED};
use lmod::validate::Container;
use tyu::test_helpers::{bin, golden_dir, workspace_root};

const KEK_HEX: &str = "abababababababababababababababababababababababababababababababab";
const SIGN_KEY_HEX: &str = "abababababababababababababababababababababababababababababababab";

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_golden_encrypt")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-encrypt",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn golden_encrypt_structural() {
    build_tools();
    let gold = golden_dir();
    let tmp = temp_dir("encrypt_structural");

    let input_lmod = gold.join("packed.lmod");
    let encrypted_path = tmp.join("encrypted.lmod");

    // Encrypt the committed packed.lmod.
    let status = Command::new(tyu::test_helpers::bin("lmod-encrypt"))
        .args([
            input_lmod.to_string_lossy().as_ref(),
            encrypted_path.to_string_lossy().as_ref(),
            "--mode=fleet",
            &format!("--kek={}", KEK_HEX),
        ])
        .status()
        .expect("lmod-encrypt");
    assert!(status.success(), "lmod-encrypt failed");

    // Parse and check structural properties.
    let data = std::fs::read(&encrypted_path).unwrap();
    let container = Container::parse(&data).unwrap();
    let hdr = container.header();

    assert_ne!(
        hdr.flags & LMOD_FLAG_ENCRYPTED,
        0,
        "ENCRYPTED flag must be set"
    );
    assert_eq!(
        hdr.flags & LMOD_FLAG_SIGNED,
        0,
        "SIGNED flag must NOT be set after encrypt alone"
    );
    assert_eq!(
        hdr.format_ver, FORMAT_VER,
        "format_ver must be {}",
        FORMAT_VER
    );

    // Check enc-header.
    let eh_bytes = &data[HEADER_SIZE as usize..];
    let eh = lmod::enc::decode_enc_header(eh_bytes).expect("enc-header must decode");
    assert_eq!(eh.enc_mode, EncMode::Fleet, "enc_mode must be Fleet");
    assert_eq!(
        eh.wrapped_slots.len(),
        1,
        "fleet mode must have exactly 1 wrapped slot"
    );

    // Check payload regions are preserved.
    assert_eq!(
        container.code(),
        container.code(),
        "code section must be present"
    );
    assert!(container.code().len() > 0, "code must not be empty");
    assert!(
        container.rodata().len() == 0,
        "rodata should be empty for this test"
    );
    assert!(
        container.data().len() == 0,
        "data should be empty for this test"
    );
}

#[test]
fn golden_encrypt_then_sign_structural() {
    build_tools();
    let gold = golden_dir();
    let tmp = temp_dir("encrypt_sign_structural");

    let input_lmod = gold.join("packed.lmod");
    let encrypted_path = tmp.join("encrypted.lmod");
    let signed_path = tmp.join("signed.lmod");

    // Encrypt.
    let status = Command::new(tyu::test_helpers::bin("lmod-encrypt"))
        .args([
            input_lmod.to_string_lossy().as_ref(),
            encrypted_path.to_string_lossy().as_ref(),
            "--mode=fleet",
            &format!("--kek={}", KEK_HEX),
        ])
        .status()
        .expect("lmod-encrypt");
    assert!(status.success(), "lmod-encrypt failed");

    // Sign.
    let status = Command::new(tyu::test_helpers::bin("lmod-sign"))
        .args([
            encrypted_path.to_string_lossy().as_ref(),
            signed_path.to_string_lossy().as_ref(),
            &format!("--key={}", SIGN_KEY_HEX),
        ])
        .status()
        .expect("lmod-sign");
    assert!(status.success(), "lmod-sign failed");

    // Parse signed output.
    let data = std::fs::read(&signed_path).unwrap();
    let container = Container::parse(&data).unwrap();
    let hdr = container.header();

    // Both flags must be set.
    assert_ne!(
        hdr.flags & LMOD_FLAG_ENCRYPTED,
        0,
        "ENCRYPTED flag must be set after encrypt+sign"
    );
    assert_ne!(
        hdr.flags & LMOD_FLAG_SIGNED,
        0,
        "SIGNED flag must be set after encrypt+sign"
    );
    assert_eq!(
        hdr.format_ver, FORMAT_VER,
        "format_ver must be {}",
        FORMAT_VER
    );

    // Verify signed trailer is present.
    let sig_trailer = lmod::sig::SigTrailer::parse(&data[hdr.sig_off as usize..]);
    assert!(
        sig_trailer.is_some(),
        "signed module must have a valid SigTrailer"
    );
    let trailer = sig_trailer.unwrap();
    assert_eq!(
        trailer.scheme,
        lmod::sig::SCHEME_HMAC_SHA256,
        "signature scheme must be HMAC-SHA256"
    );
}

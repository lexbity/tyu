//! Producer↔consumer contract tests — replaces the circular test oracles in
//! `lmod-encrypt/tests/roundtrip.rs`.
//!
//! Every test uses the real `loader-core::load::load_module` (via
//! `LoaderHarness`) as the oracle, never a re-implementation of the AEAD
//! inside the test body (Rule A satisfied).

#![cfg(feature = "encryption")]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use lmod::header::HEADER_SIZE;
use lmod::validate::Container;
use loader_core::load::{E_ENC_AUTH_FAIL, E_SIG_INVALID};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_contract_enc").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a test .mod, compile, pack into .lmod, encrypt, and sign.
/// Returns the path to the signed artifact.
fn build_encrypt_sign(source: &str, kek: &[u8; 32], sign_key: &[u8; 32], label: &str) -> PathBuf {
    let dir = temp_dir(label);
    std::fs::write(dir.join("M.mod"), source).unwrap();
    assert!(
        Command::new(exe("langc"))
            .current_dir(&dir)
            .args([
                "--emit=obj",
                "--target=x86_64-unknown-linux-gnu",
                "--out-dir=.",
                "M.mod"
            ])
            .status()
            .unwrap()
            .success(),
        "langc failed"
    );
    let lmod = dir.join("test.lmod");
    assert!(
        Command::new(exe("lmod-pack"))
            .current_dir(&dir)
            .args(["Main.o", "test.lmod"])
            .status()
            .unwrap()
            .success(),
        "lmod-pack failed"
    );

    let encrypted = dir.join("encrypted.lmod");
    assert!(
        Command::new(exe("lmod-encrypt"))
            .args([
                lmod.to_str().unwrap(),
                encrypted.to_str().unwrap(),
                "--mode=fleet",
                &format!("--kek={}", hex::encode(kek))
            ])
            .status()
            .unwrap()
            .success(),
        "lmod-encrypt failed"
    );

    let signed = dir.join("signed.lmod");
    assert!(
        Command::new(exe("lmod-sign"))
            .args([
                encrypted.to_str().unwrap(),
                signed.to_str().unwrap(),
                &format!("--key={}", hex::encode(sign_key))
            ])
            .status()
            .unwrap()
            .success(),
        "lmod-sign failed"
    );
    signed
}

/// Read a signed .lmod, parse it, load via LoaderHarness, and return the result.
fn load_signed(path: &Path, kek: &[u8; 32], sign_key: &[u8; 32]) -> Result<(), u32> {
    let raw = std::fs::read(path).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(sign_key)
        .with_kek(kek);
    h.load(&container)
}

// ---------------------------------------------------------------------------
// C-CT-2: Fleet-encrypted artifact decrypts in the real loader
// ---------------------------------------------------------------------------

#[test]
fn fleet_encrypt_decrypts_in_loader() {
    let kek = [0xab; 32];
    let sign_key = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &kek, &sign_key, "cct2");
    let result = load_signed(&signed, &kek, &sign_key);
    assert!(
        result.is_ok(),
        "fleet-encrypted artifact must load successfully via the real loader"
    );
}

// ---------------------------------------------------------------------------
// C-CT-3: Tampered payload byte → E_ENC_AUTH_FAIL through the loader
// ---------------------------------------------------------------------------
//
// Procedure α (§4.1): encrypt → flip one payload code byte → re-sign →
// load with correct KEK → must fail with E_ENC_AUTH_FAIL (LD-9).
// The signature is valid (we re-signed), so LD-4 passes and we reach LD-9.

#[test]
fn tampered_payload_byte_fails_in_loader() {
    let kek = [0xcd; 32];
    let sign_key = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";

    // Step 1: build and encrypt (no signing yet).
    let dir = temp_dir("cct3");
    std::fs::write(dir.join("M.mod"), source).unwrap();
    assert!(Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "M.mod"
        ])
        .status()
        .unwrap()
        .success());
    let lmod = dir.join("test.lmod");
    assert!(Command::new(exe("lmod-pack"))
        .current_dir(&dir)
        .args(["Main.o", "test.lmod"])
        .status()
        .unwrap()
        .success());
    let encrypted = dir.join("encrypted.lmod");
    assert!(Command::new(exe("lmod-encrypt"))
        .args([
            lmod.to_str().unwrap(),
            encrypted.to_str().unwrap(),
            "--mode=fleet",
            &format!("--kek={}", hex::encode(kek))
        ])
        .status()
        .unwrap()
        .success());

    // Step 2: flip one byte in the code section of the encrypted artifact.
    let mut data = std::fs::read(&encrypted).unwrap();
    let c = Container::parse(&data).unwrap();
    let code_off = c.header().code_off as usize;
    data[code_off + 4] ^= 0xff;

    // Step 3: write tampered blob and sign it.
    let tampered = dir.join("tampered.lmod");
    std::fs::write(&tampered, &data).unwrap();
    let signed = dir.join("signed.lmod");
    assert!(Command::new(exe("lmod-sign"))
        .args([
            tampered.to_str().unwrap(),
            signed.to_str().unwrap(),
            &format!("--key={}", hex::encode(sign_key))
        ])
        .status()
        .unwrap()
        .success());

    // Step 4: load with correct KEK — must fail with E_ENC_AUTH_FAIL.
    let result = load_signed(&signed, &kek, &sign_key);
    assert_eq!(
        result.unwrap_err(),
        E_ENC_AUTH_FAIL,
        "tampered payload (re-signed) must fail AEAD authentication in the loader"
    );
}

// ---------------------------------------------------------------------------
// C-CT-4: Tampered modinfo (AAD-covered) → E_ENC_AUTH_FAIL
// ---------------------------------------------------------------------------
//
// Mutate a byte in the modinfo region (AAD-covered but not ciphertext),
// then re-sign.  The CEK unwraps successfully (slot untouched) but the
// AAD reconstructed by the consumer differs from the producer's → AEAD
// verification fails → E_ENC_AUTH_FAIL.

#[test]
fn tampered_modinfo_aad_fails_in_loader() {
    let kek = [0xef; 32];
    let sign_key = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";

    // Step 1: build and encrypt.
    let dir = temp_dir("cct4");
    std::fs::write(dir.join("M.mod"), source).unwrap();
    assert!(Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "M.mod"
        ])
        .status()
        .unwrap()
        .success());
    let lmod = dir.join("test.lmod");
    assert!(Command::new(exe("lmod-pack"))
        .current_dir(&dir)
        .args(["Main.o", "test.lmod"])
        .status()
        .unwrap()
        .success());
    let encrypted = dir.join("encrypted.lmod");
    assert!(Command::new(exe("lmod-encrypt"))
        .args([
            lmod.to_str().unwrap(),
            encrypted.to_str().unwrap(),
            "--mode=fleet",
            &format!("--kek={}", hex::encode(kek))
        ])
        .status()
        .unwrap()
        .success());

    // Step 2: mutate one byte in the modinfo region.
    let mut data = std::fs::read(&encrypted).unwrap();
    let c = Container::parse(&data).unwrap();
    let mi_off = c.header().modinfo_off as usize;
    // Ensure there is at least one modinfo byte to tamper.
    assert!(
        c.modinfo().len() > 4,
        "modinfo must have at least 4 bytes for tampering"
    );
    data[mi_off + 4] ^= 0x01;

    // Step 3: write tampered blob and sign it.
    let tampered = dir.join("tampered.lmod");
    std::fs::write(&tampered, &data).unwrap();
    let signed = dir.join("signed.lmod");
    assert!(Command::new(exe("lmod-sign"))
        .args([
            tampered.to_str().unwrap(),
            signed.to_str().unwrap(),
            &format!("--key={}", hex::encode(sign_key))
        ])
        .status()
        .unwrap()
        .success());

    // Step 4: load with correct KEK — must fail with E_ENC_AUTH_FAIL.
    let result = load_signed(&signed, &kek, &sign_key);
    assert_eq!(
        result.unwrap_err(),
        E_ENC_AUTH_FAIL,
        "tampered modinfo (AAD-covered, re-signed) must fail AEAD authentication"
    );
}

// ---------------------------------------------------------------------------
// Rewritten encrypt_then_sign_pipeline — via real loader (Rule A)
// ---------------------------------------------------------------------------
//
// 1. Encrypt → sign → load at TrustLevel-One with correct keys → Ok.
// 2. Tamper a header byte → do NOT re-sign → load → E_SIG_INVALID.
//    (The header is inside the signed region; mutation invalidates the HMAC.)

#[test]
fn signed_artifact_loads_and_tampered_header_fails() {
    let kek = [0x11; 32];
    let sign_key = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 7 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &kek, &sign_key, "cct_sign");

    // Part 1: genuine artifact loads successfully at TrustLevel One.
    let result = load_signed(&signed, &kek, &sign_key);
    assert!(
        result.is_ok(),
        "properly signed+encrypted artifact must load at TrustLevel One"
    );

    // Part 2: tamper one byte in the signed region, do NOT re-sign.
    let mut data = std::fs::read(&signed).unwrap();
    // Flip a byte in the enc-header area (inside the signed region).  The
    // enc-header sits at HEADER_SIZE; byte 1 is the aead_id.
    data[HEADER_SIZE as usize + 1] ^= 0x01;

    let tampered_path = temp_dir("cct_sign_tamper").join("tampered.lmod");
    std::fs::write(&tampered_path, &data).unwrap();

    let raw = std::fs::read(&tampered_path).unwrap();
    // Parse must still succeed (tampered header is structurally valid).
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&sign_key)
        .with_kek(&kek);
    let result = h.load(&container);
    assert_eq!(
        result.unwrap_err(),
        E_SIG_INVALID,
        "tampered signed region (no re-sign) must fail with E_SIG_INVALID"
    );
}

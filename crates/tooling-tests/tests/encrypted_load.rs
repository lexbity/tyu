//! Encrypted module load and execute tests.
//!
//! Uses `lmod-encrypt` to produce a Fleet-encrypted `.lmod`, signs it,
//! and loads it through `LoaderHarness` (which wraps the real loader).
//!
//! All encrypted tests use the real loader as the oracle (Rule A).

#![cfg(feature = "encryption")]

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::*;
use lmod::validate::Container;
use loader_core::load::{E_ENC_NO_KEY, E_SIG_INVALID};

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
    let dir =
        std::env::temp_dir()
            .join("tyu_enc_load")
            .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build, compile, pack, encrypt, and sign a test .mod.
/// Returns the path to the signed artifact.
fn build_encrypt_sign(source: &str, kek: &[u8; 32], label: &str) -> PathBuf {
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
            .args([encrypted.to_str().unwrap(), signed.to_str().unwrap()])
            .status()
            .unwrap()
            .success(),
        "lmod-sign failed"
    );
    signed
}

/// Build, compile, pack, and encrypt (without signing).
/// Returns the path to the encrypted artifact.
fn build_encrypt_only(source: &str, kek: &[u8; 32], label: &str) -> PathBuf {
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
    encrypted
}

// ---------------------------------------------------------------------------
// E-1: Encrypted module loads successfully
// ---------------------------------------------------------------------------

#[test]
fn encrypted_fleet_loads() {
    let kek = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &kek, "e1");
    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&kek);
    let result = h.load(&container);
    assert!(result.is_ok(), "encrypted fleet module must load (E-1)");
}

// ---------------------------------------------------------------------------
// E-2: Encrypted module loads and executes, returning main's value
// ---------------------------------------------------------------------------

#[test]
fn encrypted_fleet_executes() {
    let kek = [0xab; 32];
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &kek, "e2");
    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&kek);
    let result = h.load_and_run(&container).unwrap();
    assert_eq!(result, 42, "encrypted fleet module must return 42 (E-2)");
}

// ---------------------------------------------------------------------------
// E-3: Wrong KEK must fail with E_ENC_NO_KEY
// ---------------------------------------------------------------------------

#[test]
fn encrypted_wrong_kek_fails() {
    let enc_kek = [0xcd; 32];
    let wrong_kek = [0xef; 32];
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &enc_kek, "e3");
    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&wrong_kek);
    let result = h.load(&container);
    assert_eq!(
        result.unwrap_err(),
        E_ENC_NO_KEY,
        "wrong KEK must give E_ENC_NO_KEY (E-3)"
    );
}

// ---------------------------------------------------------------------------
// E-4: Unsigned encrypted module at TrustLevel One must fail with E_SIG_INVALID
// ---------------------------------------------------------------------------
//
// The loader checks signature (LD-3/4) before decryption (LD-5+).  An
// encrypted container without LMOD_FLAG_SIGNED presented to a TrustLevel-One
// platform fails at LD-4 with E_SIG_INVALID.

#[test]
fn encrypted_unsigned_rejected() {
    let kek = [0x11; 32];
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";
    let encrypted = build_encrypt_only(source, &kek, "e4");
    let raw = std::fs::read(&encrypted).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&kek);
    let result = h.load(&container);
    assert_eq!(
        result.unwrap_err(),
        E_SIG_INVALID,
        "unsigned encrypted module at TrustLevel One must give E_SIG_INVALID (E-4)"
    );
}

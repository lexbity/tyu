//! S2 Phase 17 — `52xx` negative corpus.
//!
//! One end-to-end test per `52xx` error code.  Each test compiles a .mod,
//! packs to .lmod, then attempts to load and verifies the expected error.

mod common;

use std::path::PathBuf;
use std::process::Command;

use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet, E_ENC_AUTH_FAIL, E_ENC_BAD_HEADER, E_ENC_NO_KEY, E_ENC_REQUIRES_SIGNED, E_ENC_UNSUPPORTED};
use loader_core::platform::Tier;
use loader_core::symbols::SymMap;
use common::*;

/// Set up the platform and global map for loading.
fn setup_loader<'a>(
    container: &Container,
    expected_abi_hash: u64,
) -> (HostedLoaderPlatform, SymMap<'a, 256>, LoadedSet<64>) {
    let block_size = container.code().len() + container.rodata().len() + container.data().len() + 32;
    let block_size = (block_size + 4095) & !4095;
    let mut plat = HostedLoaderPlatform::new(expected_abi_hash);
    plat.reserve(block_size).unwrap();
    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let loaded_set = LoadedSet::<64>::new();
    (plat, global_map, loaded_set)
}

// -- Tests --

// ---------------------------------------------------------------------------
// 5200 — E_ABI_MISMATCH
// ---------------------------------------------------------------------------

#[test]
fn e_5200_abi_hash_mismatch() {
    let dir = fresh_dir("e5200");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    // Use a wrong expected_abi_hash.
    let wrong_hash = 0xDEADBEEF;
    let (mut plat, mut map, mut set) = setup_loader(&container, wrong_hash);
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5200: mismatched abi_hash should fail");
    assert_eq!(result.unwrap_err(), 5200);
}

// ---------------------------------------------------------------------------
// 5201 — E_BAD_CONTAINER
// ---------------------------------------------------------------------------

#[test]
fn e_5201_bad_container() {
    let dir = fresh_dir("e5201");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Corrupt the magic bytes.
    raw[0] = 0xFF;

    let result = Container::parse(&raw);
    assert!(result.is_err(), "5201: corrupted container should fail parse");
    assert_eq!(result.unwrap_err().0, 5201);
}

// ---------------------------------------------------------------------------
// 5202 — E_SIG_INVALID
// ---------------------------------------------------------------------------

#[test]
fn e_5202_signed_flag_without_tier1() {
    let dir = fresh_dir("e5202a");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Set the SIGNED flag without actually providing a signature trailer.
    raw[6] |= 0x01;

    let container = Container::parse(&raw).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let (mut plat, mut map, mut set) = setup_loader(&container, abi_hash);
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5202: signed flag without signature should fail");
    assert_eq!(result.unwrap_err(), 5202);
}

#[test]
fn e_5202_tier1_without_signature() {
    let dir = fresh_dir("e5202b");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    // Use a Tier-1 platform without a valid signature.
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let bsize = (container.code().len() + 4095) & !4095;
    let mut plat_tier1 = HostedLoaderPlatform::new(abi_hash).with_key(b"test-key", Tier::One);
    plat_tier1.reserve(bsize).unwrap();

    let ds_high_addr = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high_addr);
    let mut set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat_tier1, &mut global_map, &mut set);
    assert!(result.is_err(), "5202: Tier 1 without signature should fail");
    assert_eq!(result.unwrap_err(), 5202);
}

// ---------------------------------------------------------------------------
// 5205 — E_SYMBOL_UNRESOLVED
// ---------------------------------------------------------------------------

#[test]
fn e_5205_unresolved_symbol() {
    let dir = fresh_dir("e5205");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let bsize = (container.code().len() + 4095) & !4095;
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    plat.reserve(bsize).unwrap();

    // Empty symbol map — no runtime symbols registered.
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    let mut set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat, &mut global_map, &mut set);
    assert!(result.is_err(), "5205: unresolved symbol should fail");
    assert_eq!(result.unwrap_err(), 5205);
}

// ---------------------------------------------------------------------------
// 5210 — E_MODULE_ALREADY_LOADED
// ---------------------------------------------------------------------------

#[test]
fn e_5210_already_loaded() {
    let dir = fresh_dir("e5210");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let (mut plat, mut map, mut set) = setup_loader(&container, abi_hash);

    // First load should succeed.
    let r1 = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(r1.is_ok(), "first load should succeed: {:?}", r1.err());

    // Second load should fail with already-loaded.
    // We need a fresh container because the first load's regions are gone.
    let raw2 = std::fs::read(&lmod_path).unwrap();
    let container2 = Container::parse(&raw2).unwrap();

    // Actually the issue is that the same abi_hash is in the loaded_set.
    // We parse the container again (different object but same abi_hash).
    let r2 = load_module(&container2, &mut plat, &mut map, &mut set);
    assert!(r2.is_err(), "5210: duplicate load should fail");
    assert_eq!(r2.unwrap_err(), 5210);
}

// ---------------------------------------------------------------------------
// 5213 — E_ENC_UNSUPPORTED (encrypted container without encryption feature)
// ---------------------------------------------------------------------------
//
// Note: this error code only fires when the `encryption` feature is OFF.
// When the feature IS enabled, a Tier-0 platform returns E_ENC_REQUIRES_SIGNED
// (5214) instead — tested by e_5214_encrypted_at_tier0 below.

#[test]
#[cfg(not(feature = "encryption"))]
fn e_5213_encrypted_container_unsupported() {
    let dir = fresh_dir("e5213");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Set the encrypted flag.
    raw[6] |= 0x02;

    let container = Container::parse(&raw).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let (mut plat, mut map, mut set) = setup_loader(&container, abi_hash);
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5213: encrypted container should fail without encryption feature");
    assert_eq!(result.unwrap_err(), E_ENC_UNSUPPORTED);
}

// ---------------------------------------------------------------------------
// I-NC-1: 5214 — E_ENC_REQUIRES_SIGNED
// ---------------------------------------------------------------------------
//
// LD-6: encrypted container at Tier 0 (feature on) → E_ENC_REQUIRES_SIGNED.
// This path is only reachable when the encryption feature is enabled.

#[test]
#[cfg(feature = "encryption")]
fn e_5214_encrypted_at_tier0() {
    let dir = fresh_dir("e5214");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Set the encrypted flag.  The container has no valid enc-header, but
    // the Tier-0 check (LD-6) fires before any enc-header parsing.
    raw[6] |= 0x02;

    let container = Container::parse(&raw).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let (mut plat, mut map, mut set) = setup_loader(&container, abi_hash);
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5214: encrypted at Tier 0 should fail");
    assert_eq!(result.unwrap_err(), E_ENC_REQUIRES_SIGNED);
}

// ---------------------------------------------------------------------------
// Helpers for encrypted test artifacts
// ---------------------------------------------------------------------------

fn exe(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap()
        .join("target").join("debug").join(name)
}

/// Build, compile, pack, encrypt (fleet mode), and sign a test .mod.
/// Returns the path to the signed artifact.
fn build_encrypt_sign(source: &str, kek: &[u8; 32], label: &str) -> PathBuf {
    let dir = fresh_dir(label);
    std::fs::write(dir.join("M.mod"), source).unwrap();
    assert!(Command::new(exe("langc")).current_dir(&dir)
        .args(["--emit=obj", "--target=x86_64-unknown-linux-gnu", "--out-dir=.", "M.mod"])
        .status().unwrap().success(), "langc failed");
    let lmod = dir.join("test.lmod");
    assert!(Command::new(exe("lmod-pack")).current_dir(&dir)
        .args(["Main.o", "test.lmod"]).status().unwrap().success(), "lmod-pack failed");

    let encrypted = dir.join("encrypted.lmod");
    assert!(Command::new(exe("lmod-encrypt"))
        .args([lmod.to_str().unwrap(), encrypted.to_str().unwrap(),
               "--mode=fleet", &format!("--kek={}", hex::encode(kek))])
        .status().unwrap().success(), "lmod-encrypt failed");

    let signed = dir.join("signed.lmod");
    assert!(Command::new(exe("lmod-sign"))
        .args([encrypted.to_str().unwrap(), signed.to_str().unwrap()])
        .status().unwrap().success(), "lmod-sign failed");
    signed
}

// ---------------------------------------------------------------------------
// I-NC-2: 5215 — E_ENC_NO_KEY
// ---------------------------------------------------------------------------
//
// LD-8: fleet-encrypted artifact loaded at Tier 1 with the wrong KEK.

#[test]
#[cfg(feature = "encryption")]
fn e_5215_no_matching_kek() {
    let enc_kek = [0xcd; 32];
    let wrong_kek = [0xef; 32];
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";
    let signed = build_encrypt_sign(source, &enc_kek, "e5215");

    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&wrong_kek);
    let result = h.load(&container);
    assert_eq!(result.unwrap_err(), E_ENC_NO_KEY, "wrong KEK must give E_ENC_NO_KEY (5215)");
}

// ---------------------------------------------------------------------------
// I-NC-3: 5216 — E_ENC_AUTH_FAIL
// ---------------------------------------------------------------------------
//
// LD-9: tamper a payload byte in the encrypted blob, re-sign, load with
// correct KEK → AEAD auth failure.  The signature remains valid (we re-signed
// the tampered ciphertext), so LD-4 passes and we reach LD-9.

#[test]
#[cfg(feature = "encryption")]
fn e_5216_payload_auth_fail() {
    let kek = [0x11; 32];
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";

    let dir = fresh_dir("e5216");
    std::fs::write(dir.join("M.mod"), source).unwrap();
    assert!(Command::new(exe("langc")).current_dir(&dir)
        .args(["--emit=obj", "--target=x86_64-unknown-linux-gnu", "--out-dir=.", "M.mod"])
        .status().unwrap().success());
    let lmod = dir.join("test.lmod");
    assert!(Command::new(exe("lmod-pack")).current_dir(&dir)
        .args(["Main.o", "test.lmod"]).status().unwrap().success());

    let encrypted = dir.join("encrypted.lmod");
    assert!(Command::new(exe("lmod-encrypt"))
        .args([lmod.to_str().unwrap(), encrypted.to_str().unwrap(),
               "--mode=fleet", &format!("--kek={}", hex::encode(kek))])
        .status().unwrap().success());

    // Flip one byte in the code section of the encrypted ciphertext.
    let mut data = std::fs::read(&encrypted).unwrap();
    let c = Container::parse(&data).unwrap();
    let code_off = c.header().code_off as usize;
    data[code_off + 4] ^= 0xff;

    // Re-sign the tampered blob.
    let tampered = dir.join("tampered.lmod");
    std::fs::write(&tampered, &data).unwrap();
    let signed = dir.join("signed.lmod");
    assert!(Command::new(exe("lmod-sign"))
        .args([tampered.to_str().unwrap(), signed.to_str().unwrap()])
        .status().unwrap().success());

    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash)
        .tier_one(&[0xab; 32])
        .with_kek(&kek);
    let result = h.load(&container);
    assert_eq!(result.unwrap_err(), E_ENC_AUTH_FAIL,
        "tampered payload (re-signed) must give E_ENC_AUTH_FAIL (5216)");
}

// ---------------------------------------------------------------------------
// I-NC-4: 5217 — E_ENC_BAD_HEADER
// ---------------------------------------------------------------------------
//
// LD-7: set LMOD_FLAG_ENCRYPTED on a plaintext container (no real enc-header),
// then sign it.  At Tier 1 the signature verifies (LD-4 passes), but the
// enc-header is missing/malformed → decode_enc_header returns None →
// E_ENC_BAD_HEADER.

#[test]
#[cfg(feature = "encryption")]
fn e_5217_bad_enc_header() {
    let dir = fresh_dir("e5217");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Set the encrypted flag.  No enc-header exists at HEADER_SIZE —
    // those bytes are the modinfo section, which won't parse as a valid
    // enc-header.
    raw[6] |= 0x02;

    // Sign the container so Tier-1 signature verification passes.
    let enc_flag_set = dir.join("enc_flag.lmod");
    std::fs::write(&enc_flag_set, &raw).unwrap();
    let signed = dir.join("signed.lmod");
    assert!(Command::new(exe("lmod-sign"))
        .args([enc_flag_set.to_str().unwrap(), signed.to_str().unwrap()])
        .status().unwrap().success(), "lmod-sign failed");

    let raw = std::fs::read(&signed).unwrap();
    let container = Container::parse(&raw).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let bsize = (container.code().len() + 4095) & !4095;
    let sign_key = [0xab; 32]; // must match lmod-sign default
    let mut plat = HostedLoaderPlatform::new(abi_hash)
        .with_key(&sign_key, Tier::One);
    plat.reserve(bsize).unwrap();
    let ds_high = allocate_runtime_page();
    let mut map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut map, ds_high);
    let mut set = LoadedSet::<64>::new();
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5217: bad enc-header should fail");
    assert_eq!(result.unwrap_err(), E_ENC_BAD_HEADER);
}

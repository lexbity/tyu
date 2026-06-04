//! S2 Phase 17 — `52xx` negative corpus.
//!
//! One end-to-end test per `52xx` error code.  Each test compiles a .mod,
//! packs to .lmod, then attempts to load and verifies the expected error.

mod common;

use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
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
// 5212 — E_CONTAINER_ENCRYPTED
// ---------------------------------------------------------------------------

#[test]
fn e_5212_encrypted_container() {
    let dir = fresh_dir("e5212");
    let source = "module Main;\n: main ( -- i64 ) 0 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let mut raw = std::fs::read(&lmod_path).unwrap();

    // Set the encrypted flag.
    raw[6] |= 0x02;

    let container = Container::parse(&raw).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let (mut plat, mut map, mut set) = setup_loader(&container, abi_hash);
    let result = load_module(&container, &mut plat, &mut map, &mut set);
    assert!(result.is_err(), "5212: encrypted container should fail");
    assert_eq!(result.unwrap_err(), 5212);
}

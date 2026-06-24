//! S2 Phase 14 — Module acquisition backends.
//!
//! Tests modpack scanning and hosted FS loading.

mod common;

use common::*;
use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
use loader_core::modpack::ModpackIter;
use loader_core::symbols::SymMap;

fn load_lmod_bytes(lmod_bytes: &[u8], plat: &mut HostedLoaderPlatform) -> i64 {
    let container = Container::parse(lmod_bytes).unwrap();
    let bsize = (container.code().len() + 4095) & !4095;
    plat.reserve(bsize).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);

    let mut loaded_set = LoadedSet::<64>::new();
    let _loaded = load_module(&container, plat, &mut global_map, &mut loaded_set).unwrap();

    let main_sym = global_map.lookup_by_name(b"main").unwrap();
    let code_base = main_sym.addr;

    const DS_SIZE: usize = 65536;
    let mut ds_buf = vec![0u8; DS_SIZE];
    let ds_base = ds_buf.as_ptr() as u64;
    let ds_limit = ds_base + DS_SIZE as u64;

    let result: i64;
    unsafe {
        core::arch::asm!(
            "mov r15, {base}", "mov r14, {limit}", "call {fn}", "mov {result}, rax",
            base = in(reg) ds_base, limit = in(reg) ds_limit,
            fn = in(reg) code_base, result = lateout(reg) result,
            out("r15") _, out("r14") _, out("rax") _,
            out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
        );
    }
    result
}

#[test]
fn phase14_modpack_single_blob() {
    let dir = fresh_dir("modpack_single_blob");
    let lmod_path = compile_and_pack("module Main;\n: main ( -- i64 ) 42 ;\nend;\n", &dir);
    let lmod_bytes = std::fs::read(&lmod_path).unwrap();

    let mut modpack = Vec::new();
    modpack.extend_from_slice(&(lmod_bytes.len() as u32).to_le_bytes());
    modpack.extend_from_slice(&lmod_bytes);

    let mut iter = ModpackIter::new_from_slice(&modpack);
    assert_eq!(iter.next_blob().unwrap(), lmod_bytes.as_slice());
    assert!(iter.next_blob().is_none());
}

#[test]
fn phase14_modpack_multi_blob() {
    let dir = fresh_dir("modpack_multi_blob");
    let lmod1 = compile_and_pack("module Main;\n: main ( -- i64 ) 7 ;\nend;\n", &dir);
    let lmod2 = compile_and_pack("module Main;\n: main ( -- i64 ) 3 ;\nend;\n", &dir);
    let bytes1 = std::fs::read(&lmod1).unwrap();
    let bytes2 = std::fs::read(&lmod2).unwrap();

    let mut modpack = Vec::new();
    modpack.extend_from_slice(&(bytes1.len() as u32).to_le_bytes());
    modpack.extend_from_slice(&bytes1);
    modpack.extend_from_slice(&(bytes2.len() as u32).to_le_bytes());
    modpack.extend_from_slice(&bytes2);

    let mut iter = ModpackIter::new_from_slice(&modpack);
    assert_eq!(iter.next_blob().unwrap(), bytes1);
    assert_eq!(iter.next_blob().unwrap(), bytes2);
    assert!(iter.next_blob().is_none());
}

#[test]
fn phase14_host_fs_load_and_run() {
    let dir = fresh_dir("host_fs_load_and_run");
    let lmod_path = compile_and_pack("module Main;\n: main ( -- i64 ) 99 ;\nend;\n", &dir);
    let lmod_bytes = std::fs::read(&lmod_path).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    assert_eq!(load_lmod_bytes(&lmod_bytes, &mut plat), 99);
}

#[test]
fn phase14_modpack_load_and_run() {
    let dir = fresh_dir("modpack_load_and_run");
    let lmod_path = compile_and_pack("module Main;\n: main ( -- i64 ) 77 ;\nend;\n", &dir);
    let lmod_bytes = std::fs::read(&lmod_path).unwrap();

    let mut modpack = Vec::new();
    modpack.extend_from_slice(&(lmod_bytes.len() as u32).to_le_bytes());
    modpack.extend_from_slice(&lmod_bytes);

    let mut iter = ModpackIter::new_from_slice(&modpack);
    let blob = iter.next_blob().expect("modpack should have a blob");

    let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    assert_eq!(load_lmod_bytes(blob, &mut plat), 77);
}

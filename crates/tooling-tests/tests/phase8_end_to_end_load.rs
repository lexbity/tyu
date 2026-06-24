//! S2 Phase 8 — End-to-end hosted load (TrustLevel Zero).
//!
//! Compiles a .mod, packs to .lmod, loads dynamically, calls the exported
//! function, and asserts the result matches a statically-linked build.

mod common;

use common::*;
use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;

#[test]
fn phase8_dynamic_load_matches_static() {
    let dir = fresh_dir("dynamic_load_matches_static");
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nend;\n";

    let expected = static_exit_code(source, &dir);
    assert_eq!(expected, 42);

    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let code_len = container.code().len();
    let rodata_len = container.rodata().len();
    let data_len = container.data().len();
    let bsize = (code_len + rodata_len + data_len + 4095) & !4095;

    let abi_hash_val = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash_val);
    plat.reserve(bsize).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);

    let mut loaded_set = LoadedSet::<64>::new();
    let _loaded = load_module(&container, &mut plat, &mut global_map, &mut loaded_set).unwrap();

    let main_sym = global_map.lookup_by_name(b"main").unwrap();
    let code_base = main_sym.addr;

    let result: i64;
    const DS_SIZE: usize = 65536;
    let mut ds_buf = vec![0u8; DS_SIZE];
    let ds_base = ds_buf.as_ptr() as u64;
    let ds_limit = ds_base + DS_SIZE as u64;
    unsafe {
        core::arch::asm!(
            "mov r15, {base}", "mov r14, {limit}", "call {fn}", "mov {result}, rax",
            base = in(reg) ds_base, limit = in(reg) ds_limit,
            fn = in(reg) code_base, result = lateout(reg) result,
            out("r15") _, out("r14") _, out("rax") _,
            out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
        );
    }
    assert_eq!(
        result, 42,
        "dynamic load returned {result}, expected {expected}"
    );
}

#[test]
fn phase8_module_without_runtime_symbols_fails() {
    let dir = fresh_dir("without_runtime_symbols_fails");
    let lmod_path = compile_and_pack("module Main;\n: main ( -- i64 ) 7 ;\nend;\n", &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let bsize = (container.code().len() + 4095) & !4095;
    let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    plat.reserve(bsize).unwrap();

    let mut global_map: SymMap<'_, 256> = SymMap::new();
    let mut loaded_set = LoadedSet::<64>::new();
    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert!(result.is_err(), "load should fail without runtime symbols");
    assert_eq!(result.unwrap_err(), 5205);
}

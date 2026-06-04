//! S2 Phase 8 — End-to-end hosted load (Tier 0).
//!
//! Compiles a .mod, packs to .lmod, loads dynamically via `load_module`,
//! calls the exported function, and asserts the result matches static.

use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
use loader_core::platform::LoaderPlatform;
use loader_core::symbols::SymMap;
use std::path::PathBuf;
use std::process::Command;

/// Stack overflow handler — panics if ever called.
extern "C" fn __stack_overflow() {
    panic!("__stack_overflow was called at runtime");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_phase8")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Register runtime symbols into the global map.
///
/// All addresses must lie within the same 2 GB window as the loaded code
/// section so that PC-relative relocations fit in a 32-bit signed displacement.
/// The caller provides `data_block` — an mmap'd region adjacent to the code.
fn register_runtime_symbols<'a>(
    map: &mut SymMap<'a, 256>,
    block_base: usize,
    block_len: usize,
) {
    // Place data variables at the end of the combined block.
    let ds_high_addr = block_base + block_len - 16;
    let expected_abi_hash_addr = block_base + block_len - 8;

    // Write initial values.
    unsafe {
        *(ds_high_addr as *mut u64) = block_base as u64;          // __lang_ds_high
        *(expected_abi_hash_addr as *mut u64) = 0;                // __lang_expected_abi_hash
    }

    let stack_overflow_stub = __stack_overflow as usize;
    map.register(b"__stack_overflow", stack_overflow_stub).unwrap();
    map.register(b"__lang_ds_high", ds_high_addr).unwrap();
    map.register(b"__lang_expected_abi_hash", expected_abi_hash_addr).unwrap();
    map.register(b"__lang_trap", stack_overflow_stub).ok();
    map.register(b"__lang_trap_loc", stack_overflow_stub).ok();
}

/// Compile a .mod and pack to .lmod.
fn compile_and_pack(source: &str, out_dir: &PathBuf) -> PathBuf {
    std::fs::write(out_dir.join("M.mod"), source).unwrap();
    let status = Command::new(exe("langc"))
        .current_dir(out_dir)
        .args(["--emit=obj", "--target=x86_64-unknown-linux-gnu", "--out-dir=.", "M.mod"])
        .status().unwrap();
    assert!(status.success(), "langc failed");
    let status = Command::new(exe("lmod-pack"))
        .current_dir(out_dir)
        .args(["Main.o", "Main.lmod"])
        .status().unwrap();
    assert!(status.success(), "lmod-pack failed");
    out_dir.join("Main.lmod")
}

/// Static build — returns the process exit code.
fn static_exit_code(source: &str, dir: &PathBuf) -> i32 {
    std::fs::write(dir.join("S.mod"), source).unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(dir)
        .args(["--emit=asm", "S.mod"])
        .output().unwrap();
    assert!(out.status.success());
    std::fs::write(dir.join("S.asm"), &out.stdout).unwrap();
    Command::new("fasm").current_dir(dir).args(["S.asm", "S_bin"]).status().unwrap();
    Command::new(dir.join("S_bin")).status().unwrap().code().unwrap_or(-1)
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn phase8_dynamic_load_matches_static() {
    let dir = fresh_dir("dynamic_load_matches_static");
    let source = "module Main;\n: main ( -- i64 ) 42 ;\nend;\n";

    let expected = static_exit_code(source, &dir);
    assert_eq!(expected, 42);

    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    // Compute the total allocation needed: code + rodata + data + bss + runtime-data.
    let code_len = container.code().len();
    let rodata_len = container.rodata().len();
    let data_len = container.data().len();
    let runtime_data = 32usize;
    let total = (code_len + rodata_len + data_len + runtime_data + 15) & !15;

    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    plat.reserve(total).unwrap(); // single mmap — everything is adjacent
    let block_base = plat.block_base().unwrap() as usize;

    // Register runtime data at the tail of the reserved block.
    // After load_module carves code+rodata+data from the front, the
    // remaining bytes hold __lang_ds_high and __lang_expected_abi_hash.
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, block_base, total);

    // Call load_module — all sections come from the single adjacent block.
    let mut loaded_set = LoadedSet::<64>::new();
    let _loaded = load_module(&container, &mut plat, &mut global_map, &mut loaded_set).unwrap();

    // The export table registered `main` at the code base address
    // (the start of the code section).  Find it in the map.
    let main_sym = global_map.lookup_by_name(b"main")
        .expect("main should be registered in global map");

    let code_base = main_sym.addr;
    // Verify the code isn't all zeros.
    let code_preview = unsafe { std::slice::from_raw_parts(code_base as *const u8, 8) };
    assert_ne!(code_preview[0], 0, "code starts with zero");

    // Call the loaded function with a proper data stack.
    const DS_SIZE: usize = 65536;
    let mut ds_buf = vec![0u8; DS_SIZE];
    let ds_base = ds_buf.as_ptr() as u64;
    let ds_limit = ds_base + DS_SIZE as u64;

    let result: i64;
    unsafe {
        core::arch::asm!(
            "mov r15, {base}",
            "mov r14, {limit}",
            "call {fn}",
            "mov {result}, rax",
            base = in(reg) ds_base,
            limit = in(reg) ds_limit,
            fn = in(reg) code_base,
            result = lateout(reg) result,
            out("r15") _, out("r14") _, out("rax") _,
            out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
        );
    }
    assert_eq!(result, 42,
        "dynamic load returned {result}, static returned {expected}");
}

#[test]
fn phase8_module_without_runtime_symbols_fails() {
    let dir = fresh_dir("without_runtime_symbols_fails");
    let source = "module Main;\n: main ( -- i64 ) 7 ;\nend;\n";
    let lmod_path = compile_and_pack(source, &dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let total = container.code().len() + container.rodata().len() + container.data().len() + 32;
    let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    plat.reserve(total).unwrap();

    let mut global_map: SymMap<'_, 256> = SymMap::new();
    let mut loaded_set = LoadedSet::<64>::new();
    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert!(result.is_err(), "load should fail without runtime symbols");
    assert_eq!(result.unwrap_err(), 5205); // E_SYMBOL_UNRESOLVED
}

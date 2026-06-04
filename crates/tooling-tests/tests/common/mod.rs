//! Shared test helpers for S2 phase integration tests.
//!
//! Consolidates duplicated helper functions across phase test files.

use hosted::loader::HostedLoaderPlatform;
use hosted::mem;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

pub fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

pub fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_lang_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

/// Compile a .mod with --emit=asm (static build) and return the exit code.
pub fn static_exit_code(source: &str, dir: &PathBuf) -> i32 {
    std::fs::write(dir.join("S.mod"), source).unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(dir)
        .args(["--emit=asm", "S.mod"])
        .output().unwrap();
    assert!(out.status.success(), "langc --emit=asm failed:\n{}",
        String::from_utf8_lossy(&out.stderr));
    std::fs::write(dir.join("S.asm"), &out.stdout).unwrap();
    let status = Command::new("fasm")
        .current_dir(dir)
        .args(["S.asm", "S_bin"])
        .status().unwrap();
    assert!(status.success(), "fasm failed");
    Command::new(dir.join("S_bin")).status().unwrap().code().unwrap_or(-1)
}

/// Compile a .mod to .o and pack to .lmod.  Returns the .lmod path.
pub fn compile_and_pack(source: &str, out_dir: &PathBuf) -> PathBuf {
    std::fs::write(out_dir.join("M.mod"), source).unwrap();
    let status = Command::new(exe("langc"))
        .current_dir(out_dir)
        .args(["--emit=obj", "--target=x86_64-unknown-linux-gnu", "--out-dir=.", "M.mod"])
        .status().unwrap();
    assert!(status.success(), "langc --emit=obj failed");
    let o_path = out_dir.join("Main.o");
    assert!(o_path.exists(), "Main.o not produced");
    let lmod_path = out_dir.join("Main.lmod");
    let status = Command::new(exe("lmod-pack"))
        .current_dir(out_dir)
        .args(["Main.o", "Main.lmod"])
        .status().unwrap();
    assert!(status.success(), "lmod-pack failed");
    lmod_path
}

// ---------------------------------------------------------------------------
// Dynamic loading helpers
// ---------------------------------------------------------------------------

pub extern "C" fn extern_c_fn_stub() {}

/// Allocate a separate writable page for runtime data variables
/// (__lang_ds_high).  These must stay writable even after code pages
/// are flipped to RX.
pub fn allocate_runtime_page() -> usize {
    let page = unsafe {
        mem::mmap_anon(4096, mem::prot::READ | mem::prot::WRITE).unwrap() as *mut u8
    };
    page as usize
}

/// Register the minimal set of runtime symbols needed to load a module.
pub fn register_runtime_symbols<'a>(
    map: &mut SymMap<'a, 256>,
    ds_high_addr: usize,
) {
    let stub = extern_c_fn_stub as usize;
    map.register(b"__stack_overflow", stub).unwrap();
    map.register(b"__lang_ds_high", ds_high_addr).unwrap();
    map.register(b"__lang_trap", stub).ok();
    map.register(b"__lang_trap_loc", stub).ok();
}

/// Load a .lmod, call `main`, and return the result.
pub fn dynamic_load_value(source: &str, dir: &PathBuf) -> i64 {
    let lmod_path = compile_and_pack(source, dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();

    let abi_hash = container.header().abi_hash;
    let bsize = (container.code().len() + 4095) & !4095;
    let mut plat = HostedLoaderPlatform::new(abi_hash);
    plat.reserve(bsize).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);

    let mut set = LoadedSet::<64>::new();
    let _loaded = load_module(&container, &mut plat, &mut global_map, &mut set).unwrap();

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

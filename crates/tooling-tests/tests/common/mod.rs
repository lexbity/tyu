//! Shared test helpers for S2 phase integration tests.
//!
//! Consolidates duplicated helper functions across phase test files.

// This module is included by multiple test binaries; each uses only a subset
// of the helpers, so unused-item warnings here are false positives.
#![allow(dead_code)]

pub mod bin;

use hosted::loader::HostedLoaderPlatform;
use hosted::mem;
use lmod::validate::Container;
use loader_core::load::load_module;
use loader_core::apertures::ApertureRegistry;
use loader_core::platform::TrustLevel;
use loader_core::symbols::SymMap;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

/// Product binaries some helpers spawn (`langc`, `lmod-pack`, …), built once
/// on first use. `cargo test --workspace` does not guarantee another suite
/// built them first, so spawning without this gate makes pass/fail depend on
/// suite execution order (the `lmod-pack` NotFound race).
static BINS_ONCE: std::sync::Once = std::sync::Once::new();
pub fn ensure_bins() {
    BINS_ONCE.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .current_dir(workspace_root())
            .args([
                "build",
                "-q",
                "-p",
                "langc",
                "-p",
                "lmod-pack",
                "-p",
                "lmod-sign",
                "-p",
                "lmod-encrypt",
            ])
            .status()
            .expect("cargo build for helper binaries");
        assert!(status.success(), "building helper binaries failed");
    });
}

/// The `--platform=<dir>` argument pointing at the hosted runtime descriptor
/// (P4). Tooling-test register maps bind `board.<instance>` against it.
pub fn platform_arg() -> String {
    format!(
        "--platform={}",
        workspace_root().join("runtime/linux-x86_64-hosted").display()
    )
}

pub fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_lang_tests").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=asm failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("S.asm"), &out.stdout).unwrap();
    let status = Command::new("fasm")
        .current_dir(dir)
        .args(["S.asm", "S_bin"])
        .status()
        .unwrap();
    assert!(status.success(), "fasm failed");
    Command::new(dir.join("S_bin"))
        .status()
        .unwrap()
        .code()
        .unwrap_or(-1)
}

/// Compile a .mod to .o and pack to .lmod.  Returns the .lmod path.
pub fn compile_and_pack(source: &str, out_dir: &PathBuf) -> PathBuf {
    ensure_bins();
    std::fs::write(out_dir.join("M.mod"), source).unwrap();
    let status = Command::new(exe("langc"))
        .current_dir(out_dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "M.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "langc --emit=obj failed");
    let o_path = out_dir.join("Main.o");
    assert!(o_path.exists(), "Main.o not produced");
    let lmod_path = out_dir.join("Main.lmod");
    let status = Command::new(exe("lmod-pack"))
        .current_dir(out_dir)
        .args(["Main.o", "Main.lmod"])
        .status()
        .unwrap();
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
    let page = mem::mmap_anon(4096, mem::prot::READ | mem::prot::WRITE).unwrap() as *mut u8;
    page as usize
}

/// Register the test runtime symbols through the generated `.lang.symtab`
/// byte-table API used by device boot.
pub fn register_test_runtime_symtab(map: &mut SymMap<'_, 256>, ds_high_addr: usize) {
    let stub = extern_c_fn_stub as *const () as usize;
    let entries = [
        (lmod::hash::fnv1a_u64(b"__stack_overflow"), stub),
        (lmod::hash::fnv1a_u64(b"__lang_ds_high"), ds_high_addr),
        (lmod::hash::fnv1a_u64(b"__lang_trap"), stub),
        (lmod::hash::fnv1a_u64(b"__lang_trap_loc"), stub),
    ];

    let mut bytes = Vec::with_capacity(8 + entries.len() * 16);
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for (hash, addr) in entries {
        bytes.extend_from_slice(&hash.to_le_bytes());
        bytes.extend_from_slice(&(addr as u64).to_le_bytes());
    }
    map.register_symtab_bytes(&bytes).unwrap();
}

/// Load a .lmod, call `main`, and return the result.
pub fn dynamic_load_value(source: &str, dir: &PathBuf) -> i64 {
    let lmod_path = compile_and_pack(source, dir);
    let raw = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&raw).unwrap();
    let h = LoaderHarness::new(container.header().abi_hash);
    h.load_and_run(&container).unwrap()
}

// ---------------------------------------------------------------------------
// LoaderHarness — shared loader + executor for contract and E2E tests
// ---------------------------------------------------------------------------

/// A reusable loader harness that encapsulates platform config and provides
/// `load` and `load_and_run` methods.
///
/// Builder pattern: `LoaderHarness::new(abi_hash).tier_one(key).with_kek(kek)`
/// then call `load` or `load_and_run` with a parsed container.
///
/// Each call creates its own platform, symbol map, and loaded-set internally
/// (no persistent state that borrows from container data), so the harness can
/// be reused across multiple containers.
pub struct LoaderHarness {
    abi_hash: u64,
    sign_key: [u8; 64],
    sign_key_len: usize,
    trust_level: TrustLevel,
    kek: [u8; 32],
    kek_set: bool,
}

impl LoaderHarness {
    /// Create a new harness with a given expected ABI hash and TrustLevel Zero.
    pub fn new(expected_abi_hash: u64) -> Self {
        LoaderHarness {
            abi_hash: expected_abi_hash,
            sign_key: [0u8; 64],
            sign_key_len: 0,
            trust_level: TrustLevel::Zero,
            kek: [0u8; 32],
            kek_set: false,
        }
    }

    /// Set the platform to TrustLevel One with the given HMAC signing key.
    pub fn tier_one(mut self, sign_key: &[u8; 32]) -> Self {
        let n = sign_key.len().min(64);
        self.sign_key[..n].copy_from_slice(&sign_key[..n]);
        self.sign_key_len = n;
        self.trust_level = TrustLevel::One;
        self
    }

    /// Set the KEK for encrypted-module decryption.
    pub fn with_kek(mut self, kek: &[u8; 32]) -> Self {
        self.kek = *kek;
        self.kek_set = true;
        self
    }

    /// Build the platform from the stored configuration.
    fn build_platform(&self) -> HostedLoaderPlatform {
        let mut plat = HostedLoaderPlatform::new(self.abi_hash);
        if self.sign_key_len > 0 {
            plat = plat.with_key(&self.sign_key[..self.sign_key_len], self.trust_level);
        }
        if self.kek_set {
            plat = plat.with_kek(&self.kek);
        }
        plat
    }

    /// Create a fresh symbol map with runtime stubs registered.
    fn fresh_map(ds_page_addr: usize) -> SymMap<'static, 256> {
        let mut map: SymMap<'static, 256> = SymMap::new();
        register_test_runtime_symtab(&mut map, ds_page_addr);
        map
    }

    /// Load a container, returning its error code or `Ok(())`.
    ///
    /// Each call uses a fresh platform and symbol map (no persistent loaded-set,
    /// so repeated calls for the same module will not trigger
    /// `E_MODULE_ALREADY_LOADED`).
    pub fn load(&self, c: &Container) -> Result<(), u32> {
        let mut plat = self.build_platform();
        let bsize = (c.code().len() + c.rodata().len() + c.data().len() + 4095) & !4095;
        plat.reserve(bsize).map_err(|_| 1u32)?;
        let ds_page = allocate_runtime_page();
        let mut map = Self::fresh_map(ds_page);
        let mut set = loader_core::load::LoadedSet::<64>::new();
        load_module(c, &mut plat, &mut map, &mut set, &mut ApertureRegistry::new())?;
        Ok(())
    }

    /// Load a container, call `main`, and return its value.
    ///
    /// Uses the DS register convention (r15 = ds_base, r14 = ds_limit).
    /// The return value is read from the data stack after `main` returns
    /// (matching the hosted runtime convention: `sub r15, 8; mov rax, [r15]`).
    pub fn load_and_run(&self, c: &Container) -> Result<i64, u32> {
        let mut plat = self.build_platform();
        let bsize = (c.code().len() + c.rodata().len() + c.data().len() + 4095) & !4095;
        plat.reserve(bsize).map_err(|_| 1u32)?;
        let ds_page = allocate_runtime_page();
        let mut map = Self::fresh_map(ds_page);
        let mut set = loader_core::load::LoadedSet::<64>::new();
        let _loaded = load_module(c, &mut plat, &mut map, &mut set, &mut ApertureRegistry::new())?;

        let main_sym = map.lookup_by_name(b"main").ok_or(0u32)?;
        let code_base = main_sym.addr;

        const DS_SIZE: usize = 65536;
        let ds_buf = vec![0u8; DS_SIZE];
        let ds_base = ds_buf.as_ptr() as u64;
        let ds_limit = ds_base + DS_SIZE as u64;
        let result: i64;
        unsafe {
            core::arch::asm!(
                "mov r15, {base}",
                "mov r14, {limit}",
                "call {fn}",
                "sub r15, 8",
                "mov rax, [r15]",
                base = in(reg) ds_base,
                limit = in(reg) ds_limit,
                fn = in(reg) code_base,
                out("rax") result,
                out("r15") _, out("r14") _,
                out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
            );
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Self-tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_loads_plaintext_and_runs() {
        if !exe("langc").is_file() {
            eprintln!("SKIP: langc not built (run `cargo build -p langc`)");
            return;
        }

        let dir = fresh_dir("harness_self_test");
        let source = "module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";
        let lmod_path = compile_and_pack(source, &dir);
        let raw = std::fs::read(&lmod_path).unwrap();
        let container = Container::parse(&raw).unwrap();
        let h = LoaderHarness::new(container.header().abi_hash);
        let result = h.load_and_run(&container).unwrap();
        assert_eq!(result, 42, "harness must return main's value (42)");
    }

    #[test]
    fn harness_load_returns_ok_on_valid_module() {
        if !exe("langc").is_file() {
            eprintln!("SKIP: langc not built (run `cargo build -p langc`)");
            return;
        }

        let dir = fresh_dir("harness_load_ok");
        let source = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";
        let lmod_path = compile_and_pack(source, &dir);
        let raw = std::fs::read(&lmod_path).unwrap();
        let container = Container::parse(&raw).unwrap();
        let h = LoaderHarness::new(container.header().abi_hash);
        assert!(
            h.load(&container).is_ok(),
            "valid module must load successfully"
        );
    }
}

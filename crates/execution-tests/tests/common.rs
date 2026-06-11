//! Shared test helpers for execution-tests.
//!
//! Provides utility functions for tool discovery, compilation, assembly, and
//! linking.  Pipeline orchestration (build_test_image, qemu_run, assert helpers)
//! has moved into `tyu test` — these tests call the driver binary instead.

use codegen_core::{AssemblerKind, Target};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

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

pub fn tyu_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("tyu")
}

pub fn fixtures_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("manifest.toml")
}

pub fn runtime_dir(target: Target) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    workspace_root().join("runtime").join(triple)
}

pub fn sysroot_dir() -> PathBuf {
    workspace_root().join("sysroot")
}

pub fn langc_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("langc")
}

pub fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_exec_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Tool availability
// ---------------------------------------------------------------------------

/// Returns true if a named binary exists — either on `PATH` or in
/// `target/debug/` (for workspace-built binaries like `langc`, `tyu`).
pub fn tool_available(name: &str) -> bool {
    if Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    let target = workspace_root().join("target").join("debug").join(name);
    target.exists()
}

/// Environment-aware tool gating.
pub fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools
        .iter()
        .filter(|t| !tool_available(t))
        .copied()
        .collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!(
            "Required tools not available under CI: {}. \
             Install them or add them to PATH.",
            missing.join(", ")
        );
    }
    eprintln!(
        "SKIP: required tools not available ({})",
        missing.join(", ")
    );
    false
}

// ---------------------------------------------------------------------------
// Compilation helpers (used by runtime symbol checks)
// ---------------------------------------------------------------------------

/// Compile a .mod source file with langc for the given target.
pub fn langc_compile(target: Target, src: &Path, out_dir: &Path, is_lib: bool) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        format!("--target={triple}"),
        format!("--sysroot={}", sysroot_dir().display()),
        format!("--out-dir={}", out_dir.display()),
        format!("-I={}", out_dir.display()),
    ];
    if is_lib {
        args.push("--lib".into());
    }
    args.push(src.to_str().unwrap().into());

    let status = Command::new(langc_exe())
        .args(&args)
        .status()
        .expect("langc invocation failed");
    assert!(status.success(), "langc failed to compile {}", src.display());

    std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .expect("langc produced no .o file")
}

/// Assemble `runtime.asm` and any feature-specific optional runtime units
/// for the given target.  Returns a `Vec` of object paths.
///
/// Optional units (e.g. `modload.asm`) are assembled only when the
/// corresponding `.asm` file exists in the runtime directory.
pub fn assemble_runtime(target: Target, out_dir: &Path) -> Vec<PathBuf> {
    let spec = target.spec();
    let rt_dir = runtime_dir(target);

    let mut objs = Vec::new();
    let stems = &["runtime", "concurrency", "modload"];
    for stem in stems {
        let asm = rt_dir.join(format!("{}.asm", stem));
        if !asm.exists() {
            continue;
        }
        let out = out_dir.join(format!("{}.o", stem));
        match spec.assembler {
            AssemblerKind::Fasm => {
                let status = Command::new("fasm")
                    .args([asm.to_str().unwrap(), out.to_str().unwrap()])
                    .status()
                    .expect("fasm invocation failed");
                assert!(status.success(), "fasm failed to assemble {stem}");
            }
            AssemblerKind::GasArm => {
                let status = Command::new("arm-none-eabi-as")
                    .args(["-mcpu=cortex-m3", "-mthumb", asm.to_str().unwrap(), "-o", out.to_str().unwrap()])
                    .status()
                    .expect("arm-none-eabi-as invocation failed");
                assert!(status.success(), "arm-none-eabi-as failed to assemble {stem}");
            }
            AssemblerKind::GasRiscV => {
                let status = Command::new("riscv64-unknown-elf-as")
                    .args(["-march=rv32i", "-mabi=ilp32", asm.to_str().unwrap(), "-o", out.to_str().unwrap()])
                    .status()
                    .expect("riscv64-unknown-elf-as invocation failed");
                assert!(status.success(), "riscv64-unknown-elf-as failed to assemble {stem}");
            }
        }
        objs.push(out);
    }
    objs
}

/// Link object files + runtime into an ELF.
pub fn link_image(target: Target, objs: &[PathBuf], out_dir: &Path) -> PathBuf {
    let spec = target.spec();
    let rt_dir = runtime_dir(target);
    let linker_script = rt_dir.join("link.ld");
    let out = out_dir.join("test.elf");
    let linker = core::str::from_utf8(spec.linker).expect("non-UTF-8 linker name");
    let mut cmd = Command::new(linker);
    cmd.arg("-T").arg(&linker_script).arg("-o").arg(&out);
    for obj in objs {
        cmd.arg(obj);
    }
    let status = cmd.status().unwrap_or_else(|_| panic!("{linker} invocation failed"));
    assert!(status.success(), "{linker} failed to link test image");
    out
}

/// Return the set of symbol names that MUST be present in a linked image
/// for a given feature set.  Core symbols are always required; optional
/// unit symbols are included iff the corresponding feature is enabled.
pub fn expected_symbols(feature_set: codegen_core::FeatureSet) -> Vec<&'static str> {
    let mut syms: Vec<&'static str> = vec![
        // Core runtime symbols (abi-contract §4.4.1).
        "__lang_start",
        "__lang_trap",
        "__lang_trap_loc",
        "__stack_overflow",
        "__lang_ds_base",
        "__lang_ds_limit",
        "__lang_ds_high",
    ];

    if feature_set.contains(codegen_core::Feature::Concurrency) {
        syms.extend_from_slice(&[
            "__task_spawn",
            "__task_yield",
            "__task_join",
            "__task_current",
            "__task_state",
            "__task_rsp",
            "__task_r15",
            "__task_r14",
            "__task_entry",
            "__task_ds_mem",
            "__task_cs_mem",
            "__task_g_head",
            "__task_g_tail",
            "__task_g_buf",
            "__chan_next",
            "__chan_inuse",
            "__chan_head",
            "__chan_tail",
            "__chan_buf",
        ]);
    }

    if feature_set.contains(codegen_core::Feature::ModuleLoading) {
        syms.extend_from_slice(&[
            "__lang_modpack_start",
            "__lang_modpack_end",
        ]);
    }

    syms
}



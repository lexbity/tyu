//! Build pipeline orchestration.
//!
//! Compiles each module in dependency order, assembles the runtime, and links
//! everything into a final ELF image.  Uses `langc`, the target assembler, and
//! linker via `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

use codegen_core::{AssemblerKind, Target};

use crate::args::BuildArgs;
use crate::cache::BuildCache;
use crate::graph::{resolve_graph, ModuleNode};

/// Build an ELF image from the given build arguments.
///
/// Returns the path to the produced ELF.
pub fn build(args: &BuildArgs) -> Result<PathBuf, String> {
    let target = args.target;
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 target triple")?;

    // Ensure output directory exists.
    fs::create_dir_all(&args.out_dir)
        .map_err(|e| format!("creating out_dir '{}': {}", args.out_dir.display(), e))?;

    // Resolve module graph.
    let modules = resolve_graph(
        &args.input,
        &args.include_dirs,
        args.sysroot.as_deref(),
    )?;

    // Load or create build cache.
    let cache_path = args.out_dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    // Compute ABI hash for this target.
    let spec = target.spec();
    let slot_bytes = spec.slot_bytes;
    let pointer_bits = spec.pointer_bits;
    let abi_hash = lmod::abi_hash::compute_abi_hash(
        slot_bytes,
        pointer_bits,
        lmod::modinfo::MODINFO_VER,
    );

    // Find langc binary.
    let langc = find_tool("langc")?;

    // Compile each module in dependency order.
    let mut objs: Vec<PathBuf> = Vec::new();
    for module in &modules {
        let obj_path = compile_module(
            &langc,
            target,
            module,
            &args.include_dirs,
            args.sysroot.as_deref(),
            &args.out_dir,
            &mut cache,
            abi_hash,
            triple,
        )?;
        objs.push(obj_path);
    }

    // Assemble runtime.
    let runtime_o = assemble_runtime(target, &args.out_dir)?;
    objs.push(runtime_o);

    // Link.
    let image = link_image(target, &objs, &args.out_dir)?;

    // Persist cache.
    cache.save()?;

    Ok(image)
}

/// Find a tool binary by name.  Checks PATH and the workspace target dir.
fn find_tool(name: &str) -> Result<PathBuf, String> {
    // First check PATH.
    if let Ok(path) = which(name) {
        return Ok(path);
    }
    // Check workspace target/debug for workspace-local tools.
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join(name);
    if local.is_file() {
        return Ok(local);
    }
    Err(format!("tool '{name}' not found in PATH or workspace target/debug"))
}

/// Cross-platform `which` via `PATH` environment variable.
fn which(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").ok_or("PATH not set")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let candidate_exe = dir.join(format!("{}.exe", name));
            if candidate_exe.is_file() {
                return Ok(candidate_exe);
            }
        }
    }
    Err(format!("{name} not found in PATH"))
}

/// Compile a single module with `langc`.
fn compile_module(
    langc: &Path,
    _target: Target,
    module: &ModuleNode,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    out_dir: &Path,
    cache: &mut BuildCache,
    abi_hash: u64,
    triple: &str,
) -> Result<PathBuf, String> {
    // Check cache first.
    if let Some(cached) = cache.lookup(&module.path, triple, abi_hash)? {
        if cached.object_path.exists() {
            return Ok(cached.object_path);
        }
    }

    let mut cmd = Command::new(langc);
    cmd.arg("--emit=obj");
    cmd.arg(format!("--target={}", triple));
    cmd.arg(format!("--out-dir={}", out_dir.display()));

    if let Some(sr) = sysroot {
        cmd.arg(format!("--sysroot={}", sr.display()));
    }

    for inc in include_dirs {
        cmd.arg("-I");
        cmd.arg(inc);
    }

    if module.is_lib {
        cmd.arg("--lib");
    }

    cmd.arg(&module.path);

    let status = cmd.status()
        .map_err(|e| format!("running langc: {}", e))?;
    if !status.success() {
        return Err(format!(
            "langc failed on '{}' (exit code {:?})",
            module.path.display(),
            status.code(),
        ));
    }

    // Find the produced .o file (langc names it after the module name).
    let obj_name = module.path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let obj_path = out_dir.join(format!("{}.o", obj_name));

    if !obj_path.exists() {
        return Err(format!(
            "langc did not produce expected .o at '{}'",
            obj_path.display(),
        ));
    }

    cache.insert(&module.path, triple, abi_hash, &obj_path)?;

    Ok(obj_path)
}

/// Assemble the runtime for the given target.
pub fn assemble_runtime(target: Target, out_dir: &Path) -> Result<PathBuf, String> {
    let spec = target.spec();
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 triple")?;

    let rt_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("runtime")
        .join(triple);
    let asm_path = rt_dir.join("runtime.asm");
    let out_path = out_dir.join("runtime.o");

    match spec.assembler {
        AssemblerKind::Fasm => {
            let status = Command::new("fasm")
                .args([asm_path.to_str().unwrap(), out_path.to_str().unwrap()])
                .status()
                .map_err(|e| format!("running fasm: {}", e))?;
            if !status.success() {
                return Err("fasm failed to assemble runtime".to_string());
            }
        }
        AssemblerKind::GasArm => {
            let status = Command::new("arm-none-eabi-as")
                .args([
                    "-mcpu=cortex-m3",
                    "-mthumb",
                    asm_path.to_str().unwrap(),
                    "-o",
                    out_path.to_str().unwrap(),
                ])
                .status()
                .map_err(|e| format!("running arm-none-eabi-as: {}", e))?;
            if !status.success() {
                return Err("arm-none-eabi-as failed to assemble runtime".to_string());
            }
        }
        AssemblerKind::GasRiscV => {
            let status = Command::new("riscv64-unknown-elf-as")
                .args([
                    "-march=rv32i",
                    "-mabi=ilp32",
                    asm_path.to_str().unwrap(),
                    "-o",
                    out_path.to_str().unwrap(),
                ])
                .status()
                .map_err(|e| format!("running riscv64-unknown-elf-as: {}", e))?;
            if !status.success() {
                return Err("riscv64-unknown-elf-as failed to assemble runtime".to_string());
            }
        }
    }

    Ok(out_path)
}

/// Link object files into the final ELF image.
pub fn link_image(target: Target, objs: &[PathBuf], out_dir: &Path) -> Result<PathBuf, String> {
    let spec = target.spec();
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 triple")?;

    let linker_name = std::str::from_utf8(spec.linker)
        .map_err(|_| "non-UTF-8 linker name")?;
    let linker = find_tool(linker_name)?;

    let rt_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("runtime")
        .join(triple);
    let linker_script = rt_dir.join("link.ld");
    let out_path = out_dir.join("image.elf");

    let mut cmd = Command::new(&linker);
    cmd.arg("-T")
        .arg(&linker_script)
        .arg("-o")
        .arg(&out_path);
    for obj in objs {
        cmd.arg(obj);
    }

    let status = cmd.status()
        .map_err(|e| format!("running linker '{}': {}", linker_name, e))?;
    if !status.success() {
        return Err(format!("{} failed to link image", linker_name));
    }

    Ok(out_path)
}

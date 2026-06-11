//! Build pipeline orchestration.
//!
//! Compiles each module in dependency order, assembles the runtime, and links
//! everything into a final ELF image.  Uses `langc`, the target assembler, and
//! linker via `std::process::Command`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

use codegen_core::{AssemblerKind, FeatureSet, Target};

use crate::args::BuildArgs;
use crate::cache::{self, BuildCache};
use crate::error::TyuError;
use crate::graph::{resolve_graph, ModuleNode};
use crate::toolchain;

/// Build an ELF image from the given build arguments.
///
/// Returns the path to the produced ELF.
pub fn build(args: &BuildArgs) -> Result<PathBuf, TyuError> {
    let target = args.target;
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 target triple")?;
    let feature_set = args.feature_set;

    // Ensure output directory exists.
    fs::create_dir_all(&args.out_dir)
        .map_err(|e| TyuError::Io(e))?;

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

    // Find langc binary via PATH.
    let langc = toolchain::resolve_tool("langc")?;

    // Compute compiler fingerprint (stable per build invocation).
    let compiler_fp = cache::compiler_fingerprint();

    // Pre-compute content hashes for every module path.
    let mut path_to_hash: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for module in &modules {
        if let Ok(h) = cache::content_hash(&module.path) {
            path_to_hash.insert(module.path.clone(), h);
        }
    }

    // Transitive-dep hash cache: module path → sorted hashes of all transitive deps.
    let mut transitive_cache: BTreeMap<PathBuf, Vec<u64>> = BTreeMap::new();

    // Compile each module in dependency order.
    let mut objs: Vec<PathBuf> = Vec::new();
    for module in &modules {
        let inputs_fp = {
            let own_hash = path_to_hash.get(&module.path).copied().unwrap_or(0);
            let transitive = cache::collect_transitive_hashes(module, &path_to_hash, &mut transitive_cache);
            cache::inputs_fingerprint(own_hash, triple, &transitive)
        };

        let obj_path = compile_module(
            &langc,
            target,
            module,
            &args.include_dirs,
            args.sysroot.as_deref(),
            &args.out_dir,
            &mut cache,
            compiler_fp,
            inputs_fp,
            abi_hash,
            triple,
            feature_set,
        )?;
        objs.push(obj_path);
    }

    // Assemble runtime units.
    let runtime_objs = assemble_runtime(target, &args.out_dir, feature_set)?;
    objs.extend(runtime_objs);

    // Link.
    let image = link_image(target, &objs, &args.out_dir)?;

    // Persist cache.
    cache.save()?;

    Ok(image)
}

/// Compile a single `.mod` file with langc, without caching.
/// Returns the path to the produced `.o`.
pub fn compile_simple(
    target: Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
    sysroot: Option<&Path>,
    include_dirs: &[PathBuf],
    feature_set: FeatureSet,
) -> Result<PathBuf, TyuError> {
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 target triple")?;
    let langc = toolchain::resolve_tool("langc")?;

    let mut cmd = Command::new(&langc);
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
    if is_lib {
        cmd.arg("--lib");
    }
    // Pass resolved feature set.
    let mut flag_buf = [""; 8];
    let n = feature_set.write_flags(&mut flag_buf);
    if n > 0 {
        cmd.arg(format!("--features={}", flag_buf[..n].join(",")));
    }
    // Record .o files present before compilation (langc names the .o
    // after the module name, which may differ from the source file stem).
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(out_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.filter_map(|e| e.ok()))
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    cmd.arg(src);

    let status = cmd.status()
        .map_err(|e| TyuError::Build(format!("running langc: {}", e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!("langc failed on '{}'", src.display()).into()));
    }

    // Find the .o that wasn't there before.
    let obj_path = std::fs::read_dir(out_dir)
        .map_err(|e| TyuError::Build(format!("reading out_dir: {}", e)))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .ok_or_else(|| {
            TyuError::Build(format!(
                "langc produced no .o file for '{}' in '{}'",
                src.display(),
                out_dir.display(),
            ))
        })?;

    Ok(obj_path)
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
    compiler_fp: u64,
    inputs_fp: u64,
    abi_hash: u64,
    triple: &str,
    feature_set: FeatureSet,
) -> Result<PathBuf, TyuError> {
    // Check cache first.
    if let Some(cached) = cache.lookup(compiler_fp, inputs_fp, abi_hash) {
        if cached.object_path.exists() {
            eprintln!("tyu: cache hit for '{}'", module.path.display());
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

    // Pass resolved feature set.
    let mut flag_buf = [""; 8];
    let n = feature_set.write_flags(&mut flag_buf);
    if n > 0 {
        cmd.arg(format!("--features={}", flag_buf[..n].join(",")));
    }

    cmd.arg(&module.path);

    let status = cmd.status()
        .map_err(|e| TyuError::Build(format!("running langc: {}", e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!(
            "langc failed on '{}' (exit code {:?})",
            module.path.display(),
            status.code(),
        )));
    }


    // Find the produced .o file (langc names it after the module name).
    let obj_name = module.path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let obj_path = out_dir.join(format!("{}.o", obj_name));

    if !obj_path.exists() {
        return Err(TyuError::Build(format!(
            "langc did not produce expected .o at '{}'",
            obj_path.display(),
        )));
    }


    cache.insert(compiler_fp, inputs_fp, abi_hash, triple, &obj_path)?;

    Ok(obj_path)
}

/// Assemble the runtime unit `stem` (e.g. `"runtime"`, `"concurrency"`) for
/// the given target, producing `<out_dir>/<stem>.o`.
///
/// This is the extracted helper from the original monolithic `assemble_runtime`
/// (DEBT-2).  `runtime_dir` is `<workspace_root>/runtime/<triple>`.
fn assemble_unit(target: Target, rt_dir: &Path, stem: &str, out_dir: &Path) -> Result<PathBuf, TyuError> {
    let spec = target.spec();
    let asm_path = rt_dir.join(format!("{}.asm", stem));
    if !asm_path.exists() {
        // Optional unit that does not exist on this target — skip silently.
        return Err(TyuError::Build(format!(
            "runtime unit '{}' not found for target",
            asm_path.display(),
        )));
    }
    let out_path = out_dir.join(format!("{}.o", stem));

    match spec.assembler {
        AssemblerKind::Fasm => {
            let status = Command::new("fasm")
                .args([asm_path.to_string_lossy().as_ref(), out_path.to_string_lossy().as_ref()])
                .status()
                .map_err(|e| TyuError::Build(format!("running fasm: {}", e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!("fasm failed to assemble '{}'", stem)).into());
            }
        }
        AssemblerKind::GasArm => {
            let status = Command::new("arm-none-eabi-as")
                .args([
                    "-mcpu=cortex-m3",
                    "-mthumb",
                    asm_path.to_string_lossy().as_ref(),
                    "-o",
                    out_path.to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| TyuError::Build(format!("running arm-none-eabi-as: {}", e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!("arm-none-eabi-as failed to assemble '{}'", stem)).into());
            }
        }
        AssemblerKind::GasRiscV => {
            let status = Command::new("riscv64-unknown-elf-as")
                .args([
                    "-march=rv32i",
                    "-mabi=ilp32",
                    asm_path.to_string_lossy().as_ref(),
                    "-o",
                    out_path.to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| TyuError::Build(format!("running riscv64-unknown-elf-as: {}", e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!("riscv64-unknown-elf-as failed to assemble '{}'", stem)).into());
            }
        }
    }

    Ok(out_path)
}

/// Assemble the mandatory core runtime and any optional feature-specific
/// units for `target`.  Returns a `Vec` of object-file paths to link.
///
/// Core (`runtime.asm`) is always assembled.  Feature-specific units
/// (e.g. `modload.asm` for `module-loading`) are assembled only when
/// the corresponding feature is enabled.
pub fn assemble_runtime(target: Target, out_dir: &Path, feature_set: FeatureSet) -> Result<Vec<PathBuf>, TyuError> {
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 triple")?;

    let rt_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("runtime")
        .join(triple);

    // Core runtime is always assembled.
    let mut objs = Vec::new();
    objs.push(assemble_unit(target, &rt_dir, "runtime", out_dir)?);

    // Feature-specific runtime units: assemble each stem that maps to
    // an enabled feature.  `assemble_unit` returns an error for missing
    // files (the unit must exist for at least the targets that enable it).
    for f in feature_set.iter() {
        if let Some(stem) = f.runtime_unit() {
            let unit_path = rt_dir.join(format!("{}.asm", stem));
            if unit_path.exists() {
                objs.push(assemble_unit(target, &rt_dir, stem, out_dir)?);
            }
        }
    }

    Ok(objs)
}

/// Link object files into the final ELF image.
pub fn link_image(target: Target, objs: &[PathBuf], out_dir: &Path) -> Result<PathBuf, TyuError> {
    let spec = target.spec();
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| "non-UTF-8 triple")?;

    let linker_name = std::str::from_utf8(spec.linker)
        .map_err(|_| "non-UTF-8 linker name")?;
    let linker = toolchain::resolve_tool(linker_name)?;

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
        .map_err(|e| TyuError::Build(format!("running linker '{}': {}", linker_name, e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!("{} failed to link image", linker_name)).into());
    }

    Ok(out_path)
}

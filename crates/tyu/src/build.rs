//! Build pipeline orchestration.
//!
//! Compiles each module in dependency order, assembles the runtime, and links
//! everything into a final image. Uses `langc`, the target assembler, and
//! linker via `std::process::Command`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use codegen_core::{AssemblerKind, FeatureSet, Target};
use lang_symtab_gen::{extract_runtime_symbols, render_asm, render_names, AsmFlavor};

use crate::args::BuildArgs;
use crate::cache::{self, BuildCache};
use crate::error::TyuError;
use crate::graph::{resolve_graph, ModuleNode};
use crate::platform::{self, ResolvedPlatformSelection};
use crate::toolchain;

/// Build an image from the given build arguments.
///
/// Returns the path to the produced final artifact.
pub fn build(args: &BuildArgs) -> Result<PathBuf, TyuError> {
    let ctx = resolve_build_context(args)?;
    build_resolved(args, ctx).map(|outcome| outcome.final_image)
}

/// Result of a fully resolved build.
#[derive(Debug, Clone)]
pub struct BuildOutcome {
    pub final_image: PathBuf,
    pub execution_image: PathBuf,
    pub target: Target,
    pub platform_selection: Option<ResolvedPlatformSelection>,
}

/// Resolved build context shared by build, run, and test flows.
#[derive(Debug, Clone)]
pub struct BuildContext {
    pub target: Target,
    pub out_dir: PathBuf,
    pub platform_selection: Option<ResolvedPlatformSelection>,
}

impl BuildContext {
    pub fn platform_selection(&self) -> Option<&ResolvedPlatformSelection> {
        self.platform_selection.as_ref()
    }
}

/// Build an image from a resolved build context.
pub fn build_resolved(args: &BuildArgs, ctx: BuildContext) -> Result<BuildOutcome, TyuError> {
    let target = ctx.target;
    let out_dir = ctx.out_dir.clone();
    let platform_selection = ctx.platform_selection.clone();
    let triple = std::str::from_utf8(target.triple()).map_err(|_| "non-UTF-8 target triple")?;
    let feature_set = args.feature_set;
    let workspace_root = platform::workspace_root();
    platform::ensure_build_platform_interface(&workspace_root, target).map_err(TyuError::Build)?;

    // Ensure output directory exists.
    fs::create_dir_all(&out_dir).map_err(|e| TyuError::Io(e))?;

    // Resolve module graph.
    let modules = resolve_graph(&args.input, &args.include_dirs, args.sysroot.as_deref())?;
    let module_count = modules.len();

    // Load or create build cache.
    let cache_path = out_dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    // Compute ABI hash for this target. MUST mirror `TargetSpec::expected_abi_hash`
    // and `langc` driver inputs exactly (arch_tag, slot_bytes, word_bits) — using
    // `word_bits` (not `pointer_bits`) so the producer and the runtime agree even on
    // a target where the two differ (Harvard / non-flat pointer model).
    let spec = target.spec();
    let abi_hash = lmod::abi_hash::compute_abi_hash(
        spec.calling_conv.arch_tag(),
        spec.slot_bytes,
        spec.word_bits,
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
            let transitive =
                cache::collect_transitive_hashes(module, &path_to_hash, &mut transitive_cache);
            cache::inputs_fingerprint(own_hash, triple, &transitive)
        };

        let obj_path = compile_module(
            &langc,
            target,
            module,
            &args.include_dirs,
            args.sysroot.as_deref(),
            &out_dir,
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
    let runtime_objs = assemble_runtime_for_context(&ctx, feature_set)?;
    objs.extend(runtime_objs);

    if let Some(selection) = platform_selection.as_ref() {
        if selection
            .pack
            .manifest
            .deploy
            .as_ref()
            .map(|deploy| deploy.boot.as_str())
            == Some("image_def")
        {
            let image_def_obj = assemble_image_def(target, &out_dir, selection)?;
            objs.push(image_def_obj);
        }
    }

    // Link the execution image. Bare-metal targets still need this ELF
    // intermediate for QEMU/device execution, but the final distributable
    // artifact is the packed `.lmod`.
    let exec_image = link_image_for_context(&ctx, &objs)?;

    let final_image = if matches!(target, Target::X86_64UnknownLinuxGnu) {
        exec_image.clone()
    } else {
        let root_obj = objs
            .get(module_count.saturating_sub(1))
            .ok_or_else(|| TyuError::Build("no root module object to pack".into()))?;
        pack_final_lmod(root_obj, &out_dir, modules.last().map(|m| m.name.as_str()))
            .map_err(TyuError::Build)?
    };

    // Persist cache.
    cache.save()?;

    Ok(BuildOutcome {
        final_image,
        execution_image: exec_image,
        target,
        platform_selection,
    })
}

pub fn resolve_build_context(args: &BuildArgs) -> Result<BuildContext, TyuError> {
    let workspace_root = platform::workspace_root();
    let platform_selection = match args.platform.as_deref() {
        Some(name) => Some(
            platform::resolve_platform_selection(&workspace_root, name, args.isa.as_deref())
                .map_err(TyuError::Build)?,
        ),
        None => None,
    };
    let target = platform_selection
        .as_ref()
        .map(|selection| selection.target)
        .unwrap_or(args.target);
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| TyuError::Build("non-UTF-8 target triple".into()))?;
    let out_dir = if platform_selection.is_some()
        && args.out_dir
            == PathBuf::from("target")
                .join("tyu")
                .join(std::str::from_utf8(args.target.triple()).unwrap())
    {
        PathBuf::from("target").join("tyu").join(triple)
    } else {
        args.out_dir.clone()
    };
    let out_dir = if out_dir.is_absolute() {
        out_dir
    } else {
        std::env::current_dir().map_err(TyuError::Io)?.join(out_dir)
    };
    Ok(BuildContext {
        target,
        out_dir,
        platform_selection,
    })
}

fn pack_final_lmod(
    obj_path: &Path,
    out_dir: &Path,
    module_name: Option<&str>,
) -> Result<PathBuf, String> {
    let lmod_name = match module_name {
        Some(name) if !name.is_empty() => format!("{}.lmod", name),
        _ => "image.lmod".to_string(),
    };
    let lmod_path = out_dir.join(lmod_name);
    let obj_bytes =
        std::fs::read(obj_path).map_err(|e| format!("reading '{}': {}", obj_path.display(), e))?;
    let packed = lmod_pack::pack(&obj_bytes).map_err(|e| format!("lmod-pack: {}", e))?;
    std::fs::write(&lmod_path, &packed)
        .map_err(|e| format!("writing '{}': {}", lmod_path.display(), e))?;
    Ok(lmod_path)
}

fn assemble_image_def(
    target: Target,
    out_dir: &Path,
    selection: &platform::ResolvedPlatformSelection,
) -> Result<PathBuf, TyuError> {
    let memory = selection
        .pack
        .manifest
        .memory
        .as_ref()
        .ok_or_else(|| TyuError::Build("boot=image_def requires [memory]".into()))?;
    let flash = memory
        .flash
        .as_ref()
        .ok_or_else(|| TyuError::Build("boot=image_def requires memory.flash".into()))?;
    let sram = memory
        .sram
        .as_ref()
        .ok_or_else(|| TyuError::Build("boot=image_def requires memory.sram".into()))?;
    let ds_size = memory
        .ds_size
        .ok_or_else(|| TyuError::Build("boot=image_def requires memory.ds_size".into()))?;

    let asm_path = out_dir.join("image_def.s");
    let obj_path = out_dir.join("image_def.o");
    let asm = render_image_def_asm(flash, sram, memory.ds_region.as_deref(), ds_size);
    fs::write(&asm_path, asm).map_err(TyuError::Io)?;

    match target.spec().assembler {
        AssemblerKind::GasArm => {
            let status = Command::new("arm-none-eabi-as")
                .current_dir(out_dir)
                .args([
                    "-mcpu=cortex-m3",
                    "-mthumb",
                    asm_path.file_name().unwrap().to_string_lossy().as_ref(),
                    "-o",
                    obj_path.file_name().unwrap().to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| {
                    TyuError::Build(format!("running arm-none-eabi-as for image_def: {}", e))
                })?;
            if !status.success() {
                return Err(TyuError::Build(
                    "arm-none-eabi-as failed to assemble image_def".into(),
                ));
            }
        }
        AssemblerKind::GasRiscV => {
            let asm = toolchain::resolve_tool_candidates(&[
                "riscv64-unknown-elf-as",
                "riscv64-linux-gnu-as",
            ])
            .map_err(|e| {
                TyuError::Build(format!("resolving riscv assembler for image_def: {}", e))
            })?;
            let status = Command::new(&asm)
                .current_dir(out_dir)
                .args([
                    "-march=rv32im",
                    "-mabi=ilp32",
                    asm_path.file_name().unwrap().to_string_lossy().as_ref(),
                    "-o",
                    obj_path.file_name().unwrap().to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| {
                    TyuError::Build(format!("running {} for image_def: {}", asm.display(), e))
                })?;
            if !status.success() {
                return Err(TyuError::Build(format!(
                    "{} failed to assemble image_def",
                    asm.display()
                )));
            }
        }
        AssemblerKind::Fasm => {
            return Err(TyuError::Build(
                "boot=image_def is unsupported for x86 hosted builds".into(),
            ));
        }
    }

    Ok(obj_path)
}

fn render_image_def_asm(
    flash: &platform::MemoryRegion,
    sram: &platform::MemoryRegion,
    ds_region: Option<&str>,
    ds_size: u64,
) -> String {
    let ds_origin = match ds_region {
        Some(name) if name == flash.name => flash.origin,
        Some(name) if name == sram.name => sram.origin,
        _ => sram.origin,
    };

    let mut out = String::new();
    let _ = writeln!(&mut out, ".section .image_def, \"a\", %progbits");
    let _ = writeln!(&mut out, ".globl __lang_image_def");
    let _ = writeln!(&mut out, "__lang_image_def:");
    let _ = writeln!(&mut out, "    .ascii \"IMAGE_DEF\\0\"");
    let _ = writeln!(&mut out, "    .word 1");
    let _ = writeln!(&mut out, "    .word 0x{:08x}", flash.origin);
    let _ = writeln!(&mut out, "    .word 0x{:08x}", flash.length);
    let _ = writeln!(&mut out, "    .word 0x{:08x}", sram.origin);
    let _ = writeln!(&mut out, "    .word 0x{:08x}", sram.length);
    let _ = writeln!(&mut out, "    .word 0x{:08x}", ds_origin);
    let _ = writeln!(&mut out, "    .word 0x{:08x}", ds_size);
    out
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
    let triple = std::str::from_utf8(target.triple()).map_err(|_| "non-UTF-8 target triple")?;
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

    let status = cmd
        .status()
        .map_err(|e| TyuError::Build(format!("running langc: {}", e)))?;
    if !status.success() {
        return Err(TyuError::Build(
            format!("langc failed on '{}'", src.display()).into(),
        ));
    }

    let expected_obj_path = expected_object_path(src, out_dir);
    let obj_path = if expected_obj_path.exists() {
        expected_obj_path
    } else {
        // Fall back to the first newly created .o if the source stem and
        // module name differ in a way we cannot infer here.
        std::fs::read_dir(out_dir)
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
            })?
    };

    Ok(obj_path)
}

pub fn compile_module_for_context(
    ctx: &BuildContext,
    src: &Path,
    is_lib: bool,
    sysroot: Option<&Path>,
    include_dirs: &[PathBuf],
    feature_set: FeatureSet,
) -> Result<PathBuf, TyuError> {
    compile_simple(
        ctx.target,
        src,
        &ctx.out_dir,
        is_lib,
        sysroot,
        include_dirs,
        feature_set,
    )
}

fn expected_object_path(src: &Path, out_dir: &Path) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    let module_name = stem
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<String>();
    out_dir.join(format!("{module_name}.o"))
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

    let status = cmd
        .status()
        .map_err(|e| TyuError::Build(format!("running langc: {}", e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!(
            "langc failed on '{}' (exit code {:?})",
            module.path.display(),
            status.code(),
        )));
    }

    // Langc names the object after the module declaration.  Prefer the
    // deterministic path so recompiles that overwrite an existing object do
    // not get mistaken for a miss.
    let expected_obj_path = out_dir.join(format!("{}.o", module.name));
    let obj_path = if expected_obj_path.exists() {
        expected_obj_path
    } else {
        std::fs::read_dir(out_dir)
            .map_err(|e| TyuError::Build(format!("reading out_dir: {}", e)))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
            .ok_or_else(|| {
                TyuError::Build(format!(
                    "langc produced no .o file for '{}' in '{}'",
                    module.path.display(),
                    out_dir.display(),
                ))
            })?
    };

    cache.insert(compiler_fp, inputs_fp, abi_hash, triple, &obj_path)?;

    Ok(obj_path)
}

/// Assemble the runtime unit `stem` (e.g. `"runtime"`, `"concurrency"`) for
/// the given target, producing `<out_dir>/<stem>.o`.
///
/// This is the extracted helper from the original monolithic `assemble_runtime`
/// (DEBT-2).  `runtime_dir` is `<workspace_root>/runtime/<triple>`.
fn assemble_unit(
    target: Target,
    rt_dir: &Path,
    stem: &str,
    out_dir: &Path,
) -> Result<PathBuf, TyuError> {
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

    assemble_asm_file(target, &asm_path, &out_path, Some(rt_dir), stem)?;

    Ok(out_path)
}

fn assemble_asm_file(
    target: Target,
    asm_path: &Path,
    out_path: &Path,
    current_dir: Option<&Path>,
    label: &str,
) -> Result<(), TyuError> {
    match target.spec().assembler {
        AssemblerKind::Fasm => {
            let status = Command::new("fasm")
                .args([
                    asm_path.to_string_lossy().as_ref(),
                    out_path.to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| TyuError::Build(format!("running fasm: {}", e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!("fasm failed to assemble '{}'", label)).into());
            }
        }
        AssemblerKind::GasArm => {
            let asm_name = asm_path
                .file_name()
                .ok_or_else(|| {
                    TyuError::Build(format!("invalid asm path '{}'", asm_path.display()))
                })?
                .to_owned();
            let mut cmd = Command::new("arm-none-eabi-as");
            if let Some(dir) = current_dir {
                cmd.current_dir(dir);
            }
            let status = cmd
                .args([
                    "-mcpu=cortex-m3",
                    "-mthumb",
                    asm_name.to_string_lossy().as_ref(),
                    "-o",
                    out_path.to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| TyuError::Build(format!("running arm-none-eabi-as: {}", e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!(
                    "arm-none-eabi-as failed to assemble '{}'",
                    label
                ))
                .into());
            }
        }
        AssemblerKind::GasRiscV => {
            let asm_name = asm_path
                .file_name()
                .ok_or_else(|| {
                    TyuError::Build(format!("invalid asm path '{}'", asm_path.display()))
                })?
                .to_owned();
            let asm = toolchain::resolve_tool_candidates(&[
                "riscv64-unknown-elf-as",
                "riscv64-linux-gnu-as",
            ])
            .map_err(|e| TyuError::Build(format!("resolving riscv assembler: {}", e)))?;
            let mut cmd = Command::new(&asm);
            if let Some(dir) = current_dir {
                cmd.current_dir(dir);
            }
            let status = cmd
                .args([
                    "-march=rv32im",
                    "-mabi=ilp32",
                    asm_name.to_string_lossy().as_ref(),
                    "-o",
                    out_path.to_string_lossy().as_ref(),
                ])
                .status()
                .map_err(|e| TyuError::Build(format!("running {}: {}", asm.display(), e)))?;
            if !status.success() {
                return Err(TyuError::Build(format!(
                    "{} failed to assemble '{}'",
                    asm.display(),
                    label
                ))
                .into());
            }
        }
    }

    Ok(())
}

/// Assemble the mandatory core runtime and any optional feature-specific
/// units for `target`.  Returns a `Vec` of object-file paths to link.
///
/// Core (`runtime.asm`) is always assembled.  Feature-specific units
/// (e.g. `modload.asm` for `module-loading`) are assembled only when
/// the corresponding feature is enabled.
pub fn assemble_runtime(
    target: Target,
    out_dir: &Path,
    feature_set: FeatureSet,
    platform_selection: Option<&platform::ResolvedPlatformSelection>,
) -> Result<Vec<PathBuf>, TyuError> {
    let triple = std::str::from_utf8(target.triple()).map_err(|_| "non-UTF-8 triple")?;
    let rt_dir = if let Some(selection) = platform_selection {
        selection.pack_root().join(&selection.metal().path)
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("runtime")
            .join(triple)
    };

    // Core runtime is always assembled.
    let mut objs = Vec::new();
    let runtime_obj = assemble_unit(target, &rt_dir, "runtime", out_dir)?;
    let symtab_obj = generate_runtime_symtab(target, &runtime_obj, out_dir)?;
    objs.push(runtime_obj);
    objs.push(symtab_obj);

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

fn generate_runtime_symtab(
    target: Target,
    runtime_obj: &Path,
    out_dir: &Path,
) -> Result<PathBuf, TyuError> {
    let bytes = fs::read(runtime_obj).map_err(TyuError::Io)?;
    let symbols = extract_runtime_symbols(&bytes)
        .map_err(|e| TyuError::Build(format!("generating runtime symbol table: {}", e)))?;
    let flavor = match target.spec().assembler {
        AssemblerKind::Fasm => AsmFlavor::FasmX86_64,
        AssemblerKind::GasArm | AssemblerKind::GasRiscV => AsmFlavor::Gas32,
    };

    let asm_path = out_dir.join("lang_symtab.asm");
    let names_path = out_dir.join("lang_symtab.names");
    let obj_path = out_dir.join("lang_symtab.o");
    fs::write(&asm_path, render_asm(&symbols, flavor)).map_err(TyuError::Io)?;
    fs::write(&names_path, render_names(&symbols)).map_err(TyuError::Io)?;
    assemble_asm_file(target, &asm_path, &obj_path, Some(out_dir), "lang_symtab")?;
    Ok(obj_path)
}

pub fn assemble_runtime_for_context(
    ctx: &BuildContext,
    feature_set: FeatureSet,
) -> Result<Vec<PathBuf>, TyuError> {
    assemble_runtime(
        ctx.target,
        &ctx.out_dir,
        feature_set,
        ctx.platform_selection(),
    )
}

/// Link object files into the final ELF image.
pub fn link_image(
    target: Target,
    objs: &[PathBuf],
    out_dir: &Path,
    platform_selection: Option<&platform::ResolvedPlatformSelection>,
) -> Result<PathBuf, TyuError> {
    let spec = target.spec();
    let triple = std::str::from_utf8(target.triple()).map_err(|_| "non-UTF-8 triple")?;

    let linker_name = std::str::from_utf8(spec.linker).map_err(|_| "non-UTF-8 linker name")?;
    let linker = toolchain::resolve_tool(linker_name)?;

    // `None` means "no explicit linker script" — let the linker use its
    // default. Host-native targets (`qemu: None`, e.g. x86_64-unknown-linux-gnu)
    // link as ordinary Linux ELF executables via ld's built-in script; only
    // bare-metal targets need a custom `link.ld` to place sections in flash/RAM.
    let linker_script: Option<PathBuf> =
        if let Some(selection) = platform_selection {
            let boot_is_image_def = selection
                .pack
                .manifest
                .deploy
                .as_ref()
                .map(|deploy| deploy.boot.as_str())
                == Some("image_def");
            if boot_is_image_def {
                let memory =
                    selection.pack.manifest.memory.as_ref().ok_or_else(|| {
                        TyuError::Build("boot=image_def requires [memory]".into())
                    })?;
                let rendered = render_linker_script(memory)?;
                let path = out_dir.join(format!("{}.link.ld", selection.pack.name()));
                fs::write(&path, rendered).map_err(TyuError::Io)?;
                Some(path)
            } else if selection.metal().linker.is_empty() {
                // Hosted packs (e.g. linux-x86_64-hosted) declare `linker = ""`.
                None
            } else {
                Some(
                    selection
                        .pack_root()
                        .join(&selection.metal().path)
                        .join(&selection.metal().linker),
                )
            }
        } else if spec.qemu.is_none() {
            None
        } else {
            Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("runtime")
                    .join(triple)
                    .join("link.ld"),
            )
        };
    let out_path = out_dir.join("image.elf");

    let mut cmd = Command::new(&linker);
    if matches!(spec.assembler, AssemblerKind::GasRiscV) {
        cmd.arg("-m").arg("elf32lriscv");
    }
    if let Some(script) = &linker_script {
        cmd.arg("-T").arg(script);
    }
    cmd.arg("-o").arg(&out_path);
    for obj in objs {
        cmd.arg(obj);
    }

    let status = cmd
        .status()
        .map_err(|e| TyuError::Build(format!("running linker '{}': {}", linker_name, e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!("{} failed to link image", linker_name)).into());
    }

    Ok(out_path)
}

pub fn link_image_for_context(ctx: &BuildContext, objs: &[PathBuf]) -> Result<PathBuf, TyuError> {
    link_image(ctx.target, objs, &ctx.out_dir, ctx.platform_selection())
}

fn render_linker_script(memory: &platform::MemorySection) -> Result<String, TyuError> {
    let flash = memory
        .flash
        .as_ref()
        .ok_or_else(|| TyuError::Build("platform memory is missing flash region".into()))?;
    let sram = memory
        .sram
        .as_ref()
        .ok_or_else(|| TyuError::Build("platform memory is missing sram region".into()))?;

    let mut out = String::new();
    writeln!(&mut out, "ENTRY(__lang_start)").unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "MEMORY").unwrap();
    writeln!(&mut out, "{{").unwrap();
    writeln!(
        &mut out,
        "    {} (rx) : ORIGIN = 0x{:08x}, LENGTH = 0x{:08x}",
        flash.name, flash.origin, flash.length
    )
    .unwrap();
    writeln!(
        &mut out,
        "    {} (rwx) : ORIGIN = 0x{:08x}, LENGTH = 0x{:08x}",
        sram.name, sram.origin, sram.length
    )
    .unwrap();
    writeln!(&mut out, "}}").unwrap();
    writeln!(&mut out).unwrap();
    writeln!(
        &mut out,
        "__stack_top = ORIGIN({}) + LENGTH({});",
        sram.name, sram.name
    )
    .unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "SECTIONS").unwrap();
    writeln!(&mut out, "{{").unwrap();
    writeln!(&mut out, "    .vectors : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        KEEP(*(.vectors))").unwrap();
    writeln!(&mut out, "    }} > {}", flash.name).unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "    .image_def : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        KEEP(*(.image_def))").unwrap();
    writeln!(&mut out, "        KEEP(*(.image_def.*))").unwrap();
    writeln!(&mut out, "    }} > {}", flash.name).unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "    .text : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        *(.text*)").unwrap();
    writeln!(&mut out, "        *(.rodata*)").unwrap();
    writeln!(&mut out, "        *(.lang.symtab)").unwrap();
    writeln!(&mut out, "    }} > {}", flash.name).unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "    .data : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        *(.data*)").unwrap();
    writeln!(&mut out, "    }} > {} AT > {}", sram.name, flash.name).unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "    .modpack : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        *(.modpack)").unwrap();
    writeln!(&mut out, "    }} > {}", sram.name).unwrap();
    writeln!(&mut out).unwrap();
    writeln!(&mut out, "    .bss : ALIGN(4)").unwrap();
    writeln!(&mut out, "    {{").unwrap();
    writeln!(&mut out, "        __bss_start = .;").unwrap();
    writeln!(&mut out, "        *(.bss*)").unwrap();
    writeln!(&mut out, "        *(COMMON)").unwrap();
    writeln!(&mut out, "        __bss_end = .;").unwrap();
    writeln!(&mut out, "    }} > {}", sram.name).unwrap();
    writeln!(&mut out, "}}").unwrap();
    Ok(out)
}

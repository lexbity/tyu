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

use codegen_core::{AssemblerKind, Feature, FeatureSet, Target};
use lang_symtab_gen::{extract_runtime_symbols, render_asm, render_names, AsmFlavor};

use crate::args::{BuildArgs, BuildMode, EncryptMode, VerifyPolicy};
use crate::cache::{self, BuildCache};
use crate::error::TyuError;
use crate::graph::{resolve_graph, ModuleNode};
use crate::keys::{KeyMaterial, KeyRef};
use crate::platform::{self, desc, ResolvedPlatformSelection};
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
    pub mode: BuildMode,
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
    let triple = std::str::from_utf8(target.triple()).map_err(|_| TyuError::NonUtf8Triple)?;
    let feature_set = args.feature_set;
    let workspace_root = platform::workspace_root();
    platform::ensure_build_platform_interface(&workspace_root, target)?;

    // Capture before any compilation so pruning only reclaims objects that
    // predate this build (concurrent builds sharing the out-dir are safe).
    let build_started = std::time::SystemTime::now();

    // Ensure output directory exists.
    fs::create_dir_all(&out_dir).map_err(TyuError::Io)?;

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

    // When a platform is selected, compile its descriptor to the runtime-side
    // form (validating it — E3646/E3647 — and writing `platform.desc`), so
    // langc's `--platform=<dir>` always reads a fresh compiled form (P3).
    let platform_hash = if let Some(selection) = platform_selection.as_ref() {
        Some(
            desc::ensure_compiled_descriptor(
                &selection.pack.manifest_path,
                selection.pack.pack_root(),
            )?
            .platform_hash,
        )
    } else {
        None
    };

    // P6 observability (§5.11): `-v` reports the board identity and aperture
    // table the loader will enforce against.
    if args.verbose {
        match &platform_selection {
            Some(sel) => {
                let cd = desc::ensure_compiled_descriptor(
                    &sel.pack.manifest_path,
                    sel.pack.pack_root(),
                )?;
                eprintln!(
                    "tyu: platform={} platform_hash=0x{:016x}",
                    sel.pack.name(),
                    cd.platform_hash
                );
                for w in cd.apertures() {
                    eprintln!(
                        "tyu:   aperture id={} name={} kind={} base={:#x} size={:#x}",
                        w.id,
                        String::from_utf8_lossy(w.name.as_bytes()),
                        if w.kind == codegen_core::target::MmioApertureKind::Bus {
                            "bus"
                        } else {
                            "emulated"
                        },
                        w.base.unwrap_or(0),
                        w.size
                    );
                }
            }
            None => eprintln!("tyu: no platform selection (modules are unplatformed)"),
        }
    }

    // Compute compiler fingerprint (stable per build invocation). The
    // platform hash is folded in so a descriptor change invalidates cached
    // artifacts exactly as a source change would.
    let mut compiler_fp = cache::compiler_fingerprint();
    if let Some(h) = platform_hash {
        compiler_fp = cache::fnv1a_u64(&compiler_fp.to_le_bytes())
            .wrapping_mul(0x100000001b3)
            .wrapping_add(h);
    }
    // P4: the verify mode changes langc's emission (`--checks=undischarged`
    // elides discharged sites; `--verify=off` keeps every check — FR-22), so
    // it MUST change the object cache key: an off-build object would
    // otherwise satisfy an on-build lookup and the report's `emitted_checks`
    // would disagree with the object (FR-16).
    match args.verify {
        crate::args::VerifyMode::Off => {
            compiler_fp = cache::fnv1a_u64(&compiler_fp.to_le_bytes()).wrapping_add(1);
        }
        crate::args::VerifyMode::On => {}
    }
    // Slice P7 (Q5/FR-11): the guard-elision decision changes what pass 2
    // emits (guards omitted vs present) — it MUST change the object cache
    // key, exactly like the verify mode: an elided-build object would
    // otherwise satisfy a guarded-build lookup and the object would disagree
    // with the report's `emitted_checks.data_stack_guards` (FR-16).
    if args.elide_stack_guards {
        compiler_fp = cache::fnv1a_u64(&compiler_fp.to_le_bytes()).wrapping_add(64);
    }

    let mode = effective_build_mode(args, target);

    // P12.1 (§6.7/§Q15): the bundle's model-semantics id, resolved from the
    // selected pack's `[model]` section (`"unmodeled"` when no pack is
    // selected or the pack declares none). It stamps every langc invocation
    // below and keys the verdicts cache.
    //
    // P12 finding 4c: the id is only *grounded* when the declared↔artifact
    // pairing holds — a third-party pack cannot build as "modeled" while
    // its model artifact/vectors are missing or mismatched (CI guards the
    // in-tree packs; this is the structural protection).
    let model_semantics: &str = match platform_selection.as_ref() {
        Some(selection) => {
            platform::ensure_model_pairing(&selection.pack)?;
            selection.pack.model_semantics()
        }
        None => verifier::model::MODEL_UNMODELED,
    };

    // §7.2 (P6 amendment): the verification-environment components of every
    // verdicts-cache slot — semantics, stmt, toolchain pin hash, model id,
    // proof-files hash. A proof-file edit, a toolchain change, or a bundle-
    // model change must rotate each module's slot so P7's harvested
    // (`proof`-class) verdicts can never be silently reused stale.
    let verify_env = crate::proof::VerifyEnvKey::for_build(model_semantics)?;

    // Slice P7 (Q5): guard elision is a per-image fact of the *single*
    // derived data-stack geometry. A dynamic image loads modules into its own
    // stack regions — there is no one `N_main` — so elision is refused there
    // (loud, never a silent guarded build).
    if args.elide_stack_guards && mode == BuildMode::Dynamic {
        return Err(TyuError::Build(
            "--elide-stack-guards requires a static link/load mode (a dynamic image's unit budgets are not a single derived geometry)"
                .into(),
        ));
    }

    // Slice P7: the two-pass decision. When elision is requested, the runtime
    // is assembled FIRST (so the `N_main` geometry symbols exist for
    // `derive_main_budget`), every module's facts are extracted (pass 1 —
    // `--emit=obligations`, no codegen), and the image-level
    // `stack-budget(main)` verdict decides whether pass 2 forwards
    // `--elide-ds-guards`. One decision for the whole image (FR-11 per-image
    // atomicity — all modules or none); the decision is written to a
    // validated image-verdicts record (E6415 on a corrupt record) that the
    // report's `guards` field is derived from.
    let mut elide_ds = false;
    let mut runtime_objs: Vec<PathBuf> = Vec::new();
    if args.elide_stack_guards {
        runtime_objs = assemble_runtime_for_context_mode(&ctx, feature_set, mode)?;
        let pass1 = extract_obligations_for_elision(
            &langc,
            &modules,
            &args.include_dirs,
            args.sysroot.as_deref(),
            &out_dir,
            platform_selection.as_ref().map(|s| s.pack.pack_root()),
            model_semantics,
            feature_set,
        )?;
        let verdict = super::verify::elision_main_verdict(
            &ctx,
            &pass1,
            modules.last().map(|m| m.name.as_str()),
        );
        elide_ds = verdict == "discharged";
        let main = super::verify::main_context(
            &pass1,
            modules.last().map(|m| m.name.as_str()),
            super::verify::derive_main_budget(&ctx),
        );
        let record = verifier::codec::ImageVerdicts {
            elided: elide_ds,
            main,
        };
        write_image_verdicts(&out_dir, &record)?;
        if elide_ds {
            eprintln!(
                "tyu: eliding x86 data-stack guards (image stack-budget(main) discharged: {})",
                record.main.verdict
            );
        } else {
            eprintln!(
                "tyu: keeping x86 data-stack guards (image stack-budget(main) = {})",
                record.main.verdict
            );
        }
    }

    // Pre-compute content hashes for every module path.
    let mut path_to_hash: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for module in &modules {
        if let Ok(h) = cache::content_hash(&module.path) {
            path_to_hash.insert(module.path.clone(), h);
        }
    }

    // Transitive-dep hash cache: module path → sorted hashes of all transitive deps.
    let mut transitive_cache: BTreeMap<PathBuf, Vec<u64>> = BTreeMap::new();

    // Module name → fingerprints built in this run (for stale-artifact pruning).
    let mut built_fps: BTreeMap<String, Vec<u64>> = BTreeMap::new();

    // Compile each module in dependency order.
    let mut module_objs: Vec<PathBuf> = Vec::new();
    let mut module_obl: Vec<(String, Option<PathBuf>)> = Vec::new();

    // P7.3 (FR-3): under `--verify-tool=lean` the build is TWO-PASS — the
    // proof pipeline (package gen → E6418 → lake build → harvest) needs the
    // obligation artifacts BEFORE codegen, and pass-2 codegen consumes the
    // harvested `tyu.verdicts/v2` files (`proof`-class discharges now elide
    // checks). Pass 1 extracts every module's artifact (`--emit=obligations`,
    // no codegen) into `<out>/.tyu-oblig/`; the harvest verdict files then
    // drive pass 2. Everything below stays single-pass otherwise.
    type ProofPipeline = (
        crate::proof::VerifyEnvKey,
        verifier::report::ProofStatus,
        Vec<(String, PathBuf)>,
        Vec<(String, PathBuf)>,
    );
    let proof_pipeline: Option<ProofPipeline> = match args.verify_tool {
        Some(tool) => {
            let artifacts = extract_module_artifacts(
                &langc,
                &modules,
                &args.include_dirs,
                args.sysroot.as_deref(),
                &out_dir,
                platform_selection.as_ref().map(|s| s.pack.pack_root()),
                model_semantics,
                feature_set,
                triple,
            )?;
            let (status, verdict_files) = crate::proof::run_lean_pipeline(tool, &artifacts)?;
            Some((verify_env.clone(), status, verdict_files, artifacts))
        }
        None => None,
    };
    // P13.1: the refinement-context document, when the proof pipeline ran
    // and the artifacts' bundle is MODELED (the package wrote the manifest;
    // `refinement_context` decides PRESENCE — an unmodeled bundle does not
    // carry a context, so langc gets no `--refinements` at all). Resolved
    // ONCE before the compile loop; langc's pass-2 statement binding consumes
    // it (`--refinements`, §Q3 relativism).
    let refinements_ctx: Option<PathBuf> = match &proof_pipeline {
        Some((_, _, _, artifacts)) => {
            let paths: Vec<PathBuf> = artifacts.iter().map(|(_, p)| p.clone()).collect();
            let modeled = crate::proof::refinement_context(&paths)
                .ok()
                .flatten()
                .is_some();
            crate::proof::project_root_for(None)
                .ok()
                .map(|root| crate::proof::package_refinements_path(&root))
                .filter(|p| modeled && p.is_file())
        }
        None => None,
    };

    for module in &modules {
        let inputs_fp = {
            let own_hash = path_to_hash.get(&module.path).copied().unwrap_or(0);
            let transitive =
                cache::collect_transitive_hashes(module, &path_to_hash, &mut transitive_cache);
            cache::inputs_fingerprint(own_hash, triple, &transitive)
        };
        built_fps
            .entry(module.name.clone())
            .or_default()
            .push(inputs_fp);

        // P7.3: pass-2 feeds the harvested v2 verdicts as `--verdicts`, and
        // binds statement hashes against the pass-1 artifact as `--bind-obl`
        // (FR-5: the certified artifact, not the verdict-dependent live
        // lowering — see proof.rs run_lean_pipeline + langc --bind-obl).
        let harvest_verdicts = proof_pipeline
            .as_ref()
            .and_then(|(_, _, files, _)| files.iter().find(|(m, _)| m == &module.name))
            .map(|(_, p)| p.clone());
        let bind_obl = proof_pipeline
            .as_ref()
            .and_then(|(_, _, _, arts)| arts.iter().find(|(m, _)| m == &module.name))
            .map(|(_, p)| p.clone());
        // P13.1: the bundle's refinement-context document (the package's
        // `refinements.json`, present when the bundle declares
        // `[refinements]`) — langc's FR-5 recompute binds the same
        // statements the renderer produced.
        let refinements_manifest = refinements_ctx.as_deref();

        let compiled = compile_module(
            &langc,
            target,
            module,
            &args.include_dirs,
            args.sysroot.as_deref(),
            &out_dir,
            platform_selection.as_ref().map(|s| s.pack.pack_root()),
            model_semantics,
            &mut cache,
            compiler_fp,
            inputs_fp,
            abi_hash,
            triple,
            feature_set,
            args.verify,
            elide_ds,
            args.verify_tool,
            &verify_env,
            args.verify_policy,
            harvest_verdicts.as_deref(),
            bind_obl.as_deref(),
            refinements_manifest,
        )?;
        module_objs.push(compiled.object_path);
        module_obl.push((module.name.clone(), compiled.obl_path));
    }

    // P11.1: emit the per-module `tyu.vm/1` summaries (the verdicts-v2 →
    // manifest-summary converter) beside the report — the automatic source
    // every manifest path (this build's root lmod, deploy pairing) consumes.
    super::vm_summary::write_module_summaries(
        &ctx.out_dir,
        &module_obl,
        args.verify_policy,
        &verify_env,
    )?;

    let (final_image, exec_image) = if mode == BuildMode::Dynamic {
        build_dynamic_image(
            &ctx,
            feature_set,
            &module_objs,
            &modules,
            args.metal_sign_key.as_deref(),
            args.metal_kek.as_deref(),
            args.metal_encrypt_mode,
            args.verify_manifest.as_deref(),
            args.verify_policy,
        )?
    } else {
        let mut objs = module_objs.clone();
        // Slice P7: the two-pass flow assembles the runtime BEFORE pass 1
        // (for the `N_main` geometry); the single-pass flow assembles it here
        // as before. `mode` is the *chosen* link/load mode — the runtime
        // object set is mode-independent for the static path.
        if runtime_objs.is_empty() {
            runtime_objs = assemble_runtime_for_context_mode(&ctx, feature_set, BuildMode::Static)?;
        }
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
                let image_def_obj = assemble_image_def(target, &out_dir)?;
                objs.push(image_def_obj);
            }
        }

        let build_manifest_record = build_manifest_record_for(
            args.verify_manifest.as_deref(),
            args.verify_policy,
            &out_dir,
            modules.last().map(|m| m.name.as_str()),
        )?;
        let exec_image = link_image_for_context(&ctx, &objs)?;
        let final_image = if matches!(target, Target::X86_64UnknownLinuxGnu) {
            exec_image.clone()
        } else {
            let root_obj = module_objs
                .get(module_count.saturating_sub(1))
                .ok_or_else(|| TyuError::Build("no root module object to pack".into()))?;
            pack_final_lmod(
                root_obj,
                &out_dir,
                modules.last().map(|m| m.name.as_str()),
                &build_manifest_record,
            )?
        };
        (final_image, exec_image)
    };

    // Persist cache (and prune stale artifacts from this out-dir first).
    prune_out_dir(&out_dir, &mut cache, &built_fps, build_started, &verify_env)?;
    cache.save()?;

    // P7.3 (FR-3): the proof pipeline ran in pass 1 (before codegen) for
    // `--verify-tool=lean` — its status carries the harvest's per-module
    // statement accounting; a plain build carries the inert `none` status.
    let proof_status = match &proof_pipeline {
        Some((_, status, _, _)) => status.clone(),
        None => verifier::report::ProofStatus::none(),
    };

    // P3/P4: compose the verification report from every module's obligation
    // artifact + verdict echo (§7.3: per-context stack-budget verdicts +
    // per-module class accounting; §6.5: the `emitted_checks` honesty block
    // summed from each echo) and write `<out_dir>/verify-report.json`.
    // Written only for a produced image (a link/pack failure leaves no report
    // behind). Enforces the verify policy (E6410, FR-18) and prints the
    // one-line accounting summary (NFR-9).
    super::verify::compose_and_write_report(
        &ctx,
        &module_obl,
        modules.last().map(|m| m.name.as_str()),
        args.verify,
        args.verify_policy,
        args.feature_set
            .contains(codegen_core::Feature::ModuleLoading),
        elide_ds,
        &proof_status,
        &verify_env,
        args.proven_no_candidates,
    )?;

    Ok(BuildOutcome {
        final_image,
        execution_image: exec_image,
        mode,
        target,
        platform_selection,
    })
}

fn effective_build_mode(args: &BuildArgs, target: Target) -> BuildMode {
    match args.mode {
        Some(mode) => mode,
        None if target.spec().qemu.is_some() => BuildMode::Dynamic,
        None => BuildMode::Static,
    }
}

#[allow(clippy::too_many_arguments)] // the image builder takes one context item per parameter
fn build_dynamic_image(
    ctx: &BuildContext,
    feature_set: FeatureSet,
    module_objs: &[PathBuf],
    modules: &[ModuleNode],
    metal_sign_key: Option<&str>,
    metal_kek: Option<&str>,
    metal_encrypt_mode: Option<EncryptMode>,
    verify_manifest: Option<&Path>,
    verify_policy: VerifyPolicy,
) -> Result<(PathBuf, PathBuf), TyuError> {
    if module_objs.len() != modules.len() {
        return Err(TyuError::Build(
            "dynamic image: module objects and graph nodes are misaligned".into(),
        ));
    }
    let root_name = modules.last().map(|m| m.name.as_str()).to_owned();
    // Pack EVERY graph module (the objects/graph are already deps-first) —
    // the modpack is the shipped module set, and the firmware loader loads
    // all of it, binding each module's imports against the previous ones.
    // Each .lmod carries its OWN verify_manifest record (P11.1: derived from
    // the module's `tyu.vm/1` summary under a requiring policy; the root
    // additionally honors an explicit `--verify-manifest`).
    let mut lmod_paths: Vec<PathBuf> = Vec::new();
    for (obj, node) in module_objs.iter().zip(modules) {
        let record = if Some(node.name.as_str()) == root_name {
            build_manifest_record_for(
                verify_manifest,
                verify_policy,
                &ctx.out_dir,
                Some(node.name.as_str()),
            )?
        } else {
            build_manifest_record_for(None, verify_policy, &ctx.out_dir, Some(node.name.as_str()))?
        };
        let lmod = pack_final_lmod(obj, &ctx.out_dir, Some(node.name.as_str()), &record)?;
        lmod_paths.push(lmod);
    }
    let sign_key = resolve_metal_sign_key(metal_sign_key)?;
    let kek = resolve_metal_kek(metal_kek)?;
    if kek.is_some() && sign_key.is_none() {
        return Err(TyuError::Build(
            "--metal-kek requires --metal-sign-key so encrypted modules are authenticated before decrypt".into(),
        ));
    }
    if metal_encrypt_mode.is_some() && kek.is_none() {
        return Err(TyuError::Build(
            "--metal-encrypt requires --metal-kek".into(),
        ));
    }
    let root_lmod = lmod_paths
        .last()
        .ok_or_else(|| TyuError::Build("no root module object to pack".into()))?;
    if let Some(kek) = kek.as_ref() {
        let encrypt_kek = test_encrypt_kek_override()?.unwrap_or(*kek);
        for p in &lmod_paths {
            encrypt_lmod_in_place(
                p,
                &encrypt_kek,
                metal_encrypt_mode.unwrap_or(EncryptMode::Fleet),
            )?;
        }
    }
    if let Some(key) = sign_key.as_ref() {
        for p in &lmod_paths {
            sign_lmod_in_place(p, key)?;
        }
    }
    // The test-mutation hook targets the ROOT module (the tamper tests break
    // the shipped root's signature and expect the E_SIG_INVALID trap).
    maybe_apply_test_lmod_mutation(root_lmod)?;

    let mut firmware_objs =
        assemble_runtime_for_context_mode(ctx, feature_set, BuildMode::Dynamic)?;

    // The dynamic firmware is the flashable image, so it carries the PICOBIN
    // boot block just like the static image does (DS2 5.9.5).
    if let Some(selection) = ctx.platform_selection() {
        if selection
            .pack
            .manifest
            .deploy
            .as_ref()
            .map(|deploy| deploy.boot.as_str())
            == Some("image_def")
        {
            firmware_objs.insert(0, assemble_image_def(ctx.target, &ctx.out_dir)?);
        }
    }

    if sign_key.is_some() || kek.is_some() {
        firmware_objs.push(assemble_keys_object(
            ctx.target,
            &ctx.out_dir,
            sign_key.as_ref(),
            kek.as_ref(),
            metal_encrypt_mode.unwrap_or(EncryptMode::Fleet),
        )?);
    }
    firmware_objs.push(assemble_modpack_object(
        ctx.target,
        &ctx.out_dir,
        &lmod_paths,
    )?);
    firmware_objs.push(build_device_loader_staticlib(
        ctx.target,
        sign_key.is_some(),
        kek.is_some(),
    )?);
    let firmware = link_image_for_context(ctx, &firmware_objs)?;
    Ok((firmware.clone(), firmware))
}

fn resolve_metal_sign_key(keyref: Option<&str>) -> Result<Option<[u8; 32]>, TyuError> {
    let Some(keyref) = keyref else {
        return Ok(None);
    };
    let parsed = KeyRef::parse(keyref)?;
    let material = KeyMaterial::resolve(&parsed)?;
    let key = material.try_as_32bytes()?.to_owned();
    Ok(Some(key))
}

fn resolve_metal_kek(keyref: Option<&str>) -> Result<Option<[u8; 32]>, TyuError> {
    let Some(keyref) = keyref else {
        return Ok(None);
    };
    let parsed = KeyRef::parse(keyref)?;
    let material = KeyMaterial::resolve(&parsed)?;
    let key = material.try_as_32bytes()?.to_owned();
    Ok(Some(key))
}

fn test_encrypt_kek_override() -> Result<Option<[u8; 32]>, TyuError> {
    let Ok(hex_key) = std::env::var("TYU_TEST_ENCRYPT_WITH_KEK") else {
        return Ok(None);
    };
    let bytes = hex::decode(hex_key.trim()).map_err(|e| {
        TyuError::Key(format!(
            "TYU_TEST_ENCRYPT_WITH_KEK must be a 32-byte hex key: {}",
            e
        ))
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
        TyuError::Key(format!(
            "TYU_TEST_ENCRYPT_WITH_KEK must be 32 bytes, got {} bytes",
            bytes.len()
        ))
    })?;
    Ok(Some(arr))
}

fn encrypt_lmod_in_place(
    lmod_path: &Path,
    kek: &[u8; 32],
    mode: EncryptMode,
) -> Result<(), TyuError> {
    let bytes = fs::read(lmod_path).map_err(TyuError::Io)?;
    let encrypted = match mode {
        EncryptMode::Fleet => lmod_encrypt::encrypt_fleet(&bytes, kek),
        EncryptMode::Device => {
            let keys = [(String::from("qemu-device"), *kek)];
            lmod_encrypt::encrypt_device(&bytes, &keys)
        }
        EncryptMode::None => {
            return Err(TyuError::Build(
                "--metal-encrypt=none is invalid with --metal-kek".into(),
            ));
        }
    }
    .map_err(|e| TyuError::Build(format!("lmod-encrypt: {}", e)))?;
    fs::write(lmod_path, encrypted).map_err(TyuError::Io)
}

fn sign_lmod_in_place(lmod_path: &Path, key: &[u8; 32]) -> Result<(), TyuError> {
    let bytes = fs::read(lmod_path).map_err(TyuError::Io)?;
    let signed =
        lmod_sign::sign(&bytes, key).map_err(|e| TyuError::Build(format!("lmod-sign: {}", e)))?;
    fs::write(lmod_path, signed).map_err(TyuError::Io)
}

fn maybe_apply_test_lmod_mutation(lmod_path: &Path) -> Result<(), TyuError> {
    let Ok(mutation) = std::env::var("TYU_TEST_MUTATE_LMOD") else {
        return Ok(());
    };
    let mut bytes = fs::read(lmod_path).map_err(TyuError::Io)?;
    match mutation.as_str() {
        "truncate" => {
            let new_len = bytes.len().saturating_sub(1).max(1);
            bytes.truncate(new_len);
        }
        "abi-zero" => {
            if bytes.len() < 16 {
                return Err(TyuError::Build(
                    "TYU_TEST_MUTATE_LMOD=abi-zero needs a full lmod header".into(),
                ));
            }
            bytes[8..16].fill(0);
        }
        "has-isr" => {
            let modinfo_flags = lmod_modinfo_flags_offset(&bytes)?;
            bytes[modinfo_flags..modinfo_flags + 2].copy_from_slice(&1u16.to_le_bytes());
        }
        "reloc-unsupported" => {
            let reloc_kind = lmod_first_reloc_kind_offset(&bytes)?;
            bytes[reloc_kind] = 0xff;
        }
        "symbol-unresolved" => {
            let reloc_sym_hash = lmod_first_reloc_sym_hash_offset(&bytes)?;
            bytes[reloc_sym_hash..reloc_sym_hash + 8]
                .copy_from_slice(&0xfeed_dead_beef_cafeu64.to_le_bytes());
        }
        "aperture-base-mismatch" => {
            // Flip the first MmioApertureBase reloc site so the loader's
            // defense-in-depth `check` (E5219, behind the E5220 gate) rejects
            // a module whose site claims a foreign geometry (P6).
            let site = lmod_first_aperture_base_site_offset(&bytes)?;
            let orig = u32::from_le_bytes(bytes[site..site + 4].try_into().unwrap());
            bytes[site..site + 4].copy_from_slice(&(orig ^ 0x1000).to_le_bytes());
        }
        "platform-hash-mismatch" => {
            // Flip the modinfo `platform_hash` so the loader's board-identity
            // gate rejects with E5220 (decision D-5).
            let mi = lmod_modinfo_off(&bytes)?;
            let orig = u64::from_le_bytes(bytes[mi + 36..mi + 44].try_into().unwrap());
            bytes[mi + 36..mi + 44].copy_from_slice(&(orig ^ 1).to_le_bytes());
        }
        "modinfo-version" => {
            // Downgrade the modinfo version to 3 → E5224 (no shim).
            let mi = lmod_modinfo_off(&bytes)?;
            bytes[mi + 4..mi + 6].copy_from_slice(&3u16.to_le_bytes());
        }
        "aperture-name-hash" => {
            // Corrupt the first aperture-use entry's name_hash → E5222.
            let mi = lmod_modinfo_off(&bytes)?;
            let wu = lmod::modinfo::aperture_use_offset(&bytes[mi..])
                .ok_or_else(|| TyuError::Build("no aperture-use table to mutate".into()))?
                + mi;
            let orig = u64::from_le_bytes(bytes[wu..wu + 8].try_into().unwrap());
            bytes[wu..wu + 8].copy_from_slice(&(orig ^ 0xdead_beef).to_le_bytes());
        }
        "aperture-table-malformed" => {
            // Corrupt the modinfo aperture_count → E5223.
            let mi = lmod_modinfo_off(&bytes)?;
            bytes[mi + 32..mi + 34].copy_from_slice(&99u16.to_le_bytes());
        }
        other => {
            return Err(TyuError::Build(format!(
                "unknown TYU_TEST_MUTATE_LMOD value '{}'",
                other
            )));
        }
    }
    fs::write(lmod_path, bytes).map_err(TyuError::Io)
}

fn lmod_modinfo_flags_offset(bytes: &[u8]) -> Result<usize, TyuError> {
    let mi = lmod_modinfo_off(bytes)?;
    Ok(mi + 6)
}

/// The `.lmod` byte offset of the modinfo section (container header fields at
/// bytes 20..24), validated as a v4 modinfo header.
fn lmod_modinfo_off(bytes: &[u8]) -> Result<usize, TyuError> {
    if bytes.len() < lmod::header::HEADER_SIZE as usize {
        return Err(TyuError::Build(
            "test lmod mutation: header too short".into(),
        ));
    }
    let modinfo_off = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let modinfo_len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if modinfo_len < lmod::modinfo::MODINFO_HEADER_SIZE as usize
        || modinfo_off
            .checked_add(lmod::modinfo::MODINFO_HEADER_SIZE as usize)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(TyuError::Build(
            "test lmod mutation: modinfo out of range".into(),
        ));
    }
    Ok(modinfo_off)
}

fn lmod_first_reloc_kind_offset(bytes: &[u8]) -> Result<usize, TyuError> {
    let reloc_off = lmod_first_reloc_offset(bytes)?;
    Ok(reloc_off + 12)
}

fn lmod_first_reloc_sym_hash_offset(bytes: &[u8]) -> Result<usize, TyuError> {
    let reloc_off = lmod_first_reloc_offset(bytes)?;
    Ok(reloc_off + 4)
}

fn lmod_first_reloc_offset(bytes: &[u8]) -> Result<usize, TyuError> {
    if bytes.len() < lmod::header::HEADER_SIZE as usize {
        return Err(TyuError::Build(
            "test lmod mutation: header too short".into(),
        ));
    }
    let reloc_off = u32::from_le_bytes(bytes[56..60].try_into().unwrap()) as usize;
    let reloc_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap());
    if reloc_count == 0 {
        return Err(TyuError::Build(
            "test lmod mutation needs at least one relocation".into(),
        ));
    }
    if reloc_off
        .checked_add(lmod::reloc::RELOC_ENTRY_SIZE as usize)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(TyuError::Build(
            "test lmod mutation: first relocation out of range".into(),
        ));
    }
    Ok(reloc_off)
}

/// The .lmod offset of the first `MmioApertureBase` reloc site (P6), for the
/// `aperture-base-mismatch` test mutation.
fn lmod_first_aperture_base_site_offset(bytes: &[u8]) -> Result<usize, TyuError> {
    if bytes.len() < lmod::header::HEADER_SIZE as usize {
        return Err(TyuError::Build(
            "test lmod mutation: header too short".into(),
        ));
    }
    let reloc_off = u32::from_le_bytes(bytes[56..60].try_into().unwrap()) as usize;
    let reloc_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap());
    for i in 0..reloc_count {
        let off = reloc_off + i as usize * lmod::reloc::RELOC_ENTRY_SIZE as usize;
        if off + 13 > bytes.len() {
            return Err(TyuError::Build(
                "test lmod mutation: reloc entry out of range".into(),
            ));
        }
        let site = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        let kind = bytes[off + 12];
        if kind == lmod::reloc::RelocKind::MmioApertureBase as u8 {
            if site + 4 > bytes.len() {
                return Err(TyuError::Build(
                    "test lmod mutation: aperture-base site out of range".into(),
                ));
            }
            return Ok(site);
        }
    }
    Err(TyuError::Build(
        "test lmod mutation needs a aperture-base reloc".into(),
    ))
}

pub fn resolve_build_context(args: &BuildArgs) -> Result<BuildContext, TyuError> {
    let workspace_root = platform::workspace_root();
    let platform_selection = match args.platform.as_deref() {
        Some(name) => Some(platform::resolve_platform_selection(
            &workspace_root,
            name,
            args.isa.as_deref(),
        )?),
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

/// The verify_manifest record for a build: the explicit `--verify-manifest`
/// summary when supplied, else — under a requiring policy — the ROOT
/// module's derived `tyu.vm/1` summary (the P11.1 automatic path). Ordinary
/// open-ok builds keep the empty record (no golden churn).
fn build_manifest_record_for(
    verify_manifest: Option<&Path>,
    policy: VerifyPolicy,
    out_dir: &Path,
    root: Option<&str>,
) -> Result<Vec<u8>, TyuError> {
    match verify_manifest {
        Some(path) => manifest_record(Some(path)),
        None if policy != VerifyPolicy::OpenOk => match root {
            Some(root) => {
                let sum = crate::vm_summary::vm_summary_path(out_dir, root);
                if sum.is_file() {
                    manifest_record(Some(&sum))
                } else {
                    Ok(Vec::new())
                }
            }
            None => Ok(Vec::new()),
        },
        None => Ok(Vec::new()),
    }
}

/// Assemble the verify_manifest record from the build's `tyu.vm/1`
/// summary (P11: verdict provenance rides in the packed image). An empty
/// record when no summary was supplied.
pub fn manifest_record(summary: Option<&Path>) -> Result<Vec<u8>, TyuError> {
    match summary {
        None => Ok(Vec::new()),
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(TyuError::Io)?;
            lmod_pack::verify::encode_from_json_text(&text)
                .map_err(|e| TyuError::Build(format!("verify-manifest: {e}")))
        }
    }
}

fn pack_final_lmod(
    obj_path: &Path,
    out_dir: &Path,
    module_name: Option<&str>,
    record: &[u8],
) -> Result<PathBuf, TyuError> {
    let lmod_name = match module_name {
        Some(name) if !name.is_empty() => format!("{}.lmod", name),
        _ => "image.lmod".to_string(),
    };
    let lmod_path = out_dir.join(lmod_name);
    let obj_bytes = std::fs::read(obj_path)
        .map_err(|e| TyuError::Build(format!("reading '{}': {}", obj_path.display(), e)))?;
    // P6 (decision D-4): the pack records `MmioApertureBase` relocs but binds no
    // bases; the loader writes the board's aperture bases at load time.
    let packed = lmod_pack::pack_with_verify_manifest(&obj_bytes, record)
        .map_err(|e| TyuError::Build(format!("lmod-pack: {}", e)))?;
    std::fs::write(&lmod_path, &packed)
        .map_err(|e| TyuError::Build(format!("writing '{}': {}", lmod_path.display(), e)))?;
    Ok(lmod_path)
}

fn assemble_image_def(target: Target, out_dir: &Path) -> Result<PathBuf, TyuError> {
    let asm_path = out_dir.join("image_def.s");
    let obj_path = out_dir.join("image_def.o");
    let asm = render_image_def_asm(target)?;
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
            let asm = toolchain::resolve_tool_candidates(toolchain::RISCV_AS_CANDIDATES).map_err(
                |e| TyuError::Build(format!("resolving riscv assembler for image_def: {}", e)),
            )?;
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

/// PICOBIN image-definition block for the RP2350 bootrom (DS2 5.9).
///
/// Layout (all words little-endian):
///   word  0      block marker start 0xffffded3
///   word  1      IMAGE_DEF item: type 0x42, 1 word, image_type_flags
///   words 2-5    ENTRY_POINT item: type 0x44, 4 words, { pc, sp, sp_limit }
///   word  6      2BS_LAST item 0x000001ff (per the DS2 5.9.5 minimum blocks)
///   word  7      next-block relative pointer (0: block loop of one block)
///   word  8      block marker end 0xab123579
///
/// The ENTRY_POINT item removes any dependence on section order: the bootrom
/// enters at the given PC with the given SP instead of deriving them from a
/// vector table at image start. The block must live in the first 4 kB of
/// flash (DS2 5.9.5); the pack's linker script places it right after the
/// vector table.
fn render_image_def_asm(target: Target) -> Result<String, TyuError> {
    let (flags, entry) = match target {
        Target::ArmV7MUnknownNone => ("0x1021", "__lang_start + 1"), // EXE | Secure | Arm | RP2350
        Target::RiscV32UnknownNone => ("0x1101", "__lang_start"),    // EXE | RISC-V | RP2350
        other => {
            return Err(TyuError::Build(format!(
                "boot=image_def is only defined for the RP2350 triples, got {other:?}"
            )))
        }
    };

    // GAS comment character differs per ISA: `@` for Arm, `#` for RISC-V.
    let c = match target {
        Target::ArmV7MUnknownNone => '@',
        _ => '#',
    };

    let mut out = String::new();
    let w = |out: &mut String, text: &str, comment: &str| {
        let _ = writeln!(out, "    {text:<32}{c} {comment}");
    };
    let _ = writeln!(&mut out, ".section .image_def, \"a\", %progbits");
    let _ = writeln!(&mut out, ".globl __lang_image_def");
    let _ = writeln!(&mut out, "__lang_image_def:");
    w(&mut out, ".word 0xffffded3", "PICOBIN_BLOCK_MARKER_START");
    w(&mut out, ".byte 0x42", "PICOBIN_BLOCK_ITEM_1BS_IMAGE_TYPE");
    w(&mut out, ".byte 0x01", "item is 1 word");
    let _ = writeln!(&mut out, "    .short {flags}");
    w(&mut out, ".byte 0x44", "PICOBIN_BLOCK_ITEM_1BS_ENTRY_POINT");
    w(&mut out, ".byte 0x04", "item is 4 words (pc, sp, sp_limit)");
    let _ = writeln!(&mut out, "    .short 0x0000");
    let _ = writeln!(&mut out, "    .word {entry}");
    w(&mut out, ".word __stack_top", "initial sp");
    w(&mut out, ".word __lang_stack_limit", "sp limit");
    w(&mut out, ".word 0x000001ff", "PICOBIN_BLOCK_ITEM_2BS_LAST");
    w(&mut out, ".word 0x00000000", "next block in loop: self");
    w(&mut out, ".word 0xab123579", "PICOBIN_BLOCK_MARKER_END");
    Ok(out)
}

/// Compile a single `.mod` file with langc, without caching.
/// Returns the path to the produced `.o`.
// 8 parameters mirror the langc invocation surface 1:1; bundling them would obscure the call sites.
#[allow(clippy::too_many_arguments)]
pub fn compile_simple(
    target: Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
    sysroot: Option<&Path>,
    include_dirs: &[PathBuf],
    feature_set: FeatureSet,
    platform_dir: Option<&Path>,
) -> Result<PathBuf, TyuError> {
    let triple = std::str::from_utf8(target.triple()).map_err(|_| TyuError::NonUtf8Triple)?;
    let langc = toolchain::resolve_tool("langc")?;

    let mut cmd = Command::new(&langc);
    cmd.arg("--emit=obj");
    cmd.arg(format!("--target={}", triple));
    cmd.arg(format!("--out-dir={}", out_dir.display()));
    if let Some(sr) = sysroot {
        cmd.arg(format!("--sysroot={}", sr.display()));
    }
    if let Some(dir) = platform_dir {
        cmd.arg(format!("--platform={}", dir.display()));
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
        return Err(TyuError::Build(format!(
            "langc failed on '{}'",
            src.display()
        )));
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
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
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
        ctx.platform_selection.as_ref().map(|s| s.pack.pack_root()),
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
///
/// langc writes its object into a per-process scratch directory (named after
/// the module *declaration*, e.g. `<Module>.o`), which is then moved into
/// `out_dir` under a source-keyed `<Module>-<inputs_fp>.o`.  Using a scratch
/// directory (rather than `out_dir` directly) means two concurrent builds that
/// share an `out_dir` can never cross-wire the shared `<Module>.o` slot.
///
/// P4 (`--verify=on`, the default): langc additionally receives
/// `--checks=undischarged --verdicts=<cache>` where the verdicts cache lives
/// at `<out_dir>/.tyu-verify/<Module>-<inputs_fp>.verdicts.json` (§7.4). On a
/// cache miss the file starts empty (the in-tree discharger still resolves the
/// descriptor-dischargeable sites); langc echoes the resolved verdicts +
/// emitted accounting into `<scratch>/<Module>.verdicts.inTree.json`, which is
/// re-homed over the cache slot via rename — so the next identical build
/// resolves from cache (Q11/FR-17). `--verify=off` passes exactly today's
/// argv (FR-22).
// 15 parameters model the compile pipeline state (hashes, cache, mode flags); a struct would
// just relocate the same fields without reducing real coupling.
#[allow(clippy::too_many_arguments)]
fn compile_module(
    langc: &Path,
    _target: Target,
    module: &ModuleNode,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    out_dir: &Path,
    platform_dir: Option<&Path>,
    model_semantics: &str,
    cache: &mut BuildCache,
    compiler_fp: u64,
    inputs_fp: u64,
    abi_hash: u64,
    triple: &str,
    feature_set: FeatureSet,
    verify: crate::args::VerifyMode,
    elide_ds_guards: bool,
    verify_tool: Option<crate::args::VerifyTool>,
    verify_env: &crate::proof::VerifyEnvKey,
    verify_policy: crate::args::VerifyPolicy,
    verdicts_override: Option<&Path>,
    bind_obl: Option<&Path>,
    refinements_manifest: Option<&Path>,
) -> Result<CompiledModule, TyuError> {
    let features = feature_set.bits();
    // Check cache first. The obligation artifact rides beside the object under
    // the same source-keyed fingerprint name; a cache hit returns the paths.
    if let Some(cached) = cache.lookup(compiler_fp, inputs_fp, abi_hash, features) {
        if cached.object_path.exists() {
            let obl_path = out_dir.join(rehomed_obl_filename(module, inputs_fp));
            // P4: the verdicts echo must be present AND schema-valid for the
            // report composition to be honest — a missing/corrupt slot
            // invalidates the cache entry (treated as a miss, §7.4: self-
            // healing, never a stale-echo report).
            let verdicts_slot = out_dir
                .join(".tyu-verify")
                .join(crate::proof::verdicts_slot_name(
                    &module.name,
                    inputs_fp,
                    verify_env,
                ));
            let verdicts_usable =
                verifier::verdict::read_verdicts(&fs::read(&verdicts_slot).unwrap_or_default())
                    .is_ok();
            if verdicts_usable {
                eprintln!("tyu: cache hit for '{}'", module.path.display());
                return Ok(CompiledModule {
                    object_path: cached.object_path,
                    obl_path: obl_path.exists().then_some(obl_path),
                });
            }
            eprintln!(
                "tyu: cache miss for '{}' (verdicts echo missing/invalid)",
                module.path.display()
            );
        }
    }

    let scratch = out_dir.join(format!(".langc-scratch-{}", std::process::id()));
    fs::create_dir_all(&scratch).map_err(TyuError::Io)?;

    let mut cmd = Command::new(langc);
    cmd.arg("--emit=obj");
    cmd.arg(format!("--target={}", triple));
    cmd.arg(format!("--out-dir={}", scratch.display()));
    // P3: extract the obligation artifact alongside the object so the image
    // verification report can be composed. Binaries are unchanged (the
    // artifact is the only new output; FR-22 holds).
    cmd.arg("--write-obl");
    // P4: verdict-driven emission (default). The verdicts file is the
    // `.tyu-verify` cache slot for this module/fingerprint: created empty on a
    // miss, re-homed from langc's echo on success (§7.4). `--verify=off`
    // passes no --checks/--verdicts flags (today's argv — FR-22) but still
    // re-homes the echo, so the report's accounting is honest in both modes.
    let verdicts_slot = out_dir
        .join(".tyu-verify")
        .join(crate::proof::verdicts_slot_name(
            &module.name,
            inputs_fp,
            verify_env,
        ));
    if verify == crate::args::VerifyMode::On {
        // P7.3: pass-2 feeds the harvest's v2 verdicts (`proof`-class
        // discharges elide checks); the echo still re-homes over the slot.
        let path = match verdicts_override {
            Some(v) => v.to_path_buf(),
            None => ensure_verdicts_cache_file(&verdicts_slot)?,
        };
        cmd.arg("--checks=undischarged");
        cmd.arg(format!("--verdicts={}", path.display()));
    }
    // P6.2 pass-through: langc stays tool-agnostic (it consumes verdicts);
    // the flag records the requested proof tool in the verdicts echo so the
    // report's provenance names it.
    if let Some(tool) = verify_tool {
        cmd.arg(format!("--verify-tool={}", tool.as_str()));
    }
    // P7.3: `proven` — the langc-side trust gate (§Q12): interval-only
    // `checked` and unrecognized trust stay open so only exact-method checked
    // and proof certificates discharge.
    if verify_policy.is_proven() {
        cmd.arg("--verify-policy=proven");
    }
    // P7.3 (FR-5): under the two-pass proof pipeline, langc recomputes the
    // statement binding against the pass-1 (`--emit=obligations`) artifact —
    // the one the Gen statements and the proofs were certified against. The
    // live lowering's word IR is verdict-dependent (elided checks), so
    // without this every harvested certificate stale-evals (E6421) and
    // nothing ever elides.
    if let Some(bind) = bind_obl {
        cmd.arg(format!("--bind-obl={}", bind.display()));
    }
    // P13.1 (§Q13): the refinement context relativizes langc's FR-5
    // statement-binding recompute — a certificate bound under a refinement
    // (`tyu.refinements/1`) binds only when the consuming build carries it.
    if let Some(refs) = refinements_manifest {
        cmd.arg(format!("--refinements={}", refs.display()));
    }
    // Slice P7 (Q5/FR-11): the *image-level* decision — all modules or none
    // (per-image atomicity). A dedicated codegen input, never derived from
    // `--checks`.
    if elide_ds_guards {
        cmd.arg("--elide-ds-guards");
    }

    if let Some(sr) = sysroot {
        cmd.arg(format!("--sysroot={}", sr.display()));
    }

    if let Some(dir) = platform_dir {
        cmd.arg(format!("--platform={}", dir.display()));
    }
    // P12.1 (§6.7/§Q3): the artifact identity stamp — always forwarded so a
    // modeled pack and a bare build emit visibly different (triple, model)
    // identities.
    cmd.arg(format!("--model-semantics={}", model_semantics));

    for inc in include_dirs {
        cmd.arg("-I");
        cmd.arg(inc);
    }
    // Slice P6 (Q7): the callee modules' `.obl.json` artifacts live in the
    // out-dir (compiled in dependency order by earlier `compile_module`
    // calls). langc resolves a contract callee's predicate facts through the
    // same include-dir search `.def` uses — so `out_dir` is an implicit
    // include dir. Additive: the search falls through for every other
    // extension (the out-dir holds no `.def`/`.mod` in a normal build).
    cmd.arg("-I");
    cmd.arg(out_dir);

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
        let _ = fs::remove_dir_all(&scratch);
        return Err(TyuError::Build(format!(
            "langc failed on '{}' (exit code {:?})",
            module.path.display(),
            status.code(),
        )));
    }

    // Langc names the object after the module *declaration*, so distinct
    // source files that declare the same module (e.g. every tutorial's
    // `module Main;`) would share one `<ModuleName>.o` slot — a later cache
    // hit could then return an object clobbered by a different program
    // (BUG-002).  Re-home the produced object under a source-keyed name
    // (inputs_fp folds in the source content hash), so each distinct source
    // owns an artifact nothing else can overwrite.
    let obj_path = rehome_object(&module.name, inputs_fp, &scratch, out_dir)?;

    // The obligation artifact `<Module>.obl.json` rides beside the object
    // under the same fingerprint discipline (P3) — the composition step reads
    // it for the verify-report.
    let obl_path = rehome_obl(&module.name, inputs_fp, &scratch, out_dir)?;

    // P4: re-home langc's verdicts echo `<Module>.verdicts.inTree.json` over
    // the `.tyu-verify` cache slot (atomic rename — §7.4's temp+rehome
    // discipline). langc writes the echo whenever it extracted obligations
    // (`--write-obl`, always on in tyu), so the cache slot ends up holding the
    // resolved verdicts after every build; a later identical build reads it
    // back as its `--verdicts` input (Q11/FR-17).
    {
        let echo = scratch.join(format!("{}.verdicts.inTree.json", module.name));
        if !echo.exists() {
            let _ = fs::remove_dir_all(&scratch);
            return Err(TyuError::Build(format!(
                "langc produced no '{}.verdicts.inTree.json' in '{}'",
                module.name,
                scratch.display(),
            )));
        }
        if let Some(dir) = verdicts_slot.parent() {
            fs::create_dir_all(dir).map_err(TyuError::Io)?;
        }
        fs::rename(&echo, &verdicts_slot).map_err(|e| {
            TyuError::Build(format!(
                "re-homing verdicts echo '{}' -> '{}': {}",
                echo.display(),
                verdicts_slot.display(),
                e,
            ))
        })?;
    }

    // The scratch directory is per-process and per-compilation; drop it now.
    let _ = fs::remove_dir_all(&scratch);

    cache.insert(
        compiler_fp,
        inputs_fp,
        abi_hash,
        features,
        triple,
        &obj_path,
    )?;

    Ok(CompiledModule {
        object_path: obj_path,
        obl_path: Some(obl_path),
    })
}

/// The object + obligation-artifact pair produced for one module.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub object_path: PathBuf,
    /// The re-homed `<Module>-<inputs_fp>.obl.json`, when present.
    pub obl_path: Option<PathBuf>,
}

/// The shared `langc --emit=obligations` pass-1 command (P7 / P7.3): the
/// common flag surface both extraction passes build — model-semantics
/// identity, sysroot, platform dir, include dirs, features, lib flag, input.
/// The two callers diverge only on the out-dir (scratch vs `.tyu-oblig`),
/// the elision pass's extra include dirs (P6 Q7 cross-module facts), and
/// the explicit `--target` the proof-pipeline pass passes. One builder so
/// the flag surface cannot drift between the two passes.
#[allow(clippy::too_many_arguments)] // the builder carries the full pass-1 identity surface
fn langc_obligations_command(
    langc: &Path,
    out_dir_arg: &Path,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    platform_dir: Option<&Path>,
    model_semantics: &str,
    feature_set: FeatureSet,
    is_lib: bool,
    path: &Path,
) -> Command {
    let mut cmd = Command::new(langc);
    cmd.arg("--emit=obligations");
    cmd.arg(format!("--out-dir={}", out_dir_arg.display()));
    if let Some(sr) = sysroot {
        cmd.arg(format!("--sysroot={}", sr.display()));
    }
    if let Some(dir) = platform_dir {
        cmd.arg(format!("--platform={}", dir.display()));
    }
    cmd.arg(format!("--model-semantics={}", model_semantics));
    for inc in include_dirs {
        cmd.arg("-I");
        cmd.arg(inc);
    }
    let mut flag_buf = [""; 8];
    let n = feature_set.write_flags(&mut flag_buf);
    if n > 0 {
        cmd.arg(format!("--features={}", flag_buf[..n].join(",")));
    }
    if is_lib {
        cmd.arg("--lib");
    }
    cmd.arg(path);
    cmd
}

/// Slice P7 pass 1: extract every module's obligation facts
/// (`langc --emit=obligations`, no codegen) into a per-process scratch dir,
/// read each artifact back (`read_obl` — fail-closed, E6400/E6401), delete
/// the scratch, and return `(module name → OblSet or None)`. `None` = the
/// module produced no artifact (mixed-mode/compiled without extraction) —
/// which forces the image verdict open (§7.3: absence can only cause more
/// checking, never less). The scratch dir is an implicit include dir so
/// callee `.obl.json` facts resolve for cross-module contracts (P6/Q7).
#[allow(clippy::too_many_arguments)] // the elision pass carries the per-build identity context
fn extract_obligations_for_elision(
    langc: &Path,
    modules: &[ModuleNode],
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    out_dir: &Path,
    platform_dir: Option<&Path>,
    model_semantics: &str,
    feature_set: FeatureSet,
) -> Result<Vec<(String, Option<verifier::model::OblSet>)>, TyuError> {
    let scratch = out_dir.join(format!(".tyu-elide-pass1-{}", std::process::id()));
    fs::create_dir_all(&scratch).map_err(TyuError::Io)?;
    let mut sets: Vec<(String, Option<verifier::model::OblSet>)> =
        Vec::with_capacity(modules.len());
    let fail = |e: TyuError| -> TyuError {
        let _ = fs::remove_dir_all(&scratch);
        e
    };
    for module in modules {
        let mut cmd = langc_obligations_command(
            langc,
            &scratch,
            include_dirs,
            sysroot,
            platform_dir,
            model_semantics,
            feature_set,
            module.is_lib,
            &module.path,
        );
        // The scratch holds this pass's own artifacts (callee obl facts for
        // cross-module contract transclusion); out_dir holds the re-homed
        // artifacts of *previous* builds (P6 Q7 include-dir search).
        cmd.arg("-I").arg(&scratch);
        cmd.arg("-I").arg(out_dir);
        let status = cmd
            .status()
            .map_err(|e| fail(TyuError::Build(format!("running langc: {e}"))))?;
        if !status.success() {
            return Err(fail(TyuError::Build(format!(
                "pass-1 (obligations extraction) failed on '{}' (exit code {:?})",
                module.path.display(),
                status.code(),
            ))));
        }
        let obl_path = scratch.join(format!("{}.obl.json", module.name));
        let set = if obl_path.exists() {
            let bytes = fs::read(&obl_path).map_err(|e| {
                fail(TyuError::Build(format!(
                    "reading pass-1 artifact '{}': {e}",
                    obl_path.display()
                )))
            })?;
            match verifier::codec::read_obl(&bytes) {
                Ok(s) => Some(s),
                Err(e) => {
                    return Err(fail(TyuError::Build(format!(
                        "pass-1 artifact '{}' invalid (E{}): {e:?}",
                        obl_path.display(),
                        e.code()
                    ))))
                }
            }
        } else {
            None
        };
        sets.push((module.name.clone(), set));
    }
    let _ = fs::remove_dir_all(&scratch);
    Ok(sets)
}

/// P7.3 pass-1 (FR-3): extract every module's obligation artifact
/// (`langc --emit=obligations`, no codegen) into `<out_dir>/.tyu-oblig/`
/// (`<Module>.obl.json`), read each back (fail-closed E6400/E6401), and
/// return `(module, artifact_path)`. The artifacts feed the proof pipeline
/// (gen → digest → harvest) whose verdicts drive pass-2 codegen.
// 8 positional parameters mirror langc's pass-1 invocation surface; a struct
// would just relocate the same fields.
#[allow(clippy::too_many_arguments)]
fn extract_module_artifacts(
    langc: &Path,
    modules: &[ModuleNode],
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    out_dir: &Path,
    platform_dir: Option<&Path>,
    model_semantics: &str,
    feature_set: FeatureSet,
    triple: &str,
) -> Result<Vec<(String, PathBuf)>, TyuError> {
    let dir = out_dir.join(".tyu-oblig");
    fs::create_dir_all(&dir).map_err(TyuError::Io)?;
    let mut out = Vec::with_capacity(modules.len());
    for module in modules {
        let mut cmd = langc_obligations_command(
            langc,
            &dir,
            include_dirs,
            sysroot,
            platform_dir,
            model_semantics,
            feature_set,
            module.is_lib,
            &module.path,
        );
        // This pass's artifacts feed the proof pipeline; the explicit
        // target makes the artifact's (triple, model) identity explicit.
        cmd.arg(format!("--target={}", triple));
        let status = cmd.status().map_err(|e| {
            TyuError::Build(format!(
                "pass-1 extraction for '{}': {e}",
                module.path.display()
            ))
        })?;
        if !status.success() {
            return Err(TyuError::Build(format!(
                "pass-1 (obligations) failed on '{}'",
                module.path.display()
            )));
        }
        let obl_path = dir.join(format!("{}.obl.json", module.name));
        if obl_path.exists() {
            out.push((module.name.clone(), obl_path));
        }
    }
    Ok(out)
}

/// Write the image-verdicts record (`.tyu-verify/image-verdicts.json`) —
/// the durable, schema-validated evidence of the two-pass guard-elision
/// decision. Written via temp file + rename (the scratch+rehome discipline,
/// so concurrent builds sharing the out-dir cannot observe a torn file) and
/// immediately read back + validated: a malformed record is E6415 (fail-loud
/// — a corrupt decision record can never silently report a different guard
/// state than the object actually has, FR-16).
fn write_image_verdicts(
    out_dir: &Path,
    record: &verifier::codec::ImageVerdicts,
) -> Result<(), TyuError> {
    let dir = out_dir.join(".tyu-verify");
    fs::create_dir_all(&dir).map_err(TyuError::Io)?;
    let bytes = verifier::codec::encode_image_verdicts(record).map_err(|e| {
        TyuError::Build(format!(
            "image-verdicts encode failed (E{}): {e:?}",
            e.code()
        ))
    })?;
    let tmp = dir.join(format!(".image-verdicts.tmp-{}", std::process::id()));
    fs::write(&tmp, &bytes)
        .map_err(|e| TyuError::Build(format!("writing '{}': {e}", tmp.display())))?;
    let final_path = dir.join("image-verdicts.json");
    fs::rename(&tmp, &final_path)
        .map_err(|e| TyuError::Build(format!("re-homing image verdicts: {e}")))?;
    // Round-trip validation — E6415 on a corrupt/mismatched record.
    let read_back = fs::read(&final_path)
        .map_err(|e| TyuError::Build(format!("re-reading image verdicts: {e}")))?;
    verifier::codec::read_image_verdicts(&read_back).map_err(|e| {
        TyuError::Build(format!(
            "image-verdicts record invalid (E{}): {e:?}",
            e.code()
        ))
    })?;
    Ok(())
}

/// Move `langc`'s output object `<scratch>/<ModuleName>.o` to a
/// source-keyed `<out_dir>/<ModuleName>-<inputs_fp:016x>.o`.
///
/// `inputs_fp` is content-addressed (own source + deps + triple), so two
/// programs with identical content share an object (which is identical too),
/// while distinct programs never collide.  The exact `<ModuleName>.o` slot is
/// deterministic per `langc` (`crates/langc/src/driver.rs`), so the old
/// first-`.o` fallback is dropped: if langc did not write it, that is an
/// error, not a reason to guess.
fn rehome_object(
    module_name: &str,
    inputs_fp: u64,
    scratch: &Path,
    out_dir: &Path,
) -> Result<PathBuf, TyuError> {
    let langc_obj = scratch.join(format!("{}.o", module_name));
    if !langc_obj.exists() {
        return Err(TyuError::Build(format!(
            "langc produced no '{}.o' in '{}'",
            module_name,
            scratch.display(),
        )));
    }
    let unique_obj = out_dir.join(format!("{}-{:016x}.o", module_name, inputs_fp));
    fs::rename(&langc_obj, &unique_obj).map_err(|e| {
        TyuError::Build(format!(
            "re-homing '{}' -> '{}': {}",
            langc_obj.display(),
            unique_obj.display(),
            e,
        ))
    })?;
    Ok(unique_obj)
}

/// The source-keyed filename of a module's obligation artifact
/// (`<Module>-<inputs_fp:016x>.obl.json`) — the same fingerprint discipline as
/// the re-homed object, so cache hits and stale pruning treat both alike.
fn rehomed_obl_filename(module: &ModuleNode, inputs_fp: u64) -> String {
    format!("{}-{:016x}.obl.json", module.name, inputs_fp)
}

/// Move `langc`'s obligation artifact `<scratch>/<Module>.obl.json` to the
/// source-keyed `<out_dir>/<Module>-<inputs_fp>.obl.json`.
fn rehome_obl(
    module_name: &str,
    inputs_fp: u64,
    scratch: &Path,
    out_dir: &Path,
) -> Result<PathBuf, TyuError> {
    let langc_obl = scratch.join(format!("{module_name}.obl.json"));
    if !langc_obl.exists() {
        return Err(TyuError::Build(format!(
            "langc produced no '{module_name}.obl.json' in '{}'",
            scratch.display(),
        )));
    }
    let unique_obl = out_dir.join(format!("{module_name}-{inputs_fp:016x}.obl.json"));
    fs::rename(&langc_obl, &unique_obl).map_err(|e| {
        TyuError::Build(format!(
            "re-homing '{}' -> '{}': {}",
            langc_obl.display(),
            unique_obl.display(),
            e,
        ))
    })?;
    Ok(unique_obl)
}

/// Ensure the `.tyu-verify` verdicts cache slot exists for `(module, fp)`
/// (static-verification.md §7.4). On a miss, write an *empty but valid*
/// verdicts file ([`verifier::verdict::encode_verdicts`] with no records):
/// langc resolves the in-tree (descriptor-dischargeable) sites itself and
/// echoes the resolved set back over this slot. A *corrupt* existing entry is
/// treated as a miss (self-healing: re-derived from langc, never passed to it
/// as-is — a hostile/garbled cache can only cause more checking or a clean
/// rebuild, R10). Written via temp file + rename (the scratch+rehome
/// discipline), so concurrent builds sharing the out-dir cannot observe a
/// torn file.
fn ensure_verdicts_cache_file(slot: &Path) -> Result<PathBuf, TyuError> {
    if slot.exists() {
        let valid = fs::read(slot)
            .ok()
            .and_then(|b| verifier::verdict::read_verdicts(&b).ok())
            .is_some();
        if valid {
            return Ok(slot.to_path_buf());
        }
        // Corrupt/stale entry — drop it and re-derive (a miss).
        let _ = fs::remove_file(slot);
    }
    let dir = slot.parent().ok_or_else(|| {
        TyuError::Build(format!(
            "verdicts slot '{}' has no parent dir",
            slot.display()
        ))
    })?;
    fs::create_dir_all(dir).map_err(TyuError::Io)?;
    let bytes = verifier::verdict::encode_verdicts(
        "tyu",
        env!("CARGO_PKG_VERSION"),
        None,
        "",
        "",
        &[],
        0,
        &verifier::verdict::EmittedChecksData::default(),
    )
    .map_err(|e| TyuError::Build(format!("encoding empty verdicts cache: {e:?}")))?;
    let tmp = dir.join(format!(
        ".{}.tmp{}",
        slot.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("verdicts"),
        std::process::id(),
    ));
    fs::write(&tmp, &bytes)
        .map_err(|e| TyuError::Build(format!("writing '{}': {}", tmp.display(), e)))?;
    fs::rename(&tmp, slot).map_err(|e| {
        TyuError::Build(format!(
            "re-homing '{}' -> '{}': {}",
            tmp.display(),
            slot.display(),
            e,
        ))
    })?;
    Ok(slot.to_path_buf())
}

/// Remove stale artifacts from a custom `--out-dir`:
///
/// - cache records whose object file no longer exists;
/// - re-homed `<Module>-<inputs_fp>.o` files whose fingerprint is stale for a
///   module rebuilt in this build (a source edit changes the inputs fp, so the
///   previous `<Module>-<old_fp>.o` and its cache record are both reclaimed);
/// - re-homed objects that no cache record references (left by interrupted
///   builds);
/// - scratch directories left by interrupted `langc` invocations.
///
/// Only objects that predate this build are reclaimed, so a concurrent build
/// sharing this out-dir can never have its in-flight artifact deleted
/// (`build_started` is captured before any compilation begins).  Objects for
/// modules *not* part of this build are left alone (they may be legitimately
/// cached for a later program sharing this out-dir).
fn prune_out_dir(
    out_dir: &Path,
    cache: &mut BuildCache,
    current_fps: &BTreeMap<String, Vec<u64>>,
    build_started: std::time::SystemTime,
    verify_env: &crate::proof::VerifyEnvKey,
) -> Result<(), TyuError> {
    let predates = |path: &Path| -> bool {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|t| t < build_started)
            .unwrap_or(false)
    };

    cache.prune_missing();

    let live: std::collections::HashSet<PathBuf> = cache.referenced_objects().cloned().collect();
    let entries = fs::read_dir(out_dir).map_err(TyuError::Io)?;
    let mut removed: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(TyuError::Io)?.path();
        if path.is_dir() {
            if is_scratch_dir(&path) && predates(&path) {
                let _ = fs::remove_dir_all(&path);
            }
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_obl = name.ends_with(".obl.json");
        let rehomed = parse_rehomed_object_name(name).or_else(|| parse_rehomed_obl_name(name));
        let Some((module, fp)) = rehomed else {
            continue;
        };
        let current = current_fps
            .get(&module)
            .map(|fps| fps.contains(&fp))
            .unwrap_or(false);
        if current || !predates(&path) {
            continue;
        }
        // Stale fingerprint of a module rebuilt here, or an orphan with no
        // cache record — either way the artifact is dead. Obligation artifacts
        // (`<Mod>-<fp>.obl.json`) HAVE no cache records (they are re-derived
        // from source on every compile), so a shared out-dir must keep another
        // program's obl for its own next cache-hit build: they are pruned only
        // when a module in THIS build superseded the fingerprint.
        let referenced = live.contains(&path);
        let stale_in_build = current_fps.contains_key(&module);
        let dead = if is_obl {
            stale_in_build
        } else {
            stale_in_build || !referenced
        };
        if dead {
            removed.push(path.clone());
            let _ = fs::remove_file(&path);
        }
    }
    if !removed.is_empty() {
        cache.remove_objects(&removed);
    }

    // P4: prune stale verdicts-cache entries in `<out_dir>/.tyu-verify/` with
    // the same discipline as the re-homed obl artifacts: an entry whose
    // (module, fp) fingerprint was superseded by THIS build, which predates
    // this build and names a module that no longer exists in it, or which
    // names a (module, fp) that IS current but under a *different* §7.2
    // environment slug (a proof-file/toolchain/model change rotated the key —
    // the old slot is superseded) is reclaimed.
    let verify_dir = out_dir.join(".tyu-verify");
    // This build's live slot names: keep those verbatim.
    let expected: std::collections::HashSet<String> = current_fps
        .iter()
        .flat_map(|(m, fps)| {
            fps.iter()
                .map(move |&fp| crate::proof::verdicts_slot_name(m, fp, verify_env))
        })
        .collect();
    let entries = fs::read_dir(&verify_dir);
    if let Ok(entries) = entries {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(String::from) else {
                continue;
            };
            if expected.contains(&name) {
                continue;
            }
            // Loose parse (any env-slug) to apply the per-module rules; a slot
            // we cannot identify is left alone (conservative).
            let Some((module, fp)) = crate::proof::parse_any_verdicts_name(&name) else {
                continue;
            };
            let current = current_fps
                .get(&module)
                .map(|fps| fps.contains(&fp))
                .unwrap_or(false);
            let stale_in_build = current_fps.contains_key(&module);
            let dead = if current {
                // Current (module, fp) but not under THIS env's slug: an old
                // key, superseded by the slot `expected` names.
                true
            } else {
                // Never touch a module not part of this build if its artifact
                // is newer than the build start (concurrent-build safety).
                stale_in_build || predates(&path)
            };
            if dead {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

/// Is `path` a per-process langc scratch directory we can reclaim?
fn is_scratch_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with(".langc-scratch-"))
        .unwrap_or(false)
}

/// Split a re-homed `<Module>-<inputs_fp:016x>.o` name into `(module, fp)`.
fn parse_rehomed_object_name(name: &str) -> Option<(String, u64)> {
    parse_rehomed_fp_name(name, ".o")
}

/// Split a re-homed `<Module>-<inputs_fp:016x>.obl.json` name into
/// `(module, fp)`.
fn parse_rehomed_obl_name(name: &str) -> Option<(String, u64)> {
    parse_rehomed_fp_name(name, ".obl.json")
}

fn parse_rehomed_fp_name(name: &str, suffix: &str) -> Option<(String, u64)> {
    let stem = name.strip_suffix(suffix)?;
    let (module, fp) = stem.rsplit_once('-')?;
    if fp.len() != 16 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let fp_val = u64::from_str_radix(fp, 16).ok()?;
    Some((module.to_string(), fp_val))
}

/// Assemble the runtime unit `stem` (e.g. `"runtime"`, `"concurrency"`) for
/// the given target, producing `<out_dir>/<stem>.o`.
///
/// `rt_dir` is the per-target runtime directory: the selected platform pack's
/// metal `path` (relative to the pack root) when a platform is selected,
/// otherwise `<workspace_root>/runtime/<triple>` (see
/// `assemble_runtime_with_mode`).
fn assemble_unit(
    target: Target,
    rt_dir: &Path,
    stem: &str,
    out_dir: &Path,
) -> Result<PathBuf, TyuError> {
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
                return Err(TyuError::Build(format!(
                    "fasm failed to assemble '{}'",
                    label
                )));
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
                )));
            }
        }
        AssemblerKind::GasRiscV => {
            let asm_name = asm_path
                .file_name()
                .ok_or_else(|| {
                    TyuError::Build(format!("invalid asm path '{}'", asm_path.display()))
                })?
                .to_owned();
            let asm = toolchain::resolve_tool_candidates(toolchain::RISCV_AS_CANDIDATES)
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
                )));
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
    assemble_runtime_with_mode(
        target,
        out_dir,
        feature_set,
        platform_selection,
        BuildMode::Static,
    )
}

fn assemble_runtime_with_mode(
    target: Target,
    out_dir: &Path,
    feature_set: FeatureSet,
    platform_selection: Option<&platform::ResolvedPlatformSelection>,
    mode: BuildMode,
) -> Result<Vec<PathBuf>, TyuError> {
    let triple = std::str::from_utf8(target.triple()).map_err(|_| TyuError::NonUtf8Triple)?;
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
    if let Some(entry_obj) = assemble_entry_unit(target, &rt_dir, out_dir, mode)? {
        objs.push(entry_obj);
    }

    // P6: the MMIO aperture-base table the on-device loader re-derives against.
    objs.push(assemble_mmio_apertures_object(
        target,
        out_dir,
        platform_selection,
    )?);

    // Feature-specific runtime units: assemble each stem that maps to
    // an enabled feature.  `assemble_unit` returns an error for missing
    // files (the unit must exist for at least the targets that enable it).
    for f in feature_set.iter() {
        if let Some(stem) = f.runtime_unit() {
            if mode == BuildMode::Dynamic && f == Feature::ModuleLoading {
                continue;
            }
            let unit_path = rt_dir.join(format!("{}.asm", stem));
            if unit_path.exists() {
                objs.push(assemble_unit(target, &rt_dir, stem, out_dir)?);
            }
        }
    }

    Ok(objs)
}

fn assemble_entry_unit(
    target: Target,
    rt_dir: &Path,
    out_dir: &Path,
    mode: BuildMode,
) -> Result<Option<PathBuf>, TyuError> {
    let stem = match mode {
        BuildMode::Static => "static_entry",
        BuildMode::Dynamic => "dynamic_entry",
    };
    let path = rt_dir.join(format!("{}.asm", stem));
    if path.exists() {
        assemble_unit(target, rt_dir, stem, out_dir).map(Some)
    } else if mode == BuildMode::Static {
        Ok(None)
    } else {
        Err(TyuError::Build(format!(
            "dynamic runtime entry unit '{}' is missing",
            path.display()
        )))
    }
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

/// Assemble the firmware's board identity + MMIO aperture table (P6, design doc
/// §5.8). The compiled descriptor's projection (`encode_board_table`) is
/// embedded as `__lang_platform_desc`; the on-device loader decodes it with
/// the shared codec and enforces `platform_hash` (E5220) + aperture binding.
/// The per-aperture `__lang_aperture_{id}_base` symbols remain so statically-
/// linked modules' reloc sites resolve at link time. Emitted unconditionally
/// (an empty table when no platform selection) so the loader's symbols always
/// resolve.
fn assemble_mmio_apertures_object(
    target: Target,
    out_dir: &Path,
    selection: Option<&platform::ResolvedPlatformSelection>,
) -> Result<PathBuf, TyuError> {
    let compiled = match selection {
        Some(sel) => Some(desc::ensure_compiled_descriptor(
            &sel.pack.manifest_path,
            sel.pack.pack_root(),
        )?),
        None => None,
    };
    let asm_path = out_dir.join("mmio_apertures_generated.asm");
    let obj_path = out_dir.join("mmio_apertures_generated.o");
    let asm = render_mmio_apertures_asm(target, compiled.as_ref());
    fs::write(&asm_path, asm).map_err(TyuError::Io)?;
    assemble_asm_file(
        target,
        &asm_path,
        &obj_path,
        Some(out_dir),
        "mmio_apertures",
    )?;
    Ok(obj_path)
}

/// Encode the board aperture table blob (P6, design doc §5.8) from a compiled
/// descriptor: `platform_hash` + per-aperture (`name_hash`, `base`, `size`,
/// capability) via the shared `lmod::board_table` codec. Deterministic.
fn encode_board_table_blob(cd: &codegen_core::compiled_desc::CompiledDescriptor) -> Vec<u8> {
    let apertures: Vec<lmod::board_table::BoardAperture> = cd
        .apertures()
        .iter()
        .map(|w| lmod::board_table::BoardAperture {
            name_hash: lmod::hash::fnv1a_u64(w.name.as_bytes()),
            base: w.base.unwrap_or(0) as u32,
            size: w.size,
            capability: codegen_core::compiled_desc::aperture_capability(cd, w.id),
        })
        .collect();
    let mut buf = [0u8; lmod::board_table::BOARD_TABLE_ENCODED_SIZE];
    let n = lmod::board_table::encode_into(&mut buf, cd.platform_hash, &apertures)
        .expect("board table encode into fixed buffer");
    buf[..n].to_vec()
}

fn render_mmio_apertures_asm(
    target: Target,
    compiled: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
) -> String {
    let bytes = compiled.map(encode_board_table_blob).unwrap_or_default();
    let bases: Vec<u32> = compiled
        .map(|cd| {
            cd.apertures()
                .iter()
                .filter_map(|w| w.base.map(|b| b as u32))
                .collect()
        })
        .unwrap_or_default();
    let count = bases.len().min(8);
    match target.spec().assembler {
        AssemblerKind::Fasm => {
            // fasm: `db` data section carrying the board table blob.
            let mut out = String::new();
            out.push_str("format ELF64\n\nsection '.rodata' writeable\n    align 4\n    public __lang_platform_desc_start\n__lang_platform_desc_start:\n");
            if !bytes.is_empty() {
                out.push_str("    db ");
                for (i, b) in bytes.iter().enumerate() {
                    if i != 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("{:#x}", b));
                }
                out.push('\n');
            }
            out.push_str("    public __lang_platform_desc_end\n__lang_platform_desc_end:\n");
            out
        }
        _ => {
            // gas: `.byte` blob + per-aperture absolute symbols for static links.
            let mut out = String::new();
            out.push_str(".section .rodata, \"a\", %progbits\n.balign 4\n.global __lang_platform_desc_start\n__lang_platform_desc_start:\n");
            if !bytes.is_empty() {
                out.push_str("  .byte ");
                for (i, b) in bytes.iter().enumerate() {
                    if i != 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("{}", b));
                }
                out.push('\n');
            }
            out.push_str(".global __lang_platform_desc_end\n__lang_platform_desc_end:\n");
            out.push_str(".section .note.GNU-stack, \"\", %progbits\n");
            for (id, base) in bases.iter().take(count).enumerate() {
                out.push_str(&format!(
                    ".global __lang_aperture_{}_base\n.set __lang_aperture_{}_base, {:#x}\n",
                    id, id, base
                ));
            }
            out
        }
    }
}

fn assemble_modpack_object(
    target: Target,
    out_dir: &Path,
    lmod_paths: &[PathBuf],
) -> Result<PathBuf, TyuError> {
    if lmod_paths.is_empty() {
        return Err(TyuError::Build(
            "dynamic image modpack requires at least one module".into(),
        ));
    }
    let mut lmods: Vec<(u32, String)> = Vec::new();
    for lmod_path in lmod_paths {
        let len = fs::metadata(lmod_path).map_err(TyuError::Io)?.len();
        if len > u32::MAX as u64 {
            return Err(TyuError::Build(format!(
                "module '{}' is too large for v1 modpack",
                lmod_path.display()
            )));
        }
        let lmod_abs = if lmod_path.is_absolute() {
            lmod_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(TyuError::Io)?
                .join(lmod_path)
        };
        let lmod_str = lmod_abs
            .to_str()
            .ok_or_else(|| TyuError::Build(format!("non-UTF-8 path '{}'", lmod_abs.display())))?;
        lmods.push((len as u32, lmod_str.to_string()));
    }

    let asm_path = out_dir.join("modpack_generated.asm");
    let obj_path = out_dir.join("modpack_generated.o");
    let asm = render_modpack_asm(target, &lmods)?;
    fs::write(&asm_path, asm).map_err(TyuError::Io)?;
    assemble_asm_file(target, &asm_path, &obj_path, Some(out_dir), "modpack")?;
    Ok(obj_path)
}

/// One `u32` length + blob per module, back-to-back — the v1 modpack layout
/// `loader_core::modpack` documents and scans (deps-first embed order). No
/// padding between blobs (the scanner reads lengths contiguously); the
/// leading/trailing `.balign 4` keep the section 4-aligned exactly as the
/// single-module rendering always did (N=1 output is byte-identical).
fn render_modpack_asm(target: Target, lmods: &[(u32, String)]) -> Result<String, TyuError> {
    let mut asm = String::new();
    match target.spec().assembler {
        AssemblerKind::Fasm => {
            asm.push_str("format ELF64\n\nsection '.modpack' writeable\n");
            for (len, path) in lmods {
                if path.contains('\'') {
                    return Err(TyuError::Build(format!(
                        "FASM modpack path contains an unsupported quote: '{path}'"
                    )));
                }
                asm.push_str(&format!("    dd {len}\n    file '{path}'\n"));
            }
        }
        AssemblerKind::GasArm | AssemblerKind::GasRiscV => {
            if matches!(target.spec().assembler, AssemblerKind::GasArm) {
                asm.push_str(".syntax unified\n.thumb\n\n.section .modpack, \"a\", %progbits\n");
            } else {
                asm.push_str(".section .modpack, \"a\", @progbits\n");
            }
            asm.push_str(".balign 4\n");
            for (len, path) in lmods {
                let quoted = gas_string_literal(path, std::path::Path::new(path))?;
                asm.push_str(&format!(".word {len}\n.incbin \"{quoted}\"\n"));
            }
            asm.push_str(".balign 4\n");
            if matches!(target.spec().assembler, AssemblerKind::GasArm) {
                asm.push_str(".section .note.GNU-stack, \"\", %progbits\n");
            } else {
                asm.push_str(".section .note.GNU-stack, \"\", @progbits\n");
            }
        }
    }
    Ok(asm)
}

fn assemble_keys_object(
    target: Target,
    out_dir: &Path,
    sign_key: Option<&[u8; 32]>,
    kek: Option<&[u8; 32]>,
    metal_encrypt_mode: EncryptMode,
) -> Result<PathBuf, TyuError> {
    let asm_path = out_dir.join("keys_generated.asm");
    let obj_path = out_dir.join("keys_generated.o");
    let asm = render_keys_asm(target, sign_key, kek, metal_encrypt_mode);
    fs::write(&asm_path, asm).map_err(TyuError::Io)?;
    assemble_asm_file(target, &asm_path, &obj_path, Some(out_dir), "keys")?;
    Ok(obj_path)
}

fn render_keys_asm(
    target: Target,
    sign_key: Option<&[u8; 32]>,
    kek: Option<&[u8; 32]>,
    metal_encrypt_mode: EncryptMode,
) -> String {
    let mut mask = 0u8;
    let mut key_data = Vec::new();
    if let Some(sign_key) = sign_key {
        mask |= 1;
        key_data.extend_from_slice(sign_key);
    }
    if let Some(kek) = kek {
        mask |= match metal_encrypt_mode {
            EncryptMode::Fleet | EncryptMode::None => 2,
            EncryptMode::Device => 4,
        };
        key_data.extend_from_slice(kek);
    }
    let key_bytes = key_data
        .iter()
        .map(|byte| format!("0x{byte:02x}"))
        .collect::<Vec<_>>()
        .join(", ");
    let key_line_fasm = if key_bytes.is_empty() {
        String::new()
    } else {
        format!("    db {key_bytes}\n")
    };
    let key_line_gas = if key_bytes.is_empty() {
        String::new()
    } else {
        format!(".byte {key_bytes}\n")
    };
    match target.spec().assembler {
        AssemblerKind::Fasm => format!(
            "format ELF64\n\nsection '.lang.keys' writeable\n    db {mask}\n    db 0, 0, 0, 0, 0, 0, 0\n{key_line_fasm}"
        ),
        AssemblerKind::GasArm => format!(
            ".syntax unified\n.thumb\n\n.section .lang.keys, \"a\", %progbits\n.balign 8\n.byte {mask}\n.byte 0, 0, 0, 0, 0, 0, 0\n{key_line_gas}.balign 8\n.section .note.GNU-stack, \"\", %progbits\n"
        ),
        AssemblerKind::GasRiscV => format!(
            ".section .lang.keys, \"a\", @progbits\n.balign 8\n.byte {mask}\n.byte 0, 0, 0, 0, 0, 0, 0\n{key_line_gas}.balign 8\n.section .note.GNU-stack, \"\", @progbits\n"
        ),
    }
}

fn gas_string_literal(path: &str, display_path: &Path) -> Result<String, TyuError> {
    if path.contains('"') || path.contains('\\') || path.bytes().any(|b| b < 0x20) {
        return Err(TyuError::Build(format!(
            "GAS modpack path contains an unsupported character: '{}'",
            display_path.display()
        )));
    }
    Ok(path.to_string())
}

fn build_device_loader_staticlib(
    target: Target,
    signing: bool,
    encryption: bool,
) -> Result<PathBuf, TyuError> {
    let triple = device_loader_rust_target(target)?;
    let profile = device_loader_profile(target)?;
    // Per-feature-set target dir: concurrent builds (unsigned + signed test
    // matrices, parallel fleet tooling) must not share one archive path —
    // two cargo invocations with different feature sets otherwise overwrite
    // each other's `libdevice_loader_archive.a`, and a signed firmware can
    // link the unsigned loader (no key → trust Zero → E5202 at load).
    let feature_tag = match (signing, encryption) {
        (true, true) => "signing_encryption",
        (true, false) => "signing",
        (false, true) => "encryption",
        (false, false) => "plain",
    };
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| platform::workspace_root().join("target"))
        .join("device-loader")
        .join(feature_tag);
    let manifest = platform::workspace_root()
        .join("crates")
        .join("device-loader-archive")
        .join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.env("CARGO_TARGET_DIR", &target_dir).args([
        "build",
        "--locked",
        "--offline",
        "--manifest-path",
        manifest.to_string_lossy().as_ref(),
        "--target",
        triple,
    ]);
    if target == Target::RiscV32UnknownNone {
        cmd.args(["-Z", "build-std=core,alloc"]);
    }
    if signing {
        cmd.args(["--features", "signing"]);
    }
    if encryption {
        cmd.args(["--features", "encryption"]);
    }
    if profile == "release" {
        cmd.arg("--release");
    }
    let status = cmd
        .status()
        .map_err(|e| TyuError::Build(format!("spawning cargo for device loader: {}", e)))?;
    if !status.success() {
        return Err(TyuError::Build(format!(
            "building loader-core staticlib for {} failed",
            triple
        )));
    }

    Ok(target_dir
        .join(triple)
        .join(profile)
        .join("libdevice_loader_archive.a"))
}

fn device_loader_rust_target(target: Target) -> Result<&'static str, TyuError> {
    match target {
        Target::X86_64UnknownNone => Ok("x86_64-unknown-none"),
        Target::ArmV7MUnknownNone => Ok("thumbv7m-none-eabi"),
        Target::RiscV32UnknownNone => Ok("riscv32im-unknown-none-elf"),
        Target::X86_64UnknownLinuxGnu => Err(TyuError::Build(
            "--mode=dynamic is only supported for bare-metal QEMU targets".into(),
        )),
    }
}

fn device_loader_profile(target: Target) -> Result<&'static str, TyuError> {
    match target {
        Target::X86_64UnknownNone | Target::ArmV7MUnknownNone | Target::RiscV32UnknownNone => {
            Ok("release")
        }
        Target::X86_64UnknownLinuxGnu => Err(TyuError::Build(
            "--mode=dynamic is only supported for bare-metal QEMU targets".into(),
        )),
    }
}

pub fn assemble_runtime_for_context(
    ctx: &BuildContext,
    feature_set: FeatureSet,
) -> Result<Vec<PathBuf>, TyuError> {
    assemble_runtime_with_mode(
        ctx.target,
        &ctx.out_dir,
        feature_set,
        ctx.platform_selection(),
        BuildMode::Static,
    )
}

fn assemble_runtime_for_context_mode(
    ctx: &BuildContext,
    feature_set: FeatureSet,
    mode: BuildMode,
) -> Result<Vec<PathBuf>, TyuError> {
    assemble_runtime_with_mode(
        ctx.target,
        &ctx.out_dir,
        feature_set,
        ctx.platform_selection(),
        mode,
    )
}

/// Link object files into the final ELF image.
pub fn link_image(
    target: Target,
    objs: &[PathBuf],
    out_dir: &Path,
    platform_selection: Option<&platform::ResolvedPlatformSelection>,
) -> Result<PathBuf, TyuError> {
    // Provenance gate (flat word-symbol namespace): reject duplicate exports
    // and undeclared word references with module-attributed errors before the
    // linker sees them. See `provenance` for the accepted residual.
    crate::provenance::verify_link_provenance(objs)?;

    let spec = target.spec();
    let triple = std::str::from_utf8(target.triple()).map_err(|_| TyuError::NonUtf8Triple)?;

    let linker_name = std::str::from_utf8(spec.linker)
        .map_err(|_| TyuError::Build("non-UTF-8 linker name".into()))?;
    let linker = toolchain::resolve_tool(linker_name)?;

    // `None` means "no explicit linker script" — let the linker use its
    // default. Host-native targets (`qemu: None`, e.g. x86_64-unknown-linux-gnu)
    // link as ordinary Linux ELF executables via ld's built-in script; only
    // bare-metal targets need a custom `link.ld` to place sections in flash/RAM.
    let linker_script: Option<PathBuf> = if let Some(selection) = platform_selection {
        // `boot = "image_def"` only adds the PICOBIN block object to the
        // link (see `assemble_image_def`); layout stays owned by the
        // pack's `metal/<isa>/link.ld`, which must place `.image_def`
        // within the bootrom's first-4 kB scan window (DS2 5.9.5).
        if selection.metal().linker.is_empty() {
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
        return Err(TyuError::Build(format!(
            "{} failed to link image",
            linker_name
        )));
    }

    Ok(out_path)
}

pub fn link_image_for_context(ctx: &BuildContext, objs: &[PathBuf]) -> Result<PathBuf, TyuError> {
    link_image(ctx.target, objs, &ctx.out_dir, ctx.platform_selection())
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::args::{BuildMode, DeployArgs};
use crate::build;
use crate::error::TyuError;
use crate::keys::{KeyMaterial, KeyRef};
use crate::platform::{self, PlatformPack, ResolvedPlatformSelection};
use crate::runner::Runner;

pub fn run(args: &DeployArgs) -> Result<(), TyuError> {
    enforce_otp_guardrails(args)?;

    let build_args = args.to_build_args();
    let ctx = build::resolve_build_context(&build_args)?;
    let target = ctx.target;
    let resolved_out_dir = ctx.out_dir.clone();
    let build_selection = ctx.platform_selection.clone();
    let build_out = build::build_resolved(&build_args, ctx)?;
    let build_mode = build_out.mode;
    let built_image = build_out.final_image;
    // Static deploys may ship an lmod while QEMU executes the ELF it was
    // packed from. Dynamic builds use the firmware ELF directly.
    let exec_image = build_out.execution_image;
    let workspace_root = platform::workspace_root();
    let selection = match build_selection {
        Some(selection) => selection,
        None => {
            let triple = std::str::from_utf8(target.triple())
                .map_err(|_| TyuError::Build("non-UTF-8 target triple".into()))?;
            platform::resolve_platform_selection(&workspace_root, triple, None)?
        }
    };
    let deploy = selection
        .pack
        .manifest
        .deploy
        .as_ref()
        .ok_or_else(|| TyuError::Deploy("missing [deploy] section".into()))?;

    let deploy_dir = resolved_out_dir.join("deploy");
    fs::create_dir_all(&deploy_dir).map_err(TyuError::Io)?;

    let obj_stem = args
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let packed_path = deploy_dir.join(format!("{}.lmod", obj_stem));
    let encrypted_path = deploy_dir.join("encrypted.lmod");
    let signed_path = deploy_dir.join("signed.lmod");

    let packed_bytes = read_deploy_lmod(
        &built_image,
        &exec_image,
        &resolved_out_dir,
        &args.input,
        build_mode,
        args.verify_manifest.as_deref(),
    )?;
    // P11.3 (§Q7 rule 3, FR-8): the deployed module's verify_manifest must
    // satisfy the deploy's --verify-policy (E6510). Image-level closure is
    // enforced HERE, at deploy time, by walking the import graph (§Q7 rule
    // 3): every module in the image must carry a policy-satisfying summary
    // (a proven caller with an unproven callee is rejected by name). The
    // loader's per-module hook (E6502) is a device-side backstop that
    // defaults to Off — the deploy gate is the compositional enforcement.
    check_deploy_policy(&packed_bytes, args.verify_policy, args.proven_no_candidates)?;
    render_candidate_ratios(&resolved_out_dir, &packed_bytes, args.verify_policy);
    check_image_pairing(
        &resolved_out_dir,
        &args.input,
        &args.include_dirs,
        args.sysroot.as_deref(),
        args.verify_policy,
        args.proven_no_candidates,
    )?;
    write_atomic(&packed_path, &packed_bytes).map_err(TyuError::Io)?;

    let lmod_bytes = fs::read(&packed_path).map_err(TyuError::Io)?;

    let encrypted = match &args.enc_mode {
        crate::args::EncryptMode::None => lmod_bytes,
        crate::args::EncryptMode::Fleet => {
            let kek = resolve_key_material(
                &args.key_encrypt,
                "--key-encrypt is required for fleet mode",
            )?;
            lmod_encrypt::encrypt_fleet(&lmod_bytes, kek.try_as_32bytes()?)
                .map_err(|e| TyuError::Deploy(format!("lmod-encrypt: {}", e)))?
        }
        crate::args::EncryptMode::Device => {
            let keys_dir = args.device_keys_dir.as_ref().ok_or_else(|| {
                TyuError::Deploy("--device-keys=<dir> is required for device mode".into())
            })?;
            let device_keys = load_device_keys(keys_dir)?;
            lmod_encrypt::encrypt_device(&lmod_bytes, &device_keys)
                .map_err(|e| TyuError::Deploy(format!("lmod-encrypt: {}", e)))?
        }
    };
    write_atomic(&encrypted_path, &encrypted).map_err(TyuError::Io)?;

    if args.sign || args.key_sign.is_some() {
        let sign_key = resolve_key_material(&args.key_sign, "--key-sign is required for signing")?;
        let signed = lmod_sign::sign(&encrypted, sign_key.try_as_32bytes()?)
            .map_err(|e| TyuError::Deploy(format!("lmod-sign: {}", e)))?;
        write_atomic(&signed_path, &signed).map_err(TyuError::Io)?;
    } else {
        fs::copy(&encrypted_path, &signed_path).map_err(TyuError::Io)?;
    }

    let recipe = DeployRecipeContext {
        pack: &selection.pack,
        selection: &selection,
        deploy,
        out_dir: &resolved_out_dir,
        deploy_dir: &deploy_dir,
        packed_path: &packed_path,
        encrypted_path: &encrypted_path,
        signed_path: &signed_path,
        built_image: &built_image,
    };
    run_deploy_steps(recipe)?;

    let deploy_method = normalized_deploy_method(&deploy.method);
    if deploy.method == "elf-qemu" {
        eprintln!("tyu: warning: deploy method 'elf-qemu' is deprecated; use 'qemu'");
    }

    if deploy_method == "qemu" {
        let runner = Runner::for_target(target);
        let timeout = Duration::from_secs(10);
        let outcome = if build_mode == BuildMode::Dynamic {
            runner.run(&exec_image, timeout)?
        } else {
            let runner_image = if signed_path.extension().and_then(|s| s.to_str()) == Some("lmod") {
                // The deployed artifact is an lmod; static QEMU execution uses
                // the ELF it was packed from. Co-locate that ELF next to the lmod
                // so the runner's static lmod->ELF resolution finds it.
                if exec_image.extension().and_then(|s| s.to_str()) != Some("lmod") {
                    fs::copy(&exec_image, deploy_dir.join("image.elf")).map_err(TyuError::Io)?;
                }
                signed_path.clone()
            } else {
                built_image.clone()
            };
            runner.run_static_artifact(&runner_image, timeout)?
        };

        if outcome.timed_out {
            return Err(TyuError::Deploy(format!(
                "HANG — timed out after {:?}",
                timeout
            )));
        }

        let summary = harness_core::parse_output(&outcome.stdout);
        if !summary.completed {
            return Err(TyuError::Deploy(format!(
                "NO_COMPLETION — exit code {} but no `S\\n` marker",
                outcome.exit_code,
            )));
        }
        if summary.failures > 0 {
            return Err(TyuError::Deploy(format!(
                "FAIL_MARKER — {} failure(s) reported",
                summary.failures,
            )));
        }

        let expected = match target.spec().qemu {
            Some(spec) => spec.exit_convention.host_pass_exit(),
            None => 0,
        };
        if outcome.exit_code != expected {
            return Err(TyuError::Deploy(format!(
                "EXIT_MISMATCH — exit code {} != expected {}",
                outcome.exit_code, expected,
            )));
        }
    }

    Ok(())
}

fn read_deploy_lmod(
    built_image: &Path,
    _exec_image: &Path,
    out_dir: &Path,
    input: &Path,
    build_mode: BuildMode,
    verify_manifest: Option<&std::path::Path>,
) -> Result<Vec<u8>, TyuError> {
    // The build's packed lmod already embeds the verify_manifest record
    // when `--verify-manifest` was supplied to the build — or, from P11.1,
    // when the build derived it from the ROOT module's `tyu.vm/1` summary
    // under a requiring policy: use it as-is.
    if built_image.extension().and_then(|s| s.to_str()) == Some("lmod") {
        return fs::read(built_image).map_err(TyuError::Io);
    }

    if build_mode == BuildMode::Static {
        // The static re-pack consumes the explicit summary, or the ROOT
        // module's derived summary when the build did not pack one
        // (e.g. an ELF target).
        let summary = match verify_manifest {
            Some(p) => Some(p.to_path_buf()),
            None => root_vm_summary(out_dir, input),
        };
        return pack_static_with_manifest(built_image, summary.as_deref());
    }

    let input_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let preferred = out_dir.join(format!("{}.lmod", input_stem));
    if preferred.is_file() {
        return fs::read(preferred).map_err(TyuError::Io);
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(out_dir).map_err(TyuError::Io)? {
        let entry = entry.map_err(TyuError::Io)?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("lmod") {
            candidates.push(path);
        }
    }
    candidates.sort();
    match candidates.as_slice() {
        [path] => fs::read(path).map_err(TyuError::Io),
        [] => Err(TyuError::Deploy(format!(
            "dynamic deploy expected an application .lmod in {}",
            out_dir.display()
        ))),
        _ => Err(TyuError::Deploy(format!(
            "dynamic deploy found multiple application .lmod candidates in {}; expected {}.lmod",
            out_dir.display(),
            input_stem
        ))),
    }
}

fn enforce_otp_guardrails(args: &DeployArgs) -> Result<(), TyuError> {
    let ci = std::env::var_os("CI").is_some();
    let allow_otp = matches!(std::env::var("TYU_ALLOW_OTP").as_deref(), Ok("1"));
    let readback_ok = matches!(std::env::var("TYU_OTP_READBACK").as_deref(), Ok("verified"));
    commit_otp_gate(args.commit_otp, ci, allow_otp, readback_ok)
}

fn commit_otp_gate(
    commit_otp: bool,
    ci: bool,
    allow_otp: bool,
    readback_ok: bool,
) -> Result<(), TyuError> {
    if !commit_otp {
        return Ok(());
    }
    if ci {
        return Err(TyuError::Deploy(
            "--commit-otp is forbidden in CI; run the manual board procedure only".into(),
        ));
    }
    if !allow_otp {
        return Err(TyuError::Deploy(
            "--commit-otp requires TYU_ALLOW_OTP=1 for the manual board procedure".into(),
        ));
    }
    if !readback_ok {
        return Err(TyuError::Deploy(
            "--commit-otp requires TYU_OTP_READBACK=verified before any irreversible write".into(),
        ));
    }

    Err(TyuError::Deploy(
        "OTP commit path is not implemented in this phase; dry-run only".into(),
    ))
}

fn resolve_key_material(
    key_ref: &Option<String>,
    error_msg: &str,
) -> Result<KeyMaterial, TyuError> {
    let kr_str = key_ref
        .as_ref()
        .ok_or_else(|| TyuError::Key(error_msg.to_string()))?;
    let kr = KeyRef::parse(kr_str)?;
    KeyMaterial::resolve(&kr)
}

fn load_device_keys(keys_dir: &Path) -> Result<Vec<(String, [u8; 32])>, TyuError> {
    use crate::provision::DeviceRegistry;
    let reg = DeviceRegistry::load(keys_dir)?;
    let mut keys: Vec<(String, [u8; 32])> = Vec::new();
    for (id, kek_bytes) in reg.iter() {
        keys.push((id.to_string(), *kek_bytes));
    }
    if keys.is_empty() {
        return Err(TyuError::Provision(
            "no device keys found in directory".into(),
        ));
    }
    Ok(keys)
}

struct DeployRecipeContext<'a> {
    pack: &'a PlatformPack,
    selection: &'a ResolvedPlatformSelection,
    deploy: &'a crate::platform::DeploySection,
    out_dir: &'a Path,
    deploy_dir: &'a Path,
    packed_path: &'a Path,
    encrypted_path: &'a Path,
    signed_path: &'a Path,
    built_image: &'a Path,
}

fn run_deploy_steps(ctx: DeployRecipeContext<'_>) -> Result<(), TyuError> {
    for (idx, step) in ctx.deploy.steps.iter().enumerate() {
        let run = substitute_step_text(&step.run, &ctx);
        let mut cmd = Command::new(&run);
        cmd.current_dir(ctx.pack.pack_root());
        for arg in &step.args {
            cmd.arg(substitute_step_text(arg, &ctx));
        }
        cmd.env("TYU_DEPLOY_PACK_ROOT", ctx.pack.pack_root());
        cmd.env("TYU_DEPLOY_OUT_DIR", ctx.out_dir);
        cmd.env("TYU_DEPLOY_DIR", ctx.deploy_dir);
        cmd.env("TYU_DEPLOY_PACKED", ctx.packed_path);
        cmd.env("TYU_DEPLOY_ENCRYPTED", ctx.encrypted_path);
        cmd.env("TYU_DEPLOY_SIGNED", ctx.signed_path);
        cmd.env(
            "TYU_DEPLOY_ELF",
            exec_image_path(ctx.built_image, ctx.out_dir),
        );
        cmd.env(
            "TYU_DEPLOY_METHOD",
            normalized_deploy_method(&ctx.deploy.method),
        );
        cmd.env("TYU_DEPLOY_PLATFORM", ctx.pack.name());
        cmd.env(
            "TYU_DEPLOY_TARGET",
            std::str::from_utf8(ctx.selection.target.triple()).unwrap_or(""),
        );
        if let Some(debug) = &ctx.pack.manifest.debug {
            cmd.env("TYU_DEPLOY_PROBE_CONFIG", debug.probe_config.as_str());
            cmd.env("TYU_DEPLOY_PROBE", debug.probe.as_str());
        }

        let status = cmd.status().map_err(|e| {
            TyuError::Deploy(format!("running deploy step {} '{}': {}", idx, run, e))
        })?;
        if !status.success() {
            return Err(TyuError::Deploy(format!(
                "deploy step {} '{}' failed with status {:?}",
                idx,
                run,
                status.code(),
            )));
        }
    }

    Ok(())
}

fn substitute_step_text(text: &str, ctx: &DeployRecipeContext<'_>) -> String {
    let mut out = text.to_string();
    let replacements = [
        ("{packed}", ctx.packed_path.display().to_string()),
        ("{encrypted}", ctx.encrypted_path.display().to_string()),
        ("{signed}", ctx.signed_path.display().to_string()),
        ("{image}", ctx.signed_path.display().to_string()),
        (
            "{elf}",
            exec_image_path(ctx.built_image, ctx.out_dir)
                .display()
                .to_string(),
        ),
        ("{out_dir}", ctx.out_dir.display().to_string()),
        ("{deploy_dir}", ctx.deploy_dir.display().to_string()),
        ("{pack_root}", ctx.pack.pack_root().display().to_string()),
        ("{platform}", ctx.pack.name().to_string()),
        (
            "{method}",
            normalized_deploy_method(&ctx.deploy.method).to_string(),
        ),
        (
            "{probe_config}",
            ctx.pack
                .manifest
                .debug
                .as_ref()
                .map(|d| d.probe_config.clone())
                .unwrap_or_default(),
        ),
        (
            "{probe}",
            ctx.pack
                .manifest
                .debug
                .as_ref()
                .map(|d| d.probe.clone())
                .unwrap_or_default(),
        ),
        (
            "{target}",
            std::str::from_utf8(ctx.selection.target.triple())
                .unwrap_or("")
                .to_string(),
        ),
    ];
    for (needle, value) in replacements {
        out = out.replace(needle, &value);
    }
    out
}

fn exec_image_path(built_image: &Path, out_dir: &Path) -> PathBuf {
    if built_image.extension().and_then(|s| s.to_str()) == Some("lmod") {
        out_dir.join("image.elf")
    } else {
        built_image.to_path_buf()
    }
}

fn normalized_deploy_method(method: &str) -> &str {
    match method {
        "elf-qemu" => "qemu",
        other => other,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Pack a static ELF with the verify_manifest record (P11.3): assemble the
/// record from the build's `tyu.vm/1` summary when present.
fn pack_static_with_manifest(
    elf_path: &Path,
    verify_manifest: Option<&std::path::Path>,
) -> Result<Vec<u8>, TyuError> {
    let elf_bytes = fs::read(elf_path).map_err(TyuError::Io)?;
    let record = match verify_manifest {
        Some(path) => {
            let text = fs::read_to_string(path).map_err(TyuError::Io)?;
            lmod_pack::verify::encode_from_json_text(&text)
                .map_err(|e| TyuError::Deploy(format!("verify-manifest: {e}")))?
        }
        None => Vec::new(),
    };
    lmod_pack::pack_with_verify_manifest(&elf_bytes, &record)
        .map_err(|e| TyuError::Deploy(format!("lmod-pack: {}", e)))
}

/// P11.3: the deploy-time pairing gate (§Q7 rule 3, FR-8, E6510).
///
/// The deployed `.lmod` must carry a `verify_manifest` whose declared policy
/// satisfies the deploy's requirement:
/// - `OpenOk`: no requirement;
/// - `NoOpen`: policy ≥ `no-open`;
/// - `Proven`: policy == `proven` AND a modeled bundle (`model != "unmodeled"`).
///
/// `no_candidates` is the §Q10 `--proven-no-candidates` knob: under `proven`,
/// a nonzero `candidate_ratio` fails the deploy.
///
/// A missing or malformed manifest under any requiring policy is rejected
/// (E6510) — fail-closed, never a silent pair.
fn check_deploy_policy(
    packed: &[u8],
    policy: crate::args::DeployVerifyPolicy,
    no_candidates: bool,
) -> Result<(), TyuError> {
    use crate::args::DeployVerifyPolicy;

    if policy == DeployVerifyPolicy::OpenOk {
        return Ok(());
    }
    const E6510: u32 = 6510;
    let einterr = |detail: String| TyuError::VerifyPairing {
        code: E6510,
        detail,
    };
    let container = lmod::validate::Container::parse(packed)
        .map_err(|_| einterr("container malformed or unsigned".to_string()))?;
    let vm = match lmod::verify_manifest::scan_verify_manifest(container.modinfo()) {
        Ok(Some(vm)) => vm,
        Ok(None) => {
            return Err(einterr(
                "deployed module has no verify_manifest".to_string(),
            ))
        }
        Err(_) => return Err(einterr("verify_manifest malformed".to_string())),
    };
    // The shared policy comparison (`lmod::verify_manifest::satisfies` —
    // the same byte-level rule the loader enforces for RequireNoOpen/
    // RequireProven, so the two enforcers cannot drift).
    let require = match policy {
        DeployVerifyPolicy::OpenOk => 0,
        DeployVerifyPolicy::NoOpen => 1,
        DeployVerifyPolicy::Proven => 2,
    };
    if require != 0 && !lmod::verify_manifest::satisfies(vm.policy, require) {
        return Err(einterr(format!(
            "declared policy ({}) does not satisfy the requirement ({})",
            vm.policy, require
        )));
    }
    // The deploy-side-only bundle-model rule (§Q15): a proven pairing must
    // target a modeled bundle.
    if policy == DeployVerifyPolicy::Proven && vm.model == b"unmodeled" {
        return Err(einterr(
            "proven pairing against an unmodeled bundle (E_MODEL_UNMODELED)".to_string(),
        ));
    }
    // §Q10 `--proven-no-candidates`: a proven deploy that must carry no
    // candidate-authored certificates rejects a nonzero shipped ratio
    // (default: candidates admissible — kernel-checked; refusing them is
    // process preference, not soundness).
    if policy == DeployVerifyPolicy::Proven && no_candidates && vm.candidate_ratio > 0 {
        return Err(einterr(format!(
            "--proven-no-candidates: the shipped manifest carries candidate-authored \
             certificates (ratio {} basis 10000) — review proofs/candidates/ and \
             re-verify, or drop the flag",
            vm.candidate_ratio
        )));
    }
    Ok(())
}

/// §Q10 rendering: under a proven-required deploy, print the candidate ratio
/// (basis 10000) the shipped manifest carries, plus the per-module ratios
/// from the derived summaries — the operator-facing surface the plan names
/// ("deploy under `proven` prints the candidate ratio").
fn render_candidate_ratios(out_dir: &Path, packed: &[u8], policy: crate::args::DeployVerifyPolicy) {
    if policy != crate::args::DeployVerifyPolicy::Proven {
        return;
    }
    // The shipped root manifest's ratio.
    let root_ratio = lmod::validate::Container::parse(packed).ok().and_then(|c| {
        match lmod::verify_manifest::scan_verify_manifest(c.modinfo()) {
            Ok(Some(vm)) => Some(vm.candidate_ratio),
            _ => None,
        }
    });
    eprintln!(
        "tyu: deploy (proven): candidate ratio {} (basis 10000; candidate-authored \
         certificates in the shipped manifest)",
        root_ratio.unwrap_or(0)
    );
    // Per-module ratios from the derived `tyu.vm/1` summaries (best-effort).
    let state_dir = crate::vm_summary::verify_state_dir(out_dir);
    if let Ok(rd) = fs::read_dir(&state_dir) {
        let mut rows: Vec<(String, u16)> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if !p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".vm.json"))
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            if let Ok(spec) = lmod_pack::verify::verify_manifest_from_json(&text) {
                let module = p
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .map(String::from)
                    .unwrap_or_default();
                let module = module.strip_suffix(".vm").unwrap_or(&module).to_string();
                rows.push((module, spec.candidate_ratio));
            }
        }
        rows.sort();
        for (module, ratio) in rows {
            eprintln!("tyu:   {module}: candidate ratio {ratio}");
        }
    }
}

/// The ROOT module's derived `tyu.vm/1` summary in the out dir: the module
/// name follows the input file stem (PascalCase — the same convention the
/// module declaration uses).
fn root_vm_summary(out_dir: &Path, input: &Path) -> Option<PathBuf> {
    let stem = input.file_stem()?.to_str()?;
    let name: String = stem
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect();
    if name.is_empty() {
        return None;
    }
    let p = crate::vm_summary::vm_summary_path(out_dir, &name);
    p.is_file().then_some(p)
}

/// P11.3 (FR-8, §Q7 rule 3): the image-level pairing gate — walk the import
/// graph so a policy-requiring deploy refuses an image containing callees
/// that do not satisfy the claim. The loader's per-module hook (E6502)
/// defaults to `Off`; the deploy gate is the compositional enforcement:
/// a proven caller paired with an unproven callee is rejected (E6510),
/// naming the offending module.
fn check_image_pairing(
    out_dir: &Path,
    input: &Path,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    policy: crate::args::DeployVerifyPolicy,
    no_candidates: bool,
) -> Result<(), TyuError> {
    use crate::args::DeployVerifyPolicy;

    if policy == DeployVerifyPolicy::OpenOk {
        return Ok(());
    }
    const E6510: u32 = 6510;
    let einterr = |detail: String| TyuError::VerifyPairing {
        code: E6510,
        detail,
    };
    let require = match policy {
        DeployVerifyPolicy::OpenOk => 0,
        DeployVerifyPolicy::NoOpen => 1,
        DeployVerifyPolicy::Proven => 2,
    };
    // The resolved import graph (dependencies first, root last) IS the
    // image's closure; platform-sysroot imports are excluded (they are not
    // application modules).
    let graph = crate::graph::resolve_graph(input, include_dirs, sysroot).map_err(|e| {
        einterr(format!(
            "resolving the image's module graph for pairing: {e}"
        ))
    })?;
    if graph.is_empty() {
        return Err(einterr(
            "image pairing found an empty module graph to check (malformed)".to_string(),
        ));
    }
    // Walk every module in the image EXCEPT the root (the caller): the root's
    // packaged `verify_manifest` is vetted by [`check_deploy_policy`]; the
    // callees' summaries are the image-level closure a proven/no-open deploy
    // must accept. A missing callee summary under a requiring policy is a
    // fail-closed "callee has no verify state".
    let root = graph.last().map(|n| n.name.as_str());
    for node in &graph {
        if Some(node.name.as_str()) == root {
            continue;
        }
        let sum_path = crate::vm_summary::vm_summary_path(out_dir, &node.name);
        if !sum_path.is_file() {
            return Err(einterr(format!(
                "callee '{}' has no verify_manifest summary ({}): the image \
                 was not built with the required policy",
                node.name,
                sum_path.display()
            )));
        }
        let text = fs::read_to_string(&sum_path).map_err(TyuError::Io)?;
        let spec = lmod_pack::verify::verify_manifest_from_json(&text)
            .map_err(|e| einterr(format!("callee '{}' summary malformed: {e}", node.name)))?;
        // The shared policy comparison (`lmod::verify_manifest::satisfies`).
        if require != 0 && !lmod::verify_manifest::satisfies(spec.policy, require) {
            return Err(einterr(format!(
                "unproven callee '{}' in a {} image (the callee was not built \
                 under the required policy)",
                node.name,
                if policy == DeployVerifyPolicy::Proven {
                    "proven"
                } else {
                    "no-open"
                }
            )));
        }
        if policy == DeployVerifyPolicy::Proven && spec.model == "unmodeled" {
            return Err(einterr(format!(
                "unmodeled callee '{}' in a proven image (E_MODEL_UNMODELED)",
                node.name
            )));
        }
        // §Q10 `--proven-no-candidates`: the image-level closure covers the
        // knob too — a proven deploy that admits no candidates rejects a
        // candidate-authored callee summary.
        if policy == DeployVerifyPolicy::Proven && no_candidates && spec.candidate_ratio > 0 {
            return Err(einterr(format!(
                "--proven-no-candidates: callee '{}' carries candidate-authored \
                 certificates (ratio {} basis 10000)",
                node.name, spec.candidate_ratio
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegen_core::Target;

    /// The compositional-challenger rule (§Q7 rule 3 / FR-8): `check_image_pairing`
    /// walks the import graph and rejects a proven caller whose CALLEE does not
    /// satisfy the claim — the unit version of the deploy gate (hermetic: crafted
    /// `tyu.vm/1` summaries + a real two-module graph).
    #[test]
    fn image_pairing_rejects_unproven_callee() {
        // The graph resolver parses with the language frontend, whose deep
        // parse frames exceed the 2 MiB default test-thread stack — run the
        // body on a 64 MiB stack.
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(image_pairing_rejects_unproven_callee_body)
            .expect("spawn big-stack pairing thread");
        h.join().expect("pairing thread");
    }

    fn image_pairing_rejects_unproven_callee_body() {
        let dir = std::env::temp_dir().join(format!("tyu_pairing_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Cal.mod defines a subword; Main.mod imports it (the graph edge).
        fs::write(
            dir.join("Cal.mod"),
            "module Cal;\n: cal ( -- i64 ) 5 ;\nexport { cal };\nend;\n",
        )
        .unwrap();
        fs::write(
            dir.join("Main.mod"),
            "module Main;\nimport Cal { cal };\n: main ( -- i64 ) cal 0 ;\nexport { main };\nend;\n",
        )
        .unwrap();
        let includes = [dir.clone()];

        let vm_doc = |policy: &str, model: &str, ratio: u16| -> String {
            format!(
                r#"{{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-none","model":"{model}","policy":"{policy}","certifier":{{"class":"none","name":"","recognition":""}},"candidate_ratio":{ratio},"counts":{{"proof":0,"checked":0,"assumed":0,"open":0}},"obligations":[]}}"#
            )
        };
        // The ROOT caller's summary only (the root is vetted by its packed
        // manifest; the callee's summary is the image-level claim).
        fs::create_dir_all(dir.join("out/.tyu-verify")).unwrap();
        fs::write(
            dir.join("out/.tyu-verify/Main.vm.json"),
            vm_doc("proven", "tyu.model/x86_64-unknown-none/1", 0),
        )
        .unwrap();
        let prove_config = crate::args::DeployVerifyPolicy::Proven;

        // 1. Unproven callee ⇒ E6510 naming the callee.
        fs::write(
            dir.join("out/.tyu-verify/Cal.vm.json"),
            vm_doc("open-ok", "tyu.model/x86_64-unknown-none/1", 0),
        )
        .unwrap();
        let err = check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            false,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("E6510"), "err: {msg}");
        assert!(
            msg.contains("Cal") && msg.contains("unproven callee"),
            "must name the callee: {msg}"
        );

        // 2. Proven + modeled callee ⇒ OK.
        fs::write(
            dir.join("out/.tyu-verify/Cal.vm.json"),
            vm_doc("proven", "tyu.model/x86_64-unknown-none/1", 0),
        )
        .unwrap();
        check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            false,
        )
        .expect("proven+modeled callee must pair");

        // 3. Proven but unmodeled callee ⇒ E6510 (E_MODEL_UNMODELED).
        fs::write(
            dir.join("out/.tyu-verify/Cal.vm.json"),
            vm_doc("proven", "unmodeled", 0),
        )
        .unwrap();
        let err2 = check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            false,
        )
        .unwrap_err();
        assert!(
            err2.to_string().contains("unmodeled callee"),
            "err2: {err2}"
        );

        // 4. Missing callee summary ⇒ E6510 (no verified state).
        fs::remove_file(dir.join("out/.tyu-verify/Cal.vm.json")).unwrap();
        let err3 = check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            false,
        )
        .unwrap_err();
        assert!(
            err3.to_string().contains("has no verify_manifest summary"),
            "err3: {err3}"
        );

        // 5. §Q10 `--proven-no-candidates`: a candidate-authored callee is
        // rejected only when the knob is set (default admits candidates).
        fs::write(
            dir.join("out/.tyu-verify/Cal.vm.json"),
            vm_doc("proven", "tyu.model/x86_64-unknown-none/1", 5000),
        )
        .unwrap();
        check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            false,
        )
        .expect("candidate callee pairs when the knob is off");
        let err4 = check_image_pairing(
            &dir.join("out"),
            &dir.join("Main.mod"),
            &includes,
            None,
            prove_config,
            true,
        )
        .unwrap_err();
        assert!(
            err4.to_string().contains("--proven-no-candidates") && err4.to_string().contains("Cal"),
            "knob must reject the candidate-authored callee: {err4}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    fn demo_pack(root: &Path) -> (PlatformPack, ResolvedPlatformSelection) {
        let manifest_path = root.join("platforms/demo/platform.toml");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        let manifest = r#"
[platform]
name = "demo"
compiler-interface = 1

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[deploy]
method = "openocd"
boot = "raw_vectors"

[[deploy.step]]
run = "sh"
args = ["-c", "printf '%s' \"$TYU_DEPLOY_METHOD:$TYU_DEPLOY_PLATFORM:$TYU_DEPLOY_OUT_DIR:$1\" > \"$TYU_DEPLOY_OUT_DIR/step.txt\"", "--", "{signed}"]

[debug]
diag_transport = "qemu-stub"
probe_config = "demo.cfg"

[test]
rung = "untested"
"#;
        fs::write(&manifest_path, manifest).unwrap();
        let pack = platform::load_platform_pack(root, &manifest_path).unwrap();
        let isa = pack.manifest.platform.isa[0].clone();
        let selection = ResolvedPlatformSelection {
            pack: pack.clone(),
            isa,
            target: Target::X86_64UnknownNone,
        };
        (pack, selection)
    }

    #[test]
    fn deploy_recipe_executes_steps_with_placeholders() {
        let root = std::env::temp_dir().join("tyu_deploy_recipe");
        let _ = fs::remove_dir_all(&root);
        let (pack, selection) = demo_pack(&root);
        let out_dir = root.join("out");
        let deploy_dir = out_dir.join("deploy");
        fs::create_dir_all(&deploy_dir).unwrap();
        let packed = deploy_dir.join("packed.lmod");
        let encrypted = deploy_dir.join("encrypted.lmod");
        let signed = deploy_dir.join("signed.lmod");
        let built_image = out_dir.join("image.elf");
        fs::write(&packed, b"packed").unwrap();
        fs::write(&encrypted, b"encrypted").unwrap();
        fs::write(&signed, b"signed").unwrap();
        fs::write(&built_image, b"elf").unwrap();

        let ctx = DeployRecipeContext {
            pack: &pack,
            selection: &selection,
            deploy: pack.manifest.deploy.as_ref().unwrap(),
            out_dir: &out_dir,
            deploy_dir: &deploy_dir,
            packed_path: &packed,
            encrypted_path: &encrypted,
            signed_path: &signed,
            built_image: &built_image,
        };

        run_deploy_steps(ctx).unwrap();

        let rendered = fs::read_to_string(out_dir.join("step.txt")).unwrap();
        assert!(rendered.contains("openocd"));
        assert!(rendered.contains("demo"));
        assert!(rendered.contains("signed.lmod"));
    }

    #[test]
    fn write_atomic_replaces_file() {
        let root = std::env::temp_dir().join("tyu_deploy_atomic");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("artifact.lmod");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    #[test]
    fn otp_guard_allows_dry_run_by_default() {
        commit_otp_gate(false, false, false, false).unwrap();
    }

    #[test]
    fn otp_guard_blocks_ci_even_with_commit_flag() {
        let err = commit_otp_gate(true, true, true, true).unwrap_err();
        assert!(err.to_string().contains("forbidden in CI"));
    }

    #[test]
    fn otp_guard_requires_backup_readback() {
        let err = commit_otp_gate(true, false, true, false).unwrap_err();
        assert!(err.to_string().contains("TYU_OTP_READBACK"));
    }

    #[test]
    fn otp_guard_rejects_unimplemented_commit_path_after_checks_pass() {
        let err = commit_otp_gate(true, false, true, true).unwrap_err();
        assert!(err
            .to_string()
            .contains("OTP commit path is not implemented"));
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::args::DeployArgs;
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
    let built_image = build_out.final_image;
    let workspace_root = platform::workspace_root();
    let selection = match build_selection {
        Some(selection) => selection,
        None => {
            let triple = std::str::from_utf8(target.triple())
                .map_err(|_| TyuError::Build("non-UTF-8 target triple".into()))?;
            platform::resolve_platform_selection(&workspace_root, triple, None)
                .map_err(TyuError::Build)?
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

    let packed_bytes = if built_image.extension().and_then(|s| s.to_str()) == Some("lmod") {
        fs::read(&built_image).map_err(TyuError::Io)?
    } else {
        let elf_bytes = fs::read(&built_image).map_err(TyuError::Io)?;
        lmod_pack::pack(&elf_bytes).map_err(|e| TyuError::Deploy(format!("lmod-pack: {}", e)))?
    };
    write_atomic(&packed_path, &packed_bytes).map_err(TyuError::Io)?;

    let lmod_bytes = fs::read(&packed_path).map_err(TyuError::Io)?;

    let encrypted = match &args.enc_mode {
        crate::args::EncryptMode::None => lmod_bytes,
        crate::args::EncryptMode::Fleet => {
            let kek = resolve_key_material(
                &args.key_encrypt,
                "--key-encrypt is required for fleet mode",
            )?;
            lmod_encrypt::encrypt_fleet(
                &lmod_bytes,
                kek.try_as_32bytes().map_err(|e| TyuError::Key(e))?,
            )
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
        let signed = lmod_sign::sign(
            &encrypted,
            sign_key.try_as_32bytes().map_err(|e| TyuError::Key(e))?,
        )
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

    if deploy.method == "elf-qemu" {
        let runner = Runner::for_target(target);
        let runner_image = if signed_path.extension().and_then(|s| s.to_str()) == Some("lmod") {
            signed_path.clone()
        } else {
            built_image.clone()
        };
        let timeout = Duration::from_secs(10);
        let outcome = runner
            .run(&runner_image, timeout)
            .map_err(|e| TyuError::Runner(e))?;

        if outcome.timed_out {
            return Err(TyuError::Deploy(format!("HANG — timed out after {:?}", timeout)).into());
        }

        let summary = harness_core::parse_output(&outcome.stdout);
        if !summary.completed {
            return Err(TyuError::Deploy(format!(
                "NO_COMPLETION — exit code {} but no `S\\n` marker",
                outcome.exit_code,
            ))
            .into());
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
            ))
            .into());
        }
    }

    Ok(())
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
    let kr = KeyRef::parse(kr_str).map_err(|e| TyuError::Key(e))?;
    KeyMaterial::resolve(&kr).map_err(|e| TyuError::Key(e))
}

fn load_device_keys(keys_dir: &Path) -> Result<Vec<(String, [u8; 32])>, TyuError> {
    use crate::provision::DeviceRegistry;
    let reg = DeviceRegistry::load(keys_dir).map_err(|e| TyuError::Provision(e))?;
    let mut keys: Vec<(String, [u8; 32])> = Vec::new();
    for (id, kek_bytes) in reg.iter() {
        keys.push((id.to_string(), *kek_bytes));
    }
    if keys.is_empty() {
        return Err(TyuError::Provision("no device keys found in directory".into()).into());
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
        cmd.env("TYU_DEPLOY_METHOD", ctx.deploy.method.as_str());
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
        ("{method}", ctx.deploy.method.clone()),
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

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegen_core::Target;

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

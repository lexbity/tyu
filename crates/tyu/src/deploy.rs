//! `tyu deploy` — end-to-end deployment pipeline.
//!
//! Pipeline: build → pack → encrypt → sign → run.
//! For QEMU targets the image is executed under the emulator;
//! physical device flashing is Phase 10.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use codegen_core::Target;

use crate::args::DeployArgs;
use crate::build;
use crate::runner::Runner;

/// Run the `deploy` subcommand.
pub fn run(args: &DeployArgs) -> Result<(), String> {
    let triple = std::str::from_utf8(args.target.triple())
        .map_err(|_| "non-UTF-8 target triple")?;

    // Step 1: Build the image (same as `tyu build`).
    let build_args = args.to_build_args();
    let image = build::build(&build_args)?;

    // Determine output directory and workspace root.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();

    let deploy_dir = args.out_dir.join("deploy");
    std::fs::create_dir_all(&deploy_dir)
        .map_err(|e| format!("creating deploy dir: {}", e))?;

    // Step 2: Pack the .o into .lmod.
    // lmod-pack takes the .o and produces a .lmod container.
    let obj_stem = args.input.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let lmod_in = deploy_dir.join(format!("{}.lmod", obj_stem));
    let lmod_pack = workspace.join("target").join("debug").join("lmod-pack");

    // Find the .o produced by `tyu build`.
    let obj_path = deploy_dir.parent()
        .ok_or("no parent dir")?
        .join(format!("{}.o", obj_stem));
    let obj_path = if obj_path.exists() {
        obj_path
    } else {
        // Fallback: search for .o files in the out_dir.
        let candidates: Vec<PathBuf> = std::fs::read_dir(&args.out_dir)
            .map_err(|e| format!("reading out_dir: {}", e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("o"))
            .collect();
        candidates.into_iter().next()
            .ok_or("no .o file found — build may not have produced one")?
    };

    let status = Command::new(&lmod_pack)
        .args([obj_path.to_str().unwrap(), lmod_in.to_str().unwrap()])
        .status()
        .map_err(|e| format!("running lmod-pack: {}", e))?;
    if !status.success() {
        return Err("lmod-pack failed".into());
    }

    // Step 3: Encrypt.
    let lmod_enc = deploy_dir.join("encrypted.lmod");
    let lmod_encrypt = workspace.join("target").join("debug").join("lmod-encrypt");

    let mut enc_args = vec![
        lmod_in.to_str().unwrap().to_string(),
        lmod_enc.to_str().unwrap().to_string(),
    ];

    match &args.enc_mode {
        crate::args::EncryptMode::None => {
            // No encryption — just copy the lmod.
            std::fs::copy(&lmod_in, &lmod_enc)
                .map_err(|e| format!("copying lmod: {}", e))?;
        }
        crate::args::EncryptMode::Fleet => {
            let kek_hex = resolve_key(&args.key_encrypt)
                .ok_or("--key-encrypt is required for fleet mode")?;
            enc_args.push("--mode=fleet".into());
            enc_args.push(format!("--kek={}", kek_hex));
            let status = Command::new(&lmod_encrypt)
                .args(&enc_args)
                .status()
                .map_err(|e| format!("running lmod-encrypt: {}", e))?;
            if !status.success() {
                return Err("lmod-encrypt failed".into());
            }
        }
        crate::args::EncryptMode::Device => {
            let keys_dir = &args.device_keys_dir;
            if keys_dir.is_none() {
                return Err("--device-keys=<dir> is required for device mode".into());
            }
            let devices_str = resolve_device_list(keys_dir.as_ref().unwrap())?;
            enc_args.push("--mode=device".into());
            enc_args.push(format!("--device-keys={}", keys_dir.as_ref().unwrap().display()));
            enc_args.push(format!("--devices={}", devices_str));
            let status = Command::new(&lmod_encrypt)
                .args(&enc_args)
                .status()
                .map_err(|e| format!("running lmod-encrypt: {}", e))?;
            if !status.success() {
                return Err("lmod-encrypt failed".into());
            }
        }
    }

    // Step 4: Sign (if --sign flag is set or key_sign is provided).
    let signed_path = deploy_dir.join("signed.lmod");
    if args.sign || args.key_sign.is_some() {
        let lmod_sign = workspace.join("target").join("debug").join("lmod-sign");
        let sign_key = resolve_key(&args.key_sign)
            .unwrap_or_else(|| "abababababababababababababababababababababababababababababababab".into());
        let status = Command::new(&lmod_sign)
            .args([
                lmod_enc.to_str().unwrap(),
                signed_path.to_str().unwrap(),
                &format!("--key={}", sign_key),
            ])
            .status()
            .map_err(|e| format!("running lmod-sign: {}", e))?;
        if !status.success() {
            return Err("lmod-sign failed".into());
        }
    } else {
        // No signing — just copy encrypted to signed.
        std::fs::copy(&lmod_enc, &signed_path)
            .map_err(|e| format!("copying: {}", e))?;
    }

    // Step 5: Run under the target's runner (QEMU for bare-metal).
    let runner = Runner::for_target(args.target);
    let timeout = Duration::from_secs(10);
    let outcome = runner.run(&signed_path, timeout)?;

    if outcome.timed_out {
        return Err(format!("HANG — timed out after {:?}", timeout));
    }

    let summary = harness_core::parse_output(&outcome.stdout);
    if !summary.completed {
        return Err(format!(
            "NO_COMPLETION — exit code {} but no `S\\n` marker",
            outcome.exit_code,
        ));
    }
    if summary.failures > 0 {
        return Err(format!(
            "FAIL_MARKER — {} failure(s) reported",
            summary.failures,
        ));
    }

    // Check exit code.
    let expected = match args.target.spec().qemu {
        Some(spec) => spec.exit_convention.host_pass_exit(),
        None => 0,
    };
    if outcome.exit_code != expected {
        return Err(format!(
            "EXIT_MISMATCH — exit code {} != expected {}",
            outcome.exit_code, expected,
        ));
    }

    Ok(())
}

/// Resolve a key reference.  Supports `env:<VAR>` syntax.
fn resolve_key(key_ref: &Option<String>) -> Option<String> {
    let kr = key_ref.as_ref()?;
    if let Some(var) = kr.strip_prefix("env:") {
        std::env::var(var).ok()
    } else {
        Some(kr.clone())
    }
}

/// Read device IDs from the keys directory.
fn resolve_device_list(keys_dir: &Path) -> Result<String, String> {
    use crate::provision::DeviceRegistry;
    let reg = DeviceRegistry::load(keys_dir)?;
    if reg.is_empty() {
        return Err("no device keys found in directory".into());
    }
    let ids: Vec<&str> = reg.iter().map(|(id, _)| id).collect();
    Ok(ids.join(","))
}

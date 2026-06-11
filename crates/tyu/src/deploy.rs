use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::args::DeployArgs;
use crate::build;
use crate::error::TyuError;
use crate::keys::{KeyMaterial, KeyRef};
use crate::runner::Runner;

pub fn run(args: &DeployArgs) -> Result<(), TyuError> {
    let build_args = args.to_build_args();
    let _image = build::build(&build_args)?;

    let deploy_dir = args.out_dir.join("deploy");
    fs::create_dir_all(&deploy_dir).map_err(TyuError::Io)?;

    let obj_stem = args.input.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let lmod_path = deploy_dir.join(format!("{}.lmod", obj_stem));

    let obj_path = deploy_dir.parent()
        .ok_or_else(|| TyuError::Build("no parent dir".into()))?
        .join(format!("{}.o", obj_stem));
    let obj_path = if obj_path.exists() {
        obj_path
    } else {
        let candidates: Vec<PathBuf> = fs::read_dir(&args.out_dir)
            .map_err(TyuError::Io)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("o"))
            .collect();
        candidates.into_iter().next()
            .ok_or_else(|| TyuError::Build("no .o file found".into()))?
    };

    let elf_bytes = fs::read(&obj_path).map_err(TyuError::Io)?;
    let packed = lmod_pack::pack(&elf_bytes)
        .map_err(|e| TyuError::Deploy(format!("lmod-pack: {}", e)))?;
    fs::write(&lmod_path, &packed).map_err(TyuError::Io)?;

    let lmod_enc = deploy_dir.join("encrypted.lmod");
    let lmod_bytes = fs::read(&lmod_path).map_err(TyuError::Io)?;

    let encrypted = match &args.enc_mode {
        crate::args::EncryptMode::None => lmod_bytes,
        crate::args::EncryptMode::Fleet => {
            let kek = resolve_key_material(&args.key_encrypt,
                "--key-encrypt is required for fleet mode")?;
            lmod_encrypt::encrypt_fleet(&lmod_bytes, kek.try_as_32bytes()
                .map_err(|e| TyuError::Key(e))?)
                .map_err(|e| TyuError::Deploy(format!("lmod-encrypt: {}", e)))?
        }
        crate::args::EncryptMode::Device => {
            let keys_dir = args.device_keys_dir.as_ref()
                .ok_or_else(|| TyuError::Deploy("--device-keys=<dir> is required for device mode".into()))?;
            let device_keys = load_device_keys(keys_dir)?;
            lmod_encrypt::encrypt_device(&lmod_bytes, &device_keys)
                .map_err(|e| TyuError::Deploy(format!("lmod-encrypt: {}", e)))?
        }
    };
    fs::write(&lmod_enc, &encrypted).map_err(TyuError::Io)?;

    let signed_path = deploy_dir.join("signed.lmod");
    if args.sign || args.key_sign.is_some() {
        let sign_key = resolve_key_material(&args.key_sign, "--key-sign is required for signing")?;
        let signed = lmod_sign::sign(&encrypted, sign_key.try_as_32bytes()
            .map_err(|e| TyuError::Key(e))?)
            .map_err(|e| TyuError::Deploy(format!("lmod-sign: {}", e)))?;
        fs::write(&signed_path, &signed).map_err(TyuError::Io)?;
    } else {
        fs::copy(&lmod_enc, &signed_path).map_err(TyuError::Io)?;
    }

    let runner = Runner::for_target(args.target);
    let timeout = Duration::from_secs(10);
    let outcome = runner.run(&signed_path, timeout)
        .map_err(|e| TyuError::Runner(e))?;

    if outcome.timed_out {
        return Err(TyuError::Deploy(format!("HANG — timed out after {:?}", timeout)).into());
    }

    let summary = harness_core::parse_output(&outcome.stdout);
    if !summary.completed {
        return Err(TyuError::Deploy(format!(
            "NO_COMPLETION — exit code {} but no `S\\n` marker", outcome.exit_code,
        )).into());
    }
    if summary.failures > 0 {
        return Err(TyuError::Deploy(format!(
            "FAIL_MARKER — {} failure(s) reported", summary.failures,
        )));
    }

    let expected = match args.target.spec().qemu {
        Some(spec) => spec.exit_convention.host_pass_exit(),
        None => 0,
    };
    if outcome.exit_code != expected {
        return Err(TyuError::Deploy(format!(
            "EXIT_MISMATCH — exit code {} != expected {}", outcome.exit_code, expected,
        )).into());
    }

    Ok(())
}

fn resolve_key_material(key_ref: &Option<String>, error_msg: &str) -> Result<KeyMaterial, TyuError> {
    let kr_str = key_ref.as_ref().ok_or_else(|| TyuError::Key(error_msg.to_string()))?;
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

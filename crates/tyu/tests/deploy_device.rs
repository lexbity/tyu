//! Tests for `tyu deploy --encrypt=device`.
//!
//! D-3: structural introspection (slot count matches device count).
//! D-4: device-key isolation via the real loader.
//! D-6: missing --device-keys errors.
//! D-7: empty --device-keys directory errors.

use std::path::PathBuf;
use std::process::Command;

use lmod::enc::EncMode;
use tyu::test_helpers::*;

use hosted::loader::HostedLoaderPlatform;
use loader_core::load::{load_module, LoadedSet, E_ENC_NO_KEY};
use loader_core::platform::TrustLevel;
use loader_core::symbols::SymMap;

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn ensure_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "langc",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-encrypt",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn create_device_keys(dir: &PathBuf, ids_and_keys: &[(&str, &[u8; 32])]) -> PathBuf {
    let keys_dir = dir.join("device-keys");
    std::fs::create_dir_all(&keys_dir).unwrap();
    for (id, key) in ids_and_keys {
        std::fs::write(keys_dir.join(format!("{}.key", id)), hex::encode(key)).unwrap();
    }
    keys_dir
}

/// Minimal loader setup for inspecting a signed .lmod.
fn load_with_kek(signed_path: &PathBuf, kek: &[u8; 32]) -> Result<(), u32> {
    let raw = std::fs::read(signed_path).unwrap();
    let container = lmod::validate::Container::parse(&raw).unwrap();
    let abi_hash = container.header().abi_hash;
    let bsize = (container.code().len() + 4095) & !4095;

    let mut plat = HostedLoaderPlatform::new(abi_hash)
        .with_key(&[0xab; 32], TrustLevel::One)
        .with_kek(kek);
    plat.reserve(bsize).map_err(|_| 1u32)?;

    let stub = stub_fn as *const () as usize;
    let ds_high = allocate_runtime_page();
    let mut map: SymMap<'_, 256> = SymMap::new();
    map.register(b"__stack_overflow", stub).unwrap();
    map.register(b"__lang_ds_high", ds_high).unwrap();
    map.register(b"__lang_trap", stub).ok();
    map.register(b"__lang_trap_loc", stub).ok();
    // PASS_MOD imports `testio.write-byte` (fnv1a_u64 = accb676a903a06d9);
    // stub it so symbol resolution completes for this load-only check.
    map.register(b"w_accb676a903a06d9", stub).ok();

    let mut set = LoadedSet::<64>::new();
    load_module(&container, &mut plat, &mut map, &mut set)?;
    Ok(())
}

pub extern "C" fn stub_fn() {}

fn allocate_runtime_page() -> usize {
    let page = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert_ne!(page, libc::MAP_FAILED, "mmap failed");
    page as usize
}

// ---------------------------------------------------------------------------
// D-3: Device-mode artifact has N slots matching device count
// ---------------------------------------------------------------------------

#[test]
fn deploy_device_produces_n_slots() {
    if !require_tools(&[
        "langc",
        "fasm",
        "ld",
        "lmod-pack",
        "lmod-encrypt",
        "lmod-sign",
    ]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("d3");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let keys_dir = create_device_keys(
        &dir,
        &[("device-a", &[0xaa; 32]), ("device-b", &[0xbb; 32])],
    );
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    // Signing requires an explicit --key-sign (deploy enforces this; --sign
    // alone is an error). Device-mode signing uses a fleet-wide sign key,
    // independent of the per-device KEKs in --device-keys.
    let sign_key = dir.join("sign.key");
    std::fs::write(&sign_key, hex::encode([0xcd; 32])).unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            &format!("--device-keys={}", keys_dir.display()),
            "--sign",
            &format!("--key-sign=file:{}", sign_key.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");
    assert!(
        output.status.success(),
        "deploy device failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let facts = introspect_lmod(&out_dir.join("deploy").join("signed.lmod"));
    assert!(facts.encrypted, "D-3: device artifact must be encrypted");
    assert!(facts.signed, "D-3: device artifact must be signed");
    assert_eq!(
        facts.enc_mode,
        Some(EncMode::Device),
        "D-3: enc_mode must be Device"
    );
    assert_eq!(
        facts.slot_count, 2,
        "D-3: must have 2 slots for 2 device keys"
    );
}

// ---------------------------------------------------------------------------
// D-4: Device-key isolation via the real loader
// ---------------------------------------------------------------------------
//
// Deploy device-mode for {a, b}.  Load with a's KEK → Ok (a is targeted).
// Load with c's KEK → E_ENC_NO_KEY (c has no slot in the artifact).

#[test]
fn deploy_device_isolation_via_loader() {
    if !require_tools(&[
        "langc",
        "fasm",
        "ld",
        "lmod-pack",
        "lmod-encrypt",
        "lmod-sign",
    ]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("d4");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let keys_dir = create_device_keys(
        &dir,
        &[("device-a", &[0xaa; 32]), ("device-b", &[0xbb; 32])],
    );
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    // Signing requires an explicit --key-sign (deploy enforces this; --sign
    // alone is an error). The sign key must match the loader's signature
    // verification key below (with_key(&[0xab; 32], TrustLevel::One)).
    let sign_key = dir.join("sign.key");
    std::fs::write(&sign_key, hex::encode([0xab; 32])).unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            &format!("--device-keys={}", keys_dir.display()),
            "--sign",
            &format!("--key-sign=file:{}", sign_key.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");
    assert!(
        output.status.success(),
        "deploy device failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let signed = out_dir.join("deploy").join("signed.lmod");

    // Loading with a's key must succeed.
    assert!(
        load_with_kek(&signed, &[0xaa; 32]).is_ok(),
        "D-4: device-a's KEK must decrypt the artifact"
    );

    // Loading with c's key (not in the device set) must fail.
    let result = load_with_kek(&signed, &[0xcc; 32]);
    assert_eq!(
        result.unwrap_err(),
        E_ENC_NO_KEY,
        "D-4: device-c's KEK must not decrypt device-a/b artifact"
    );
}

// ---------------------------------------------------------------------------
// D-6: Missing --device-keys errors
// ---------------------------------------------------------------------------

#[test]
fn deploy_device_missing_keysdir_errors() {
    if !require_tools(&[
        "langc",
        "fasm",
        "ld",
        "lmod-pack",
        "lmod-encrypt",
        "lmod-sign",
    ]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("d6");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            // Intentionally omit --device-keys.
            "--sign",
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");
    assert!(
        !output.status.success(),
        "D-6: deploy must fail without --device-keys"
    );
}

// ---------------------------------------------------------------------------
// D-7: Empty --device-keys directory errors
// ---------------------------------------------------------------------------

#[test]
fn deploy_device_empty_keysdir_errors() {
    if !require_tools(&[
        "langc",
        "fasm",
        "ld",
        "lmod-pack",
        "lmod-encrypt",
        "lmod-sign",
    ]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("d7");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let empty_keys = dir.join("empty-keys");
    std::fs::create_dir_all(&empty_keys).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            &format!("--device-keys={}", empty_keys.display()),
            "--sign",
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");
    assert!(
        !output.status.success(),
        "D-7: deploy must fail with empty keys dir"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no device keys found"),
        "D-7: stderr must mention empty keys dir, got: {}",
        stderr
    );
}

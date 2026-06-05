//! Tests for `tyu deploy` in Device mode.
//!
//! Creates mock device keys, deploys with device encryption,
//! and verifies that only provisioned devices can decrypt.

use std::path::PathBuf;
use std::process::Command;
use tyu::test_helpers::*;

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn ensure_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack", "-p", "lmod-encrypt", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn create_device_keys(dir: &PathBuf) -> PathBuf {
    let keys_dir = dir.join("device-keys");
    std::fs::create_dir_all(&keys_dir).unwrap();
    // Device A key
    std::fs::write(keys_dir.join("device-a.key"), hex::encode([0xaa; 32])).unwrap();
    // Device B key
    std::fs::write(keys_dir.join("device-b.key"), hex::encode([0xbb; 32])).unwrap();
    keys_dir
}

#[test]
fn deploy_device_qemu() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "lmod-pack", "lmod-encrypt", "lmod-sign"]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("deploy_device");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();

    let keys_dir = create_device_keys(&dir);
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    // Deploy targeting both devices.
    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            &format!("--device-keys={}", keys_dir.display()),
            "--sign",
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu deploy device mode failed:\n{}", stderr);
    }
}

#[test]
fn device_b_key_cannot_decrypt_a_artifact() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "lmod-pack", "lmod-encrypt", "lmod-sign"]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("cross_decrypt");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();

    // Create keys for device-a and device-b only.
    let keys_dir = dir.join("keys");
    std::fs::create_dir_all(&keys_dir).unwrap();
    std::fs::write(keys_dir.join("device-a.key"), hex::encode([0xaa; 32])).unwrap();
    std::fs::write(keys_dir.join("device-b.key"), hex::encode([0xbb; 32])).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let build_dir = dir.join("build");

    // Build: produce the .o and .lmod.
    let build_out = Command::new(tyu_exe())
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", build_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output().expect("tyu build");
    assert!(build_out.status.success(), "build failed");

    // Manually run lmod-encrypt with only device-a as the target.
    let lmod_obj = build_dir.join("main.o");
    let lmod_packed = dir.join("packed.lmod");
    assert!(Command::new(workspace_root().join("target").join("debug").join("lmod-pack"))
        .args([lmod_obj.to_str().unwrap(), lmod_packed.to_str().unwrap()])
        .status().unwrap().success());

    let lmod_enc = dir.join("encrypted.lmod");
    assert!(Command::new(workspace_root().join("target").join("debug").join("lmod-encrypt"))
        .args([
            lmod_packed.to_str().unwrap(), lmod_enc.to_str().unwrap(),
            "--mode=device",
            &format!("--device-keys={}", keys_dir.display()),
            "--devices=device-a",  // Only device-a
        ])
        .status().unwrap().success());

    let lmod_signed = dir.join("signed.lmod");
    assert!(Command::new(workspace_root().join("target").join("debug").join("lmod-sign"))
        .args([lmod_enc.to_str().unwrap(), lmod_signed.to_str().unwrap()])
        .status().unwrap().success());

    // Load with device-a's key — should succeed.
    let raw = std::fs::read(&lmod_signed).unwrap();
    let container = lmod::validate::Container::parse(&raw).unwrap();
    let abi_hash = container.header().abi_hash;

    let mut plat_a = hosted::loader::HostedLoaderPlatform::new(abi_hash)
        .with_key(&[0xab; 32], loader_core::platform::Tier::One)
        .with_kek(&[0xaa; 32]);
    plat_a.reserve(65536).unwrap();
    let ds_high_a = allocate_ds_page();
    let mut map_a = loader_core::symbols::SymMap::new();
    register_test_symbols(&mut map_a, ds_high_a);
    let mut set_a = loader_core::load::LoadedSet::<64>::new();
    assert!(
        loader_core::load::load_module(&container, &mut plat_a, &mut map_a, &mut set_a).is_ok(),
        "device-a should load its own artifact"
    );

    // Try with device-b's key — should fail with E_ENC_NO_KEY.
    let mut plat_b = hosted::loader::HostedLoaderPlatform::new(abi_hash)
        .with_key(&[0xab; 32], loader_core::platform::Tier::One)
        .with_kek(&[0xbb; 32]);
    plat_b.reserve(65536).unwrap();
    let ds_high_b = allocate_ds_page();
    let mut map_b = loader_core::symbols::SymMap::new();
    register_test_symbols(&mut map_b, ds_high_b);
    let mut set_b = loader_core::load::LoadedSet::<64>::new();
    let result_b = loader_core::load::load_module(&container, &mut plat_b, &mut map_b, &mut set_b);
    assert_eq!(
        result_b.unwrap_err(),
        loader_core::load::E_ENC_NO_KEY,
        "device-b should not decrypt device-a's artifact"
    );
}

fn allocate_ds_page() -> usize {
    let page = unsafe {
        libc::mmap(
            std::ptr::null_mut(), 4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0,
        )
    };
    assert_ne!(page, libc::MAP_FAILED, "mmap failed");
    page as usize
}

fn register_test_symbols<'a>(map: &mut loader_core::symbols::SymMap<'a, 256>, ds_high_addr: usize) {
    let stub = stub_fn as usize;
    map.register(b"__stack_overflow", stub).unwrap();
    map.register(b"__lang_trap", stub).ok();
    map.register(b"__lang_trap_loc", stub).ok();
    map.register(b"__lang_ds_high", ds_high_addr).unwrap();
}

pub extern "C" fn stub_fn() {}

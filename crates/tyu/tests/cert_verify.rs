//! Certification-package verification (PLAN-VERIFY-3 P11.3, FR-9): the
//! B4 → B1 → B2 → B3 fail-fast bindings, tested as tamper negatives.
//!
//! The package is assembled hermetically (a fabricated deploy out-dir + a
//! module container carrying a real `verify_manifest` record), then each
//! binding is attacked: mutate module bytes (B1 ⇒ E6503), tamper the
//! manifest digest + recompute B4 (B2 ⇒ E6503), tamper the index without
//! recomputing (B4 catches first). `show`/`diff` output is pinned.
//!
//! A tool-gated e2e additionally deploys a SIGNED image and checks
//! `package.sig` HMAC verification through the CLI.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest as _, Sha256};
use tyu::cert::{self, E_CERT_PAIRING};
use tyu::error::TyuError;
use tyu::test_helpers::*;

fn vm_summary(policy: &str, model: &str) -> String {
    format!(
        r#"{{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
           "target":"x86_64-unknown-none","model":"{model}","policy":"{policy}",
           "certifier":{{"class":"port","name":"lean","recognition":"tyu-port/lean/1"}},
           "candidate_ratio":0,
           "counts":{{"proof":1,"checked":0,"assumed":0,"open":0}},
           "obligations":[{{"id":"Main::main::subtype-range::0","id_hash":7,
             "status":"discharged","trust":"proof",
             "statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]}}"#
    )
}

fn module_container_with_manifest(summary: &str) -> Vec<u8> {
    let record = lmod_pack::verify::encode_verify_manifest(
        &lmod_pack::verify::verify_manifest_from_json(summary).unwrap(),
    )
    .unwrap();
    let modinfo_len = 8 + record.len();
    let layout = lmod::header::compute_layout(42, modinfo_len as u32, 0, 0, 0, 0, 0, 0);
    let mut buf = vec![0u8; layout.total_len as usize];
    lmod::header::encode_header(&mut buf, &layout);
    let mi = layout.modinfo_off as usize;
    buf[mi..mi + 8].copy_from_slice(b"MODIMAIN");
    buf[mi + 8..mi + modinfo_len].copy_from_slice(&record);
    buf
}

/// The hermetic deploy-out fixture (shared with `cert_assembly`).
fn fixture(label: &str) -> (PathBuf, PathBuf, Vec<u8>, String) {
    let dir = temp_dir(label);
    let summary = vm_summary("proven", "tyu.model/x86_64-unknown-none/1");
    let module_bytes = module_container_with_manifest(&summary);
    let image = dir.join("out").join("deploy").join("Main.lmod");
    std::fs::create_dir_all(image.parent().unwrap()).unwrap();
    std::fs::write(&image, &module_bytes).unwrap();
    std::fs::create_dir_all(dir.join("out/.tyu-verify")).unwrap();
    std::fs::write(dir.join("out/.tyu-verify/Main.vm.json"), &summary).unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n",
    )
    .unwrap();
    (dir, image, module_bytes, summary)
}

fn assemble(dir: &Path, image: &Path, module_bytes: &[u8]) -> PathBuf {
    // The graph resolver parses with the language frontend, whose deep parse
    // frames exceed the 2 MiB default test-thread stack (deploy.rs
    // convention) — run the body on a 64 MiB stack.
    let dir = dir.to_path_buf();
    let image = image.to_path_buf();
    let module_bytes = module_bytes.to_vec();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || assemble_body(&dir, &image, &module_bytes))
        .expect("spawn big-stack assembly thread")
        .join()
        .expect("assembly thread")
}

fn assemble_body(dir: &Path, image: &Path, module_bytes: &[u8]) -> PathBuf {
    let image_bytes = std::fs::read(image).unwrap();
    cert::assemble_for_deploy(&cert::AssembleInput {
        image,
        image_bytes: &image_bytes,
        module_bytes,
        out_dir: &dir.join("out"),
        input: &dir.join("Main.mod"),
        include_dirs: &[dir.to_path_buf()],
        sysroot: None,
        policy: tyu::args::DeployVerifyPolicy::Proven,
        sign_key: None,
    })
    .expect("assembly succeeds")
    .expect("package produced")
}

fn pairing_code(err: &TyuError) -> Option<u32> {
    match err {
        TyuError::VerifyPairing { code, .. } | TyuError::Cert { code, .. } => Some(*code),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tamper matrix
// ---------------------------------------------------------------------------

#[test]
fn mutated_module_bytes_fail_b1_with_e6503() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_b1");
    let pkg = assemble(&dir, &image, &module_bytes);

    // Attack: ship DIFFERENT module bytes than the package certifies.
    let mut tampered = image.clone();
    tampered.set_extension("tampered.lmod");
    let mut fake = vec![0u8; 64];
    fake.copy_from_slice(&module_bytes[..64]);
    fake[63] ^= 0x01;
    std::fs::write(&tampered, &fake).unwrap();

    let err = cert::verify_package(&pkg, Some(&tampered)).expect_err("B1 must fail");
    assert_eq!(pairing_code(&err), Some(E_CERT_PAIRING));
    assert!(err.to_string().contains("B1"), "err: {err}");
}

#[test]
fn missing_image_fails_b1_fail_closed() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_b1_missing");
    let pkg = assemble(&dir, &image, &module_bytes);
    // No image supplied and the sibling path does not exist.
    std::fs::remove_file(&image).ok();
    let err = cert::verify_package(&pkg, None).expect_err("B1 must fail without an image");
    assert_eq!(pairing_code(&err), Some(E_CERT_PAIRING));
    assert!(err.to_string().contains("B1"), "err: {err}");
}

#[test]
fn tampered_manifest_digest_fails_b2_even_with_recomputed_b4() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_b2");
    let pkg = assemble(&dir, &image, &module_bytes);

    // Attack: rewrite cert.json flipping the manifest_digest nibble and
    // RECOMPUTING package_digest (B4 passes; the module's signed record is
    // unchanged — so B2 must catch the lie).
    let mut idx = cert::parse_index(&std::fs::read(pkg.join("cert.json")).unwrap()).unwrap();
    idx.package_digest = None;
    let m = &mut idx.modules[0];
    let md = m.manifest_digest.clone().expect("B2 target present");
    let mut flipped = md.into_bytes();
    flipped[0] = if flipped[0] == b'0' { b'1' } else { b'0' };
    m.manifest_digest = Some(String::from_utf8(flipped).unwrap());
    idx.members.sort_by(|a, b| a.path.cmp(&b.path));
    idx.modules.sort_by(|a, b| a.name.cmp(&b.name));
    let digest = hex::encode(Sha256::digest(idx.canonical_pre_digest().as_bytes()));
    idx.package_digest = Some(digest);
    std::fs::write(pkg.join("cert.json"), idx.canonical()).unwrap();

    let err = cert::verify_package(&pkg, Some(&image)).expect_err("B2 must fail");
    assert_eq!(pairing_code(&err), Some(E_CERT_PAIRING));
    assert!(err.to_string().contains("B2"), "err: {err}");
}

#[test]
fn tampered_index_fails_b4_first() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_b4");
    let pkg = assemble(&dir, &image, &module_bytes);

    // Attack: flip a module count WITHOUT recomputing package_digest — B4
    // (checked first) must fail before B1/B2 are even reached.
    let mut text = std::fs::read_to_string(pkg.join("cert.json")).unwrap();
    text = text.replace("\"candidate_ratio\":0", "\"candidate_ratio\":1");
    std::fs::write(pkg.join("cert.json"), text).unwrap();

    let err = cert::verify_package(&pkg, Some(&image)).expect_err("B4 must fail first");
    assert_eq!(pairing_code(&err), Some(E_CERT_PAIRING));
    assert!(err.to_string().contains("B4"), "err: {err}");
}

#[test]
fn missing_member_fails_b3() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_b3");
    let pkg = assemble(&dir, &image, &module_bytes);

    let idx = cert::parse_index(&std::fs::read(pkg.join("cert.json")).unwrap()).unwrap();
    let member = &idx.members[0].path;
    std::fs::remove_file(pkg.join(member)).unwrap();

    let err = cert::verify_package(&pkg, Some(&image)).expect_err("B3 must fail");
    assert_eq!(pairing_code(&err), Some(E_CERT_PAIRING));
    assert!(err.to_string().contains("B3"), "err: {err}");
}

#[test]
fn intents_and_tcb_render_in_show() {
    let (dir, image, module_bytes, _) = fixture("cert_verify_show");
    let pkg = assemble(&dir, &image, &module_bytes);
    let shown = tyu::cert::show(&pkg).unwrap();
    // The shipped TCB boundary renders (§6.8).
    assert!(shown.contains("tcb (shipped boundary):"), "{shown}");
    assert!(shown.contains("T-C"), "{shown}");
    assert!(shown.contains("schema tyu.cert/v1"), "{shown}");
}

#[test]
fn diff_is_structural() {
    let (a_dir, a_image, a_bytes, _) = fixture("cert_diff_a");
    let a = assemble(&a_dir, &a_image, &a_bytes);

    // Identical content ⇒ identical packages are reported identical
    // (same module name + bytes + image name).
    let (b_dir, b_image, b_bytes, _) = fixture("cert_diff_b");
    let b = assemble(&b_dir, &b_image, &b_bytes);
    let same = tyu::cert::diff(&a, &b).unwrap();
    assert!(same.contains("identical"), "identical packages: {same}");

    // A structurally different package (different image name) is reported
    // with its `-`/`+` lines.
    let alt_image = b_dir.join("out/deploy/Alt.lmod");
    std::fs::copy(&b_image, &alt_image).unwrap();
    let b_alt = assemble(&b_dir, &alt_image, &b_bytes);
    let diff_lines = tyu::cert::diff(&a, &b_alt).unwrap();
    assert!(
        diff_lines.contains("- image ") || diff_lines.contains("+ image "),
        "{diff_lines}"
    );
    assert!(!diff_lines.contains("identical"), "{diff_lines}");
}

// ---------------------------------------------------------------------------
// Signed package (tool-gated e2e)
// ---------------------------------------------------------------------------

#[test]
fn signed_package_sig_verifies_with_the_right_key() {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        return;
    }
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "langc",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let dir = temp_dir("cert_signed_e2e");
    std::fs::write(
        dir.join("Main.mod"),
        "module Main;\nimport platform/testio { testio.write-byte };\n: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n",
    )
    .unwrap();
    let key = [0x42u8; 32];
    let key_file = dir.join("sign.key");
    std::fs::write(&key_file, hex::encode(key)).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let summary = dir.join("summary.json");
    std::fs::write(
        &summary,
        vm_summary("proven", "tyu.model/x86_64-unknown-none/1"),
    )
    .unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            "--sign",
            &format!("--key-sign=file:{}", key_file.display()),
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--verify-policy=proven",
            &format!("--verify-manifest={}", summary.display()),
            dir.join("Main.mod").to_str().unwrap(),
        ])
        .output()
        .expect("tyu deploy --sign");
    assert!(
        output.status.success(),
        "signed proven deploy must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let pkg = dir.join("out/deploy/signed.lmod.tyucert");
    assert!(pkg.join("package.sig").is_file(), "package.sig must exist");

    // Wrong key ⇒ B4 HMAC must fail.
    let wrong = dir.join("wrong.key");
    std::fs::write(&wrong, hex::encode([0x99u8; 32])).unwrap();
    let bad = tyu::cert::verify_package_signed(
        &pkg,
        Some(&dir.join("out/deploy/signed.lmod")),
        Some(&[0x99u8; 32]),
    )
    .expect_err("wrong key must fail B4");
    assert!(bad.to_string().contains("B4"), "bad: {bad}");

    // Right key ⇒ all four bindings pass.
    let lines = tyu::cert::verify_package_signed(
        &pkg,
        Some(&dir.join("out/deploy/signed.lmod")),
        Some(&key),
    )
    .expect("right key verifies");
    assert!(lines.iter().any(|l| l.starts_with("PASS B4: package.sig")));
    assert!(lines.iter().any(|l| l.starts_with("PASS B1")));
}

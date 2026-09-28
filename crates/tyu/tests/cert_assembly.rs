//! Certification-package assembly (PLAN-VERIFY-3 P11.3): determinism + the
//! content-addressed member surface.
//!
//! Two tiers:
//! - **Hermetic** (always runs): a fabricated deploy out-dir + module
//!   container exercises `tyu::cert::assemble_for_deploy` end-to-end —
//!   B1/B2/F4 bindings, member digests, and *byte-identical* re-assembly
//!   (determinism, FR-16).
//! - **Tool-gated e2e** (runs when the toolchain is present): two real
//!   `tyu deploy` runs over the same source yield byte-identical `cert.json`
//!   indexes; the module bound by B1 is the deployed image.
//!
//! The dependency the package binds against is the same one the deploy
//! pairing gate and `tyu cert verify` consume: B4 → B1 → B2 → B3.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest as _, Sha256};
use tyu::args::CertArgs;
use tyu::cert::{self, CertIndex, CERT_SCHEMA, E_CERT_INDEX_MALFORMED};
use tyu::test_helpers::*;

// ---------------------------------------------------------------------------
// Hermetic fixtures
// ---------------------------------------------------------------------------

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

/// Build a minimal valid .lmod container whose `.lang.modinfo` tail carries a
/// real `verify_manifest` record for the given summary. `Container::parse` +
/// `scan_verify_manifest` accept it — the B2 binding source.
fn module_container_with_manifest(summary: &str) -> Vec<u8> {
    let spec = lmod_pack::verify::verify_manifest_from_json(summary).unwrap();
    let record = lmod_pack::verify::encode_verify_manifest(&spec).unwrap();
    let modinfo_len = 8 + record.len(); // "MODIMAIN" prefix + trailing record
    let layout = lmod::header::compute_layout(42, modinfo_len as u32, 0, 0, 0, 0, 0, 0);
    let mut buf = vec![0u8; layout.total_len as usize];
    lmod::header::encode_header(&mut buf, &layout);
    let mi_start = layout.modinfo_off as usize;
    buf[mi_start..mi_start + 8].copy_from_slice(b"MODIMAIN");
    buf[mi_start + 8..mi_start + modinfo_len].copy_from_slice(&record);
    buf
}

/// Build the deploy-out-dir shape the assembler reads (`<out>/.tyu-verify/`,
/// the image, the packed-module bytes, the input source + graph).
struct Fixture {
    dir: PathBuf,
    image: PathBuf,
    module_bytes: Vec<u8>,
}

fn fixture(label: &str) -> Fixture {
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
    Fixture {
        dir,
        image,
        module_bytes,
    }
}

fn assemble(fx: &Fixture) -> PathBuf {
    // The graph resolver parses with the language frontend, whose deep parse
    // frames exceed the 2 MiB default test-thread stack (same convention as
    // deploy.rs's pairing test) — run the body on a 64 MiB stack.
    let dir = fx.dir.clone();
    let image = fx.image.clone();
    let module_bytes = fx.module_bytes.clone();
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
    .expect("assembly produces a package (state present)")
}

fn read_index(pkg: &Path) -> CertIndex {
    let bytes = std::fs::read(pkg.join("cert.json")).unwrap();
    cert::parse_index(&bytes).expect("index parses")
}

// ---------------------------------------------------------------------------
// Hermetic tests
// ---------------------------------------------------------------------------

#[test]
fn assembly_is_deterministic_and_binds_members() {
    let fx = fixture("cert_asm_determinism");
    let pkg1 = assemble(&fx);
    let pkg2 = assemble(&fx);

    // Determinism (FR-16): byte-identical canonical index across runs.
    let idx1_bytes = std::fs::read(pkg1.join("cert.json")).unwrap();
    let idx2_bytes = std::fs::read(pkg2.join("cert.json")).unwrap();
    assert_eq!(
        idx1_bytes, idx2_bytes,
        "cert.json must be byte-deterministic"
    );
    assert_eq!(pkg1, pkg2, "the package dir is deterministic");

    let idx = read_index(&pkg1);
    assert_eq!(idx.schema, CERT_SCHEMA);
    assert_eq!(idx.modules.len(), 1);
    let m = &idx.modules[0];
    assert_eq!(m.name, "Main");
    // B1: binds the shipped image bytes.
    let image_bytes = std::fs::read(&fx.image).unwrap();
    assert_eq!(
        m.module_digest.as_deref(),
        Some(hex::encode(Sha256::digest(&image_bytes)).as_str())
    );
    // B2: binds the module's verify_manifest verdict digest.
    let container = lmod::validate::Container::parse(&fx.module_bytes).unwrap();
    let vm = lmod::verify_manifest::scan_verify_manifest(container.modinfo())
        .unwrap()
        .expect("manifest present");
    assert_eq!(
        m.manifest_digest.as_deref(),
        Some(hex::encode(vm.digest).as_str())
    );

    // Members are content-addressed: every member is recorded with its real
    // digest + size.
    for member in &idx.members {
        let bytes = std::fs::read(pkg1.join(&member.path)).expect("member written");
        assert_eq!(bytes.len() as u64, member.size);
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            member.sha256,
            "member {} not content-addressed",
            member.path
        );
    }
    // The evidence surface ships: axiom audit, candidates, tcb, toolchain,
    // conformance, differential.
    for want in [
        "evidence/tcb.json",
        "evidence/candidates.json",
        "evidence/axiom_audit.json",
        "evidence/toolchain.lock",
        "evidence/conformance.json",
        "evidence/differential.json",
    ] {
        assert!(
            idx.members.iter().any(|m| m.path == want),
            "missing evidence member {want}"
        );
        assert!(pkg1.join(want).is_file(), "evidence file {want} missing");
    }
}

#[test]
fn package_verifies_after_assembly() {
    let fx = fixture("cert_asm_verify");
    let pkg = assemble(&fx);
    let lines = cert::verify_package(&pkg, Some(&fx.image)).expect("verify passes");
    assert!(lines.iter().any(|l| l.starts_with("PASS B4")));
    assert!(lines.iter().any(|l| l.starts_with("PASS B1")));
    assert!(lines.iter().any(|l| l.starts_with("PASS B2")));
    assert!(lines.iter().any(|l| l.starts_with("PASS B3")));
}

#[test]
fn show_and_diff_render() {
    let fx = fixture("cert_asm_show");
    let pkg = assemble(&fx);
    let shown = tyu::cert::show(&pkg).expect("show renders");
    assert!(shown.contains("certificate package:"));
    assert!(shown.contains("Main: proof 1"));
    assert!(shown.contains("members:"));
    assert!(shown.contains("schema tyu.cert/v1"));

    let other = fixture("cert_asm_show_other");
    let pkg_b = assemble(&other);
    let d = tyu::cert::diff(&pkg, &pkg_b).expect("diff renders");
    assert!(d.contains("identical") || d.contains("+ ") || d.contains("- "));
}

#[test]
fn parser_fails_closed_on_untrusted_index() {
    // The E6504 index-malformed class, exercised through the public API.
    for bad in [
        b"".as_slice(),
        b"not json",
        b"[]",
        b"{}",
        b"{\"schema\":\"tyu.cert/v9\"}",
    ] {
        let err = cert::parse_index(bad).expect_err("must fail");
        assert_eq!(err.code(), E_CERT_INDEX_MALFORMED);
    }
}

// ---------------------------------------------------------------------------
// Tool-gated e2e: two real deploys are byte-identical
// ---------------------------------------------------------------------------

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn deploy(dir: &Path) -> bool {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        return true;
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
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let summary = dir.join("summary.json");
    std::fs::write(
        &summary,
        r#"{"schema":"tyu.vm/1","semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0",
           "target":"x86_64-unknown-none","model":"tyu.model/x86_64-unknown-none/1",
           "policy":"proven","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1"},
           "candidate_ratio":0,"counts":{"proof":1,"checked":0,"assumed":0,"open":0},"obligations":[]}"#,
    )
    .unwrap();
    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            // P12 (§Q15): `proven` requires a modeled bundle — the deploy's
            // build resolves the now-modeled x86_64-unknown-none pack so the
            // module records the pack's model id (E6510 else).
            "--platform=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--verify-policy=proven",
            &format!("--verify-manifest={}", summary.display()),
            dir.join("Main.mod").to_str().unwrap(),
        ])
        .output()
        .expect("tyu deploy");
    if output.status.success() {
        return true;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("deploy failed:\n{text}");
    false
}

#[test]
fn two_deploys_produce_byte_identical_cert_index() {
    if !require_tools(&["langc", "fasm", "ld", "lmod-pack", "lmod-sign"]) {
        return;
    }
    let dir = temp_dir("cert_e2e_det");
    std::fs::write(dir.join("Main.mod"), PASS_MOD).unwrap();
    assert!(deploy(&dir), "first deploy must succeed");

    let pkg = dir.join("out/deploy/signed.lmod.tyucert");
    assert!(
        pkg.join("cert.json").is_file(),
        "cert.json exists after deploy"
    );

    // A second deploy over the same source yields a byte-identical index.
    // (The out dir is reused — deterministic re-assembly, FR-16.)
    let first = std::fs::read(pkg.join("cert.json")).unwrap();
    assert!(deploy(&dir), "second deploy must succeed");
    let second = std::fs::read(pkg.join("cert.json")).unwrap();
    assert_eq!(
        first, second,
        "cert.json must be byte-identical across deploys"
    );

    // The package binds the shipped module: B1 digest == the deployed image.
    let idx = read_index(&pkg);
    let shipped = std::fs::read(dir.join("out/deploy/signed.lmod")).unwrap();
    let m = idx
        .modules
        .iter()
        .find(|m| m.name == "Main")
        .expect("Main entry");
    assert_eq!(
        m.module_digest.as_deref(),
        Some(hex::encode(Sha256::digest(&shipped)).as_str())
    );

    // And `tyu cert verify` passes end-to-end through the binary.
    let out = Command::new(tyu_exe())
        .args([
            "cert",
            "verify",
            pkg.to_str().unwrap(),
            &format!("--image={}", dir.join("out/deploy/signed.lmod").display()),
        ])
        .output()
        .expect("tyu cert verify");
    assert!(
        out.status.success(),
        "cert verify must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("PASS B4"), "{text}");
    assert!(text.contains("PASS B1"), "{text}");
}

/// `CertArgs` + `cert::run` dispatch (the CLI surface is exercised through
/// the binary above; this pins the enum path used by main.rs).
#[test]
fn cert_cli_verify_default_image_resolves_sibling() {
    let fx = fixture("cert_cli_default_image");
    let pkg = assemble(&fx);
    // Delete the explicit image and rely on the default sibling resolution:
    // the image path is `pkg` minus `.tyucert` — which is the fixture image.
    let run = tyu::cert::run(&CertArgs::Verify {
        pkg,
        image: None,
        key_sign: None,
    });
    assert!(run.is_ok(), "default image resolution must verify");
}

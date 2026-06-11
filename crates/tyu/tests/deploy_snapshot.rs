//! Deploy pipeline snapshot test.
//!
//! P0.6 — Records current pipeline behavior (build → pack → encrypt → sign)
//! using an explicitly-supplied key.  Tests the artifact pipeline without
//! QEMU execution (QEMU run is a pre-existing breakage tracked separately).
//!
//! Guards Phases 5–11 (B3/B4 tool rewiring) against regressions.

use std::path::PathBuf;
use std::process::Command;

use lmod::enc::EncMode;
use lmod::validate::Container;
use tyu::test_helpers::*;

const PASS_MOD: &str = "\
module Main;\n\
: main ( -- i64 ) 42 ;\nexport { main };\nend;\n";
const KEK_HEX: &str = "abababababababababababababababababababababababababababababababab";
const SIGN_KEY_HEX: &str = "abababababababababababababababababababababababababababababababab";

fn tool(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

#[test]
fn deploy_pipeline_snapshot() {
    // Build required tools.
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack", "-p", "lmod-encrypt", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let dir = temp_dir("deploy_snapshot");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    // --- Step 1: Build ---
    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output().expect("tyu build");
    assert!(output.status.success(),
        "tyu build failed:\n{}", String::from_utf8_lossy(&output.stderr));

    // Find the Main.o produced (not runtime.o).
    let main_o = out_dir.join("Main.o");
    assert!(main_o.exists(), "build must produce Main.o at {:?}", main_o);

    // --- Step 2: Pack ---
    let packed = dir.join("packed.lmod");
    let status = Command::new(tool("lmod-pack"))
        .args([main_o.to_string_lossy().as_ref(), packed.to_string_lossy().as_ref()])
        .status().expect("lmod-pack");
    assert!(status.success(), "lmod-pack failed");

    // --- Step 3: Encrypt (fleet mode with explicit key) ---
    let encrypted = dir.join("encrypted.lmod");
    let status = Command::new(tool("lmod-encrypt"))
        .args([
            packed.to_string_lossy().as_ref(),
            encrypted.to_string_lossy().as_ref(),
            "--mode=fleet",
            &format!("--kek={}", KEK_HEX),
        ])
        .status().expect("lmod-encrypt");
    assert!(status.success(), "lmod-encrypt failed");

    // --- Step 4: Sign (with explicit key) ---
    let signed = dir.join("signed.lmod");
    let status = Command::new(tool("lmod-sign"))
        .args([
            encrypted.to_string_lossy().as_ref(),
            signed.to_string_lossy().as_ref(),
            &format!("--key={}", SIGN_KEY_HEX),
        ])
        .status().expect("lmod-sign");
    assert!(status.success(), "lmod-sign failed");

    // --- Structural assertions on final artifact ---
    let data = std::fs::read(&signed).unwrap();
    let container = Container::parse(&data).unwrap();
    let hdr = container.header();

    assert_ne!(hdr.flags & lmod::header::LMOD_FLAG_ENCRYPTED, 0,
        "deploy snapshot: ENCRYPTED flag must be set");
    assert_ne!(hdr.flags & lmod::header::LMOD_FLAG_SIGNED, 0,
        "deploy snapshot: SIGNED flag must be set");
    assert_eq!(hdr.format_ver, lmod::header::FORMAT_VER,
        "deploy snapshot: format_ver must be {}", lmod::header::FORMAT_VER);

    // Check enc header.
    let eh_bytes = &data[lmod::header::HEADER_SIZE as usize..];
    let eh = lmod::enc::decode_enc_header(eh_bytes)
        .expect("deploy snapshot: valid enc-header");
    assert_eq!(eh.enc_mode, EncMode::Fleet,
        "deploy snapshot: enc_mode must be Fleet");
    assert_eq!(eh.wrapped_slots.len(), 1,
        "deploy snapshot: fleet mode must have 1 slot");

    // Check signature trailer.
    let sig_trailer = lmod::sig::SigTrailer::parse(&data[hdr.sig_off as usize..]);
    assert!(sig_trailer.is_some(), "deploy snapshot: signed module must have a valid SigTrailer");
    let trailer = sig_trailer.unwrap();
    assert_eq!(trailer.scheme, lmod::sig::SCHEME_HMAC_SHA256,
        "deploy snapshot: signature scheme must be HMAC-SHA256");
}

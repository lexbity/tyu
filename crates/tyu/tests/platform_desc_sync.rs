//! Compiled-descriptor sync (P3): every discovered platform pack's committed
//! `platform.desc` must match its `platform.toml` descriptor.
//!
//! `langc --platform=<dir>` reads `<dir>/platform.desc` for MMIO window facts,
//! so a stale compiled form would silently source wrong windows. This test
//! re-compiles each descriptor (validating it — E3646/E3647) and asserts the
//! on-disk file matches; `ensure_compiled_descriptor` rewrites it when stale,
//! making this test the generator for the committed artifacts.

use std::fs;
use std::path::PathBuf;

use tyu::platform::desc::compile::ensure_compiled_descriptor;
use tyu::platform::discover_platforms_in;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn every_pack_compiled_descriptor_is_in_sync() {
    let root = workspace_root();
    let packs = discover_platforms_in(&root).unwrap();

    let mut checked = 0usize;
    for pack in &packs {
        // A pack with a descriptor v2 must compile (E3646/E3647 are the
        // compile-stop signals) and must have a synced compiled form. The
        // v1 pack lint is intentionally not consulted here: it reports
        // pack-structure facts (e.g. the hosted runtime's symbol set) that
        // are orthogonal to descriptor validity.
        let has_descriptor = tyu::platform::desc::load_descriptor(&root, pack.name())
            .unwrap()
            .is_some();
        if !has_descriptor {
            continue;
        }

        // ensure_compiled_descriptor validates + rewrites if stale; reading it
        // back must decode and validate (sync proof).
        let compiled = ensure_compiled_descriptor(&pack.manifest_path, pack.pack_root())
            .unwrap_or_else(|e| panic!("compile {}: {e}", pack.name()));
        let desc_path = tyu::platform::descriptor_file_path(pack.pack_root());
        let bytes = fs::read(&desc_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", desc_path.display()));
        let decoded = tyu::platform::desc::compile::decode_and_validate(&bytes)
            .unwrap_or_else(|e| panic!("decode {}: {e}", desc_path.display()));
        assert_eq!(
            decoded.platform_hash, compiled.platform_hash,
            "{} compiled descriptor hash drifted from its platform.toml",
            pack.name()
        );
        assert_eq!(
            decoded.windows(), compiled.windows(),
            "{} compiled descriptor windows drifted from its platform.toml",
            pack.name()
        );
        checked += 1;
    }

    assert!(
        checked >= 5,
        "expected the 5 in-repo packs to carry a synced compiled descriptor, checked {checked}"
    );
}

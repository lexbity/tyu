//! Every in-tree platform pack must compile against the *current* compiled
//! descriptor format, and the committed `<pack>/platform.desc` binaries must
//! match a fresh compile byte-for-byte.
//!
//! This is the regeneration + drift gate for the checked-in `.desc` files:
//! a format change (e.g. the amended §6.4 dropping the declared `N_main`
//! grant, format v6) makes the stale committed artifacts fail here until they
//! are re-blessed, so a producer/consumer drift on the descriptor can never
//! merge silently. Run with TYU_BLESS_PACK_DESCRIPTORS=1 to rewrite the
//! committed files after a deliberate format change (review the diff).

use std::fs;
use std::path::PathBuf;

use codegen_core::compiled_desc::{encode_compiled_desc, decode_compiled_desc};
use tyu::platform::desc::{ensure_compiled_descriptor, descriptor_file_path};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The in-tree packs: four metal boards plus the hosted runtime pack. Each
/// entry is (pack root, manifest-relative path).
fn packs() -> Vec<(PathBuf, &'static str)> {
    vec![
        ("platforms/x86_64-unknown-none", "platform.toml"),
        ("platforms/armv7m-unknown-none", "platform.toml"),
        ("platforms/riscv32-unknown-none", "platform.toml"),
        ("platforms/rp2350", "platform.toml"),
        ("runtime", "linux-x86_64-hosted.platform.toml"),
    ]
    .into_iter()
    .map(|(dir, manifest)| (workspace_root().join(dir), manifest))
    .collect()
}

#[test]
fn every_in_tree_pack_compiles_and_committed_desc_is_current() {
    let bless = std::env::var("TYU_BLESS_PACK_DESCRIPTORS").as_deref() == Ok("1");
    for (pack_root, manifest) in packs() {
        let manifest_path = pack_root.join(manifest);
        let compiled = ensure_compiled_descriptor(&manifest_path, &pack_root)
            .unwrap_or_else(|e| panic!("pack '{}' must compile: {}", pack_root.display(), e));

        let mut buf = Vec::new();
        buf.resize(codegen_core::compiled_desc::COMPILED_DESC_MAX_BYTES, 0);
        let n = encode_compiled_desc(&compiled, &mut buf).expect("encode");
        let fresh = &buf[..n];

        let desc_path = descriptor_file_path(&pack_root);
        if bless {
            fs::write(&desc_path, fresh).expect("bless committed descriptor");
            continue;
        }
        let committed = fs::read(&desc_path).unwrap_or_else(|e| {
            panic!(
                "committed descriptor '{}' missing: {} (bless with TYU_BLESS_PACK_DESCRIPTORS=1)",
                desc_path.display(),
                e
            )
        });
        assert_eq!(
            committed, fresh,
            "committed '{}' is stale vs the current descriptor format; \
             re-bless with TYU_BLESS_PACK_DESCRIPTORS=1 and review the diff",
            desc_path.display()
        );
        // The committed bytes must also decode back to the same model.
        let decoded = decode_compiled_desc(&committed).expect("committed descriptor decodes");
        assert_eq!(decoded, compiled, "committed descriptor roundtrips");
    }
}

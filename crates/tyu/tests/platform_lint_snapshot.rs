//! Snapshot of `tyu platform lint rp2350` — the descriptor model review
//! artifact (design doc §5.11, decision D-14).
//!
//! The snapshot is the concatenation of the pack-lint outcome and the
//! descriptor report. When the rp2350 descriptor changes (P8 fills the
//! datasheet tables), regenerate the golden deliberately and review the diff:
//! every line is either a model fact or the `platform-hash`.

use std::path::PathBuf;

use tyu::platform::desc::report::format_descriptor_report;
use tyu::platform::{format_lint_outcome, lint_pack};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn rp2350_lint_output_matches_golden() {
    let root = workspace_root();
    let outcome = lint_pack(&root, "rp2350", false).unwrap();
    assert!(
        outcome.errors.is_empty(),
        "rp2350 must lint clean: {}",
        format_lint_outcome(&outcome)
    );

    let mut text = format_lint_outcome(&outcome);
    let desc = tyu::platform::desc::load_descriptor(&root, "rp2350")
        .unwrap()
        .expect("rp2350 carries a descriptor");
    text.push_str(&format_descriptor_report(&desc));

    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/platform_lint_rp2350.txt");
    if std::env::var_os("TYU_BLESS_LINT_SNAPSHOT").is_some() {
        std::fs::write(&golden_path, &text).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&golden_path).unwrap();
    assert_eq!(
        text, golden,
        "rp2350 platform lint output drifted from the snapshot golden"
    );
}
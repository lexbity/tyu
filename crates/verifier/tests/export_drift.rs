//! Port generated-data drift lock (PLAN-VERIFY-3 P3.1, FR-12).
//!
//! The committed generated files under `verification/ports/<port>/` MUST be
//! byte-identical to what `verifier::export` renders — the generator edge is
//! test-time and drift-locked, never build-time. A semantics-table change
//! (new row / changed row) that is not mirrored in the renderer flips this
//! gate; a renderer change (e.g. a Lean-format tweak) requires a *reviewed*
//! regeneration with `TYU_EXPORT_PORTS=1`.
//!
//! Negative control (run once, per §P0): perturb `op_ctor`'s output for one
//! mnemonic → this test MUST fail; revert.
//!
//! Determinism (FR-16): twice-rendered output is byte-equal.

use std::fs;
use std::path::PathBuf;

use verifier::export::render_all;

/// The port tree root relative to the workspace: `verification/ports`.
fn ports_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("verification/ports")
}

/// Render determinism: two passes are byte-identical (FR-16).
#[test]
fn export_rendering_is_deterministic() {
    let a = render_all();
    let b = render_all();
    assert_eq!(a.len(), b.len(), "port count must be stable");
    for ((pa, fa), (pb, fb)) in a.iter().zip(b.iter()) {
        assert_eq!(pa, pb, "port order must be stable");
        assert_eq!(fa.len(), fb.len(), "{}: file count unstable", pa);
        for (x, y) in fa.iter().zip(fb.iter()) {
            assert_eq!(x.path, y.path, "{}: file set unstable", pa);
            assert_eq!(
                x.content, y.content,
                "{}: render of {} is nondeterministic (FR-16)",
                pa, x.path
            );
        }
    }
}

/// Every row of the semantics table is rendered into the Lean layer: the
/// drift pin closes the "renderer forgot a row" class (the compile-time
/// wildcard-free `op_ctor` match is the structural gate; this test asserts
/// the *content* makes it into the file).
#[test]
fn export_covers_all_semantics_rows() {
    for (port, files) in render_all().iter() {
        for f in files.iter() {
            if f.path.ends_with("Semantics.lean") {
                for row in verifier::semantics::semantics().iter() {
                    assert!(
                        f.content.contains(&format!("\"{}\"", row.mnemonic)),
                        "{}: {} missing mnemonic '{}' — renderer drifted from the table",
                        port,
                        f.path,
                        row.mnemonic
                    );
                    assert!(
                        f.content.contains(
                            verifier::export::lean::op_ctor(row.mnemonic)
                                .expect("coverage: every row must have a registry entry"),
                        ),
                        "{}: {} missing constructor for '{}' — renderer drifted from the table",
                        port,
                        f.path,
                        row.mnemonic
                    );
                }
            }
        }
    }
}

/// The committed files must byte-match regeneration (the golden gate). With
/// `TYU_EXPORT_PORTS=1` the committed files are rewritten (a reviewed update).
#[test]
fn export_files_match_regeneration() {
    let regen = std::env::var("TYU_EXPORT_PORTS").is_ok();
    for (port, files) in render_all().iter() {
        for f in files.iter() {
            let path = ports_root().join(port).join(f.path);
            if regen {
                fs::create_dir_all(path.parent().unwrap()).expect("create port dir");
                fs::write(&path, &f.content).expect("write generated port file");
            } else {
                let actual = fs::read_to_string(&path).expect(
                    "committed generated file present (run with TYU_EXPORT_PORTS=1 to regenerate)",
                );
                assert_eq!(
                    actual,
                    f.content,
                    "{} drifted from the exporter — regenerate + review ({})",
                    path.display(),
                    path.display()
                );
            }
        }
    }
    if regen {
        eprintln!("TYU_EXPORT_PORTS=1: regenerated all port generated files");
    }
}

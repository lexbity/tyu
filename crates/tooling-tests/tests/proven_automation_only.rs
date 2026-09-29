//! The automation-only `proven` build (PLAN-VERIFY-3 P14.2, §Q12): a module
//! whose obligations are discharged ENTIRELY by the port's `rederive` method
//! (`trust: proof, method: rederive`, T-B-backed) builds under
//! `--verify-policy=proven` with no developer proofs — the concrete form of
//! "port re-derivation upgrades the interval engine's discharges from
//! `checked` to `proof` without developer effort".
//!
//! Flow: `langc --emit=obligations` produces the artifact; the `rederive`
//! exe re-runs the abstract interpreter `Tyu.Abs` over it and writes a
//! `tyu.verdicts/v2` document (`method: rederive`, statement-bound); langc
//! consumes it under `proven` and must close every site. The negative arms
//! prove the gate: without the rederive file (the in-tree interval
//! discharge alone, `checked`/`interval`) `proven` FORCES the sites open —
//! the interval engine is the automation layer, and `proven` is honest
//! about the gap until re-derivation confirms it.
//!
//! Tier-A style: needs `lean`/`lake` + the built `rederive` exe
//! (`TYU_REDERIVE_E2E=1` forces a loud failure when absent; `ci/port.sh`
//! and `ci/differential.sh` run it with the pinned toolchain).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn toolchain_present() -> bool {
    let lean = Command::new("lean").arg("--version").output();
    let lake = Command::new("lake").arg("--version").output();
    match (lean, lake) {
        (Ok(l), Ok(k)) => l.status.success() && k.status.success(),
        _ => false,
    }
}

fn fresh_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_proven_auto")
        .join(format!("{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A module whose subtype-range obligations are input-free constants: Lang
/// / the interval engine *can* discharge them (`checked`/`interval`), which
/// `proven` rejects — and the re-derivation must re-prove them as `proof`.
const MOD: &str = "\
module Auto;
subtype Percent = i64 range 0..100;
: const_ok ( -- Percent ) 50 as Percent ;
: thousand ( -- i64 ) 1000 ;
: main ( -- i64 ) const_ok as i64 ;
export { main };
end;
";

/// The empty verdicts document (the negative arm uses `--verdicts` with an
/// empty file so both arms share the same consumption path).
const EMPTY: &str = "{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[]}";

fn pass1(dir: &Path) -> PathBuf {
    let p1 = dir.join("p1");
    fs::create_dir_all(&p1).unwrap();
    let s = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", p1.display()))
        .arg(dir.join("Auto.mod").to_str().unwrap())
        .status()
        .unwrap();
    assert!(s.success(), "pass-1 extraction failed");
    p1.join("Auto.obl.json")
}

/// Run the port's `rederive` exe over the artifact → the verdicts file.
fn rederive_verdicts(root: &Path, dir: &Path, artifact: &Path) -> PathBuf {
    let exe = root.join("verification/ports/lean/.lake/build/bin/rederive");
    let vf = dir.join("rederive.v2.json");
    let s = Command::new(&exe)
        .arg("--obl")
        .arg(artifact)
        .arg("--out")
        .arg(&vf)
        .arg("--toolchain")
        .arg("lean4:4.27.0+port")
        .status()
        .unwrap();
    assert!(
        s.success(),
        "rederive --obl must produce the verdicts document (T-B-backed)"
    );
    vf
}

/// Compile under `--verify-policy=proven` with the given verdicts doc.
fn compile_proven(dir: &Path, verdicts: Option<&Path>, bind_obl: Option<&Path>) -> (bool, String) {
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg("--checks=undischarged")
        .arg("--verify-policy=proven");
    if let Some(v) = verdicts {
        cmd.arg(format!("--verdicts={}", v.display()));
    } else {
        let vf = dir.join("empty.json");
        fs::write(&vf, EMPTY).unwrap();
        cmd.arg(format!("--verdicts={}", vf.display()));
    }
    if let Some(b) = bind_obl {
        cmd.arg(format!("--bind-obl={}", b.display()));
    }
    cmd.arg(dir.join("Auto.mod").to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn read_echo(out_dir: &Path) -> verifier::verdict::Echo {
    verifier::verdict::read_echo(&fs::read(out_dir.join("Auto.verdicts.inTree.json")).unwrap())
        .expect("echo must parse")
}

#[test]
fn proven_build_from_rederive_alone() {
    let root = workspace_root();
    if !toolchain_present() {
        if std::env::var("TYU_REDERIVE_E2E").is_ok() {
            panic!("TYU_REDERIVE_E2E=1 requires `lean`/`lake` and the built rederive exe");
        }
        eprintln!(
            "skipping the automation-only proven e2e (no Lean toolchain on PATH; \
             ci/differential.sh runs it with the pinned toolchain)"
        );
        return;
    }
    let dir = fresh_dir();
    fs::write(dir.join("Auto.mod"), MOD).unwrap();

    // Negative arm: no re-derivation — the interval engine's `checked`
    // discharges are NOT `proven`-admissible, so every subtype-range site is
    // FORCED OPEN with the not-admitted reason and its check is retained
    // (§Q12; `proven` is honest about the gap).
    let artifact = pass1(&dir);
    let (ok_neg, _) = compile_proven(&dir, None, Some(&artifact));
    assert!(
        ok_neg,
        "forced-open sites keep the checks; the build compiles"
    );
    let neg_echo = read_echo(&dir.join("out"));
    let admitted = neg_echo
        .open_reasons
        .iter()
        .filter(|o| o.reason.contains("policy-proven"))
        .count();
    assert!(
        admitted >= 2,
        "every interval discharge must be forced open under proven without \
         re-derivation: {:?}",
        neg_echo.open_reasons
    );
    assert!(
        neg_echo.emitted.subtype_range > 0,
        "checks retained at every forced-open site"
    );

    // The rederive verdicts: discharged sites closed as `proof`/`rederive`.
    let vf = rederive_verdicts(&root, &dir, &artifact);
    let doc = fs::read_to_string(&vf).unwrap();
    assert!(
        doc.contains("\"method\":\"rederive\"") && doc.contains("\"trust\":\"proof\""),
        "the rederive verdicts must be proof-class rederive records:\n{doc}"
    );

    // Positive arm: with re-derivation, the automation-only `proven` build
    // closes every site — no developer proofs, no retained checks.
    let (ok_pos, stderr_pos) = compile_proven(&dir, Some(&vf), Some(&artifact));
    assert!(
        ok_pos,
        "proven build from rederive alone must succeed:\n{stderr_pos}"
    );
    let echo = read_echo(&dir.join("out"));
    assert_eq!(
        echo.emitted.subtype_range, 0,
        "re-derivation elides every subtype-range check"
    );
    let discharged = echo
        .verdicts
        .records
        .iter()
        .filter(|r| r.status == verifier::verdict::VerdictStatus::Discharged)
        .count();
    assert_eq!(
        discharged, 2,
        "both fixture obligations discharge via re-derivation"
    );
    for r in &echo.verdicts.records {
        assert_eq!(
            r.trust,
            verifier::verdict::Trust::Proof,
            "every closed record is proof-class (T-B-backed): {:?}",
            r
        );
        assert_eq!(
            r.method,
            Some(verifier::verdict::Method::Rederive),
            "re-derivation keeps its method identity: {:?}",
            r
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

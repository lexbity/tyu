//! Verdict-driven emission tests (static-verification.md slice P4, FR-5/FR-16):
//!
//! - under `--checks=undischarged` langc emits a runtime check exactly and
//!   only at obligation sites whose verdict is neither `discharged` nor
//!   `assumed`. A discharged subtype-range site's check provably disappears
//!   from the object (the .asm text is the bijection witness — the plan's
//!   P4 exit criterion counts trap sites in asm);
//! - `--checks=undischarged` without `--verdicts` is E6402 (fail-closed);
//! - a verdict record whose `id_hash` disagrees with the compiler's
//!   computation is ignored and counted stale — the site stays open and the
//!   check is retained (Q3 fail-closed);
//! - the verdicts echo records the resolved (non-open) verdicts and the
//!   honest emitted-check counts (FR-15).
//!
//! The trap text form is canonical (P1): every `TrapIfFalse`-derived site is
//! exactly `cmp rax, 0 / jne .trap_ok_N / jmp __lang_trap / .trap_ok_N:` in
//! the x86_64 object text, so `grep -c "jmp __lang_trap"` is an exact count
//! of emitted check sites.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::read_obl;
use verifier::verdict::{
    encode_verdicts, read_echo, EmittedChecksData, VerdictRecord, VerdictStatus,
};

fn langc_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_langc"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-discharge-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A bank fixture: `bounded_inc` has three subtype-range sites (C1 param
/// entry, C3 `as Percent`, C2 return), `main` has two (C3 + C2). 8 traps
/// total in `--checks=all` (each check = 2 `TrapIfFalse` sites: ge + le).
const BANK_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
: bounded_inc ( Percent -- Percent )
  1 + as Percent
;
: main ( -- i64 )
  50 as Percent bounded_inc as i64
;
end;
";

/// Compile `BANK_MOD` and return the emitted `.asm` text plus the out dir.
/// `checks_undischarged` selects the P4 verdict-driven mode (requires
/// `verdicts`), otherwise langc defaults to `--checks=all`.
fn compile_bank(tag: &str, checks_undischarged: bool, verdicts: Option<&Path>) -> (String, PathBuf) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl");
    if checks_undischarged {
        let vf = verdicts.expect("undischarged requires a verdicts file");
        cmd.arg("--checks=undischarged");
        cmd.arg(format!("--verdicts={}", vf.display()));
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    assert!(
        out.status.success(),
        "langc failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let asm = fs::read_to_string(out_dir.join("Bank.asm")).expect("Bank.asm written");
    (asm, out_dir)
}

fn trap_count(asm: &str) -> usize {
    asm.lines().filter(|l| l.trim() == "jmp __lang_trap").count()
}

/// Emit the module's obligations and return the `(id, id_hash)` of the first
/// subtype-range site of `word`.
fn site_of(word: &str, occurrence: u32) -> (String, String) {
    let dir = fresh_dir("obl");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let st = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(st.success());
    let bytes = fs::read(out_dir.join("Bank.obl.json")).unwrap();
    let set = read_obl(&bytes).expect("artifact parses");
    let o = set
        .obligations
        .iter()
        .find(|o| {
            o.site.word == word
                && o.kind == verifier::model::Kind::SubtypeRange
                && o.site.occurrence == occurrence
        })
        .unwrap_or_else(|| panic!("no {word} subtype-range site {occurrence}"));
    let _ = fs::remove_dir_all(&dir);
    (o.id.clone(), o.id_hash.clone())
}

/// Write a verdicts file with a single record and return its path.
fn write_verdicts(tag: &str, records: &[VerdictRecord]) -> PathBuf {
    let dir = fresh_dir(tag);
    let path = dir.join("v.json");
    let bytes =
        encode_verdicts("test", "0.1.0", records, 0, &EmittedChecksData::default()).unwrap();
    fs::write(&path, &bytes).unwrap();
    path
}

#[test]
fn checks_all_emits_every_subtype_site() {
    let (asm, _dir) = compile_bank("all", false, None);
    // 4 subtype sites (bounded_inc: C1+C3+C2; main: C3) × 2 TrapIfFalse per
    // check (lower + upper bound) = exactly 8 trap sites — the canonical
    // check form from slice P1, and the bijection witness the P4 exit
    // criterion counts.
    assert_eq!(trap_count(&asm), 8, "checks=all emits every check site");
}

#[test]
fn discharged_param_site_removes_its_trap_pair() {
    // Under P5 the interval engine ALSO discharges in-tree: the file
    // discharges bounded_inc's C1 param (occurrence 0); the engine discharges
    // bounded_inc's C2 return (the body cast narrows its value to [0,100])
    // and main's C3 cast (operand [50,50]). Only bounded_inc's C1... no:
    // after the engine runs, the only OPEN subtype site is bounded_inc's C3
    // cast (operand ⊤ — 1 + on an unknown caller value).
    let (id, id_hash) = site_of("bounded_inc", 0);
    let rec = VerdictRecord {
        id: id.clone(),
        id_hash,
        status: VerdictStatus::Discharged,
        method: Some("interval".to_string()),
        proof_ref: None,
        justification: None,
    };
    let vf = write_verdicts("disch", &[rec]);
    let (asm_all, _) = compile_bank("base", false, None);
    let (asm_und, out_dir) = compile_bank("und", true, Some(&vf));
    let base = trap_count(&asm_all);
    let und = trap_count(&asm_und);
    // base has 4 sites × 2 traps = 8. Three sites close (file C1 + in-tree
    // C2/C3s) → 1 open site → 2 traps.
    assert_eq!(base, 8);
    assert_eq!(
        und,
        2,
        "file-discharged C1 + engine-discharged C2/main-C3 leave only the ⊤-cast open"
    );

    // The echo records every non-open verdict: the file one (source file)
    // and the two in-tree ones, with the honest emitted counts.
    let echo_bytes = fs::read(out_dir.join("Bank.verdicts.inTree.json"))
        .expect("verdicts echo written under --write-obl");
    let echo = read_echo(&echo_bytes).expect("echo parses");
    assert_eq!(echo.verdicts.records.len(), 3, "file C1 + in-tree C2 + main C3");
    assert_eq!(
        echo.verdicts.records[0].status,
        VerdictStatus::Discharged
    );
    assert_eq!(echo.verdicts.records[0].method.as_deref(), Some("interval"));
    assert_eq!(echo.emitted.subtype_range, 1, "emitted == open sites");
    assert_eq!(echo.in_tree_verdicts, 2, "engine closed two sites");
    assert_eq!(echo.stale_verdicts, 0);
}

#[test]
fn undischarged_without_verdicts_is_e6402() {
    let dir = fresh_dir("noverdicts");
    let mod_path = dir.join("Bank.mod");
    fs::write(&mod_path, BANK_MOD).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let st = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--checks=undischarged")
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(!st.status.success(), "undischarged without verdicts must fail");
    let err = String::from_utf8_lossy(&st.stderr);
    assert!(err.contains("6402"), "E6402 gate: {err}");
    assert!(
        !out_dir.join("Bank.o").exists(),
        "fail-closed: no object may be produced"
    );
}

#[test]
fn hash_mismatched_record_fails_closed_and_counts_stale() {
    // A record with the right id but a wrong id_hash must NOT discharge the
    // site (Q3: hash disagreement ⇒ stale ⇒ open ⇒ check retained); the
    // interval engine still discharges what it can (bounded_inc's C2 return +
    // main's C3 cast), and the echo counts the mismatched record as stale.
    let (id, _real_hash) = site_of("bounded_inc", 0);
    let rec = VerdictRecord {
        id,
        id_hash: "0000000000000000".to_string(),
        status: VerdictStatus::Discharged,
        method: Some("interval".to_string()),
        proof_ref: None,
        justification: None,
    };
    let vf = write_verdicts("stale", &[rec]);
    let (asm_all, _) = compile_bank("base2", false, None);
    let (asm_und, out_dir) = compile_bank("und2", true, Some(&vf));
    // The stale record fails closed: bounded_inc's C1 stays open; the engine
    // closes bounded_inc C2 + main C3 (4 traps), so 8 − 4 = 4 remain.
    assert_eq!(trap_count(&asm_all), 8);
    assert_eq!(trap_count(&asm_und), 4);
    let echo_bytes = fs::read(out_dir.join("Bank.verdicts.inTree.json")).unwrap();
    let echo = read_echo(&echo_bytes).unwrap();
    assert_eq!(echo.verdicts.records.len(), 2, "the two in-tree discharges");
    assert_eq!(echo.emitted.subtype_range, 2, "open sites keep their checks");
    assert_eq!(echo.stale_verdicts, 1, "the mismatched record is stale");
}

#[test]
fn unknown_site_record_is_ignored_as_stale() {
    let rec = VerdictRecord {
        id: "Bank::nope::subtype-range::0".to_string(),
        id_hash: "1234567890abcdef".to_string(),
        status: VerdictStatus::Discharged,
        method: Some("interval".to_string()),
        proof_ref: None,
        justification: None,
    };
    let vf = write_verdicts("unknown", &[rec]);
    let (asm_und, out_dir) = compile_bank("und3", true, Some(&vf));
    let (asm_all, _) = compile_bank("base3", false, None);
    assert_eq!(trap_count(&asm_und), trap_count(&asm_all) - 4);
    let echo_bytes = fs::read(out_dir.join("Bank.verdicts.inTree.json")).unwrap();
    let echo = read_echo(&echo_bytes).unwrap();
    assert_eq!(echo.stale_verdicts, 1);
    assert_eq!(echo.in_tree_verdicts, 2, "engine discharges unaffected by the file");
}

#[test]
fn echo_is_byte_deterministic_across_runs() {
    // FR-17 covers the echo too: two identical compilations produce identical
    // `<Module>.verdicts.inTree.json` bytes (fixed key order, no timestamps).
    let vf = write_verdicts("det-v", &[]);
    let (_a1, d1) = compile_bank("det1", true, Some(&vf));
    let (_a2, d2) = compile_bank("det2", true, Some(&vf));
    let e1 = fs::read(d1.join("Bank.verdicts.inTree.json")).unwrap();
    let e2 = fs::read(d2.join("Bank.verdicts.inTree.json")).unwrap();
    assert_eq!(e1, e2, "echo must be byte-identical across runs (FR-17)");
}
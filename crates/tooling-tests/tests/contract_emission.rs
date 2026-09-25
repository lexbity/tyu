//! Contract emission is verdict-driven (static-verification.md Q6, FR-5,
//! slice P6).
//!
//! Under `--checks=undischarged` the C5/C6 runtime traps are emitted exactly
//! at sites whose obligation is open:
//!
//! - `: five ( -- i64 ) ensures [ dup 0 >= ] 5 ;` — the output is the
//!   constant 5, so the in-tree interval engine proves the ensures predicate
//!   (`[1,1]` verdict) and the `contract-post` obligation discharges — no
//!   CONTRACT_FAIL trap in the object;
//! - `: inputy ( i64 -- i64 ) ensures [ dup 0 >= ] 1 + ;` — the output is
//!   `⊤` (input-derived), so the obligation stays open and the trap remains.
//!
//! `--checks=all` (legacy) emits both traps unconditionally — FR-5 golden
//! property, identical to the pre-P6 emitter.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

const POST_MOD: &str = "\
module P;
: five ( -- i64 )
  ensures [ dup 0 >= ]
  5 ;
: inputy ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;
: main ( -- i64 )
  five ;
export { main } ;
end;
";

fn compile_with(tag: &str, extra: &[&str]) -> (bool, String) {
    let dir = std::env::temp_dir().join("tyu_contract_emission").join(format!(
        "{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mod_path = dir.join("P.mod");
    std::fs::write(&mod_path, POST_MOD).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", dir.display()));
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().unwrap();
    let asm = std::fs::read_to_string(dir.join("P.asm")).unwrap_or_default();
    (out.status.success(), asm)
}

fn count_contract_traps(asm: &str) -> usize {
    // Trap code 20 (CONTRACT_FAIL) is lowered to `mov rdi, 20` + `jmp
    // __lang_trap` — one `mov rdi, 20` per emitted contract trap.
    asm.lines().filter(|l| l.trim() == "mov rdi, 20").count()
}

#[test]
fn constant_ensures_is_discharged_under_undischarged() {
    // An empty-but-valid verdicts file; the in-tree engine decides.
    let dir = std::env::temp_dir().join("tyu_contract_emission_empty");
    let _ = std::fs::create_dir_all(&dir);
    let vfile = dir.join("empty.json");
    std::fs::write(
        &vfile,
        r#"{"schema":"tyu.verdicts/v1","tool":{"name":"t","version":"0"},"semantics":"tyu.ir-sem/1.0","verdicts":[]}"#,
    )
    .unwrap();
    let (ok, asm) = compile_with(
        "undischarged",
        &[
            // The discharge must be reachable: default langc features enable
            // module-loading, under which FR-21 retains every contract check.
            "--no-default-features",
            "--checks=undischarged",
            &format!("--verdicts={}", vfile.display()),
        ],
    );
    assert!(ok, "module must compile under undischarged");
    // `five` discharges (constant 5 ≥ 0); `inputy` stays open (⊤ output).
    // The trap for the open site remains — the discharge only ever removes
    // the *closed* site's check (FR-13).
    assert_eq!(
        count_contract_traps(&asm),
        1,
        "only inputy's open ensures trap must remain: {asm}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_all_mode_emits_both_traps() {
    // FR-5: `--checks=all` behavior is byte-identical to today's emitter —
    // every contract site emits unconditionally (both traps present).
    let (ok, asm) = compile_with("all", &["--checks=all"]);
    assert!(ok);
    assert_eq!(count_contract_traps(&asm), 2, "all-mode keeps both traps");
}

// The empty-verdicts helper is kept local; silence the unused-import lint for
// the temp dir cleanup path variance.
#[allow(dead_code)]
fn _unused() {}
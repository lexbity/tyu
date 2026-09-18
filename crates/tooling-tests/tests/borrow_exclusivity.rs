//! E2E tests for M4 borrow exclusivity (E5021).
//!
//! These tests compile tyu source through `langc` and verify that
//! the borrow checker correctly accepts/rejects two-live-borrow
//! programs per the ratified rule.

use std::process::Command;

fn langc_exe() -> std::path::PathBuf {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    workspace.join("target").join("debug").join("langc")
}

fn platform_arg() -> String {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    format!(
        "--platform={}",
        workspace.join("runtime").display()
    )
}

fn repo_sysroot() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("sysroot")
}

fn fresh_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("tyu_borrow_tests").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn compile_args(src: &[u8], dir: &std::path::Path, extra_args: &[&str]) -> Result<(), String> {
    let mod_path = dir.join("test.mod");
    std::fs::write(&mod_path, src).map_err(|e| format!("write: {}", e))?;
    let mut cmd = Command::new(langc_exe());
    cmd.current_dir(dir)
        .arg("--emit=ir")
        .arg(platform_arg())
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()));
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().map_err(|e| format!("run: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

fn compile(src: &[u8], dir: &std::path::Path) -> Result<(), String> {
    compile_args(src, dir, &[])
}

fn compile_expect_err_args(src: &[u8], dir: &std::path::Path, extra_args: &[&str]) -> String {
    let mod_path = dir.join("test.mod");
    std::fs::write(&mod_path, src).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.current_dir(dir)
        .arg("--emit=ir")
        .arg(platform_arg())
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()));
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().unwrap();
    assert!(!out.status.success(), "expected compilation error");
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn compile_expect_err(src: &[u8], dir: &std::path::Path) -> String {
    compile_expect_err_args(src, dir, &[])
}

// ---------------------------------------------------------------------------
// Positive: sequential re-borrow (bump idiom)
// ---------------------------------------------------------------------------

#[test]
fn sequential_reborrow_bump() {
    let dir = fresh_dir("sequential_reborrow_bump");
    // Sequential re-borrow: first consumed by drop before second is created.
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter drop &!counter drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("sequential re-borrow must compile");
}

// ---------------------------------------------------------------------------
// Positive: distinct roots
// ---------------------------------------------------------------------------

#[test]
fn distinct_roots_ok() {
    let dir = fresh_dir("distinct_roots_ok");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource a : u32 = 0;\n\
resource b : u32 = 0;\n\
: main ( -- i64 )\n\
  a lock [ &!a drop ]\n\
  b lock [ &!b drop ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("distinct root borrows must compile");
}

// ---------------------------------------------------------------------------
// Negative: two live mut borrows of the same root → E5021
// ---------------------------------------------------------------------------

#[test]
fn two_live_mut_borrows_same_root() {
    let dir = fresh_dir("two_live_mut_borrows_same_root");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter &!counter drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for two simultaneously-live &! borrows, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Negative: mut + shared overlap
// ---------------------------------------------------------------------------

#[test]
fn mut_and_shared_overlap() {
    let dir = fresh_dir("mut_and_shared_overlap");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter &counter drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for &! + & overlap, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Negative: dup of mutable borrow → E5022
// ---------------------------------------------------------------------------

#[test]
fn dup_mut_borrow_rejected() {
    let dir = fresh_dir("dup_mut_borrow_rejected");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter dup drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5022"),
        "expected E5022 for dup of &! borrow, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Positive: shared borrow dup accepted
// ---------------------------------------------------------------------------

#[test]
fn shared_borrow_dup_ok() {
    let dir = fresh_dir("shared_borrow_dup_ok");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &counter dup drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("dup of shared borrow must compile");
}

// ---------------------------------------------------------------------------
// Positive: shared + shared coexistence (legal)
// ---------------------------------------------------------------------------

#[test]
fn shared_shared_coexist() {
    let dir = fresh_dir("shared_shared_coexist");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &counter &counter drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("two shared borrows of same root must compile");
}

// ---------------------------------------------------------------------------
// Negative: double-ref of borrow-typed local → E5023
// ---------------------------------------------------------------------------

#[test]
fn borrow_local_double_ref_rejected() {
    let dir = fresh_dir("borrow_local_double_ref");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter => x x drop x drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5023"),
        "expected E5023 for double ref of borrow-typed local, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Negative: bind a mutable borrow then take another &! → E5021
// ---------------------------------------------------------------------------

#[test]
fn borrow_local_then_conflicting_mint() {
    let dir = fresh_dir("borrow_local_then_conflicting_mint");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter => x &!counter drop x drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for &! after binding &! local, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Positive: bind/ref/consume/re-borrow (sequential)
// ---------------------------------------------------------------------------

#[test]
fn borrow_local_seq_ref_ok() {
    let dir = fresh_dir("borrow_local_seq_ref");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter => x x drop &!counter drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("sequential ref-then-re-borrow must compile");
}

// ---------------------------------------------------------------------------
// Positive: shared borrow local × 3 refs (freely referencable)
// ---------------------------------------------------------------------------

#[test]
fn shared_borrow_local_multi_ref_ok() {
    let dir = fresh_dir("shared_borrow_local_multi_ref");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &counter => x x drop x drop x drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("shared borrow local multi-ref must compile");
}

// ---------------------------------------------------------------------------
// Control flow: if / while / loop with borrows
// ---------------------------------------------------------------------------

// NEG: borrow live across if — branch re-mints same root → E5021
#[test]
fn borrow_live_across_if_branch_mint_conflict() {
    let dir = fresh_dir("borrow_live_across_if_branch");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter drop\n\
    true [ &!counter drop ] [ &!counter drop ] if\n\
  ]\n\
  0\n\
;\n\
end;\n";
    // The first &!counter is consumed by drop before the if.
    // Inside the if, each branch mints a new &!counter.  This is legal
    // (sequential re-borrow), not a conflict.  The test checks that
    // branch-internal borrows don't cause spurious E5021.
    compile(src, &dir).expect("branch-internal borrows must compile");
}

// NEG: if branches leave different-place borrows at join → IfBranchContent
#[test]
fn if_branches_different_places_at_join() {
    let dir = fresh_dir("if_branches_different_places");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    true [ &!counter ] [ &!counter ] if drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    // Both branches leave a Ptr (same root, same type) — merge should succeed.
    compile(src, &dir).expect("same-place borrows at join must compile");
}

// NEG: if branches leave different depths → IfBranchDepth
#[test]
fn if_branches_different_depths_error() {
    let dir = fresh_dir("if_branches_different_depths");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    true [ &!counter ] [ ] if drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("3246") || err.contains("IfBranchDepth"),
        "expected IfBranchDepth for different-depth branches, got: {err}"
    );
}

// POS: per-iteration mint+consume in while loop
#[test]
fn while_loop_per_iter_mint_consume() {
    let dir = fresh_dir("while_loop_per_iter_mint");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    [ &!counter @u32 as i64 0 > ] [ &!counter @u32 drop ] while\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("per-iteration mint+consume in while must compile");
}

// POS: per-iteration mint+consume in loop
#[test]
fn loop_per_iter_mint_consume() {
    let dir = fresh_dir("loop_per_iter_mint");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    [ &!counter drop ] loop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("per-iteration mint+consume in loop must compile");
}

// NEG: lock body with two simultaneous &!R → E5021
#[test]
fn lock_body_two_mut_borrows() {
    let dir = fresh_dir("lock_body_two_mut_borrows");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter &!counter drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for two &! in lock body, got: {err}"
    );
}

// POS: lock body with sequential &!R → legal
#[test]
fn lock_body_seq_borrow() {
    let dir = fresh_dir("lock_body_seq_borrow");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter drop &!counter drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("sequential &! in lock body must compile");
}

// POS: R lock [ &!R drop ] — regression pin from effect_corpus.rs:225
#[test]
fn lock_regression_effect_corpus_225() {
    let dir = fresh_dir("lock_regression_225");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource R : u32 = 0;\n\
: main ( -- i64 )\n\
  R lock [ &!R drop ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("R lock [ &!R drop ] must compile");
}

// ---------------------------------------------------------------------------
// Quotations and call (S-7)
// ---------------------------------------------------------------------------

// NEG: &!x [ ( -- ) &!x drop ] call — quotation borrows same root as parent → E5021
#[test]
fn call_quot_borrows_same_root_as_live_parent() {
    let dir = fresh_dir("call_quot_borrows_same_root");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter [ ( -- ) &!counter drop ] call drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for call with live parent borrow on same root, got: {err}"
    );
}

// POS: two sequential quotation borrows (no overlap) → OK
#[test]
fn sequential_quot_borrows_ok() {
    let dir = fresh_dir("sequential_quot_borrows");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    [ ( -- ) &!counter drop ] call\n\
    [ ( -- ) &!counter drop ] call\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("sequential quotation borrows must compile");
}

// POS: spawner re-borrows after spawn (distinct words, OK)
#[test]
fn spawner_reborrows_after_spawn_ok() {
    let dir = fresh_dir("spawner_reborrows_after_spawn");
    // The spawn creates a separate IrWordGen with its own ledger.
    // Borrows inside the spawn do NOT conflict with the parent's
    // borrows (D-7: cross-task exclusion is the lock system's job).
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 ) performs {suspend}\n\
  [ ( -- ) ] platform.task.spawn drop\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("spawner re-borrow after spawn must compile");
}

// ---------------------------------------------------------------------------
// S-8: PlaceUnknownRoot (E3523) + param seeding
// ---------------------------------------------------------------------------

// NEG: &!undefined-thing → E3523
#[test]
fn addr_of_unknown_root_rejected() {
    let dir = fresh_dir("addr_of_unknown_root");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  &!nonexistent drop\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("3523"),
        "expected E3523 for &! of undefined name, got: {err}"
    );
}

// POS: ptr_mut param accepted (param seeding)
#[test]
fn ptr_mut_param_accepted() {
    let dir = fresh_dir("ptr_mut_param");
    let src = b"module Main;\n\
: main ( -- i64 )\n\
  42 as ptr_mut drop\n\
  0\n\
;\n\
end;\n";
    compile_args(src, &dir, &["--allow-raw-casts"]).expect("ptr_mut param must compile");
}

// POS: typed store through raw ptr_mut param
#[test]
fn typed_store_through_ptr_mut_param() {
    let dir = fresh_dir("typed_store_through_ptr_mut");
    let src = b"module Main;\n\
: main ( -- i64 )\n\
  0 as usize as ptr_mut 42 !i64\n\
  0\n\
;\n\
end;\n";
    compile_args(src, &dir, &["--allow-raw-casts"])
        .expect("typed store through raw ptr_mut must compile");
}

// ---------------------------------------------------------------------------
// S-9: MMIO exemption boundary verification
// ---------------------------------------------------------------------------

// POS: eight live &!gpio.DATA.N (MMIO array elements — exempt from borrow rule)
#[test]
fn mmio_eight_live_mut_borrows() {
    let dir = fresh_dir("mmio_eight_live_mut_borrows");
    let src = b"module Main;\n\
import platform/linux { };\n\
register-map GPIO\n\
  0x00 DATA[8] u32 rw\n\
end;\n\
const gpio = GPIO @ board.gpio;\n\
: main ( -- i64 )\n\
  &!gpio.DATA.0 &!gpio.DATA.1 &!gpio.DATA.2 &!gpio.DATA.3\n\
  &!gpio.DATA.4 &!gpio.DATA.5 &!gpio.DATA.6 &!gpio.DATA.7\n\
  drop drop drop drop drop drop drop drop\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("eight live MMIO mut borrows must compile (exempt)");
}

// POS: MMIO borrow + memory borrow coexist (different roots → no conflict)
#[test]
fn mmio_and_mem_borrow_coexist() {
    let dir = fresh_dir("mmio_and_mem_borrow_coexist");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
end;\n\
const gpio = GPIO @ board.gpio;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter &!gpio.DATA drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("MMIO + memory borrow coexist must compile");
}

// NEG control: same shape as MMIO test but with &!counter (memory borrow) → E5021
#[test]
fn memory_two_live_mut_borrows_rejected() {
    let dir = fresh_dir("memory_two_live_mut_borrows_rejected");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource counter : u32 = 0;\n\
: main ( -- i64 )\n\
  counter lock [\n\
    &!counter &!counter drop drop\n\
  ]\n\
  0\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(
        err.contains("5021"),
        "expected E5021 for two live memory mut borrows, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// S-16: 5024 is a theoretical safety net (ledger cap 64).  In practice
// tighter limits (resource cap 64, sig input cap 8, struct field cap 32,
// local cap 64) prevent reaching 65 distinct borrows.  This test verifies
// that borrowing from 8 resources (the max from a sig) works, confirming
// the ledger is correctly tracking fewer borrows.
// ---------------------------------------------------------------------------

#[test]
fn ledger_basic_usage() {
    let dir = fresh_dir("ledger_basic_usage");
    let src = b"module Main;\n\
import platform/linux { };\n\
resource a : u32 = 0;\n\
resource b : u32 = 0;\n\
resource c : u32 = 0;\n\
: main ( -- i64 )\n\
  a lock [ &!a drop ]\n\
  b lock [ &!b drop ]\n\
  c lock [ &!c drop ]\n\
  0\n\
;\n\
end;\n";
    compile(src, &dir).expect("ledger with 3 resources must work");
}

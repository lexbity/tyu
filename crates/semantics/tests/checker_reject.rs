//! Rejection-side tests for the type checker.
//!
//! Tests that the checker rejects invalid programs with the expected
//! error codes.  Each rejection test pins an exact error code.

mod common;

use common::*;

// ---------------------------------------------------------------------------
// If-arm disagreement: branches produce incompatible stack types
// ---------------------------------------------------------------------------

#[test]
fn if_branch_type_mismatch() {
    let (env, len) = builtin_env();
    check_err("true [ 1 ] [ true ] if drop", &[], &[], &env[..len], 3247);
}

#[test]
fn if_branch_depth_mismatch() {
    let (env, len) = builtin_env();
    check_err(
        "true [ 1 2 ] [ 3 ] if drop drop",
        &[],
        &[],
        &env[..len],
        3246,
    );
}

#[test]
fn if_cond_not_bool() {
    let (env, len) = builtin_env();
    check_err("1 [ 1 ] [ 2 ] if drop", &[], &[], &env[..len], 3243);
}

// ---------------------------------------------------------------------------
// While-cond not bool
// ---------------------------------------------------------------------------

#[test]
fn while_cond_not_bool() {
    let (env, len) = builtin_env();
    check_err("[ 1 ] [ ] while", &[], &[], &env[..len], 3255);
}

// ---------------------------------------------------------------------------
// While body non-zero net (depth mismatch)
// ---------------------------------------------------------------------------

#[test]
fn while_body_depth() {
    let (env, len) = builtin_env();
    check_err(
        "[ dup 0 > ] [ 1 ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        3257,
    );
}

// ---------------------------------------------------------------------------
// Loop body non-zero net (depth mismatch)
// ---------------------------------------------------------------------------

#[test]
fn loop_body_depth() {
    let (env, len) = builtin_env();
    check_err("[ 1 ] loop", &[b"i64"], &[b"i64"], &env[..len], 3262);
}

// ---------------------------------------------------------------------------
// Env at capacity: 256 entries, a 257th user word is not found
// ---------------------------------------------------------------------------

#[test]
fn env_full_rejects_user_word() {
    let (mut env, mut len) = builtin_env();
    for i in 0..(256 - len) {
        let name = [b'w', b'a', b'r', (i % 10) as u8 + b'0'];
        env[len] = entry(&name[..], &[b"i64"], &[]);
        len += 1;
        if len >= 256 {
            break;
        }
    }
    check_err("42 missing_word", &[], &[], &env[..len], 3210);
}

// ---------------------------------------------------------------------------
// Quotation call with wrong stack
// ---------------------------------------------------------------------------

#[test]
fn call_underflow() {
    let (env, len) = builtin_env();
    // call expects a quotation on the stack; empty stack → call internal error
    check_err("call", &[], &[], &env[..len], 3758);
}

// ---------------------------------------------------------------------------
// Return type mismatch
// ---------------------------------------------------------------------------

#[test]
fn return_type_count_mismatch() {
    let (env, len) = builtin_env();
    check_err("42", &[], &[b"i64", b"i64"], &env[..len], 3220);
}

#[test]
fn return_type_value_mismatch() {
    let (env, len) = builtin_env();
    // Output type is bool, word produces i64 → 3221
    check_err("42", &[], &[b"bool"], &env[..len], 3221);
}

// ---------------------------------------------------------------------------
// While-cond modified stack (cond pushes extra values)
// ---------------------------------------------------------------------------

#[test]
fn while_cond_modified_stack() {
    let (env, len) = builtin_env();
    // Condition body pushes 2 values, net +1 → WhileCondModifiedStack
    check_err(
        "[ 1 2 ] [ drop ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        3254,
    );
}

// ---------------------------------------------------------------------------
// If-arm quotations must be quotations (not values)
// ---------------------------------------------------------------------------

#[test]
fn if_then_not_quot() {
    let (env, len) = builtin_env();
    // Then-branch is not a quotation → IfThenNotQuot
    check_err("true 1 [ 2 ] if drop", &[], &[], &env[..len], 3244);
}

// ---------------------------------------------------------------------------
// Loop body must be quotation
// ---------------------------------------------------------------------------

#[test]
fn loop_body_not_quot() {
    let (env, len) = builtin_env();
    // Loop body is not a quotation → LoopBodyNotQuot
    check_err("1 loop", &[b"i64"], &[b"i64"], &env[..len], 3261);
}

// ---------------------------------------------------------------------------
// If quotation parsing errors
// ---------------------------------------------------------------------------

#[test]
fn if_missing_branches() {
    let (env, len) = builtin_env();
    // Not enough values for `if` (missing else branch) → IfPopCond (the
    // typechecker tries to pop the cond first; failing there means 3242).
    check_err("[ 1 ] if drop", &[b"bool"], &[], &env[..len], 3242);
}

#[test]
fn if_cond_missing() {
    let (env, len) = builtin_env();
    // Not enough values for `if` (missing condition) → IfPopCond
    check_err("[ 1 ] [ 2 ] if drop", &[], &[], &env[..len], 3242);
}

// ---------------------------------------------------------------------------
// While body not a quotation
// ---------------------------------------------------------------------------

#[test]
fn while_body_not_quotation() {
    let (env, len) = builtin_env();
    // Body is a value, not a quotation → WhileBodyNotQuot
    check_err(
        "[ dup 0 > ] 1 while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        3252,
    );
}

#[test]
fn while_cond_not_quotation() {
    let (env, len) = builtin_env();
    // Condition is a value, not a quotation → WhileCondNotQuot
    check_err("1 [ drop ] while", &[b"i64"], &[b"i64"], &env[..len], 3253);
}

// ---------------------------------------------------------------------------
// Stack underflow
// ---------------------------------------------------------------------------

#[test]
fn underflow_on_dup() {
    let (env, len) = builtin_env();
    check_err("dup", &[], &[], &env[..len], 3202);
}

// ---------------------------------------------------------------------------
// Type mismatch in arithmetic
// ---------------------------------------------------------------------------

#[test]
fn add_with_bool() {
    let (env, len) = builtin_env();
    check_err("true 1 +", &[], &[], &env[..len], 3212);
}

#[test]
fn and_with_i64() {
    let (env, len) = builtin_env();
    check_err("1 2 and", &[], &[], &env[..len], 3212);
}

// ---------------------------------------------------------------------------
// ChecksMode: Off mode still rejects type errors (they're not contract-only)
// ---------------------------------------------------------------------------

#[test]
fn checks_off_still_rejects_type_mismatch() {
    let (env, len) = builtin_env();
    let dbs = Dbs::new();
    // Type-mismatch errors are not gated by ChecksMode.
    check(
        "true 1 +",
        &[],
        &[],
        &env[..len],
        &dbs,
        ChecksMode::Off,
        |result| {
            assert!(
                result.is_err(),
                "type mismatch must be rejected even with Off"
            );
        },
    );
}

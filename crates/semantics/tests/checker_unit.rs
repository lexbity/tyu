mod common;

use common::*;

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[test]
fn push_literal_number() {
    let (env, len) = builtin_env();
    check_ok("42", &[], &[b"i64"], &env[..len]);
}

#[test]
fn push_literal_bool_true() {
    let (env, len) = builtin_env();
    check_ok("true", &[], &[b"bool"], &env[..len]);
}

#[test]
fn push_literal_bool_false() {
    let (env, len) = builtin_env();
    check_ok("false", &[], &[b"bool"], &env[..len]);
}

// ---------------------------------------------------------------------------
// Stack operations
// ---------------------------------------------------------------------------

#[test]
fn stack_dup() {
    let (env, len) = builtin_env();
    check_ok("42 dup drop drop", &[], &[], &env[..len]);
}

#[test]
fn stack_drop() {
    let (env, len) = builtin_env();
    check_ok("42 drop", &[], &[], &env[..len]);
}

#[test]
fn stack_swap() {
    let (env, len) = builtin_env();
    check_ok("1 2 swap drop drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_add() {
    let (env, len) = builtin_env();
    check_ok("1 2 + drop", &[], &[], &env[..len]);
}

#[test]
fn arithmetic_sub() {
    let (env, len) = builtin_env();
    check_ok("10 3 - drop", &[], &[], &env[..len]);
}

#[test]
fn arithmetic_mul() {
    let (env, len) = builtin_env();
    check_ok("4 5 * drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[test]
fn comparison() {
    let (env, len) = builtin_env();
    check_ok("1 2 < drop", &[], &[], &env[..len]);
    check_ok("1 2 > drop", &[], &[], &env[..len]);
    check_ok("1 2 == drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Boolean logic
// ---------------------------------------------------------------------------

#[test]
fn boolean_ops() {
    let (env, len) = builtin_env();
    check_ok("true false and drop", &[], &[], &env[..len]);
    check_ok("true false or drop", &[], &[], &env[..len]);
    check_ok("true not drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// String literal
// ---------------------------------------------------------------------------

#[test]
fn string_literal() {
    let (env, len) = builtin_env();
    check_ok(r#""hello" drop"#, &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Word with inputs / outputs
// ---------------------------------------------------------------------------

#[test]
fn word_with_inputs() {
    let (env, len) = builtin_env();
    check_ok("dup drop drop", &[b"i64"], &[], &env[..len]);
}

#[test]
fn word_with_outputs() {
    let (env, len) = builtin_env();
    check_ok("42", &[], &[b"i64"], &env[..len]);
}

// ---------------------------------------------------------------------------
// If / else
// ---------------------------------------------------------------------------

#[test]
fn if_expression() {
    let (env, len) = builtin_env();
    check_ok("true [ 1 ] [ 2 ] if drop", &[], &[], &env[..len]);
}

#[test]
fn if_with_input_output() {
    let (env, len) = builtin_env();
    check_ok("[ 1 ] [ 2 ] if drop", &[b"bool"], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// While loop
// ---------------------------------------------------------------------------

#[test]
fn while_loop() {
    let (env, len) = builtin_env();
    check_ok(
        "[ dup 0 > ] [ 1 - ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
    );
}

// ---------------------------------------------------------------------------
// Loop
// ---------------------------------------------------------------------------

#[test]
fn loop_expression() {
    let (env, len) = builtin_env();
    check_ok("[ ] loop", &[b"i64"], &[b"i64"], &env[..len]);
}

// ---------------------------------------------------------------------------
// Stack underflow
// ---------------------------------------------------------------------------

#[test]
fn underflow_drop_empty() {
    let (env, len) = builtin_env();
    check_err("drop", &[], &[], &env[..len], 3202);
}

#[test]
fn underflow_missing_input() {
    let (env, len) = builtin_env();
    check_err("dup", &[], &[], &env[..len], 3202);
}

// ---------------------------------------------------------------------------
// Type mismatch
// ---------------------------------------------------------------------------

#[test]
fn type_error_add_with_bool() {
    let (env, len) = builtin_env();
    check_err("true 1 +", &[], &[], &env[..len], 3212);
}

#[test]
fn type_error_and_with_i64() {
    let (env, len) = builtin_env();
    check_err("1 2 and", &[], &[], &env[..len], 3212);
}

#[test]
fn type_error_output_mismatch() {
    let (env, len) = builtin_env();
    check_err("", &[], &[b"i64"], &env[..len], 3220);
}

// ---------------------------------------------------------------------------
// Quotations (without call)
// ---------------------------------------------------------------------------

#[test]
fn quotation_value() {
    let (env, len) = builtin_env();
    check_ok("[ 1 2 + ] drop", &[], &[], &env[..len]);
}

#[test]
fn nested_quotations() {
    let (env, len) = builtin_env();
    check_ok("[ [ 1 ] ] drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Multiple operations
// ---------------------------------------------------------------------------

#[test]
fn multiple_operations() {
    let (env, len) = builtin_env();
    check_ok("1 2 + 3 * drop", &[], &[], &env[..len]);
}

// ---------------------------------------------------------------------------
// Wide environment: calling user words
// ---------------------------------------------------------------------------

#[test]
fn call_user_word() {
    let (mut env, mut len) = builtin_env();
    env[len] = entry(b"twice", &[b"i64"], &[b"i64", b"i64"]);
    len += 1;
    check_ok("3 twice drop drop", &[], &[], &env[..len]);
}

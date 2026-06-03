use frontend::parse::Output;
use frontend::span::Span;
use ir::{CapSet, Context, EffectSet, High, StackBound};
use semantics::typecheck::db::NominalDb;
use semantics::typecheck::db::SubtypeInfo;
use semantics::typecheck::mmio::MmioDb;
use semantics::typecheck::{typecheck_word_body, ChecksMode};
use semantics::types::{TypeAtom, WordEntry, WordSig};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct NullOut;
impl Output for NullOut {
    fn write(&mut self, _bytes: &[u8]) {}
}

fn ta(bytes: &[u8]) -> TypeAtom {
    TypeAtom::new(bytes).unwrap()
}

fn sig(inp: &[&[u8]], out: &[&[u8]]) -> WordSig {
    let mut s = WordSig::empty();
    s.in_len = inp.len() as u8;
    s.out_len = out.len() as u8;
    for (i, &b) in inp.iter().enumerate() {
        s.inputs[i] = ta(b);
    }
    for (i, &b) in out.iter().enumerate() {
        s.outputs[i] = ta(b);
    }
    s
}

fn entry(name: &[u8], inp: &[&[u8]], out: &[&[u8]]) -> WordEntry {
    WordEntry {
        name: ta(name),
        sig: sig(inp, out),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }
}

fn empty_env() -> [WordEntry; 256] {
    [WordEntry {
        name: ta(b""),
        sig: WordSig::empty(),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }; 256]
}

/// Run a stack-check on a word body and assert it succeeds.
fn check_ok(body: &str, inputs: &[&[u8]], outputs: &[&[u8]], env: &[WordEntry]) {
    let src = body.as_bytes();
    let span = Span::new(0, src.len());
    let decl = sig(inputs, outputs);
    let mut out = NullOut;
    let mmio = MmioDb {
        maps: frontend::fixed::FixedVec::new(),
        instances: frontend::fixed::FixedVec::new(),
    };
    let nominals = NominalDb {
        structs: frontend::fixed::FixedVec::new(),
        enums: frontend::fixed::FixedVec::new(),
    };
    let subtypes: &[SubtypeInfo] = &[];
    let result = typecheck_word_body(
        &mut out,
        src,
        span,
        &decl,
        env,
        subtypes,
        &mmio,
        &nominals,
        ChecksMode::All,
        Context::default(),
    );
    if let Err(e) = result {
        panic!(
            "expected OK, got error code {} span={:?}",
            e.code(),
            e.span()
        );
    }
}

/// Run a stack-check and assert it fails with the given error code.
fn check_err(
    body: &str,
    inputs: &[&[u8]],
    outputs: &[&[u8]],
    env: &[WordEntry],
    expected_code: u32,
) {
    let src = body.as_bytes();
    let span = Span::new(0, src.len());
    let decl = sig(inputs, outputs);
    let mut out = NullOut;
    let mmio = MmioDb {
        maps: frontend::fixed::FixedVec::new(),
        instances: frontend::fixed::FixedVec::new(),
    };
    let nominals = NominalDb {
        structs: frontend::fixed::FixedVec::new(),
        enums: frontend::fixed::FixedVec::new(),
    };
    let subtypes: &[SubtypeInfo] = &[];
    let err = typecheck_word_body(
        &mut out,
        src,
        span,
        &decl,
        env,
        subtypes,
        &mmio,
        &nominals,
        ChecksMode::All,
        Context::default(),
    )
    .expect_err("expected error");
    assert_eq!(
        err.code(),
        expected_code,
        "error code mismatch for body: {body:?}"
    );
}

/// Build a minimal environment with common builtins.
fn builtin_env() -> ([WordEntry; 256], usize) {
    let mut env = empty_env();
    let mut len = 0usize;
    macro_rules! add {
        ($name:expr, $inp:expr, $out:expr) => {{
            env[len] = entry($name, $inp, $out);
            len += 1;
        }};
    }
    add!(b"dup", &[b"i64"], &[b"i64", b"i64"]);
    add!(b"drop", &[b"i64"], &[]);
    add!(b"swap", &[b"i64", b"i64"], &[b"i64", b"i64"]);
    add!(b"+", &[b"i64", b"i64"], &[b"i64"]);
    add!(b"-", &[b"i64", b"i64"], &[b"i64"]);
    add!(b"*", &[b"i64", b"i64"], &[b"i64"]);
    add!(b">", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"<", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"==", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"and", &[b"bool", b"bool"], &[b"bool"]);
    add!(b"or", &[b"bool", b"bool"], &[b"bool"]);
    add!(b"not", &[b"bool"], &[b"bool"]);
    (env, len)
}

// ---------------------------------------------------------------------------
// Tests
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

#[test]
fn comparison() {
    let (env, len) = builtin_env();
    check_ok("1 2 < drop", &[], &[], &env[..len]);
    check_ok("1 2 > drop", &[], &[], &env[..len]);
    check_ok("1 2 == drop", &[], &[], &env[..len]);
}

#[test]
fn boolean_ops() {
    let (env, len) = builtin_env();
    check_ok("true false and drop", &[], &[], &env[..len]);
    check_ok("true false or drop", &[], &[], &env[..len]);
    check_ok("true not drop", &[], &[], &env[..len]);
}

#[test]
fn string_literal() {
    let (env, len) = builtin_env();
    check_ok(r#""hello" drop"#, &[], &[], &env[..len]);
}

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
    // Condition: dup 0 > — duplicates i64, compares with 0, leaves i64+bool
    // Body: 1 - — takes one i64, pushes 1, subtracts, leaves one i64
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
    // Loop body must leave stack unchanged. [ ] with input i64 leaves i64 unchanged.
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

// ---------------------------------------------------------------------------
// Nested quotations
// ---------------------------------------------------------------------------

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
    // Add a user word: "twice" ( i64 -- i64 i64 )
    env[len] = entry(b"twice", &[b"i64"], &[b"i64", b"i64"]);
    len += 1;
    check_ok("3 twice drop drop", &[], &[], &env[..len]);
}

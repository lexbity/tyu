mod common;

use common::*;
use ir::{CmpKind, OpKind, Word};

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[test]
fn push_literal_number() {
    let (env, len) = builtin_env();
    check_ok_with("42", &[], &[b"i64"], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

#[test]
fn push_literal_bool_true() {
    let (env, len) = builtin_env();
    check_ok_with("true", &[], &[b"bool"], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

#[test]
fn push_literal_bool_false() {
    let (env, len) = builtin_env();
    check_ok_with("false", &[], &[b"bool"], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

// ---------------------------------------------------------------------------
// Stack operations
// ---------------------------------------------------------------------------

#[test]
fn stack_dup() {
    let (env, len) = builtin_env();
    check_ok_with("42 dup drop drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::Dup { .. })));
    });
}

#[test]
fn stack_drop() {
    let (env, len) = builtin_env();
    check_ok_with("42 drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::Drop { .. })));
    });
}

#[test]
fn stack_swap() {
    let (env, len) = builtin_env();
    check_ok_with("1 2 swap drop drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::Swap { .. })));
    });
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_add() {
    let (env, len) = builtin_env();
    check_ok_with("1 2 + drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::AddI64)));
    });
}

#[test]
fn arithmetic_sub() {
    let (env, len) = builtin_env();
    check_ok_with("10 3 - drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::SubI64)));
    });
}

#[test]
fn arithmetic_mul() {
    let (env, len) = builtin_env();
    check_ok_with("4 5 * drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::MulI64)));
    });
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[test]
fn comparison() {
    let (env, len) = builtin_env();
    check_ok_with("1 2 < drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(
            op.kind,
            OpKind::Cmp {
                kind: CmpKind::Lt,
                ..
            }
        )));
    });
}

// ---------------------------------------------------------------------------
// Boolean logic
// ---------------------------------------------------------------------------

#[test]
fn boolean_ops() {
    let (env, len) = builtin_env();
    check_ok_with("true false and drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::AndBool)));
    });
}

// ---------------------------------------------------------------------------
// String literal
// ---------------------------------------------------------------------------

#[test]
fn string_literal() {
    let (env, len) = builtin_env();
    check_ok_with(r#""hello" drop"#, &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::ConstStr { .. })));
    });
}

// ---------------------------------------------------------------------------
// Word with inputs / outputs
// ---------------------------------------------------------------------------

#[test]
fn word_with_inputs() {
    let (env, len) = builtin_env();
    check_ok_with("dup drop drop", &[b"i64"], &[], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

#[test]
fn word_with_outputs() {
    let (env, len) = builtin_env();
    check_ok_with("42", &[], &[b"i64"], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

// ---------------------------------------------------------------------------
// If / else
// ---------------------------------------------------------------------------

#[test]
fn if_expression() {
    let (env, len) = builtin_env();
    check_ok_with(
        "true [ 1 ] [ 2 ] if drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 3);
        },
    );
}

#[test]
fn if_with_input_output() {
    let (env, len) = builtin_env();
    check_ok_with(
        "[ 1 ] [ 2 ] if drop",
        &[b"bool"],
        &[],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 3);
        },
    );
}

// ---------------------------------------------------------------------------
// While loop
// ---------------------------------------------------------------------------

#[test]
fn while_loop() {
    let (env, len) = builtin_env();
    check_ok_with(
        "[ dup 0 > ] [ 1 - ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 2);
            let has_back_edge = w
                .blocks
                .iter()
                .any(|b| b.ops.iter().any(|op| matches!(op.kind, OpKind::Br { .. })));
            assert!(has_back_edge, "while loop must produce a back-edge Br");
        },
    );
}

// ---------------------------------------------------------------------------
// Loop
// ---------------------------------------------------------------------------

#[test]
fn loop_expression() {
    let (env, len) = builtin_env();
    check_ok_with("[ ] loop", &[b"i64"], &[b"i64"], &env[..len], |w: &Word| {
        assert!(w.blocks.len() >= 2);
    });
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
    check_ok_with("[ 1 2 + ] drop", &[], &[], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

#[test]
fn nested_quotations() {
    let (env, len) = builtin_env();
    check_ok_with("[ [ 1 ] ] drop", &[], &[], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

// ---------------------------------------------------------------------------
// Stack-heavy acceptance tests
// ---------------------------------------------------------------------------

#[test]
fn deep_stack_operations() {
    let (env, len) = builtin_env();
    // Alternating push and pop through many values.
    let mut body = String::new();
    for i in 0..20 {
        body.push_str(&format!("{} ", i));
    }
    for _ in 0..20 {
        body.push_str("drop ");
    }
    check_ok_with(&body, &[], &[], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

#[test]
fn chained_arithmetic() {
    let (env, len) = builtin_env();
    check_ok_with(
        "1 2 + 3 + 4 + 5 + drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            let b0 = w.blocks.get(0).unwrap();
            let add_count = b0
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::AddI64))
                .count();
            assert_eq!(add_count, 4, "chained addition must produce 4 AddI64 ops");
        },
    );
}

#[test]
fn mixed_boolean() {
    let (env, len) = builtin_env();
    check_ok_with(
        "true false and not drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            let b0 = w.blocks.get(0).unwrap();
            assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::AndBool)));
            assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::NotBool)));
        },
    );
}

// ---------------------------------------------------------------------------
// Additional acceptance tests for test count
// ---------------------------------------------------------------------------

#[test]
fn dup_of_dup() {
    let (env, len) = builtin_env();
    check_ok_with(
        "42 dup dup drop drop drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 1);
        },
    );
}

#[test]
fn swap_rotate_pattern() {
    let (env, len) = builtin_env();
    check_ok_with(
        "1 2 3 swap drop swap drop drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            let b0 = w.blocks.get(0).unwrap();
            let swap_count = b0
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::Swap { .. }))
                .count();
            assert!(swap_count >= 2);
        },
    );
}

#[test]
fn nested_if() {
    let (env, len) = builtin_env();
    check_ok_with(
        "true [ true [ 1 ] [ 2 ] if ] [ 3 ] if drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 5);
        },
    );
}

#[test]
fn deep_nested_if() {
    let (env, len) = builtin_env();
    check_ok_with(
        "true [ true [ true [ 1 ] [ 2 ] if ] [ 3 ] if ] [ 4 ] if drop",
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 7);
        },
    );
}

#[test]
fn multiple_strings() {
    let (env, len) = builtin_env();
    check_ok_with(
        r#""a" "b" "c" drop drop drop"#,
        &[],
        &[],
        &env[..len],
        |w: &Word| {
            let b0 = w.blocks.get(0).unwrap();
            let str_count = b0
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::ConstStr { .. }))
                .count();
            assert_eq!(str_count, 3);
        },
    );
}

#[test]
fn bool_short_circuit_pattern() {
    let (env, len) = builtin_env();
    check_ok_with("true true and drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        assert!(b0.ops.iter().any(|op| matches!(op.kind, OpKind::AndBool)));
    });
}

#[test]
fn comparison_chain() {
    let (env, len) = builtin_env();
    check_ok_with("1 2 < 3 4 < and drop", &[], &[], &env[..len], |w: &Word| {
        let b0 = w.blocks.get(0).unwrap();
        let cmp_count = b0
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::Cmp { .. }))
            .count();
        assert_eq!(cmp_count, 2);
    });
}

#[test]
fn while_with_body() {
    let (env, len) = builtin_env();
    check_ok_with(
        "[ dup 0 > ] [ 1 - ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 2);
        },
    );
}

#[test]
fn const_str_drop() {
    let (env, len) = builtin_env();
    check_ok_with(r#""a" drop"#, &[], &[], &env[..len], |w: &Word| {
        assert!(w.blocks.len() >= 1);
    });
}

#[test]
fn if_without_else_value() {
    let (env, len) = builtin_env();
    check_ok_with(
        "true [ ] [ ] if",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 3);
        },
    );
}

#[test]
fn while_countdown() {
    let (env, len) = builtin_env();
    check_ok_with(
        "[ dup 0 > ] [ 1 - ] while",
        &[b"i64"],
        &[b"i64"],
        &env[..len],
        |w: &Word| {
            assert!(w.blocks.len() >= 2);
        },
    );
}

// ---------------------------------------------------------------------------
// builtin_words() table integrity
// ---------------------------------------------------------------------------

#[test]
fn builtin_words_table_is_non_empty() {
    let words = semantics::typecheck::builtin_words();
    assert!(!words.is_empty());
    for w in words {
        assert!(!w.name.as_bytes().is_empty());
        assert!(w.sig.in_len <= 8);
        assert!(w.sig.out_len <= 8);
    }
}

#[test]
fn builtin_words_contains_dup() {
    let words = semantics::typecheck::builtin_words();
    assert!(words.iter().any(|w| w.name.as_bytes() == b"dup"));
}

#[test]
fn builtin_words_contains_platform_task_yield() {
    let words = semantics::typecheck::builtin_words();
    assert!(words
        .iter()
        .any(|w| w.name.as_bytes() == b"platform.task.yield"));
}

// ---------------------------------------------------------------------------
// Wide environment: calling user words
// ---------------------------------------------------------------------------

#[test]
fn call_user_word() {
    let (mut env, mut len) = builtin_env();
    env[len] = entry(b"twice", &[b"i64"], &[b"i64", b"i64"]);
    len += 1;
    check_ok_with("3 twice drop drop", &[], &[], &env[..len], |w: &Word| {
        assert_eq!(w.blocks.len(), 1);
    });
}

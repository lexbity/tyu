//! `.def` boundary grammar extension (static-verification.md §6.6, slice P6).
//!
//! The interface grammar is additive (Q7): a `.def` word declaration may
//! carry the boundary annotation `performs ( … )` (paren form — the module
//! source uses braces) and the contract clauses `needs [ name ]` /
//! `ensures [ name ]` naming callee-module-local predicate words. NO
//! `bound` clause exists: the design rejects hand-declared bounds (retired
//! E5102; stack-bound-analysis §13), so a future/foreign `.def` carrying one
//! must fail loudly (E6413 band), never silently absorb it into the body.

//! test-only convenience; plain `&ast.decls` borrows work for single reads.
use frontend::parse::{DeclAst, DeclKind, Parser};

fn parse_module(src: &str) -> frontend::parse::ModuleAst {
    Parser::new(src.as_bytes())
        .parse_module_ast()
        .expect("module must parse")
}

fn first_decl(ast: &frontend::parse::ModuleAst) -> &DeclAst {
    assert_eq!(ast.decls.len(), 1, "fixture must declare exactly one word");
    ast.decls.iter().next().unwrap()
}

#[test]
fn def_word_accepts_paren_performs_annotation() {
    // `.def` boundary form: `performs ( suspend )` — parens per §6.6; the
    // lowering consumes the same effect bits as the module's brace form.
    let ast = parse_module(
        "module Bank;\n: withdraw ( i64 -- bool )\n  performs ( suspend )\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert_eq!(d.kind, DeclKind::Word);
    assert!(d.has_explicit_performs);
    assert_ne!(
        d.effect_bits, 0,
        "performs ( suspend ) must set effect bits"
    );
}

#[test]
fn def_word_accepts_multi_effect_paren_performs() {
    let ast = parse_module("module Bank;\n: w ( -- )\n  performs ( suspend, mmio ) ;\nend;\n");
    let d = first_decl(&ast);
    assert_eq!(d.kind, DeclKind::Word);
    let bits = d.effect_bits;
    assert_ne!(bits & 1, 0, "SUSPEND bit");
    assert_ne!(bits & (1 << 3), 0, "MMIO bit");
}

#[test]
fn def_word_accepts_needs_and_ensures_clauses() {
    let ast = parse_module(
        "module Bank;\n: withdraw ( bool -- bool )\n  needs [ pct-in-range ]\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(d.requires.is_some(), "needs clause must be captured");
    let ast2 = parse_module(
        "module Bank;\n: bump ( bool -- bool )\n  ensures [ ret-nonzero ]\n  drop true ;\nend;\n",
    );
    let d2 = first_decl(&ast2);
    assert!(d2.ensures.is_some(), "ensures clause must be captured");
}

#[test]
fn def_word_accepts_performs_plus_contract_clauses() {
    // The clause loop accepts any order (BUG-011 discipline, extended to the
    // paren performs form): performs then needs.
    let ast = parse_module(
        "module Bank;\n: withdraw ( bool -- bool )\n  performs ( suspend )\n  needs [ pct-in-range ]\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(d.has_explicit_performs);
    assert!(d.requires.is_some());
}

#[test]
fn bound_clause_is_rejected_with_e6413() {
    // The one clause the design explicitly rejected: a hand-declared bound.
    let err = Parser::new(
        b"module Bank;\n: withdraw ( bool -- bool )\n  bound [ 4 ]\n  drop true ;\nend;\n",
    )
    .parse_module_ast();
    let code = match err {
        Err(e) => e.code(),
        Ok(_) => panic!("a `bound` clause must be rejected, not silently absorbed"),
    };
    assert_eq!(code, 6413, "bound-clause rejection must surface E6413");
}

#[test]
fn module_brace_performs_form_still_parses() {
    // The module-source form is unchanged.
    let ast = parse_module(
        "module Bank;\n: w ( bool -- bool )\n  performs { suspend }\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(d.has_explicit_performs);
    assert_ne!(d.effect_bits, 0);
}

#[test]
fn legacy_requires_contract_migration_hint_preserved() {
    let err = Parser::new(
        b"module Bank;\n: w ( bool -- bool )\n  requires [ pred ]\n  drop true ;\nend;\n",
    )
    .parse_module_ast();
    let code = match err {
        Err(e) => e.code(),
        Ok(_) => panic!("legacy requires [ ... ] must be rejected"),
    };
    assert_eq!(code, 2195, "legacy requires [ … ] keeps its migration hint");
}

// ---------------------------------------------------------------------------
// PLAN-VERIFY-3 §Q17 (P1.2): the authored `intent "<text>"` clause.
// ---------------------------------------------------------------------------

#[test]
fn intent_clause_captured_after_needs() {
    let ast = parse_module(
        "module Bank;\n: withdraw ( bool -- bool )\n  needs [ pct-in-range ] intent \"withdraw never exceeds balance\"\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(
        d.intent.is_some(),
        "intent clause after needs must be captured"
    );
    assert!(d.requires.is_some(), "needs clause must be captured");
}

/// After `ensures`, the intent clause is captured the same way (the clause
/// parser accepts any order of the contract/performs group).
#[test]
fn intent_clause_captured_after_ensures() {
    let ast = parse_module(
        "module Bank;\n: bump ( bool -- bool )\n  ensures [ ret-nonzero ] intent \"returns a nonzero flag\"\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(d.ensures.is_some());
    assert!(d.intent.is_some(), "intent after ensures must be captured");
}

/// The `intent` string's content is retrievable from the source via the span
/// (quotes included in the span): `"withdraw never exceeds balance"`.
#[test]
fn intent_span_covers_quoted_literal() {
    let src = "module Bank;\n: withdraw ( bool -- bool )\n  needs [ pct-in-range ] intent \"withdraw never exceeds balance\"\n  drop true ;\nend;\n";
    let ast = parse_module(src);
    let d = first_decl(&ast);
    let span = d.intent.expect("intent clause must be captured");
    assert!(
        src.as_bytes()[span.start] == b'"',
        "span starts at the opening quote"
    );
    assert!(
        src.as_bytes()[span.end - 1] == b'"',
        "span ends at the closing quote"
    );
    let text = &src[span.start + 1..span.end - 1];
    assert_eq!(text, "withdraw never exceeds balance");
}

/// `intent` without a string literal is a parse error (E2197), never a
/// silent skip into the body.
#[test]
fn intent_without_string_is_rejected() {
    let err = Parser::new(
        b"module Bank;\n: w ( bool -- bool )\n  ensures [ p ] intent 42\n  drop true ;\nend;\n",
    )
    .parse_module_ast();
    let code = match err {
        Err(e) => e.code(),
        Ok(_) => panic!("intent must require a string literal"),
    };
    assert_eq!(code, 2197, "intent-without-string must surface E2197");
}

/// PLAN-VERIFY-3 §Q17 permits the intent clause at any position in the
/// contract/performs group (BUG-011 order discipline) — including before the
/// contract clause it belongs to; when no clause follows it simply binds
/// nothing (the driver only consumes it at a contract obligation).
#[test]
fn intent_accepted_in_clause_group_any_order() {
    let ast = parse_module(
        "module Bank;\n: w ( bool -- bool )\n  intent \"x\"\n  ensures [ p ]\n  drop true ;\nend;\n",
    );
    let d = first_decl(&ast);
    assert!(d.ensures.is_some());
    assert!(
        d.intent.is_some(),
        "intent in the clause group must be captured"
    );
}

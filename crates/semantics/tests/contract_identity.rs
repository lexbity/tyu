//! Contract predicate identity (static-verification.md Q6, FR-8, slice P6).
//!
//! The E3312 modified-inputs check compares *Origins*, not static types: the
//! interval interpreter's Origin lattice tracks derivation through
//! `dup`/`drop`/`swap`/binops and control-flow joins, so an identity move is
//! visible where the old `TypeAtom` equality was blind:
//!
//! - `needs [ swap true ]` — two same-typed inputs swapped: E3312 fires (the
//!   identity moved) — the exact hole the type-level check could not see;
//! - `needs [ drop true true ]` — subject replaced by a constant: E3312 fires;
//! - `needs [ dup 1 + 2 > ]` — type-preserving arithmetic on a *copy*: legal
//!   (the subject slot keeps its Arg origin);
//! - the book's range idiom (`dup 0 >= [ dup 100 <= ] [ 0 0 == ] if`):
//!   legal — both branches preserve the subject's origin, so the join keeps
//!   it.

mod common;

use std::thread;

use common::builtin_env;
use frontend::parse::Parser;

/// Drive `for_each_ir_word` over the module exactly as langc's drivers do.
/// Returns `Ok(())` or the typecheck error code.
fn compile(source: Vec<u8>) -> Result<(), u32> {
    thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let src = source;
            let module = Parser::new(&src).parse_module_ast().expect("valid module");
            let (env, len) = builtin_env();
            let env: Vec<common::WordEntry> = env[..len].to_vec();
            let subtypes: Vec<SubtypeInfo> = Vec::new();
            let mut resources = semantics::typecheck::db::build_resource_db(&module, &src)
                .expect("resource db");
            match semantics::typecheck::for_each_ir_word(
                &module,
                &src,
                &env,
                &subtypes,
                semantics::typecheck::ChecksMode::All,
                false,
                &mut resources,
                None,
                None,
                None,
                false,
                |_w, _ctx| Ok::<(), ()>(()),
            ) {
                Ok(()) => Ok(()),
                Err(e) => Err(match e {
                    semantics::typecheck::ForEachIrError::Type(t) => t.code(),
                    semantics::typecheck::ForEachIrError::Consumer(_) => 0,
                }),
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

use semantics::typecheck::db::SubtypeInfo;

#[test]
fn swap_then_bool_fires_e3312() {
    // Two same-typed inputs swapped — the E3312 hole the plan closes (Q6).
    let mod_src = "\
module M;
: f ( i64 i64 -- bool )
  needs [ swap true ]
  drop drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3312));
}

#[test]
fn drop_true_true_fires_e3312() {
    // The subject is replaced by a constant: identity lost.
    let mod_src = "\
module M;
: f ( i64 i64 -- bool )
  needs [ drop true true ]
  drop drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3312));
}

#[test]
fn type_preserving_arithmetic_on_copy_passes() {
    // `dup 1 +` refines the *copy*; the subject slot keeps its Arg origin —
    // the shape the plan calls "type-preserving arithmetic stays legal".
    let mod_src = "\
module M;
: f ( i64 -- bool )
  needs [ dup 1 + 2 > ]
  drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Ok(()));
}

#[test]
fn book_two_sided_range_idiom_passes() {
    // The book's own `if`-combinator range idiom (ch04 §4.4) must keep
    // compiling: both branches restore the subject, the join preserves the
    // Arg origin, and the comparison ops run on copies.
    let mod_src = "\
module M;
: f ( i64 -- i64 )
  needs [ dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ]
  as i64 ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Ok(()));
}

#[test]
fn pure_true_predicate_passes() {
    let mod_src = "\
module M;
: f ( i64 -- bool )
  needs [ dup 0 >= ]
  drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Ok(()));
}

#[test]
fn ensures_swap_of_outputs_fires_e3312() {
    // The same origin rule applies to `ensures`: the results the predicate
    // claims about may not be moved.
    let mod_src = "\
module M;
: f ( -- i64 i64 )
  ensures [ swap true ]
  1 2 ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3312));
}
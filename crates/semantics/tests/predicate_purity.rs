//! Contract predicate purity and size (static-verification.md Q6, FR-7,
//! slice P6).
//!
//! A contract predicate is a *question*: it computes one boolean from the
//! values it claims about, with no side roads. The ContractPredicate context
//! row forbids every effect; `Store`/`TaskSpawn` are not effects in the
//! vocabulary, so their rejection is syntactic (the "also" column, exactly
//! like `lock`'s stack neutrality) — both are E3313. A store to memory and a
//! task spawn inside `needs`/`ensures` therefore fail E3313, while pure
//! predicates pass; a predicate whose data-stack peak exceeds the word-level
//! cap (64 slots) fails E3314.
//!
//! Calls to SUSPEND-performing words inside a predicate are rejected by the
//! *existing* ambient-forbid check (E5001 via `suspend_blocker` — this is
//! the "existing check (2)" the plan's Q6 says does the effect work).

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
fn pure_constant_predicate_passes() {
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
fn store_in_needs_fires_e3313() {
    // A `!i64` store inside the predicate — memory writes are not questions.
    let mod_src = "\
module M;
: f ( i64 -- bool )
  needs [ 0 !i64 true ]
  drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3313));
}

#[test]
fn store_in_ensures_fires_e3313() {
    let mod_src = "\
module M;
: f ( i64 -- i64 )
  ensures [ 0 !i64 true ]
  1 + ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3313));
}

#[test]
fn task_spawn_in_needs_fires_e3313() {
    // `platform.task.spawn` inside the predicate — spawning is not a question.
    let mod_src = "\
module M;
: f ( i64 -- bool )
  needs [ [ ] platform.task.spawn drop true ]
  drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3313));
}

#[test]
fn suspend_call_in_needs_fires_existing_check() {
    // A SUSPEND-performing callee is rejected by the ambient forbid fold's
    // existing check (Q6: "the ambient forbid fold rejects calls to
    // effect-performing words via the existing check (2)") — E5001, the
    // contract band is not used for what an existing check already names.
    let mod_src = "\
module M;
: f ( i64 -- bool )
  needs [ platform.task.yield true ]
  drop true ;
end;
";
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(5001));
}

#[test]
fn oversized_predicate_fires_e3314() {
    // 65 `dup`s push the predicate peak to 66 slots — past the word-level
    // 64-slot cap (E3314).
    let dup_block = "dup ".repeat(65);
    let mod_src = format!(
        "module M;\n: f ( i64 -- bool )\n  needs [ {} true ]\n  drop true ;\nend;\n",
        dup_block
    );
    assert_eq!(compile(mod_src.as_bytes().to_vec()), Err(3314));
}
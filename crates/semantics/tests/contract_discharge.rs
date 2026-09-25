//! In-tree `contract-post` discharge (static-verification.md Q6, slice P6).
//!
//! The epilogue's inline `ensures` predicate leaves its abstract verdict on
//! the interval state's top; the in-tree engine resolves the contract-post
//! obligation from it: a constant output (`5` with `ensures [ dup 0 >= ]`)
//! discharges; an input-derived output (`1 +`) stays open. This is the
//! C6-side of FR-5/FR-13: the emission decision sits with the record site.
//! Driven exactly as langc's drivers do, with an empty-but-valid verdicts
//! file.

mod common;

use std::thread;

use common::builtin_env;
use frontend::parse::Parser;
use verifier::model::{ExtractionCtx, Kind};

fn compile_with_extraction(source: Vec<u8>) -> (Result<(), u32>, Vec<verifier::model::ResolvedVerdict>) {
    thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let src = source;
            let module = Parser::new(&src).parse_module_ast().expect("valid module");
            let (env, len) = builtin_env();
            let mut env: Vec<common::WordEntry> = env[..len].to_vec();
            // langc's `load_local_sigs` pre-seeds every local word (with an
            // empty sig) before the summary fixpoint; mirror it here.
            env.push(common::entry(b"five", &[], &[b"i64"]));
            env.push(common::entry(b"inputy", &[b"i64"], &[b"i64"]));
            env.push(common::entry(b"main", &[], &[b"i64"]));
            let subtypes: Vec<SubtypeInfo> = Vec::new();
            let mut resources = semantics::typecheck::db::build_resource_db(&module, &src)
                .expect("resource db");
            let mut ctx = ExtractionCtx::new(b"P");
            match semantics::typecheck::for_each_ir_word(
                &module,
                &src,
                &env,
                &subtypes,
                semantics::typecheck::ChecksMode::Undischarged,
                false,
                &mut resources,
                None,
                Some(&mut ctx),
                None,
                false,
                |_w, _ctx| Ok::<(), ()>(()),
            ) {
                Ok(()) => (Ok(()), ctx.resolved().to_vec()),
                Err(e) => (
                    Err(match e {
                        semantics::typecheck::ForEachIrError::Type(t) => t.code(),
                        semantics::typecheck::ForEachIrError::Consumer(_) => 0,
                    }),
                    Vec::new(),
                ),
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

use semantics::typecheck::db::SubtypeInfo;

const FIVE_AND_INPUTY: &str = "\
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

#[test]
fn constant_ensures_discharges_input_derived_stays_open() {
    let (result, resolved) = compile_with_extraction(FIVE_AND_INPUTY.as_bytes().to_vec());
    assert_eq!(result, Ok(()), "the module must compile");
    let posts: Vec<&verifier::model::ResolvedVerdict> = resolved
        .iter()
        .filter(|r| r.kind == Kind::ContractPost)
        .collect();
    assert_eq!(posts.len(), 2, "two words with ensures → two contract-post records");
    // `five` (constant output) discharges; `inputy` (⊤ output) stays open.
    let five = posts
        .iter()
        .find(|r| r.id.contains("five::contract-post"))
        .expect("five's contract-post record");
    let inputy = posts
        .iter()
        .find(|r| r.id.contains("inputy::contract-post"))
        .expect("inputy's contract-post record");
    assert_eq!(five.status, verifier::verdict::VerdictStatus::Discharged);
    assert_eq!(inputy.status, verifier::verdict::VerdictStatus::Open);
}
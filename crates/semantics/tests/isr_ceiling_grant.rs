//! ISR ceiling grant sourcing (static-verification.md §6.4, slice P3, FR-10).
//!
//! The per-ISR ceiling (`N_isr`) comes from the compiled descriptor's
//! `[verification] isr_stack_slots` grant — the literal 32 is gone. A pack
//! grant of 40 lets a handler peaking at 34 slots compile; the default (32)
//! rejects it with E5030, byte-compatible with the pre-grant behavior.

mod common;

use std::thread;

use codegen_core::compiled_desc::{CompiledDescriptor, VerificationGrants};
use common::builtin_env;
use frontend::parse::Parser;

/// A handler-only module: an `@interrupt(TIMER0)` word peaking at 34 slots.
const HANDLER_PEAK_34: &str = "\
module Main;
@interrupt(TIMER0) : isr ( -- )
  0
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop
;
end;
";

/// Drive `for_each_ir_word` over the module exactly as langc's drivers do,
/// with the given ISR grant. Returns `Ok(())` or the typecheck error code.
fn compile_with_isr_grant(source: Vec<u8>, isr_stack_slots: u32) -> Result<(), u32> {
    thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let src = source;
            let module = Parser::new(&src).parse_module_ast().expect("valid module");
            let (env, len) = builtin_env();
            let env: Vec<common::WordEntry> = env[..len].to_vec();
            let subtypes: Vec<SubtypeInfo> = Vec::new();

            let mut descriptor = CompiledDescriptor::default();
            descriptor.verification = VerificationGrants {
                isr_stack_slots,
                ..VerificationGrants::default()
            };

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
                Some(&descriptor),
                None,
                None,
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
fn isr_grant_above_peak_allows_the_handler() {
    // N_isr = 40 (descriptor grant) ≥ 34-peak handler → compiles.
    assert_eq!(compile_with_isr_grant(HANDLER_PEAK_34.as_bytes().to_vec(), 40), Ok(()));
}

#[test]
fn default_isr_grant_rejects_peak_above_32() {
    // N_isr = 32 (the historical budget, kept when a pack omits the grant) →
    // E5030, exactly the pre-grant behavior (FR-10 compatibility).
    assert_eq!(
        compile_with_isr_grant(HANDLER_PEAK_34.as_bytes().to_vec(), 32),
        Err(5030),
        "a 34-peak ISR under the 32-slot default must still be E5030"
    );
}

#[test]
fn boundary_exact_fit_is_legal() {
    // peak == grant is allowed (the check is `high > N_isr` rejects).
    assert_eq!(compile_with_isr_grant(HANDLER_PEAK_34.as_bytes().to_vec(), 34), Ok(()));
}
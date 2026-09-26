//! `verifier::gen` generator tests (PLAN-VERIFY-3 P2.3).
//!
//! These are integration tests (not `#[cfg(test)]` in `src/gen.rs`) because
//! CI gate G13 bans `panic!`/`unreachable!` text in `crates/verifier/src`
//! (production code paths); tests live in `tests/`.

use ir::OpKind;
use verifier::gen::{gen_program, MemDesc, Rng, SUB_HI, SUB_LO};
use verifier::target::TargetSpec;

/// Determinism: a fixed `(seed, spec, mem, n_inputs, cast_final)` reproduces a
/// byte-identical program stream.
#[test]
fn generator_is_deterministic_under_a_fixed_seed() {
    let mut a = Rng::new(0x1234_5678_9abc_def0);
    let mut b = Rng::new(0x1234_5678_9abc_def0);
    let mem = MemDesc::ram_and_dev_script(0x1000, 0x2000, 7);
    for _ in 0..64 {
        let pa = gen_program(&mut a, TargetSpec::X86_64, &mem, 2, false);
        let pb = gen_program(&mut b, TargetSpec::X86_64, &mem, 2, false);
        assert_eq!(pa.domains, pb.domains);
        assert_eq!(pa.ops, pb.ops);
        assert_eq!(pa.target, pb.target);
        assert_eq!(pa.mem, mem);
    }
}

/// `MemDesc::flat` is a no-op: the op mix is byte-identical to a program
/// generated with the same seed under a descriptor that enables nothing.
#[test]
fn flat_mem_desc_is_behaviorally_identical() {
    let mut a = Rng::new(0xbadc_0ffe_0bad_00d5);
    let mut b = Rng::new(0xbadc_0ffe_0bad_00d5);
    for _ in 0..128 {
        let pa = gen_program(&mut a, TargetSpec::X86_64, &MemDesc::flat(), 2, false);
        let pb = gen_program(&mut b, TargetSpec::X86_64, &MemDesc::default(), 2, false);
        assert_eq!(pa, pb);
    }
}

/// Spec carry (§Q3): the program records the target identity it was generated
/// under, so a differential runner can always reproduce the `(spec, seed)`
/// pair.
#[test]
fn generator_carries_the_target_identity() {
    let mut rng = Rng::new(0xdead_beef);
    let p = gen_program(
        &mut rng,
        TargetSpec::ARM_V7M_UNKNOWN_NONE,
        &MemDesc::flat(),
        1,
        true,
    );
    assert_eq!(p.spec, TargetSpec::ARM_V7M_UNKNOWN_NONE);
    assert!(p.cast_site);
    assert_eq!(p.target, (SUB_LO, SUB_HI));
}

/// Generator contract: (a) only generator-emittable ops appear, (b) every
/// `Cast` is the final op of a `cast_final` program, and (c) the generator's
/// own primitive-depth tracker never goes negative (its safety net). The
/// genuine soundness property — a discharged program has no violating concrete
/// run — is the differential harness's job (`soundness_differential`), not a
/// unit test's. (An emptied abstract stack answers `⊤` and can never
/// discharge, so the generator's rare underflow-tolerant emissions are
/// conservative by construction.)
#[test]
fn generator_programs_are_stack_safe() {
    for seed in 0..32u64 {
        let mut rng = Rng::new(seed);
        for _ in 0..100 {
            let n = 1 + (rng.below(3) as usize);
            let cast_final = rng.below(2) == 0;
            let mem = if seed % 2 == 0 {
                MemDesc::ram_and_dev_script(0x1000, 0x2000, 7)
            } else {
                MemDesc::flat()
            };
            let p = gen_program(&mut rng, TargetSpec::X86_64, &mem, n, cast_final);
            assert_eq!(p.n_inputs(), n);
            let mut prim_depth: i64 = n as i64;
            for (i, op) in p.ops.iter().enumerate() {
                let net = match op {
                    OpKind::ConstI64(_)
                    | OpKind::ConstBool(_)
                    | OpKind::ConstStr(_)
                    | OpKind::LocalGet { .. } => 1,
                    OpKind::AddI64
                    | OpKind::SubI64
                    | OpKind::MulI64
                    | OpKind::Cmp { .. }
                    | OpKind::AndBool
                    | OpKind::OrBool => -1,
                    OpKind::Store { .. }
                    | OpKind::MmioVolStore { .. }
                    | OpKind::MmioVolStoreField { .. } => -2,
                    OpKind::Load { .. }
                    | OpKind::MmioVolLoad { .. }
                    | OpKind::MmioVolLoadField { .. }
                    | OpKind::Cast { .. }
                    | OpKind::NotBool
                    | OpKind::Bitcast { .. } => 0,
                    OpKind::Swap { .. } => 0,
                    OpKind::Dup { .. } => 1,
                    OpKind::Drop { .. } | OpKind::LocalSet { .. } => -1,
                    _ => panic!("generator emitted an op it should not: {op:?}"),
                };
                prim_depth += net;
                assert!(
                    prim_depth >= 0,
                    "seed {seed}: generator's own depth tracker went negative at op {i} ({op:?})"
                );
                if let OpKind::Cast { .. } = op {
                    assert!(cast_final, "seed {seed}: cast mid-stream");
                    assert_eq!(i, p.ops.len() - 1, "seed {seed}: cast not final");
                }
            }
        }
    }
}

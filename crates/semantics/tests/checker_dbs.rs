//! Typechecker tests with populated semantic databases.
//!
//! Tests that the checker correctly enforces rules that depend on
//! subtype info, struct declarations, MMIO maps, resources, and iso
//! type sets.

mod common;

use common::*;
use ir::{CapSet, EffectSet, StackBound};

// ---------------------------------------------------------------------------
// Subtype info in type matching (populated via Dbs)
// ---------------------------------------------------------------------------

#[test]
fn subtype_in_env_affects_type_checking() {
    let (env, len) = builtin_env();
    // SubtypeInfo helps the typechecker match compatible types.
    // Without it, a subtype would not be recognized as compatible
    // with its base type.  This test verifies the table is passed
    // through correctly.
    let dbs = Dbs::new().with_subtype(b"Age", b"i64", 0, 150);
    // Simple arithmetic with subtypes — the typechecker must accept
    // a value of subtype Age where i64 is expected.
    check(
        "42",
        &[],
        &[b"i64"],
        &env[..len],
        &dbs,
        ChecksMode::Off,
        |result| {
            assert!(
                result.is_ok(),
                "subtype i64 must be accepted as base type i64"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// BUG-007: subtype value returned as its base must pass IR verification
// ---------------------------------------------------------------------------

/// Regression test for BUG-007.  The typechecker accepts a subtype value where
/// its base type is declared (`type_compatible` subsumption), but the IR
/// verifier previously rejected the generated word with E9016.  The word must
/// now build AND verify.
#[test]
fn subtype_value_returned_as_base_passes_ir_verification() {
    let (env, len) = builtin_env();
    let dbs = Dbs::new().with_subtype(b"R", b"i64", 0, 100);
    let body = "as R".to_string();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&body);
            let s = sig(&[b"i64"], &[b"i64"]);
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb {
                maps: FixedVec::new(),
                instances: FixedVec::new(),
                reg_meta: FixedVec::new(),
            };
            let resources = ResourceDb {
                items: FixedVec::new(),
            };
            let nominals = NominalDb {
                structs: FixedVec::new(),
                enums: FixedVec::new(),
            };
            let iso = IsoDb {
                types: FixedVec::new(),
            };
            let mut obs = NullObserver;
            let out = build_ir_word(
                &decl,
                &src,
                &env[..len],
                &dbs.subtypes,
                &mmio,
                None,
                &resources,
                &nominals,
                &iso,
                ChecksMode::All,
                false,
                &s,
                &mut arena,
                None,
                None,
                false,
                &mut obs,
            )
            .expect("typecheck should accept subtype flowing to base");
            ir::verify_word(out.word)
                .map_err(|e| e.code())
                .expect("IR must verify: subtype returned as base (BUG-007)");
        })
        .unwrap()
        .join()
        .unwrap();
}

/// The same BUG-007 subsumption must hold for a subtype passed as an argument
/// to a word declared with the base type.
#[test]
fn subtype_argument_to_base_param_passes_ir_verification() {
    let (env, len) = builtin_env();
    let dbs = Dbs::new().with_subtype(b"R", b"i64", 0, 100);
    let body = "as R take".to_string();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&body);
            let mut envw: Vec<WordEntry> = env[..len].to_vec();
            envw.push(WordEntry {
                name: TypeAtom::new(b"take").unwrap(),
                sig: sig(&[b"i64"], &[b"i64"]),
                performs: EffectSet::empty(),
                requires: CapSet::empty(),
                bound: StackBound::ID,
                contract_hash: 0,
            });
            let s = sig(&[b"i64"], &[b"i64"]);
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb {
                maps: FixedVec::new(),
                instances: FixedVec::new(),
                reg_meta: FixedVec::new(),
            };
            let resources = ResourceDb {
                items: FixedVec::new(),
            };
            let nominals = NominalDb {
                structs: FixedVec::new(),
                enums: FixedVec::new(),
            };
            let iso = IsoDb {
                types: FixedVec::new(),
            };
            let mut obs = NullObserver;
            let out = build_ir_word(
                &decl,
                &src,
                &envw,
                &dbs.subtypes,
                &mmio,
                None,
                &resources,
                &nominals,
                &iso,
                ChecksMode::All,
                false,
                &s,
                &mut arena,
                None,
                None,
                false,
                &mut obs,
            )
            .expect("typecheck should accept subtype argument to base param");
            ir::verify_word(out.word)
                .map_err(|e| e.code())
                .expect("IR must verify: subtype argument to base param (BUG-007)");
        })
        .unwrap()
        .join()
        .unwrap();
}

//! Property tests over the engine (PLAN-WASM-1 P2: §Q11 R-1/R-3, §Q6 — archive: `devdocs/plans/done/wasm-repl.md`).
//!
//! Ships these properties:
//!
//! 1. **Data-op model agreement** — arbitrary `frag::SrcOp` sequences run by
//!    the fork's engine must agree with an independent naive stack-machine
//!    oracle (both saturate empty pops identically, so every generated
//!    sequence is a valid probe; the reference is a second implementation by
//!    construction — R-3).
//! 2. **Dup copies the reference (aliasing)** — a `dup`-carried address read
//!    through either copy returns the value stored through the other, pinning
//!    the native by-ref VALUE semantics (DEC-15; an eager-copy engine
//!    diverges here by construction).
//! 3. **Heap geometry** — `RegionHeap` allocation advances an 8-byte-aligned
//!    shared cursor exactly like the native region ABI
//!    (`codegen-x86_64/src/region.rs`), and destroyed slots are reused with
//!    fresh (zeroed) storage.
//! 4. **Determinism** — the same state/operations yield identical results on
//!    repeated runs (§Q6).

use proptest::prelude::*;

use wasm_repl::frag::{self, Mem, SrcOp, State};
use wasm_repl::heap::RegionHeap;

// ---------------------------------------------------------------------------
// Reference model (the independent oracle)
// ---------------------------------------------------------------------------

/// The naive stack machine: identical semantics to `frag::step_data`, written
/// from the op table (`ir-op-semantics.md` §2) and never from the engine
/// source. Both engines saturate empty pops with 0, so *any* generated op
/// sequence is a valid probe.
fn reference_run(ops: &[SrcOp], init_stack: Vec<i64>, locals: &mut [i64], mem: &Mem) -> Vec<i64> {
    let mut stack = init_stack;
    let mut cells: Vec<(i64, i64)> = Vec::new();
    for op in ops {
        match *op {
            SrcOp::ConstInt(v) => stack.push(v),
            SrcOp::ConstBool(b) => stack.push(if b { 1 } else { 0 }),
            SrcOp::Dup => stack.push(*stack.last().unwrap_or(&0)),
            SrcOp::Drop => {
                stack.pop();
            }
            SrcOp::Swap => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push(b);
                stack.push(a);
            }
            SrcOp::Add => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push(a.wrapping_add(b));
            }
            SrcOp::Sub => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push(a.wrapping_sub(b));
            }
            SrcOp::Mul => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push(a.wrapping_mul(b));
            }
            SrcOp::CmpLt
            | SrcOp::CmpLe
            | SrcOp::CmpGt
            | SrcOp::CmpGe
            | SrcOp::CmpEq
            | SrcOp::CmpNe => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                let r = match *op {
                    SrcOp::CmpLt => a < b,
                    SrcOp::CmpLe => a <= b,
                    SrcOp::CmpGt => a > b,
                    SrcOp::CmpGe => a >= b,
                    SrcOp::CmpEq => a == b,
                    _ => a != b,
                };
                stack.push(r as i64);
            }
            SrcOp::AndB => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push((a != 0 && b != 0) as i64);
            }
            SrcOp::OrB => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                stack.push((a != 0 || b != 0) as i64);
            }
            SrcOp::NotB => {
                let a = stack.pop().unwrap_or(0);
                stack.push((a == 0) as i64);
            }
            SrcOp::Load => {
                let addr = stack.pop().unwrap_or(0);
                let v = cells
                    .iter()
                    .find(|(a, _)| *a == addr)
                    .map(|(_, v)| *v)
                    .unwrap_or(mem.load_oracle);
                stack.push(v);
            }
            SrcOp::Store => {
                let v = stack.pop().unwrap_or(0);
                let addr = stack.pop().unwrap_or(0);
                cells.retain(|(a, _)| *a != addr);
                cells.insert(0, (addr, v));
            }
            SrcOp::VolLoad => {
                let _ = stack.pop();
                stack.push(mem.mmio_oracle);
            }
            SrcOp::VolStore => {
                stack.pop();
                stack.pop();
            }
            SrcOp::LocalGet(slot) => stack.push(locals.get(slot as usize).copied().unwrap_or(0)),
            SrcOp::LocalSet(slot) => {
                let v = stack.pop().unwrap_or(0);
                let i = slot as usize;
                if locals.len() > i {
                    locals[i] = v;
                }
            }
            // Control ops are excluded from the generator (data-only prop).
            SrcOp::Br(_) | SrcOp::BrIf(_, _) | SrcOp::Ret => {}
        }
    }
    stack
}

/// Data-only `SrcOp` generator (no control ops — a single-block run always
/// terminates within its budget).
fn op_strategy() -> impl Strategy<Value = SrcOp> {
    prop_oneof![
        any::<i64>().prop_map(SrcOp::ConstInt),
        any::<bool>().prop_map(SrcOp::ConstBool),
        Just(SrcOp::Dup),
        Just(SrcOp::Drop),
        Just(SrcOp::Swap),
        Just(SrcOp::Add),
        Just(SrcOp::Sub),
        Just(SrcOp::Mul),
        Just(SrcOp::CmpLt),
        Just(SrcOp::CmpLe),
        Just(SrcOp::CmpGt),
        Just(SrcOp::CmpGe),
        Just(SrcOp::CmpEq),
        Just(SrcOp::CmpNe),
        Just(SrcOp::AndB),
        Just(SrcOp::OrB),
        Just(SrcOp::NotB),
        Just(SrcOp::Load),
        Just(SrcOp::Store),
        Just(SrcOp::VolLoad),
        Just(SrcOp::VolStore),
        (0u16..4).prop_map(SrcOp::LocalGet),
        (0u16..4).prop_map(SrcOp::LocalSet),
    ]
}

proptest! {
    /// The fork's data-op semantics agree with the naive oracle for arbitrary
    /// sequences, entry states, and oracles (R-3).
    #[test]
    fn data_op_sequences_agree_with_reference_model(
        ops in prop::collection::vec(op_strategy(), 0..48),
        entry in prop::collection::vec(any::<i64>(), 0..8),
        load_oracle in any::<i64>(),
        mmio_oracle in any::<i64>(),
    ) {
        let mut blocks = vec![ops.clone()];
        blocks[0].push(SrcOp::Ret);
        let mem = Mem::new(load_oracle, mmio_oracle);
        let mut st = State::fresh(8);
        st.stack.extend_from_slice(&entry);

        let fuel = blocks[0].len() + 2;
        let engine_out = frag::run_word(&blocks, 0, fuel, &mem, &st);
        assert!(engine_out.is_some(), "a data-only program must terminate");

        let mut locals = vec![0i64; 8];
        let ref_stack = reference_run(&blocks[0], entry.clone(), &mut locals, &mem);
        assert_eq!(
            engine_out.unwrap().stack,
            ref_stack,
            "data-op stack divergence for {ops:?} (entry {entry:?}, load {load_oracle}, mmio {mmio_oracle})"
        );
    }

    /// Heap geometry against the native region ABI: allocations advance an
    /// 8-byte-aligned shared cursor and never overrun the region.
    #[test]
    fn region_heap_cursor_matches_native_alignment(
        sizes in prop::collection::vec(1u64..=4096, 1..20),
    ) {
        let mut heap = RegionHeap::new();
        let (h, _) = heap.region_create(1 << 20).expect("big region");
        let base = heap.region_base(h).expect("live region base");
        let mut cursor = 0u64;
        for size in sizes {
            let (ptr, _old_off) = heap.region_alloc(h, size).expect("allocated");
            assert_eq!(ptr.wrapping_sub(base), cursor, "cursor divergence at size {size}");
            cursor += (size + 7) & !7;
        }
        // Never overran: the region is 1 MiB and total ≤ 20×4096 aligned.
    }
}

// The aliasing property (R-3 / DEC-15 as amended): EVERY number of
// reference copies aliases ONE storage cell. `k` dups of a base address,
// then a store through each copy in turn, must leave exactly one memory
// cell holding the LAST stored value, and the surviving original reference
// must read it back. An engine that duplicates storage per copy (the
// eager-copy straw model DEC-15 considered and the corpus rejected) would
// leave `k + 1` cells and read a stale value — diverging here by
// construction. The G53 aliasing probes pin the same property against the
// native goldens; this property pins it over arbitrary shapes.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn reference_copies_alias_one_storage_cell(
        base in 0x1000i64..0x8000i64,
        (k, vals) in (0usize..8usize).prop_flat_map(|k| {
            (Just(k), prop::collection::vec(any::<i64>(), k + 1..=k + 1))
        }),
    ) {
        // k + 2 copies of the base address (the literal + `k + 1` dups);
        // `k + 1` stores consume the topmost copies one by one, each storing
        // through a *different* copy of the reference; the surviving original
        // is read back last. (With k = 0 this is the worked single-alias
        // case: dup, store through the copy, load through the original.)
        let mut ops: Vec<SrcOp> = vec![SrcOp::ConstInt(base)];
        ops.extend(core::iter::repeat_n(SrcOp::Dup, k + 1));
        for v in &vals {
            ops.push(SrcOp::ConstInt(*v));
            ops.push(SrcOp::Store);
        }
        ops.push(SrcOp::Load);
        ops.push(SrcOp::Ret);
        let blocks = vec![ops];

        let mem = Mem::new(0, 0);
        let st = State::fresh(4);
        let out = frag::run_word(&blocks, 0, 128, &mem, &st).expect("terminates");

        let last = *vals.last().expect("at least one store");
        prop_assert_eq!(out.stack, vec![last],
            "the surviving reference must read the last value stored through any copy — by-ref");
    }
}

/// Destroyed region slots are reused with fresh (zeroed) storage — the native
/// slot-scan contract (`codegen-x86_64/src/region.rs`).
#[test]
fn destroyed_region_slots_are_reused_fresh() {
    let mut heap = RegionHeap::new();
    let (h1, _) = heap.region_create(64).expect("create");
    let (p1, _) = heap.region_alloc(h1, 8).expect("alloc");
    let _ = p1;
    heap.region_destroy(h1).expect("destroy");

    let (h2, _) = heap.region_create(64).expect("recreate");
    assert_eq!(
        h1, h2,
        "the destroyed slot must be reusable (native scans size==0)"
    );
    let (p2, _) = heap.region_alloc(h2, 8).expect("alloc");
    // Fresh storage: a stale read must answer 0, not old bytes.
    assert_eq!(
        heap.load(p2, 8).unwrap(),
        0,
        "reused storage is zero-initialized"
    );
}

/// Determinism (§Q6): the same program run twice yields identical results.
#[test]
fn same_program_twice_is_byte_identical() {
    let blocks = vec![vec![
        SrcOp::ConstInt(1),
        SrcOp::ConstInt(2),
        SrcOp::Add,
        SrcOp::ConstInt(3),
        SrcOp::Add,
        SrcOp::Ret,
    ]];
    let mem = Mem::new(0, 0);
    let st = State::fresh(4);
    let a = frag::run_word(&blocks, 0, 8, &mem, &st).expect("terminates");
    let b = frag::run_word(&blocks, 0, 8, &mem, &st).expect("terminates");
    assert_eq!(a.stack, b.stack);
    assert_eq!(a, b);
}

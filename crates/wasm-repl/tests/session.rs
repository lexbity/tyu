//! Session-layer unit tests (PLAN-WASM-1 P3): transactional semantics
//! (DEC-4..DEC-6, §7.2/§7.3), journal-reversal ordering, FR-5 guard elision,
//! and the transactional properties — asserted on the API directly, not
//! through the CLI transcript. P4 adds the session-level MMIO surface:
//! descriptor attached → phrase-level runs against the session peripheral
//! (`mmio_sessions_run_against_the_session_device`).
//!
//! The compile path runs on the cargo-configured test-thread stack
//! (`.cargo/config.toml` sets `RUST_MIN_STACK` — the P3 root-cause fixes
//! reduced its need from ~8 MiB to ~4 MiB, above the 2 MiB Rust default).

mod common;

use std::sync::Arc;

use wasm_repl::compile::SourceTable;
use wasm_repl::{ReplSession, SessionOpts, Status};

type Table = Arc<common::MemTable>;

fn refs(s: &[Table]) -> Vec<&dyn SourceTable> {
    common::search_refs(s)
}

fn session() -> ReplSession {
    ReplSession::new(SessionOpts::default())
}

fn submit(sess: &mut ReplSession, src: &[u8]) -> Status {
    let s = common::sysroot_search();
    let r = refs(&s);
    sess.transactional_submit(src, &r).status
}

fn is_trap_10(st: &Status) -> bool {
    matches!(
        st,
        Status::Trapped {
            code: ir::TrapCode::StackOverflow,
            ..
        }
    )
}

fn is_contract_trap(st: &Status) -> bool {
    matches!(
        st,
        Status::Trapped {
            code: ir::TrapCode::ContractFail,
            ..
        }
    )
}

// ---------------------------------------------------------------------------
// DEC-4: the data stack persists across submits
// ---------------------------------------------------------------------------

#[test]
fn stack_persists_across_submits() {
    let mut sess = session();
    assert_eq!(submit(&mut sess, b"1 2 +"), Status::Ok);
    assert_eq!(sess.stack_cells().len(), 1, "1 2 + leaves one value");
    assert_eq!(submit(&mut sess, b"3 +"), Status::Ok);
    assert_eq!(sess.stack_cells().len(), 1);
    assert_eq!(sess.stack_cells()[0].value_hex, b"0x0000000000000006");
    assert_eq!(submit(&mut sess, b"4 +"), Status::Ok);
    assert_eq!(sess.stack_cells()[0].value_hex, b"0x000000000000000a");
}

// ---------------------------------------------------------------------------
// DEC-3 / §7.2: phrase-only submissions compile against the accumulated buffer
// ---------------------------------------------------------------------------

#[test]
fn phrase_only_uses_prior_definitions() {
    let mut sess = session();
    assert_eq!(
        submit(&mut sess, b": double ( i64 -- i64 ) dup + ;"),
        Status::Ok
    );
    assert_eq!(submit(&mut sess, b"21 double"), Status::Ok);
    assert_eq!(submit(&mut sess, b"42 double"), Status::Ok);
    let cells = sess.stack_cells();
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].value_hex, b"0x000000000000002a");
    assert_eq!(cells[1].value_hex, b"0x0000000000000054");
}

// ---------------------------------------------------------------------------
// DEC-5/DEC-6 + §7.3: a trapped phrase rolls back stack+heap but keeps output
// ---------------------------------------------------------------------------

#[test]
fn trap_rolls_back_state_but_keeps_output() {
    let mut sess = session();
    assert_eq!(
        submit(
            &mut sess,
            b"import platform/testio { testio.write-byte }; import platform/mem;"
        ),
        Status::Ok
    );
    assert_eq!(submit(&mut sess, b": recurse ( -- ) recurse ;"), Status::Ok);
    let dance = b": dance ( -- ) 64 as usize platform.mem.region-create dup 8 as usize platform.mem.region-alloc 42 as i64 !i64 65 testio.write-byte platform.mem.region-destroy 66 testio.write-byte recurse ;";
    assert_eq!(submit(&mut sess, dance), Status::Ok);
    // Phrase: store+output+destroy then recurse → trap 10.
    let st = submit(&mut sess, b"dance");
    assert!(is_trap_10(&st), "expected trap 10, got {st:?}");
    // Output retained (the 'A' and 'B' bytes); stack rolled back to the
    // phrase-entry checkpoint (§Q3 DEC-6).
    assert_eq!(sess.out_total(), &[0x41, 0x42]);
    assert!(
        sess.stack_cells().is_empty(),
        "stack rolled back to the entry checkpoint"
    );
    // Heap is usable afterwards: a fresh region round-trip.
    assert_eq!(
        submit(
            &mut sess,
            b": heap-ok ( -- ) 32 as usize platform.mem.region-create dup 8 as usize platform.mem.region-alloc dup 42 as i64 !i64 dup @i64 42 == [ 68 testio.write-byte ] [ 70 testio.write-byte ] if drop platform.mem.region-destroy ;"
        ),
        Status::Ok
    );
    assert_eq!(submit(&mut sess, b"heap-ok"), Status::Ok);
    assert_eq!(sess.out_total(), &[0x41, 0x42, 0x44]);
}

// ---------------------------------------------------------------------------
// DEC-3 / transactions: definitions survive a phrase fault
// ---------------------------------------------------------------------------

#[test]
fn definitions_commit_even_when_the_phrase_faults() {
    let mut sess = session();
    assert_eq!(submit(&mut sess, b": f ( i64 -- i64 ) dup + ;"), Status::Ok);
    assert_eq!(submit(&mut sess, b"1 f"), Status::Ok);
    let st = submit(
        &mut sess,
        b": trap-now ( -- ) recurse ; : recurse ( -- ) recurse ; trap-now",
    );
    assert!(is_trap_10(&st), "expected trap 10, got {st:?}");
    // The definitions (trap-now, recurse) committed even though the phrase
    // trapped (§7.3 transactions #1/#2).
    assert!(String::from_utf8_lossy(sess.source()).contains("trap-now"));
    assert_eq!(
        submit(&mut sess, b"1 f"),
        Status::Ok,
        "the dictionary survived"
    );
    assert_eq!(sess.stack_cells().len(), 2);
}

// ---------------------------------------------------------------------------
// L1 guard elision (FR-5): elide/non-elide differ ONLY at the guard outcome
// ---------------------------------------------------------------------------

#[test]
fn elide_guards_flips_l1_only() {
    let mut sess = session();
    assert_eq!(
        submit(&mut sess, b": no-sevens ( i64 -- i64 bool ) dup 7 == not ;"),
        Status::Ok
    );
    assert_eq!(
        submit(&mut sess, b": eat ( i64 -- i64 ) needs [ no-sevens ] 1 + ;"),
        Status::Ok
    );
    assert_eq!(
        submit(&mut sess, b": check-eat ( -- ) 7 eat drop ;"),
        Status::Ok
    );

    // Guards on (default): the contract guard traps with code 20.
    let st_guards = submit(&mut sess, b"7 eat drop");
    assert!(
        is_contract_trap(&st_guards),
        "guards-on run must trap the contract: {st_guards:?}"
    );
    assert_eq!(
        sess.out_total().len(),
        0,
        "no output when the contract traps"
    );

    // Guards off: the TrapIfFalse is skipped; the body runs to D0.
    sess.set_elide_guards(true);
    let st_elided = submit(&mut sess, b"7 eat drop");
    sess.set_elide_guards(false);
    assert_eq!(
        st_elided,
        Status::Ok,
        "elide-on run completes: {st_elided:?}"
    );
    // Both runs leave the stack unchanged (rollback / empty net).
    assert!(sess.stack_cells().is_empty());
}

// ---------------------------------------------------------------------------
// §7.3: fuel exhaustion rolls back deterministically
// ---------------------------------------------------------------------------

#[test]
fn fuel_exhaustion_rolls_back() {
    let mut sess = session();
    sess.set_fuel(50);
    assert_eq!(
        submit(&mut sess, b": reloop ( i64 -- i64 ) 1 + reloop ;"),
        Status::Ok
    );
    assert!(matches!(
        submit(&mut sess, b"0 reloop"),
        Status::FuelExhausted { diag } if diag.code == wasm_repl::diag::code::E9150_FUEL
    ));
    assert!(
        sess.stack_cells().is_empty(),
        "fuel rollback restores the stack"
    );
}

// ---------------------------------------------------------------------------
// §7.5: source-cap diagnostic (E9154)
// ---------------------------------------------------------------------------

#[test]
fn oversized_definition_is_e9154() {
    let mut sess = session();
    // >64 KiB of declarations — rejected at the session boundary before the
    // compiler runs.
    let mut big = Vec::new();
    big.extend_from_slice(b": huge ( -- ) ");
    for _ in 0..12000 {
        big.extend_from_slice(b"0 ");
    }
    for _ in 0..12000 {
        big.extend_from_slice(b"drop ");
    }
    big.extend_from_slice(b";");
    match submit(&mut sess, &big) {
        Status::CompileError { diag } => assert_eq!(diag.code, 9154),
        other => panic!("expected E9154, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §7.3 journal-reversal unit test: LIFO compensation restores the heap for a
// store/destroy/store interleaving (the ordering itself is pinned here).
// ---------------------------------------------------------------------------

#[test]
fn journal_reversal_is_lifo() {
    use wasm_repl::engine::ExecState;
    use wasm_repl::{Effect, Fuel, Journal};

    // A heap with one live region (slot 0) plus a second region created
    // during the "phrase".
    let mut heap = wasm_repl::heap::RegionHeap::new();
    let (h0, _) = heap.region_create(64).unwrap();
    assert_eq!(h0, 0);
    let (p0, _) = heap.region_alloc(0, 16).unwrap();

    let mut state =
    ExecState::in_session(Vec::new(), heap, Fuel::new(1000), false, Journal::new(), Vec::new());
    let mut journal = Journal::new();
    // Simulated phrase effects, in execution order: store, create,
    // store (overwrites region 0's bytes), destroy.
    let _ = state.heap.store(p0, 8, 42).unwrap();
    journal.push(Effect::Store {
        addr: p0,
        width: 8,
        old: 0,
    });
    let (h1, old_cursor1) = state.heap.region_create(32).unwrap();
    let _ = h1;
    journal.push(Effect::RegionCreate {
        handle: 1,
        old_cursor: old_cursor1,
    });
    let (p1, old_off1) = state.heap.region_alloc(1, 8).unwrap();
    let _ = (p1, old_off1);
    journal.push(Effect::RegionAlloc {
        handle: 1,
        old_off: 0,
    });
    let _old_b = state.heap.store(p0, 8, 99).unwrap();
    journal.push(Effect::Store {
        addr: p0,
        width: 8,
        old: 42,
    });
    let saved = state.heap.region_destroy(0).unwrap();
    journal.push(Effect::RegionDestroy { handle: 0, saved });

    // Reverse-walk the journal (the session's rollback — LIFO).
    for e in journal.iter().rev() {
        match e {
            Effect::Store { addr, width, old } => state.heap.compensate_store(*addr, *width, *old),
            Effect::RegionCreate { handle, old_cursor } => {
                state.heap.compensate_region_create(*handle, *old_cursor)
            }
            Effect::RegionAlloc { handle, old_off } => {
                state.heap.compensate_cursor(*handle, *old_off)
            }
            Effect::Intern { addr, old_cursor } => state.heap.compensate_intern(*addr, *old_cursor),
            Effect::RegionReset { handle, old_off } => {
                state.heap.compensate_cursor(*handle, *old_off)
            }
            Effect::RegionDestroy { handle, saved } => {
                state.heap.compensate_region_destroy(*handle, *saved)
            }
            Effect::ScopedEnter { .. } => {}
            Effect::Output { .. } | Effect::Call { .. } | Effect::Periph { .. } => {}
        }
    }

    // The destroy was undone, then the create was undone: region 0 is
    // live with its pre-phrase contents (0 — fully rolled back), region 1
    // is reclaimed.
    assert_eq!(
        state.heap.load(p0, 8).unwrap(),
        0,
        "store compensations restored the pre-phrase bytes (LIFO: old-42 then old-0)"
    );
    assert!(
        state.heap.region_create(16).is_ok(),
        "region 1's slot was reclaimed"
    );
}

// ---------------------------------------------------------------------------
// P4: session-level MMIO — descriptor attached → phrase runs against the
// session device; device faults roll back like traps
// ---------------------------------------------------------------------------

/// The MMIO session module: same rows as the descriptor's `scratch` device
/// (E3647 divergence otherwise). `poke-a` writes the argument to A;
/// `read-a` reads A; `fault-after-poke` writes A=7 then recurses (trap 10).
const MMIO_SESSION_MOD: &[u8] = b"module M;
import platform/testio { };
register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;
const regs = Scratch @ board.scratch;
: poke-a ( i64 -- )
  &!regs.A swap as u32 !u32 ;
: read-a ( -- i64 )
  &regs.A @u32 as i64 ;
: fault-after-poke ( -- )
  &!regs.A 7 as u32 !u32 fault-after-poke ;
export { fault-after-poke };
end;
";

#[test]
fn descriptorless_mmio_session_is_e3640() {
    let mut sess = session();
    let st = submit(&mut sess, MMIO_SESSION_MOD);
    match st {
        Status::CompileError { diag } => assert_eq!(diag.code, 3640, "E3640 parity"),
        other => panic!("expected E3640, got {other:?}"),
    }
}

#[test]
fn mmio_session_phrase_runs_against_the_session_device() {
    let mut sess = ReplSession::new(SessionOpts::default());
    let bytes =
        std::fs::read(common::workspace_root().join("platforms/x86_64-unknown-none/platform.desc"))
            .expect("platform.desc committed");
    let cd = codegen_core::compiled_desc::decode_compiled_desc(&bytes).expect("decode");
    codegen_core::compiled_desc::validate_compiled_desc(&cd).expect("validate");
    sess.set_descriptor(Some(Box::new(cd)));
    // A scripted device pins the exact session-level access sequence.
    sess.set_peripheral(Box::new(wasm_repl::ScriptedPeripheral::new(
        &[0],
        vec![
            wasm_repl::ScriptStep::Write {
                aperture: 0,
                off: 0x0,
                width: 4,
                val: 42,
            },
            wasm_repl::ScriptStep::Read {
                aperture: 0,
                off: 0x0,
                width: 4,
                reply: 42,
            },
        ],
    )));

    let s = common::sysroot_search();
    let r = refs(&s);
    // Definitions commit (descriptor-backed compile).
    let st = sess.transactional_submit(MMIO_SESSION_MOD, &r).status;
    assert_eq!(st, Status::Ok, "descriptor-backed MMIO compiles");

    // Phrase: poke A=42 then read it back — 42 rests on the session stack.
    let st = sess
        .transactional_submit(
            b"42 poke-a read-a",
            &s.iter()
                .map(|t| t.as_ref() as &dyn SourceTable)
                .collect::<Vec<_>>(),
        )
        .status;
    assert_eq!(st, Status::Ok);
    let cells = sess.stack_cells();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].value_hex, b"0x000000000000002a");
}

#[test]
fn mmio_session_fault_rolls_back_the_register() {
    let mut sess = ReplSession::new(SessionOpts::default());
    let bytes =
        std::fs::read(common::workspace_root().join("platforms/x86_64-unknown-none/platform.desc"))
            .expect("platform.desc committed");
    let cd = codegen_core::compiled_desc::decode_compiled_desc(&bytes).expect("decode");
    codegen_core::compiled_desc::validate_compiled_desc(&cd).expect("validate");
    sess.set_descriptor(Some(Box::new(cd)));
    sess.set_peripheral(Box::new(wasm_repl::RamDevice::new(&[0])));

    let s = common::sysroot_search();
    let st = sess
        .transactional_submit(
            MMIO_SESSION_MOD,
            &s.iter()
                .map(|t| t.as_ref() as &dyn SourceTable)
                .collect::<Vec<_>>(),
        )
        .status;
    assert_eq!(st, Status::Ok);

    // A phrase that pokes A=7 then recurses to a stack-overflow trap: the
    // stack/heap roll back (transaction #2, §7.3), but the MMIO store is a
    // PERIPHERAL effect — never undone (DEC-6: "the LED cannot un-blink",
    // §Q3 "what is out"). The register must read back the poked value (7).
    let st = sess
        .transactional_submit(
            b"fault-after-poke",
            &s.iter()
                .map(|t| t.as_ref() as &dyn SourceTable)
                .collect::<Vec<_>>(),
        )
        .status;
    assert!(is_trap_10(&st), "expected the recursion trap, got {st:?}");
    let st = sess
        .transactional_submit(
            b"read-a",
            &s.iter()
                .map(|t| t.as_ref() as &dyn SourceTable)
                .collect::<Vec<_>>(),
        )
        .status;
    assert_eq!(st, Status::Ok);
    let cells = sess.stack_cells();
    assert_eq!(cells.len(), 1, "the register reads back exactly one value");
    assert_eq!(
        cells[0].value_hex, b"0x0000000000000007",
        "the faulted store (A=7) is a peripheral effect — it persists across the rollback (DEC-6)"
    );
}

// ---------------------------------------------------------------------------
// W1 L2 enforcement (DEC-23), engine-level: the failure classes the
// typechecker cannot produce are driven through hand-built IR via
// `Compiled::for_engine` — these pin the contract traps as defense-in-depth
// and the L2b divergence as an E9158 engine bug (never a student trap).
// ---------------------------------------------------------------------------

use wasm_repl::engine::{DetailKind, Engine, ExecState, Outcome};
use wasm_repl::compile::Compiled;

/// A hand-built IR word with the standard 7-prim type table prefix (the
/// irgen seeding order: empty, i64, bool, str, ptr, ptr_mut, mmio).
fn hand_word(
    name: &[u8],
    inputs: &[ir::TypeId],
    outputs: &[ir::TypeId],
    blocks: Vec<(ir::BlockId, Vec<ir::TypeId>, Vec<ir::OpKind>)>,
) -> ir::Word {
    use frontend::fixed::FixedVec;
    let mut types: FixedVec<ir::Atom, 64> = FixedVec::new();
    for a in [
        ir::AT_EMPTY,
        ir::AT_I64,
        ir::AT_BOOL,
        ir::AT_STR,
        ir::AT_PTR,
        ir::AT_PTR_MUT,
        ir::AT_MMIO,
    ] {
        types.push(a).expect("prim table");
    }
    let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
    let mut subtype_bases: FixedVec<ir::TypeId, 64> = FixedVec::new();
    for _ in 0..7 {
        type_sizes.push(0).expect("sizes");
        subtype_bases.push(ir::TY_EMPTY).expect("bases");
    }
    let mut sig = ir::Sig::empty();
    sig.in_len = inputs.len() as u8;
    sig.out_len = outputs.len() as u8;
    for (i, t) in inputs.iter().enumerate() {
        sig.inputs[i] = *t;
    }
    for (i, t) in outputs.iter().enumerate() {
        sig.outputs[i] = *t;
    }
    let mut bs: FixedVec<ir::Block, 16> = FixedVec::new();
    for (id, entry, ops) in blocks {
        let mut es: FixedVec<ir::TypeId, 32> = FixedVec::new();
        for t in &entry {
            es.push(*t).expect("entry stack");
        }
        let mut bos: FixedVec<ir::Op, 96> = FixedVec::new();
        for k in ops {
            bos.push(ir::Op {
                kind: k,
                span: ir::Span::new(0, 0),
            })
            .expect("ops");
        }
        bs.push(ir::Block {
            id,
            entry_stack: es,
            ops: bos,
        })
        .expect("blocks");
    }
    ir::Word {
        name: ir::Atom::new(name).unwrap(),
        sig,
        performs: ir::EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: ir::BlockId(0),
        types,
        type_sizes,
        type_classes: FixedVec::new(),
        apertures: FixedVec::new(),
        subtype_bases,
        blocks: bs,
    }
}

fn hand_module(words: Vec<ir::Word>) -> Compiled {
    Compiled::for_engine(
        ir::Module {
            name: ir::Atom::new(b"hand").unwrap(),
            words,
            apertures: frontend::fixed::FixedVec::new(),
        },
        wasm_repl::compile::X86_64_LINUX_SPEC,
        Vec::new(),
    )
}

/// L2a (DEC-23): a kind-mismatched operand is trap 20 with the op's span and
/// the sig-mismatch detail — never the coercion the old `as_i64` pop did.
/// The checker rejects this program (bools into `+`); only hand-built IR
/// reaches the arm, which is what makes the check defense-in-depth.
#[test]
fn l2a_kind_mismatch_is_trap_20_with_detail() {
    let compiled = hand_module(vec![hand_word(
        b"main",
        &[],
        &[],
        vec![(
            ir::BlockId(0),
            vec![],
            vec![ir::OpKind::ConstBool(true), ir::OpKind::ConstBool(true), ir::OpKind::AddI64],
        )],
    )]);
    let mut state = ExecState::new(&compiled);
    let outcome = Engine::new(&compiled).run(b"main", &mut state, &mut wasm_repl::periph::Disabled);
    let Outcome::Trap { code, detail, .. } = outcome else {
        panic!("expected a trap, got {outcome:?}");
    };
    assert_eq!(ir::trap_code_u32(code), 20);
    let d = detail.expect("L2a carries the Q6 detail");
    assert_eq!(d.kind, DetailKind::SigMismatch);
    assert_eq!(d.declared, b"i64");
    assert_eq!(d.actual.len(), 1, "the offending cell");
    assert_eq!(d.actual[0].ty_name, b"bool");
}

/// L2c (DEC-23): a per-slot input mismatch at `Call` is trap 20 with the
/// callee's word name, declared signature, and the actual argument cells.
/// A depth shortfall stays trap 10.
#[test]
fn l2c_call_slot_mismatch_is_trap_20_with_detail() {
    let callee = hand_word(
        b"add-two",
        &[ir::TY_I64, ir::TY_I64],
        &[ir::TY_I64, ir::TY_I64],
        vec![(ir::BlockId(0), vec![ir::TY_I64, ir::TY_I64], vec![ir::OpKind::AddI64])],
    );
    let sig = callee.sig;
    let main = hand_word(
        b"main",
        &[],
        &[],
        vec![(
            ir::BlockId(0),
            vec![],
            vec![
                ir::OpKind::ConstBool(true),
                ir::OpKind::ConstI64(5),
                ir::OpKind::Call {
                    name: ir::Atom::new(b"add-two").unwrap(),
                    sig,
                    performs: ir::EffectSet::empty(),
                    requires: ir::CapSet::empty(),
                    bound: ir::StackBound::ID,
                },
            ],
        )],
    );
    let compiled = hand_module(vec![main, callee]);
    let mut state = ExecState::new(&compiled);
    let outcome = Engine::new(&compiled).run(b"main", &mut state, &mut wasm_repl::periph::Disabled);
    let Outcome::Trap { code, detail, .. } = outcome else {
        panic!("expected a trap, got {outcome:?}");
    };
    assert_eq!(ir::trap_code_u32(code), 20);
    let d = detail.expect("L2c carries the Q6 detail");
    assert_eq!(d.word, b"add-two");
    assert_eq!(d.declared, b"( i64 i64 -- i64 i64 )");
    assert_eq!(d.actual.len(), 2);
    assert_eq!(d.actual[0].ty_name, b"bool", "slot 0 is the mismatch");
}

#[test]
fn l2c_depth_shortfall_stays_trap_10() {
    let callee = hand_word(
        b"add-two",
        &[ir::TY_I64, ir::TY_I64],
        &[ir::TY_I64],
        vec![(ir::BlockId(0), vec![ir::TY_I64, ir::TY_I64], vec![ir::OpKind::AddI64])],
    );
    let sig = callee.sig;
    let main = hand_word(
        b"main",
        &[],
        &[],
        vec![(
            ir::BlockId(0),
            vec![],
            vec![ir::OpKind::Call {
                name: ir::Atom::new(b"add-two").unwrap(),
                sig,
                performs: ir::EffectSet::empty(),
                requires: ir::CapSet::empty(),
                bound: ir::StackBound::ID,
            }],
        )],
    );
    let compiled = hand_module(vec![main, callee]);
    let mut state = ExecState::new(&compiled);
    let outcome = Engine::new(&compiled).run(b"main", &mut state, &mut wasm_repl::periph::Disabled);
    let Outcome::Trap { code, detail, .. } = outcome else {
        panic!("expected a trap, got {outcome:?}");
    };
    assert_eq!(code, ir::TrapCode::StackOverflow, "underflow is trap 10");
    assert!(detail.is_none());
}

/// L2b (DEC-23): a block-entry divergence from the checked `entry_stack` is
/// `Outcome::Internal` / E9158 — an engine bug, page-fail, never a
/// student-facing contract failure (the Q7 reachability rationale).
#[test]
fn l2b_entry_divergence_is_internal_e9158() {
    // A target block whose checked entry stack says `bool` — no verified
    // program can branch into it with an i64 (the checker enforces the
    // branch contract); the divergence is manufactured by hand.
    let mut main = hand_word(
        b"main",
        &[],
        &[],
        vec![
            (ir::BlockId(0), vec![], vec![ir::OpKind::Br { target: ir::BlockId(1) }]),
            (ir::BlockId(1), vec![ir::TY_BOOL], vec![ir::OpKind::Ret]),
        ],
    );
    let _ = &mut main;
    let compiled = hand_module(vec![main]);
    let mut state = ExecState::new(&compiled);
    let outcome = Engine::new(&compiled).run(b"main", &mut state, &mut wasm_repl::periph::Disabled);
    let Outcome::Internal { fault, .. } = outcome else {
        panic!("expected an internal fault, got {outcome:?}");
    };
    assert_eq!(fault.code, wasm_repl::diag::code::E9158_INTERNAL);
}

/// D-W5: the fabricated-value paths are gone — `Dup` on an empty stack and a
/// `LocalGet` of a missing local are real underflows (trap 10), session-level.
#[test]
fn fabricated_value_paths_are_traps_session_level() {
    let mut sess = session();
    // `dup` on an empty persistent stack: the checker admits the phrase
    // (dup's declared input is assumed ambient), the engine now faults the
    // underflow instead of fabricating a zero.
    let st = submit(&mut sess, b"dup");
    assert!(is_trap_10(&st), "expected trap 10, got {st:?}");
    // After a drop, `dup` on an empty stack faults the same way.
    let st2 = submit(&mut sess, b"1 drop dup");
    assert!(is_trap_10(&st2), "expected trap 10, got {st2:?}");
}

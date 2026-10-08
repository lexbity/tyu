//! MMIO unit + fixture tests (PLAN-WASM-1 P4).
//!
//! Engine-side MMIO semantics: the `WriteKind` RMW bit math (table test),
//! field mask/shift extraction, the aperture resolution, and the negative
//! paths — unbound aperture → E9152, descriptor-less MMIO → E3640,
//! task/interrupt → E9156 (each). The `mmio_*_x86` fixtures then run through
//! the interpreter against an emulated-aperture [`RamDevice`]; their
//! self-checks (`check` writes 'F' on any wrong value) pin the engine's
//! load/store/field/$WriteKind semantics, and the recorded device-access log
//! must equal the IR-projected access sequence (order + aperture + offset +
//! width) — an independent pin on address resolution, independent of the
//! engine's own address math.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use codegen_core::compiled_desc::{
    decode_compiled_desc, validate_compiled_desc, CompiledDescriptor,
};
use ir::{AddrOfBase, OpKind, WriteKind};
use wasm_repl::compile::Compiled;
use wasm_repl::{
    apply_write_kind, compile, extract_field, insert_field, Engine, ExecState, Outcome, RamDevice,
    ScriptStep, ScriptedPeripheral, X86_64_LINUX_SPEC,
};

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn fixtures_dir() -> PathBuf {
    common::fixtures_dir()
}

fn search() -> Vec<Arc<common::MemTable>> {
    common::sysroot_search()
}

fn refs(search: &[Arc<common::MemTable>]) -> Vec<&dyn wasm_repl::SourceTable> {
    common::search_refs(search)
}

/// The committed x86_64-unknown-none board descriptor (the MMIO fixture
/// pack's platform; OQ-3 — the harness reuses the descriptor paths the
/// existing execution tests drive).
fn x86_desc() -> CompiledDescriptor {
    let bytes = std::fs::read(workspace_root().join("platforms/x86_64-unknown-none/platform.desc"))
        .expect("x86_64-unknown-none platform.desc committed");
    let cd = decode_compiled_desc(&bytes).expect("decode platform.desc");
    validate_compiled_desc(&cd).expect("validate platform.desc");
    cd
}

fn compile_fixture(stem: &str, with_desc: bool) -> (Compiled, CompiledDescriptor) {
    let src = std::fs::read(fixtures_dir().join(format!("{stem}.mod"))).expect("fixture present");
    let s = search();
    let r = refs(&s);
    let desc = with_desc.then(x86_desc);
    let compiled = compile(&src, &r, X86_64_LINUX_SPEC, desc.as_ref()).expect("fixture compiles");
    (compiled, desc.expect("descriptor present"))
}

/// The count of an integer type's storage bytes at the hosted spec.
fn width_of(compiled: &Compiled, word_idx: usize, ty: ir::TypeId) -> u8 {
    let w = &compiled.module.words[word_idx];
    let (bits, _) = ir::storage_bits_signed(w, ty, 64).expect("storage width");
    core::cmp::max(1, bits / 8) as u8
}

/// The MMIO access sequence a word implies, in *execution* order: module
/// `Call`s are inlined (the fixture's words are acyclic), branch targets are
/// walked (a branch can only reach `check`, which has no MMIO ops — belt and
/// braces). `rw` is `true` for a write.
fn projected_word(compiled: &Compiled, word_idx: usize, out: &mut Vec<(bool, u16, u32, u8)>) {
    let w = &compiled.module.words[word_idx];
    let mut cur: Option<(u16, u32)> = None;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            match op.kind {
                OpKind::AddrOf {
                    base: AddrOfBase::Mmio { aperture, offset },
                    ..
                }
                | OpKind::MmioPlace {
                    aperture, offset, ..
                } => {
                    cur = Some((aperture, offset));
                }
                OpKind::Call { name, .. } => {
                    if let Some(i) = compiled.module.words.iter().position(|x| x.name == name) {
                        projected_word(compiled, i, out);
                    }
                }
                OpKind::MmioVolLoad { ty, .. } => {
                    if let Some((ap, off)) = cur {
                        let width = width_of(compiled, word_idx, ty);
                        out.push((false, ap, off, width));
                    }
                }
                OpKind::MmioVolStore { ty, write_kind, .. } => {
                    if let Some((ap, off)) = cur {
                        let width = width_of(compiled, word_idx, ty);
                        if write_kind != WriteKind::Plain {
                            out.push((false, ap, off, width)); // RMW read
                        }
                        out.push((true, ap, off, width));
                    }
                }
                OpKind::MmioVolLoadField { reg_ty, .. } => {
                    if let Some((ap, off)) = cur {
                        let width = width_of(compiled, word_idx, reg_ty);
                        out.push((false, ap, off, width));
                    }
                }
                OpKind::MmioVolStoreField { reg_ty, .. } => {
                    if let Some((ap, off)) = cur {
                        let width = width_of(compiled, word_idx, reg_ty);
                        out.push((false, ap, off, width)); // field RMW read
                        out.push((true, ap, off, width));
                    }
                }
                _ => {}
            }
        }
    }
}

fn projected_from_entry(compiled: &Compiled, entry: &[u8]) -> Vec<(bool, u16, u32, u8)> {
    let idx = compiled
        .module
        .words
        .iter()
        .position(|w| w.name.as_bytes() == entry)
        .expect("entry word present");
    let mut v = Vec::new();
    projected_word(compiled, idx, &mut v);
    v
}

// ---------------------------------------------------------------------------
// WriteKind RMW bit math — exhaustive table over 8/16/32-bit widths
// ---------------------------------------------------------------------------

#[test]
fn write_kind_bit_math_is_exhaustive_over_widths() {
    // Independent bitwise expectations: for each width, apply the register
    // semantics to every combination of a 2-bit pattern pair.
    for width in [1u8, 2, 4, 8] {
        let mask = match width {
            1 => 0xffu64,
            2 => 0xffff,
            4 => 0xffff_ffff,
            _ => u64::MAX,
        };
        let cur_patterns = [0x00, 0x55, 0xaa, 0xf0];
        let val_patterns = [0x00, 0x11, 0xff];
        for &cur in &cur_patterns {
            for &val in &val_patterns {
                let c = cur & mask;
                let v = val & mask;
                assert_eq!(
                    apply_write_kind(c, v, width, WriteKind::Plain),
                    v,
                    "Plain @{width}"
                );
                assert_eq!(
                    apply_write_kind(c, v, width, WriteKind::W1c),
                    c & !v & mask,
                    "W1c @{width}"
                );
                assert_eq!(
                    apply_write_kind(c, v, width, WriteKind::W1s),
                    c | v,
                    "W1s @{width}"
                );
                assert_eq!(
                    apply_write_kind(c, v, width, WriteKind::Xor),
                    c ^ v,
                    "Xor @{width}"
                );
            }
        }
    }
}

#[test]
fn field_extract_insert_round_trip() {
    // A 5-bit field at shift 3: inserting then extracting is the identity
    // on the field; the rest of the register is preserved.
    let mask = 0b11111 << 3;
    let field = 0b10101u64;
    let base = 0b1_010_010_100_001u64; // arbitrary unrelated bits
    let merged = insert_field(base, mask, 3, field);
    assert_eq!(extract_field(merged, mask, 3), field);
    assert_eq!(merged & !mask, base & !mask);
}

// ---------------------------------------------------------------------------
// mmio_smoke fixture — plain volatile load/store + emulated RAM device
// ---------------------------------------------------------------------------

#[test]
fn mmio_smoke_fixture_device_log_and_self_checks() {
    let (compiled, desc) = compile_fixture("mmio_smoke_x86", true);
    let _ = desc;
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    let mut dev = RamDevice::new(&[0]);

    let out = engine.run(b"mmio-smoke-x86-run", &mut state, &mut dev);

    let Outcome::Ok { out, .. } = out else {
        panic!("smoke run must complete: {out:?}");
    };
    assert!(
        out.is_empty(),
        "the fixture's checks must all pass (any 'F' byte is a failed check): {out:?}"
    );
    // The device-access sequence: A(0x0)=42 write-back, B(0x4)=100 write-back.
    let tokens: Vec<String> = dev.log().iter().map(|a| a.token()).collect();
    assert_eq!(tokens, ["W0:0:4:2a", "R0:0:4:2a", "W0:4:4:64", "R0:4:4:64"]);
}

// ---------------------------------------------------------------------------
// mmio_strategies fixture — W1s / W1c / Xor / field RMW + effectful reads
// ---------------------------------------------------------------------------

#[test]
fn mmio_strategies_fixture_matches_ir_projection_and_self_checks() {
    let (compiled, desc) = compile_fixture("mmio_strategies_x86", true);
    let _ = desc;
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    let mut dev = RamDevice::new(&[0]);

    let out = engine.run(b"mmio-strategies-x86-run", &mut state, &mut dev);

    let Outcome::Ok { out, .. } = out else {
        panic!("strategies run must complete: {out:?}");
    };
    assert!(
        out.is_empty(),
        "every strategy self-check must pass (no 'F' byte): {out:?}"
    );

    // The recorded access log must equal the IR-projected sequence. The
    // projection reads aperture/offset/width directly from the ops; the
    // engine derived them from the pushed addresses — independent pins.
    let projected = projected_from_entry(&compiled, b"mmio-strategies-x86-run");
    let actual: Vec<(bool, u16, u32, u8)> = dev
        .log()
        .iter()
        .map(|a| (a.is_write, a.aperture, a.off, a.width))
        .collect();
    assert_eq!(
        actual, projected,
        "device access order/addresses must match the IR"
    );

    // Smoke-check a couple of values the RMW produced (independently of the
    // fixture's own checks): SETBITS(0x8) goes 0 → 5 → 7 → 7 (w1s), and the
    // writes to STATUS(0x4) are seed 7, then w1c 4 → 3, then w1c 0 → 3.
    let log = dev.log();
    let writes_0x8: Vec<u64> = log
        .iter()
        .filter(|a| a.is_write && a.off == 0x8)
        .map(|a| a.val)
        .collect();
    assert_eq!(writes_0x8, [0x5, 0x7, 0x7], "w1s RMW on SETBITS");
    let writes_0x4: Vec<u64> = log
        .iter()
        .filter(|a| a.is_write && a.off == 0x4)
        .map(|a| a.val)
        .collect();
    assert_eq!(
        writes_0x4,
        [0x7, 0x3, 0x3],
        "seed store + w1c RMW on STATUS"
    );
}

// ---------------------------------------------------------------------------
// ScriptedPeripheral — the ordered script / access-log pin
// ---------------------------------------------------------------------------

/// A hand-written module over the descriptor's `scratch` device — the same
/// register-map declaration as `mmio_smoke_x86.mod` (the descriptor's device
/// rows must match the source map 1:1, E3647). Sequence: plain store B=9,
/// plain load A, plain store B=2, plain load B. The script pins the exact
/// order (the plan: "wrong-script = test failure").
const SMALL_MMIO: &[u8] = b"module Small;
import platform/testio { };
register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;
const regs = Scratch @ board.scratch;
: small ( -- )
  &!regs.B 9 as u32 !u32
  &regs.A @u32 drop
  &!regs.B 2 as u32 !u32
  &regs.B @u32 drop
  0 as i64 drop ;
export { small };
end;
";

fn compile_small() -> Compiled {
    let s = search();
    let r = refs(&s);
    let desc = x86_desc();
    compile(SMALL_MMIO, &r, X86_64_LINUX_SPEC, Some(&desc)).expect("small mmio compiles")
}

#[test]
fn scripted_device_pins_exact_access_sequence() {
    let compiled = compile_small();
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    // Script: store B=9 (plain), load A (reply 7), store B=2, load B.
    let script = vec![
        ScriptStep::Write {
            aperture: 0,
            off: 0x4,
            width: 4,
            val: 9,
        },
        ScriptStep::Read {
            aperture: 0,
            off: 0x0,
            width: 4,
            reply: 7,
        },
        ScriptStep::Write {
            aperture: 0,
            off: 0x4,
            width: 4,
            val: 2,
        },
        ScriptStep::Read {
            aperture: 0,
            off: 0x4,
            width: 4,
            reply: 2,
        },
    ];
    let mut sp = ScriptedPeripheral::new(&[0], script);
    let out = engine.run(b"small", &mut state, &mut sp);
    let Outcome::Ok { .. } = out else {
        panic!("small mmio run must complete: {out:?}");
    };
    assert_eq!(sp.remaining(), 0, "the whole script must be consumed");
    assert_eq!(
        sp.log().len(),
        4,
        "four device accesses recorded for the trace"
    );
}

#[test]
fn script_mismatch_is_a_test_failure_not_a_diagnostic() {
    let compiled = compile_small();
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    // Wrong script: the first write is expected at the WRONG offset — the
    // peripheral must fail loudly (panic), not let the engine produce a
    // diagnostic (the plan: "wrong-script = test failure, not engine
    // diagnostic").
    let script = vec![ScriptStep::Write {
        aperture: 0,
        off: 0x8, // WRONG register
        width: 4,
        val: 9,
    }];
    let mut sp = ScriptedPeripheral::new(&[0], script);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = engine.run(b"small", &mut state, &mut sp);
    }));
    assert!(caught.is_err(), "script mismatch must panic (test failure)");
}

// ---------------------------------------------------------------------------
// Negative paths
// ---------------------------------------------------------------------------

#[test]
fn unbound_aperture_surfaces_e9152() {
    let (compiled, desc) = compile_fixture("mmio_smoke_x86", true);
    let _ = desc;
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    // No device registered (Disabled): the first MMIO access must surface
    // E9152 (a device failure, not a trap).
    let mut periph = wasm_repl::periph::Disabled;
    let out = engine.run(b"mmio-smoke-x86-run", &mut state, &mut periph);
    match out {
        Outcome::Device { code, aperture, .. } => {
            assert_eq!(code, 9152, "E9152");
            assert_eq!(
                aperture,
                Some(0),
                "the aperture id travels with the diagnostic"
            );
        }
        other => panic!("expected E9152 device failure, got {other:?}"),
    }
}

#[test]
fn aperture_unbound_in_scripted_device_is_e9152() {
    let (compiled, desc) = compile_fixture("mmio_smoke_x86", true);
    let _ = desc;
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    // A scripted device that binds NO apertures: same E9152 surface.
    let mut sp = ScriptedPeripheral::new(&[], Vec::new());
    let out = engine.run(b"mmio-smoke-x86-run", &mut state, &mut sp);
    match out {
        Outcome::Device { code, .. } => assert_eq!(code, 9152, "E9152"),
        other => panic!("expected E9152 device failure, got {other:?}"),
    }
}

#[test]
fn descriptorless_mmio_is_e3640() {
    // Without a platform descriptor the module compiles NOWHERE — the exact
    // parity with `langc` without `--platform` (same code, verbatim).
    let src = std::fs::read(fixtures_dir().join("mmio_smoke_x86.mod")).expect("fixture present");
    let s = search();
    let r = refs(&s);
    let err = compile(&src, &r, X86_64_LINUX_SPEC, None)
        .err()
        .expect("E3640 without descriptor");
    assert_eq!(err.code, 3640, "E3640 parity");
}

#[test]
fn task_spawn_is_e9156() {
    // FR-3: task/interrupt ops are rejected at submit (E9156), never
    // mis-executed. `platform.task.spawn` (hosted sysroot) lowers to
    // `TaskSpawn` — the one reachable task construct at the hosted spec.
    // InterruptEnable/Disable ops live on ISR/bare-metal targets only and
    // cannot be produced by any hosted-sysroot word; they sit in the same
    // reject loop (compile.rs, exhaustive over `OpKind`).
    let src = b"module T;
import platform/linux { };
export { t };
: t ( -- )
  [ ( -- ) ] platform.task.spawn drop ;
end;";
    let s = search();
    let r = refs(&s);
    let err = compile(src, &r, X86_64_LINUX_SPEC, None)
        .err()
        .expect("task-spawn lowers to TaskSpawn and is E9156");
    assert_eq!(err.code, 9156, "task must be E9156, got {}", err.code);
}

// ---------------------------------------------------------------------------
// The periph events appear in the session trace (§7.4)
// ---------------------------------------------------------------------------

#[test]
fn mmio_trace_contains_periph_events() {
    let (compiled, desc) = compile_fixture("mmio_smoke_x86", true);
    let _ = desc;
    let engine = Engine::new(&compiled);
    let mut state = ExecState::new(&compiled);
    let mut dev = RamDevice::new(&[0]);
    let _ = engine.run(b"mmio-smoke-x86-run", &mut state, &mut dev);

    // Journal → trace projection: every device access is a periph event.
    let trace = wasm_repl::Trace::from_phrase(
        wasm_repl::trace::TraceHeader {
            v: wasm_repl::trace::TRACE_VERSION,
            spec: "x86_64-unknown-linux-gnu".into(),
            source_hash: "".into(),
            fuel: 0,
        },
        &state.journal,
        &Outcome::Ok { out: Vec::new() },
    );
    let norm = trace.normalize();
    assert!(
        norm.contains("PW0:0:4:2a"),
        "periph write event in trace: {norm}"
    );
    assert!(
        norm.contains("PR0:0:4:2a"),
        "periph read event in trace: {norm}"
    );
}

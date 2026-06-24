//! Authoritative builtin word table.
//!
//! Single source of truth consumed by both the langc driver (to populate
//! the typecheck environment) and the test harness (for checker unit tests).
//! Any divergence between the two is a bug — fix it here.

use core::sync::atomic::{AtomicBool, Ordering};

use ir::{CapSet, EffectSet, High, StackBound};

use crate::types::{TypeAtom, WordEntry, WordSig};

fn ta(bytes: &[u8]) -> TypeAtom {
    TypeAtom::new(bytes).unwrap()
}

fn entry(
    name: &[u8],
    inp: &[&[u8]],
    out: &[&[u8]],
    performs: EffectSet,
    bound: StackBound,
) -> WordEntry {
    let mut sig = WordSig::empty();
    sig.in_len = inp.len() as u8;
    sig.out_len = out.len() as u8;
    for (i, &b) in inp.iter().enumerate() {
        sig.inputs[i] = ta(b);
    }
    for (i, &b) in out.iter().enumerate() {
        sig.outputs[i] = ta(b);
    }
    WordEntry {
        name: ta(name),
        sig,
        performs,
        requires: CapSet::empty(),
        bound,
    }
}

const I64: &[u8] = b"i64";
const BOOL: &[u8] = b"bool";
const QUOT: &[u8] = b"quot";

const ZERO_B: StackBound = StackBound::ID;
const POP_B: StackBound = StackBound {
    net: -1,
    high: High::Slots(0),
};
const DUP_B: StackBound = StackBound {
    net: 1,
    high: High::Slots(1),
};
const CALL_B: StackBound = StackBound {
    net: 0,
    high: High::Top,
};

fn mk(
    name: &[u8],
    inp: &[&[u8]],
    out: &[&[u8]],
    performs: EffectSet,
    bound: StackBound,
) -> WordEntry {
    entry(name, inp, out, performs, bound)
}

fn make_table() -> alloc::vec::Vec<WordEntry> {
    alloc::vec![
        mk(b"dup", &[I64], &[I64, I64], EffectSet::empty(), DUP_B),
        mk(b"drop", &[I64], &[], EffectSet::empty(), POP_B),
        mk(
            b"swap",
            &[I64, I64],
            &[I64, I64],
            EffectSet::empty(),
            ZERO_B
        ),
        mk(b"+", &[I64, I64], &[I64], EffectSet::empty(), POP_B),
        mk(b"-", &[I64, I64], &[I64], EffectSet::empty(), POP_B),
        mk(b"*", &[I64, I64], &[I64], EffectSet::empty(), POP_B),
        mk(b">", &[I64, I64], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"<", &[I64, I64], &[BOOL], EffectSet::empty(), POP_B),
        mk(b">=", &[I64, I64], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"<=", &[I64, I64], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"==", &[I64, I64], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"and", &[BOOL, BOOL], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"or", &[BOOL, BOOL], &[BOOL], EffectSet::empty(), POP_B),
        mk(b"not", &[BOOL], &[BOOL], EffectSet::empty(), ZERO_B),
        mk(b"call", &[QUOT], &[], EffectSet::empty(), CALL_B),
        mk(
            b"platform.task.yield",
            &[],
            &[],
            EffectSet::from_bits(EffectSet::SUSPEND),
            ZERO_B
        ),
    ]
}

/// The authoritative list of builtin words known to the typechecker.
///
/// Uses `i64` as the native integer type (all current targets).
/// If a future target uses `i32`, the driver should append or override
/// entries after calling this function.
pub fn builtin_words() -> &'static [WordEntry] {
    static INIT: AtomicBool = AtomicBool::new(false);
    static mut TABLE: [WordEntry; 16] = [WordEntry {
        name: TypeAtom::EMPTY,
        sig: WordSig::empty(),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }; 16];
    if !INIT.load(Ordering::Acquire) {
        let v = make_table();
        // SAFETY: single-threaded init guarded by AtomicBool.
        unsafe {
            for (i, w) in v.into_iter().enumerate() {
                if i < TABLE.len() {
                    TABLE[i] = w;
                }
            }
            INIT.store(true, Ordering::Release);
        }
    }
    // SAFETY: TABLE is initialized and never modified again.
    unsafe { &TABLE[..] }
}

//! Deterministic random straight-line program generator (PLAN-VERIFY-3 P2.3).
//!
//! Promoted from the soundness-differential harness (`verifier/tests/
//! soundness_differential.rs`) into a library-owned API so the Rust harness
//! and — later — every port's differential runner (PLAN-VERIFY-3 §Q11 item 5,
//! `rederive`) share **one** generator. There is exactly one `gen_program` in
//! the tree; port conformance corpora are generated from this module, never
//! re-implemented.
//!
//! The generator is **deterministic** under a fixed `(seed, spec, n_inputs,
//! cast_final)` (a fixed LCG — no external PRNG dependency), parametrized by
//! [`crate::target::TargetSpec`] so per-target corpora share the same op
//! vocabulary: the IR data ops are full-width i64 on every target (the
//! runtime emulates 64-bit arithmetic), so the generator's op mix is
//! target-independent by design — the *statements* it feeds are
//! width-relative, the programs are not.
//!
//! `no_std`-compatible (`alloc` only), `#![forbid(unsafe_code)]` inherited.

use alloc::vec;
use alloc::vec::Vec;
use ir::{Atom, CmpKind, OpKind, TypeId};

use crate::target::TargetSpec;

/// The register Atom the generator's MMIO ops read/write (the model's script
/// keys on it). `'dev'` fits the 32-byte Atom cap.
fn mmio_place() -> Atom {
    Atom::new(b"dev").expect("'dev' fits an Atom")
}

/// A memory-model descriptor: which memory surface a generated program may
/// touch (PLAN-VERIFY-3 P2.3 — the generator "emits programs + scripted
/// device values"). [`MemDesc::flat`] reproduces the pre-P2.3 op mix
/// byte-for-byte (no memory ops, no extra RNG draws), so the existing
/// soundness corpora are unchanged.
#[derive(Clone, Default, Debug, Eq, PartialEq)]
pub struct MemDesc {
    /// Modeled RAM region (inclusive). When present, the generator may emit
    /// point `Store`/`Load` pairs inside it (the abstract engine's
    /// point-store tracking is exercised by the differential).
    pub ram: Option<(u64, u64)>,
    /// Scripted MMIO reads: register `Atom` → fixed value. The generator
    /// opens `MmioVol*` ops on these registers; an unscripted read stays
    /// nondeterministic (§Q13).
    pub scripted: Vec<(Atom, i64)>,
}

impl MemDesc {
    /// No memory surface: `FlatMem` semantics (reads `⊤`, no tracking).
    pub fn flat() -> MemDesc {
        MemDesc::default()
    }

    /// One modeled RAM region and one scripted MMIO register.
    pub fn ram_and_dev_script(ram_lo: u64, ram_hi: u64, dev_value: i64) -> MemDesc {
        MemDesc {
            ram: Some((ram_lo, ram_hi)),
            scripted: vec![(mmio_place(), dev_value)],
        }
    }
}

/// Deterministic LCG (the same one the harness always used — a fixed seed
/// reproduces a byte-identical stream on every machine).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    pub fn i64_between(&mut self, lo: i64, hi: i64) -> i64 {
        let span = hi.saturating_sub(lo).saturating_add(1) as u64;
        lo.wrapping_add((self.next_u64() % span.max(1)) as i64)
    }
}

/// One straight-line program with its input domains, its final obligation,
/// and the memory descriptor it may touch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    /// Per-input `(lo, hi)` concrete domains — the concrete enumeration is
    /// EXACTLY these intervals, so an exhaustive check is complete over the
    /// domain the discharge claims.
    pub domains: Vec<(i64, i64)>,
    pub ops: Vec<OpKind>,
    pub target: (i64, i64),
    /// When true, the obligation is on the PRE-cast value of the final
    /// `Cast` op (the cast's own site) — concrete runs trap on out-of-range.
    pub cast_site: bool,
    /// The target identity the program + its obligation are relativized to
    /// (§Q3) — carried so a differential runner can always reproduce the
    /// (spec, seed) pair.
    pub spec: TargetSpec,
    /// The memory surface the program references (its `Store`/`Load`/
    /// `MmioVol*` ops are legal only against this descriptor).
    pub mem: MemDesc,
}

impl Program {
    /// The number of inputs is the number of domains.
    pub fn n_inputs(&self) -> usize {
        self.domains.len()
    }
}

/// Stack-safe random straight-line program over `n_inputs` inputs.
/// `cast_final` produces a program whose last op is a narrowing cast (the
/// obligation sits on the cast pre). The obligation target is the subtype
/// range for cast-site programs; else a dense range so open/definite/failing
/// classes all appear. `mem` enables (and bounds) any memory op the program
/// may carry.
pub fn gen_program(
    rng: &mut Rng,
    spec: TargetSpec,
    mem: &MemDesc,
    n_inputs: usize,
    cast_final: bool,
) -> Program {
    let domains: Vec<(i64, i64)> = (0..n_inputs)
        .map(|_| {
            let lo = rng.i64_between(-3, 0);
            let hi = rng.i64_between(1, 4);
            (lo, hi)
        })
        .collect();
    let ops: Vec<OpKind> = gen_op_chain(rng, n_inputs, cast_final, mem);
    let target = if cast_final {
        (SUB_LO, SUB_HI)
    } else {
        let tlo = rng.i64_between(-6, 2);
        let thi = rng.i64_between(3, 10);
        (tlo, thi)
    };
    Program {
        domains,
        ops,
        target,
        cast_site: cast_final,
        spec,
        mem: mem.clone(),
    }
}

/// The harness's single subtype: `TypeId(2)` ranges `0..=100` (mirrors the
/// `Percent` books example).
pub const SUB_ID: u8 = 2;
pub const SUB_LO: i64 = 0;
pub const SUB_HI: i64 = 100;

/// Emit the op chain (tracking a symbolic stack depth so every program is
/// well-typed by construction). Memory ops are emitted only when `mem`
/// enables their surface; with [`MemDesc::flat`] the emission is byte-
/// identical to the pre-P2.3 generator (no extra RNG draws), so the existing
/// soundness corpora are unchanged.
pub fn gen_op_chain(
    rng: &mut Rng,
    n_inputs: usize,
    cast_final: bool,
    mem: &MemDesc,
) -> Vec<OpKind> {
    let mut depth = n_inputs.max(1);
    let mut ops: Vec<OpKind> = Vec::new();
    let budget = 1 + rng.below(8) as usize; // 1..=8 ops
    let dev = mmio_place();
    for _ in 0..budget {
        if cast_final && ops.len() + 1 == budget {
            break;
        }
        let choice = rng.below(11);
        match choice {
            0 => {
                ops.push(OpKind::ConstI64(rng.i64_between(-5, 5)));
                depth += 1;
            }
            1..=3 => {
                if depth < 2 {
                    continue;
                }
                ops.push(match choice {
                    1 => OpKind::AddI64,
                    2 => OpKind::SubI64,
                    _ => OpKind::MulI64,
                });
                depth -= 1;
            }
            4 => {
                if depth >= 1 {
                    ops.push(OpKind::Dup { ty: TypeId(0) });
                    depth += 1;
                }
            }
            5 => {
                if depth >= 2 {
                    ops.push(OpKind::Drop { ty: TypeId(0) });
                    depth -= 1;
                }
            }
            6 => {
                if depth >= 2 {
                    ops.push(OpKind::Swap {
                        a: TypeId(0),
                        b: TypeId(0),
                    });
                }
            }
            7 => {
                if depth >= 1 {
                    ops.push(OpKind::LocalSet {
                        slot: 0,
                        ty: TypeId(0),
                    });
                    depth -= 1;
                }
            }
            8 => {
                ops.push(OpKind::LocalGet {
                    slot: 0,
                    ty: TypeId(0),
                });
                depth += 1;
            }
            9 => {
                if depth >= 2 {
                    ops.push(OpKind::Cmp {
                        out: TypeId(1),
                        kind: CmpKind::Eq,
                    });
                    depth -= 1;
                }
            }
            _ => {
                ops.push(OpKind::Bitcast {
                    from: TypeId(0),
                    to: TypeId(0),
                });
            }
        }

        // P2.3 memory surface: only when the model enables it (deterministic
        // per `(seed, mem)`). A store→load pair at a point address inside RAM
        // is the ONE program shape that can make the abstract point-store
        // tracking discharge — the differential proves it sound against the
        // concrete RAM. MMIO reads answer the scripted value.
        if let Some((ram_lo, ram_hi)) = mem.ram {
            if depth >= 2 && rng.below(4) == 0 {
                let a = rng.i64_between(ram_lo as i64, ram_hi as i64);
                let v = rng.i64_between(-5, 5);
                ops.push(OpKind::ConstI64(a));
                ops.push(OpKind::ConstI64(v));
                ops.push(OpKind::Store { ty: TypeId(0) });
                ops.push(OpKind::ConstI64(a));
                ops.push(OpKind::Load { ty: TypeId(0) });
                depth += 1; // net of the 5-op pair
            }
        }
        if !mem.scripted.is_empty() {
            let read = rng.below(4) == 0;
            if read {
                if depth >= 1 {
                    ops.push(OpKind::ConstI64(0));
                    ops.push(OpKind::MmioVolLoad {
                        ty: TypeId(0),
                        place: dev,
                        read_kind: ir::ReadKind::Plain,
                        atomic_max: 64,
                        barrier: ir::BarrierKind::None,
                    });
                    depth += 1;
                }
            } else if depth >= 2 {
                ops.push(OpKind::ConstI64(0));
                ops.push(OpKind::ConstI64(rng.i64_between(-5, 5)));
                ops.push(OpKind::MmioVolStore {
                    ty: TypeId(0),
                    place: dev,
                    write_kind: ir::WriteKind::Plain,
                    read_kind: ir::ReadKind::Plain,
                    atomic_max: 64,
                    barrier: ir::BarrierKind::None,
                });
            }
        }
    }
    if cast_final {
        ops.push(OpKind::Cast {
            from: TypeId(0),
            to: TypeId(SUB_ID),
        });
    }
    ops
}

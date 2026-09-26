//! The memory-model boundary of the reference semantics (PLAN-VERIFY-3 §Q3,
//! §Q13, slice P2.1).
//!
//! "Semantics is a function of `(TargetSpec, MemModel-instance)`, never a
//! constant": the transfer table ([`crate::interp::State::step`]) consults a
//! [`MemModel`] for every load/store and every MMIO access. The *default*
//! in-tree model is [`FlatMem`] — exactly the pre-P2 behavior (reads yield
//! `⊤`, writes have no abstract effect) — so the pipeline's existing
//! semantics and goldens are unchanged by the parameterization.
//!
//! **MMIO nondeterminism lives at this trait boundary** (§Q13): an MMIO read
//! yields a nondeterministic value *within the register's width*. FlatMem
//! answers `⊤` (sound for any unknown width); [`ApertureMem`] answers the
//! width-bounded domain of the access (and can be scripted with a fixed
//! value, which is the seed of §Q13's device refinements, Phase P13).
//!
//! The trait is rendered (by the future port exporter) so a port can supply
//! its own model instance; the in-tree instances are the conformance
//! vectors' ground truth.

use alloc::vec::Vec; // ApertureMem's recorded store history (host/harness side).

use ir::Atom;

use crate::interval::Interval;

/// The abstract memory interface the transfer table steps against.
///
/// All operations are **abstract**: addresses and values travel as
/// [`Interval`]s, matching the abstract interpreter's value domain. A model
/// instance is free to be more precise than `⊤` (e.g. ApertureMem answers
/// recorded point stores) but never less sound (an over-approximated result
/// is always admissible; an under-approximation would be a false discharge).
pub trait MemModel {
    /// Abstract load from an abstract address. `width_bits` is the access
    /// width in bits. Sound default: `⊤` (memory not modeled).
    fn load(&self, addr: Interval, width_bits: u8) -> Interval;

    /// Abstract store of `val` to `addr` (abstract state update; no stack
    /// effect).
    fn store(&mut self, addr: Interval, val: Interval);

    /// MMIO aperture read — the injected nondeterminism oracle (§Q13). The
    /// value is nondeterministic within the register's `width_bits`; a model
    /// may answer a narrower domain or a fixed (scripted / refined) value.
    /// `place` identifies the register site (op payload) for refinement
    /// keying (Phase P13).
    fn aperture_read(&mut self, place: &Atom, width_bits: u8) -> Interval;

    /// MMIO aperture write. The abstract state carries only the aperture
    /// bound state; a write's only observable effect is via subsequent
    /// reads of that register (refinement models, P13).
    fn aperture_write(&mut self, place: &Atom, val: Interval);
}

/// The default model: memory is not modeled and MMIO reads are entirely
/// nondeterministic (`⊤` — sound regardless of the true register width).
/// This is byte-for-byte the pre-P2 transfer behavior, so the parameterized
/// semantics is a strict generalization of the existing engine.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FlatMem;

impl MemModel for FlatMem {
    fn load(&self, _addr: Interval, _width_bits: u8) -> Interval {
        Interval::TOP
    }

    fn store(&mut self, _addr: Interval, _val: Interval) {}

    fn aperture_read(&mut self, _place: &Atom, _width_bits: u8) -> Interval {
        // Sound for any register width: `⊤` covers the true nondeterministic
        // domain (Q4 — memory is not modeled).
        Interval::TOP
    }

    fn aperture_write(&mut self, _place: &Atom, _val: Interval) {}
}

/// An in-tree model with real abstract semantics, for tests, the per-target
/// vector corpus, and (later) the port conformance vectors:
///
/// - one RAM region `[ram.0, ram.1]` (inclusive) whose **point** stores are
///   recorded (address → abstract value); a load from a recorded point
///   address returns the joined recorded value, anything else `⊤`
///   (unbounded / non-point addresses could alias anything);
/// - an MMIO aperture whose reads answer the **width-bounded**
///   nondeterministic domain (§Q13) unless a value is *scripted* for that
///   register (the refinement seed).
///
/// Soundness: abstract store/load are a classic abstract store — a concrete
/// load from address `a` returns only what was concretely stored to `a`
/// (or garbage); the abstract join over every recorded store to `a`
/// over-approximates both, and a non-point address joins the whole recorded
/// set the address could touch (here conservatively `⊤`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApertureMem {
    /// Inclusive RAM address domain. `(0, 0)` = no modeled RAM.
    pub ram: (u64, u64),
    /// Recorded point stores: `(address, abstract value)`.
    pub cells: Vec<(i64, Interval)>,
    /// Scripted fixed reads per register key (byte slice → value). A
    /// scripted entry takes precedence over the nondeterministic domain.
    pub scripted: Vec<(Atom, Interval)>,
}

impl ApertureMem {
    /// A model with one RAM region and nothing scripted.
    pub fn new(ram: (u64, u64)) -> ApertureMem {
        ApertureMem {
            ram,
            cells: Vec::new(),
            scripted: Vec::new(),
        }
    }

    /// Attach a fixed read value for a register (refinement / oracle seed).
    /// Later entries win.
    pub fn script_read(&mut self, place: &Atom, val: Interval) {
        self.scripted.retain(|(k, _)| k != place);
        self.scripted.push((*place, val));
    }

    fn recorded_at(&self, addr: i64) -> Interval {
        let mut acc: Option<Interval> = None;
        for &(a, v) in self.cells.iter() {
            if a == addr {
                acc = Some(match acc {
                    None => v,
                    Some(x) => x.join(v),
                });
            }
        }
        acc.unwrap_or(Interval::TOP)
    }
}

impl MemModel for ApertureMem {
    fn load(&self, addr: Interval, _width_bits: u8) -> Interval {
        match addr {
            Interval::Range { lo, hi } if lo == hi => {
                let a = lo as u64;
                if a >= self.ram.0 && a <= self.ram.1 {
                    self.recorded_at(lo)
                } else {
                    // Outside RAM: unmapped/trap or never-written — ⊤.
                    Interval::TOP
                }
            }
            _ => Interval::TOP, // non-point address could alias anything
        }
    }

    fn store(&mut self, addr: Interval, val: Interval) {
        // Only point addresses within the modeled RAM region are tracked;
        // anything else (non-point address / unmapped) keeps reads at ⊤
        // (sound — the abstract load over-approximates).
        if let Interval::Range { lo, hi } = addr {
            if lo == hi && lo as u64 >= self.ram.0 && lo as u64 <= self.ram.1 {
                self.cells.retain(|(x, _)| *x != lo);
                self.cells.push((lo, val));
            }
        }
    }

    fn aperture_read(&mut self, place: &Atom, width_bits: u8) -> Interval {
        for (k, v) in self.scripted.iter().rev() {
            if k == place {
                return *v;
            }
        }
        // §Q13: nondeterministic value within the register's width. A width
        // >= 64 is the full i64 domain → `⊤` (a Range spanning it renders
        // identically but is not the lattice top).
        let (lo, hi) = crate::interval::word_domain(width_bits);
        if lo == i64::MIN && hi == i64::MAX {
            Interval::TOP
        } else {
            Interval::Range { lo, hi }
        }
    }

    fn aperture_write(&mut self, _place: &Atom, _val: Interval) {
        // Aperture writes update bound/device state only; v1 models no
        // observable abstract effect (refinements land in P13).
    }
}

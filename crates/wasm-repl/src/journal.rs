//! The undo journal (PLAN-WASM-1 §5/§Q3 DEC-5, §7.3): the transaction
//! rollback log AND the execution-trace skeleton.
//!
//! Every reversible mutation of a phrase records an [`Effect`] here; on a
//! trap or fuel exhaustion the session walks the journal **in reverse**
//! applying each effect's compensation (§7.3 — journal reversal is LIFO by
//! construction, and the reversal-order test pins that). Trace-only effects
//! (`Output`/`Call`) are never undone — the rolled-back phrase's output stays
//! before the transaction (§Q3 DEC-6).
//!
//! The journal is pooled per phrase: `Vec` capacity persists across submits,
//! contents are cleared (the container comes from the session, not a fresh
//! allocation per phrase).

use alloc::vec::Vec;

use crate::heap::RegionSaved;

/// One recorded effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// A typed store overwrote `width` bytes at `addr`; `old` is the prior
    /// contents (undo: rewrite `old`).
    Store { addr: u64, width: u8, old: u64 },
    /// `region-create` allocated slot `handle`, advancing the arena cursor
    /// from `old_cursor` (undo: free the slot + range, reclaim the bytes —
    /// §7.3 "RegionCreate (reclaim)"; the reclaimed span is zeroed so a
    /// re-reserve observes fresh storage exactly like a first reserve).
    RegionCreate { handle: u64, old_cursor: u32 },
    /// A lazy `intern` (resource-place storage) reserved arena bytes at
    /// `addr` (undo: drop the live range, reclaim + zero the span).
    Intern { addr: u64, old_cursor: u32 },
    /// `region-alloc` advanced the cursor of `handle` from `old_off` (undo:
    /// restore the cursor).
    RegionAlloc { handle: u64, old_off: u32 },
    /// `region-reset` moved the cursor of `handle` from `old_off` (undo:
    /// restore it).
    RegionReset { handle: u64, old_off: u32 },
    /// `region-destroy` freed `handle`, whose pre-destroy state is `saved`
    /// (undo: restore the slot + its live range — §7.3).
    RegionDestroy { handle: u64, saved: RegionSaved },
    /// A `ScopedEnter` slice descriptor was allocated at `addr`, advancing
    /// the arena cursor from `old_cursor` (undo: drop its live range and
    /// reclaim the span).
    ScopedEnter { addr: u64, old_cursor: u32 },
    /// Diagnostic-channel bytes written (trace-only; never undone).
    Output { start: u32, len: u32 },
    /// A word call (trace-only; the call pane). `depth_before` is the caller
    /// frame depth.
    Call { word: Vec<u8>, depth_before: u16 },
    /// A peripheral access (P4; trace-only, never undone — a device access
    /// cannot "un-blink", DEC-6). `barrier`/`ReadKind` are single-threaded
    /// no-ops and are not carried: the plan's §5 `Periph` payload is exactly
    /// `{aperture, off, width, val, is_write}`.
    Periph {
        aperture: u16,
        off: u32,
        width: u8,
        val: u64,
        is_write: bool,
    },
}

/// The pooled per-phrase journal.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    effects: Vec<Effect>,
}

impl Journal {
    pub fn new() -> Journal {
        Journal {
            effects: Vec::new(),
        }
    }

    pub fn push(&mut self, e: Effect) {
        self.effects.push(e);
    }

    pub fn clear(&mut self) {
        self.effects.clear();
    }

    pub fn iter(&self) -> core::slice::Iter<'_, Effect> {
        self.effects.iter()
    }
}

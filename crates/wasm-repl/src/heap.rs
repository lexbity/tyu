//! The region heap (PLAN-WASM-1 §6.7, NFR-4, §Q11): the concrete
//! interpreter's unbounded-arena memory model, layout-shared with the future
//! `codegen-wasm` backend.
//!
//! Linear-memory geometry (§6.7):
//! ```text
//!   [ 0 .. 4 KiB )      reserved null page (any access → bounds trap)
//!   [ 4 KiB .. )        bump arena: regions (and scoped descriptors)
//! ```
//!
//! Region semantics mirror the hosted x86_64 native runtime exactly
//! (`codegen-x86_64/src/region.rs`): at most 16 region slots; a region handle
//! is the slot index (0..15); `region-create` scans for a free slot (else
//! `next`, else `RegionExhausted`/26); allocations bump an 8-byte-aligned
//! cursor within the region; malformed handles/sizes trap `Unreachable`/23.
//! A region's base address is an arena offset, so pointers from
//! `region-alloc` (base + bump) are directly load/store-able.
//!
//! Access rules: every `load`/`store` bounds-checks against the arena and the
//! live ranges; out-of-bounds is a clean bounds diagnostic (there is no
//! `unsafe` in this crate, so OOB is impossible to express — a "missed check"
//! is a wrong-diagnostic bug, never UB; G53's fault-injection probes it).

use alloc::vec;
use alloc::vec::Vec;
use ir::TrapCode;

/// The reserved null page (§6.7): no region/arena allocation starts below it.
pub const NULL_PAGE_BYTES: u32 = 4096;
/// Region-slot count (native `__region_*` arrays are 16 entries).
pub const REGION_SLOTS: usize = 16;
/// Maximum region byte size (NFR-4).
pub const MAX_REGION_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum arena bytes (NFR-4 browser default; the host may go higher).
pub const MAX_HEAP_BYTES: u64 = 64 * 1024 * 1024;

/// Why an access was rejected (the §7.8 failure surface, P2 diagnostics).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapFault {
    /// A region handle/size argument violated the native runtime's contract
    /// (mirrors trap 23 / `Unreachable` — an invalid argument, not memory).
    BadArgument,
    /// The region store is exhausted or the arena cap is exceeded
    /// (trap 26 / `RegionExhausted`).
    Exhausted,
    /// The access fell outside every live range (the interpreter's clean
    /// analogue of the native SIGSEGV; §7.8).
    OutOfBounds,
}

impl HeapFault {
    pub const fn trap(self) -> TrapCode {
        match self {
            HeapFault::BadArgument => TrapCode::Unreachable,
            HeapFault::Exhausted => TrapCode::RegionExhausted,
            HeapFault::OutOfBounds => TrapCode::AssertFail,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Region {
    base: u32,
    size: u32,
    off: u32,
    alive: bool,
}

/// A live byte range the access checker consults (region bodies + scoped
/// descriptors).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LiveRange {
    start: u32,
    end: u32,
}

/// The pre-destroy state of a region slot (PLAN-WASM-1 §Q3/DEC-5): the undo
/// payload for a `platform.mem.region-destroy` journal effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionSaved {
    pub base: u32,
    pub size: u32,
    pub off: u32,
}

/// The concrete heap. `bytes` is the flat arena (layout-shared with the wasm
/// backend); `regions` is the 16-slot region table; `scoped` tracks
/// frame-scoped descriptor allocations (also arena-backed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionHeap {
    bytes: Vec<u8>,
    cursor: u32,
    next: usize,
    regions: Vec<Region>,
    live: Vec<LiveRange>,
}

impl Default for RegionHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionHeap {
    pub fn new() -> RegionHeap {
        RegionHeap {
            bytes: vec![0; NULL_PAGE_BYTES as usize],
            cursor: NULL_PAGE_BYTES,
            next: 0,
            regions: vec![
                Region {
                    base: 0,
                    size: 0,
                    off: 0,
                    alive: false
                };
                REGION_SLOTS
            ],
            live: Vec::new(),
        }
    }

    fn align8(n: u64) -> u64 {
        (n + 7) & !7
    }

    /// Reserve `n` arena bytes (returns the base offset), honoring caps.
    fn reserve(&mut self, n: u64) -> Result<u32, HeapFault> {
        let n = Self::align8(n);
        if n > MAX_REGION_BYTES {
            return Err(HeapFault::Exhausted);
        }
        let end = u64::from(self.cursor) + n;
        if end > MAX_HEAP_BYTES {
            return Err(HeapFault::Exhausted);
        }
        let base = self.cursor;
        self.bytes.resize(end as usize, 0);
        self.cursor = end as u32;
        Ok(base)
    }

    // -- the native region ABI (codegen-x86_64/src/region.rs semantics) ------

    /// `platform.mem.region-create ( usize -- Region )`.
    /// Returns `(handle, old_cursor)` so the caller can journal the arena
    /// reservation (DEC-5 rollback reclaims the span, §7.3).
    pub fn region_create(&mut self, size: u64) -> Result<(u64, u32), HeapFault> {
        if size == 0 {
            return Err(HeapFault::BadArgument);
        }
        let size = Self::align8(size);
        if size > MAX_REGION_BYTES {
            return Err(HeapFault::Exhausted);
        }
        let slot = if let Some(i) = self.regions.iter().position(|r| !r.alive) {
            i
        } else if self.next < REGION_SLOTS {
            let i = self.next;
            self.next += 1;
            i
        } else {
            return Err(HeapFault::Exhausted);
        };
        let old_cursor = self.cursor;
        let base = self.reserve(size)?;
        self.regions[slot] = Region {
            base,
            size: size as u32,
            off: 0,
            alive: true,
        };
        self.live.push(LiveRange {
            start: base,
            end: base + size as u32,
        });
        Ok((slot as u64, old_cursor))
    }

    /// `platform.mem.region-alloc ( Region usize -- ptr_mut )`.
    /// Returns `(base + old_off, old_off)` so the caller can journal the
    /// cursor move (DEC-5 rollback).
    pub fn region_alloc(&mut self, handle: u64, size: u64) -> Result<(u64, u64), HeapFault> {
        let slot = self.slot(handle)?;
        if size == 0 {
            return Err(HeapFault::BadArgument);
        }
        let aligned = Self::align8(size);
        if aligned > MAX_REGION_BYTES {
            return Err(HeapFault::Exhausted);
        }
        let region = &mut self.regions[slot];
        let new_off = u64::from(region.off) + aligned;
        if new_off > u64::from(region.size) {
            return Err(HeapFault::Exhausted);
        }
        let old_off = region.off;
        let ptr = u64::from(region.base) + u64::from(old_off);
        region.off = new_off as u32;
        Ok((ptr, u64::from(old_off)))
    }

    /// `platform.mem.region-reset ( Region -- )`.
    /// Returns the previous cursor so the caller can journal the move (DEC-5).
    pub fn region_reset(&mut self, handle: u64) -> Result<u64, HeapFault> {
        let slot = self.slot(handle)?;
        let old = self.regions[slot].off;
        self.regions[slot].off = 0;
        Ok(u64::from(old))
    }

    /// `platform.mem.region-destroy ( Region -- )`.
    /// Returns the slot's pre-destroy state — the journal's undo payload for
    /// a rolled-back destroy (the region is restored, §7.3).
    pub fn region_destroy(&mut self, handle: u64) -> Result<RegionSaved, HeapFault> {
        let slot = self.slot(handle)?;
        let saved = RegionSaved {
            base: self.regions[slot].base,
            size: self.regions[slot].size,
            off: self.regions[slot].off,
        };
        // Remove the live range entry; the arena bytes stay (a destroyed
        // region's stale pointer is not live; a *new* region reuses the slot).
        self.live
            .retain(|r| !(r.start == saved.base && r.end == saved.base + saved.size));
        self.regions[slot] = Region {
            base: 0,
            size: 0,
            off: 0,
            alive: false,
        };
        Ok(saved)
    }

    fn slot(&self, handle: u64) -> Result<usize, HeapFault> {
        if handle >= REGION_SLOTS as u64 {
            return Err(HeapFault::BadArgument);
        }
        let slot = handle as usize;
        if !self.regions[slot].alive {
            return Err(HeapFault::BadArgument);
        }
        Ok(slot)
    }

    /// The arena bump cursor (the caller journals it with a reservation so
    /// a rollback reclaims the span, DEC-5).
    pub fn arena_cursor(&self) -> u32 {
        self.cursor
    }

    /// The arena base address of a live region (for diagnostics/geometry
    /// probes; native calls it `__region_base[slot]`).
    pub fn region_base(&self, handle: u64) -> Option<u64> {
        self.slot(handle)
            .ok()
            .map(|s| u64::from(self.regions[s].base))
    }

    // -- scoped descriptors (ScopedEnter slice frames, §6.7) ------------------

    /// Allocate a 16-byte `{ ptr, len }` descriptor and return its address
    /// with the pre-reserve cursor (journaled by the caller, DEC-5).
    /// Descriptors are live ranges so a descriptor `load` reads the frame
    /// data and any later access is bounds-checked like any other.
    pub fn scoped_enter(&mut self, ptr: u64, len: u64) -> Result<(u64, u32), HeapFault> {
        let old_cursor = self.cursor;
        let base = self.reserve(16)?;
        let lo = ptr as u32;
        let hi = len as u32;
        let bytes = &mut self.bytes[base as usize..base as usize + 16];
        bytes[0..4].copy_from_slice(&lo.to_le_bytes());
        bytes[4..8].copy_from_slice(&(0u32).to_le_bytes());
        bytes[8..12].copy_from_slice(&hi.to_le_bytes());
        bytes[12..16].copy_from_slice(&(0u32).to_le_bytes());
        self.live.push(LiveRange {
            start: base,
            end: base + 16,
        });
        Ok((u64::from(base), old_cursor))
    }

    // -- memory access ---------------------------------------------------------

    /// Bounds-checked read of `width` bytes at `addr` (little-endian, native
    /// order). `width` ∈ {1,2,4,8}.
    pub fn load(&self, addr: u64, width: u8) -> Result<u64, HeapFault> {
        self.range(addr, width)?;
        let a = addr as usize;
        let mut out = 0u64;
        for (i, b) in self.bytes[a..a + width as usize].iter().enumerate() {
            out |= u64::from(*b) << (8 * i);
        }
        Ok(out)
    }

    /// Bounds-checked write of the low `width` bytes of `val` at `addr`.
    /// Returns the **previous** contents (the journal's undo payload, §7.3 —
    /// DEC-5 rolls a store back by writing the old bytes).
    pub fn store(&mut self, addr: u64, width: u8, val: u64) -> Result<u64, HeapFault> {
        self.range(addr, width)?;
        let a = addr as usize;
        let mut old = 0u64;
        for (i, b) in self.bytes[a..a + width as usize].iter().enumerate() {
            old |= u64::from(*b) << (8 * i);
        }
        for (i, b) in self.bytes[a..a + width as usize].iter_mut().enumerate() {
            *b = ((val >> (8 * i)) & 0xFF) as u8;
        }
        Ok(old)
    }

    /// The access must be entirely within one live range (a region body or a
    /// scoped descriptor) and never in the null page. Bit-exact liveness
    /// gives the clean bounds diagnostic where native would SIGSEGV (§7.8).
    fn range(&self, addr: u64, width: u8) -> Result<(), HeapFault> {
        if addr < u64::from(NULL_PAGE_BYTES) {
            return Err(HeapFault::OutOfBounds);
        }
        let start = addr as u32;
        let end = addr as u32 + u32::from(width);
        if self.live.iter().any(|r| r.start <= start && end <= r.end) {
            Ok(())
        } else {
            Err(HeapFault::OutOfBounds)
        }
    }

    /// Intern a `[u64 len][bytes...]` blob (the `testio.write-str`/`str` and
    /// `ConstStr` layout, native `runtime/linux-x86_64-hosted.asm`) into the
    /// arena; returns its address. Internal constants are deterministic:
    /// the same `Compiled` interns to the same addresses every run (§Q6).
    pub fn intern(&mut self, bytes: &[u8]) -> Result<u64, HeapFault> {
        let n = u64::try_from(bytes.len()).map_err(|_| HeapFault::Exhausted)?;
        let base = self.reserve(n + 8)?;
        let a = base as usize;
        self.bytes[a..a + 8].copy_from_slice(&n.to_le_bytes());
        self.bytes[a + 8..a + 8 + bytes.len()].copy_from_slice(bytes);
        self.live.push(LiveRange {
            start: base,
            end: base + 8 + bytes.len() as u32,
        });
        Ok(u64::from(base))
    }

    // -- journal compensation (DEC-5 §Q3): the undo actions -------------------
    //
    // Each method inverts ONE journal effect; the session walks the journal
    // in reverse (§7.3). Compensations never re-check bounds (the reverted
    // access was in-bounds at record time) and never journal themselves.

    /// Undo an `Effect::Store`: rewrite the old bytes.
    pub fn compensate_store(&mut self, addr: u64, width: u8, old: u64) {
        let a = addr as usize;
        for (i, b) in self.bytes[a..a + width as usize].iter_mut().enumerate() {
            *b = ((old >> (8 * i)) & 0xFF) as u8;
        }
    }

    /// Undo an `Effect::RegionAlloc`/`RegionReset`: restore the cursor.
    pub fn compensate_cursor(&mut self, handle: u64, old_off: u32) {
        if let Some(r) = self.regions.get_mut(handle as usize) {
            r.off = old_off;
        }
    }

    /// Undo an `Effect::RegionCreate`: mark the slot free, drop its live
    /// range, and RECLAIM the arena span (§7.3 — the phrase-entry heap state
    /// includes the allocator cursor). The reclaimed span is zeroed so a
    /// re-reserve observes fresh storage exactly like a first reserve.
    pub fn compensate_region_create(&mut self, handle: u64, old_cursor: u32) {
        if let Some(r) = self.regions.get_mut(handle as usize) {
            if r.alive {
                self.live
                    .retain(|lr| !(lr.start == r.base && lr.end == r.base + r.size));
            }
            *r = Region {
                base: 0,
                size: 0,
                off: 0,
                alive: false,
            };
        }
        self.reclaim(old_cursor);
    }

    /// Undo an `Effect::RegionDestroy`: restore the slot and its live range
    /// (a rolled-back destroy leaves the region exactly as it was).
    pub fn compensate_region_destroy(&mut self, handle: u64, saved: RegionSaved) {
        if let Some(r) = self.regions.get_mut(handle as usize) {
            *r = Region {
                base: saved.base,
                size: saved.size,
                off: saved.off,
                alive: true,
            };
            if saved.size > 0 {
                self.live.push(LiveRange {
                    start: saved.base,
                    end: saved.base + saved.size,
                });
            }
        }
    }

    /// Undo an `Effect::ScopedEnter`: drop the descriptor's live range and
    /// reclaim the 16-byte span (zeroed — see `compensate_region_create`).
    pub fn compensate_scoped(&mut self, addr: u64, old_cursor: u32) {
        let start = addr as u32;
        self.live
            .retain(|lr| !(lr.start == start && lr.end == start + 16));
        self.reclaim(old_cursor);
    }

    /// Undo an `Effect::Intern`: drop the interned live range and reclaim
    /// the span (zeroed).
    pub fn compensate_intern(&mut self, addr: u64, old_cursor: u32) {
        let start = addr as u32;
        self.live.retain(|lr| lr.start != start);
        self.reclaim(old_cursor);
    }

    /// Reclaim the arena tail `[old_cursor .. cursor)`: drop it entirely
    /// (a later `reserve` re-grows it with zero-fill, so reused storage is
    /// indistinguishable from a first reserve) and restore the cursor. LIFO
    /// journal reversal guarantees every live range in the span was already
    /// dropped, so the tail is unreachable.
    fn reclaim(&mut self, old_cursor: u32) {
        if (old_cursor as u64) >= u64::from(NULL_PAGE_BYTES) && old_cursor <= self.cursor {
            self.bytes.truncate(old_cursor as usize);
            self.cursor = old_cursor;
        }
    }
}

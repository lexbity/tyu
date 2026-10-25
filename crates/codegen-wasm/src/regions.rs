//! The wasm region heap (PLAN-WASM-2 B1, §6.6): the linear-memory layout the
//! wasm backend implements natively, and the region-ABI helpers
//! (`platform.mem.region-create/alloc/reset/destroy`) as native wasm
//! functions mirroring the hosted runtime exactly
//! (`codegen-x86_64/src/region.rs` — I-12: G54 pins wasm to the NATIVE
//! goldens, so the helpers reproduce that behavior check for check,
//! including trap codes and their order).
//!
//! Memory geometry (§6.6 — this module is the single home of the constants):
//!
//! ```text
//!   [ 0 .. 512 )           region table: 16 slots × {base, size, off, alive}
//!   [ 4096 .. 8192 )       resource intern area (256 places × 16 B, zero-init)
//!   [ 8192 .. 72 KiB )     ConstStr intern area ({u64 len}{bytes} — the
//!                          interpreter heap's intern layout)
//!   [ 128 KiB .. 144 KiB ) scoped-slice descriptor area (per-site 16 B
//!                          {ptr, len} — the native frame-descriptor model)
//!   [ 256 KiB .. )         the region-heap bump arena (mutable global $cursor)
//! ```
//!
//! The arena grows the memory on demand (`memory.grow`) up to the 256-page
//! maximum. Exhaustion behavior mirrors the hosted runtime: no free slot →
//! `RegionExhausted` (26); grow failure (the mmap-failure analogue) →
//! `Unreachable` (23); argument/dead-region violations → 23;
//! out-of-region-space → 26. Every trap goes through the `tyu.trap` import
//! carrying `(code, span_start, span_len)` — the span comes from the CALL
//! SITE's op span (DEC-40: a backend diagnostic without a span is a
//! defect), so the trapping helpers take the span as trailing parameters.

use super::encoder::*;
use crate::{IMPORT_TRAP, REGION_ALLOC, REGION_CREATE, REGION_DESTROY, REGION_RESET};
use alloc::vec;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// §6.6 memory layout — the single home of these constants.
// ---------------------------------------------------------------------------

/// The region table's linear-memory address (the interpreter keeps the table
/// host-side; this is the wasm mirror the helpers read and write).
pub const REGION_TABLE_ADDR: i64 = 0;
/// Region-slot count (the hosted runtime's `__region_*` arrays are 16
/// entries; a region handle IS the slot index).
pub const REGION_TABLE_SLOTS: i64 = 16;
/// Bytes per slot: {base, size, off, alive} — 4 × i64.
const SLOT_BYTES: i64 = 32;

/// Field offsets within a slot.
const OFF_BASE: i64 = 0;
const OFF_SIZE: i64 = 8;
const OFF_OFF: i64 = 16;
const OFF_ALIVE: i64 = 24;

/// The resource intern area (word `AddrOf{Runtime}` targets): 256 places ×
/// 16 B, zero-initialized by the linear-memory contract. A place's address
/// is stable for the module's lifetime and distinct from every other
/// place's — the observable the corpus pins (`&r1 != &r2`).
pub const RESOURCES_BASE: i64 = 4096;
pub const RESOURCES_CAP: usize = 256;
pub const RESOURCE_STRIDE: i64 = 16;

/// The ConstStr intern area: {u64 len}{bytes} per literal — the layout the
/// interpreter's `RegionHeap::intern` uses, so a slice descriptor built
/// over a string address has the same shape on both substrates.
pub const STRINGS_BASE: i64 = 8192;
pub const STRINGS_CAP: u64 = 64 * 1024;

/// The scoped-slice descriptor area: one 16-byte {ptr, len} descriptor per
/// `ScopedEnter{Slice}` site — the native model stores descriptors in the
/// word's stack frame; the wasm analogue is a fixed per-site slot (no
/// cursor save/restore: nothing was bumped).
pub const SCOPED_BASE: i64 = 128 * 1024;
pub const SCOPED_SITES_CAP: usize = 1024;
pub const SCOPED_STRIDE: i64 = 16;

/// The bump arena starts above every fixed area (§6.6; the cursor is the
/// module's mutable global 0).
pub const ARENA_BASE: i64 = 256 * 1024;

/// Region helper function encoding: name, parameter valtypes, result
/// valtypes, and body bytes.
pub type RegionHelper = (&'static [u8], Vec<u8>, Vec<u8>, Vec<u8>);

/// Linear memory pages (§6.6): initial 16 (1 MiB), max 256 (16 MiB).
pub const MEMORY_MIN_PAGES: u64 = 16;
pub const MEMORY_MAX_PAGES: u64 = 256;

// ---------------------------------------------------------------------------
// Helper identity (the fixed order the patch keys use).
// ---------------------------------------------------------------------------

pub const HELPER_CREATE: u32 = 0;
pub const HELPER_ALLOC: u32 = 1;
pub const HELPER_RESET: u32 = 2;
pub const HELPER_DESTROY: u32 = 3;

/// The helper's builtin word name, by HELPER_* id (the fixed order
/// `emit_region_helpers` emits in).
pub const HELPER_NAME: [&[u8]; 4] = [REGION_CREATE, REGION_ALLOC, REGION_RESET, REGION_DESTROY];

/// The patch key of a helper (the backend's key→index map).
pub fn helper_key(h: u32) -> Vec<u8> {
    let name: &[u8] = match h {
        HELPER_CREATE => b"$helper:create",
        HELPER_ALLOC => b"$helper:alloc",
        HELPER_RESET => b"$helper:reset",
        HELPER_DESTROY => b"$helper:destroy",
        _ => b"$helper:?",
    };
    name.to_vec()
}

// ---------------------------------------------------------------------------
// Emission helpers.
// ---------------------------------------------------------------------------

fn i32_const(out: &mut Vec<u8>, v: i32) {
    out.push(OP_I32_CONST);
    sleb(out, v as i64);
}

fn i64_const(out: &mut Vec<u8>, v: i64) {
    out.push(OP_I64_CONST);
    sleb(out, v);
}

fn local_get(out: &mut Vec<u8>, idx: u32) {
    out.push(OP_LOCAL_GET);
    uleb(out, idx as u64);
}

fn local_set(out: &mut Vec<u8>, idx: u32) {
    out.push(OP_LOCAL_SET);
    uleb(out, idx as u64);
}

fn if_void(out: &mut Vec<u8>) {
    out.push(OP_IF);
    out.push(0x40);
}

/// The out-of-line trap: `tyu.trap(code, span_start, span_len)` then
/// `unreachable` (the import raises host-side; the unreachable satisfies
/// the validator after a throwing call). The span arrives as the helper's
/// trailing i32 parameters — the CALL SITE's op span (DEC-40).
fn push_trap(code: i64, ss: u32, sl: u32, out: &mut Vec<u8>) {
    i64_const(out, code);
    local_get(out, ss);
    local_get(out, sl);
    out.push(OP_CALL);
    uleb(out, IMPORT_TRAP as u64);
    out.push(OP_UNREACHABLE);
}

/// `[cond on stack as i64]` → `if cond == 0 { trap(code) }`.
fn trap_if_zero(code: i64, ss: u32, sl: u32, out: &mut Vec<u8>) {
    out.push(OP_I64_EQZ); // i64 → i32 (1 = trap)
    if_void(out);
    push_trap(code, ss, sl, out);
    out.push(OP_END);
}

/// The i64 field load of a slot field for a handle param.
fn load_slot_field(handle_param: u32, field_off: i64, out: &mut Vec<u8>) {
    local_get(out, handle_param);
    i64_const(out, SLOT_BYTES);
    out.push(OP_I64_MUL);
    i64_const(out, REGION_TABLE_ADDR + field_off);
    out.push(OP_I64_ADD);
    out.push(OP_I32_WRAP_I64);
    out.push(OP_I64_LOAD);
    memarg(out, 0);
}

/// Push the store address of a slot field for a handle param (a store
/// consumes `[value, addr]` with the address on TOP).
fn push_slot_field_addr(handle_param: u32, field_off: i64, out: &mut Vec<u8>) {
    local_get(out, handle_param);
    i64_const(out, SLOT_BYTES);
    out.push(OP_I64_MUL);
    i64_const(out, REGION_TABLE_ADDR + field_off);
    out.push(OP_I64_ADD);
    out.push(OP_I32_WRAP_I64);
}

/// Store `value_expr` (already pushed) into a slot field.
fn store_slot_field(handle_param: u32, field_off: i64, out: &mut Vec<u8>) {
    push_slot_field_addr(handle_param, field_off, out);
    out.push(OP_I64_STORE);
    memarg(out, 0);
}

/// `(size param) → push (size + 7) & !7` — the hosted runtime's align8.
fn push_align8(size_param: u32, out: &mut Vec<u8>) {
    local_get(out, size_param);
    i64_const(out, 7);
    out.push(OP_I64_ADD);
    i64_const(out, -8);
    out.push(OP_I64_AND);
}

/// Grow the memory so it covers `pages` (an i64 local), or trap 23 — the
/// hosted runtime's mmap-failure analogue (a grow hitting the 256-page cap
/// returns -1).
fn emit_grow_to_fit(pages_local: u32, ss: u32, sl: u32, out: &mut Vec<u8>) {
    // if pages > memory.size:
    local_get(out, pages_local);
    out.push(OP_MEMORY_SIZE);
    out.push(0x00); // memory index 0
    out.push(OP_I64_EXTEND_I32_U);
    out.push(OP_I64_GT_U);
    if_void(out);
    //   delta = pages - memory.size
    local_get(out, pages_local);
    out.push(OP_MEMORY_SIZE);
    out.push(0x00);
    out.push(OP_I64_EXTEND_I32_U);
    out.push(OP_I64_SUB);
    out.push(OP_I32_WRAP_I64);
    //   old = memory.grow(delta); if old == -1 → trap 23
    out.push(OP_MEMORY_GROW);
    out.push(0x00);
    i32_const(out, -1);
    out.push(OP_I32_EQ);
    if_void(out);
    push_trap(ir::trap_code_u32(ir::TrapCode::Unreachable) as i64, ss, sl, out);
    out.push(OP_END);
    out.push(OP_END);
}

/// The shared argument guards: `handle >= 16 → trap 23`; `size == 0 → trap
/// 23` (only when `check_size`); `slot.size == 0 (dead region) → trap 23`.
fn emit_handle_guards(h: u32, size_param: Option<u32>, ss: u32, sl: u32, out: &mut Vec<u8>) {
    local_get(out, h);
    i64_const(out, REGION_TABLE_SLOTS);
    out.push(OP_I64_GE_U);
    if_void(out);
    push_trap(ir::trap_code_u32(ir::TrapCode::Unreachable) as i64, ss, sl, out);
    out.push(OP_END);
    if let Some(sp) = size_param {
        local_get(out, sp);
        out.push(OP_I64_EQZ);
        if_void(out);
        push_trap(ir::trap_code_u32(ir::TrapCode::Unreachable) as i64, ss, sl, out);
        out.push(OP_END);
    }
    load_slot_field(h, OFF_SIZE, out);
    out.push(OP_I64_EQZ);
    if_void(out);
    push_trap(ir::trap_code_u32(ir::TrapCode::Unreachable) as i64, ss, sl, out);
    out.push(OP_END);
}

/// The region-heap helper functions, in the fixed `HELPER_*` order. Each
/// entry: (name, param valtypes, result valtypes, body). The trailing i32
/// pair on every trapping helper is the call-site span (DEC-40).
pub fn emit_region_helpers() -> Vec<RegionHelper> {
    let mut out = Vec::new();

    // -- create ( usize -- Region ) ---------------------------------------
    // Native mirror: size == 0 → trap 23; scan the 16 slots for a free one
    // (size == 0) else trap 26; align8; grow the memory to fit; fill the
    // slot {base, size, off, alive}; bump the cursor; return the handle.
    {
        let mut b = Vec::new();
        // params: 0 = size (i64), 1 = span_start (i32), 2 = span_len (i32)
        // locals: 3 = asize, 4 = base, 5 = pages (all i64), 6 = slot (i32)
        locals_group(&mut b, 3, VAL_I64);
        locals_group(&mut b, 1, VAL_I32);
        const ASIZE: u32 = 3;
        const BASE: u32 = 4;
        const PAGES: u32 = 5;
        const SLOT: u32 = 6;
        const TRAP_23: i64 = ir::trap_code_u32(ir::TrapCode::Unreachable) as i64;
        const TRAP_26: i64 = ir::trap_code_u32(ir::TrapCode::RegionExhausted) as i64;

        push_align8(0, &mut b);
        local_set(&mut b, ASIZE);
        trap_if_zero(TRAP_23, 1, 2, &mut b);

        // The free-slot scan (the hosted `.region_scan` loop): the first
        // slot with size == 0 reserves; exhaustion when the index hits 16.
        b.push(OP_BLOCK);
        b.push(0x40); // $found — br here lands on the setup code
        b.push(OP_LOOP);
        b.push(0x40); // $scan
        // slot >= 16 → exhausted
        local_get(&mut b, SLOT);
        i32_const(&mut b, REGION_TABLE_SLOTS as i32);
        b.push(OP_I32_GE_U);
        b.push(OP_IF);
        b.push(0x40);
        push_trap(TRAP_26, 1, 2, &mut b);
        b.push(OP_END);
        // slot.size == 0 → free: br to $setup (depths: 0 = this if, 1 =
        // $scan, 2 = $found)
        load_slot_field(SLOT, OFF_SIZE, &mut b);
        b.push(OP_I64_EQZ);
        b.push(OP_IF);
        b.push(0x40);
        b.push(OP_BR);
        uleb(&mut b, 2);
        b.push(OP_END);
        // slot += 1; continue
        local_get(&mut b, SLOT);
        i32_const(&mut b, 1);
        b.push(OP_I32_ADD);
        local_set(&mut b, SLOT);
        b.push(OP_BR);
        uleb(&mut b, 0);
        b.push(OP_END); // $scan
        b.push(OP_END); // $found

        // -- setup (slot index in $slot) --
        // asize is already aligned; grow to fit cursor + asize.
        b.push(OP_GLOBAL_GET);
        uleb(&mut b, 0);
        local_get(&mut b, ASIZE);
        b.push(OP_I64_ADD);
        i64_const(&mut b, 65535);
        b.push(OP_I64_ADD);
        i64_const(&mut b, 16);
        b.push(OP_I64_SHR_U);
        local_set(&mut b, PAGES);
        emit_grow_to_fit(PAGES, 1, 2, &mut b);
        // base = cursor; cursor += asize
        b.push(OP_GLOBAL_GET);
        uleb(&mut b, 0);
        local_set(&mut b, BASE);
        b.push(OP_GLOBAL_GET);
        uleb(&mut b, 0);
        local_get(&mut b, ASIZE);
        b.push(OP_I64_ADD);
        b.push(OP_GLOBAL_SET);
        uleb(&mut b, 0);
        // slot {base, size, off, alive} — value first, then the field address
        local_get(&mut b, BASE);
        push_slot_field_addr(SLOT, OFF_BASE, &mut b);
        b.push(OP_I64_STORE);
        memarg(&mut b, 0);
        local_get(&mut b, ASIZE);
        store_slot_field(SLOT, OFF_SIZE, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(SLOT, OFF_OFF, &mut b);
        i64_const(&mut b, 1);
        store_slot_field(SLOT, OFF_ALIVE, &mut b);
        // return the handle
        local_get(&mut b, SLOT);
        b.push(OP_I64_EXTEND_I32_U);
        b.push(OP_RETURN);
        b.push(OP_END); // the function's implicit block
        out.push((
            REGION_CREATE,
            vec![VAL_I64, VAL_I32, VAL_I32],
            vec![VAL_I64],
            b,
        ));
    }

    // -- alloc ( Region usize -- ptr_mut ) ---------------------------------
    // Native mirror: handle/size/live guards → 23; align8; new off =
    // old off + aligned must fit the region size (unsigned) else 26; store
    // the new off; return base + OLD off.
    {
        let mut b = Vec::new();
        // params: 0 = handle, 1 = size (i64), 2 = span_start, 3 = span_len
        // locals: 4 = aligned, 5 = old off (i64)
        locals_group(&mut b, 2, VAL_I64);
        const ALIGNED: u32 = 4;
        const OLD_OFF: u32 = 5;

        emit_handle_guards(0, Some(1), 2, 3, &mut b);
        push_align8(1, &mut b);
        local_set(&mut b, ALIGNED);
        load_slot_field(0, OFF_OFF, &mut b);
        local_set(&mut b, OLD_OFF);
        // (old_off + aligned) > slot.size → trap 26 (the hosted jbe is
        // unsigned)
        load_slot_field(0, OFF_OFF, &mut b);
        local_get(&mut b, ALIGNED);
        b.push(OP_I64_ADD);
        load_slot_field(0, OFF_SIZE, &mut b);
        b.push(OP_I64_GT_U);
        if_void(&mut b);
        push_trap(ir::trap_code_u32(ir::TrapCode::RegionExhausted) as i64, 2, 3, &mut b);
        b.push(OP_END);
        // slot.off = old_off + aligned
        load_slot_field(0, OFF_OFF, &mut b);
        local_get(&mut b, ALIGNED);
        b.push(OP_I64_ADD);
        store_slot_field(0, OFF_OFF, &mut b);
        // return base + old_off
        load_slot_field(0, OFF_BASE, &mut b);
        local_get(&mut b, OLD_OFF);
        b.push(OP_I64_ADD);
        b.push(OP_END); // the function's implicit block
        out.push((
            REGION_ALLOC,
            vec![VAL_I64, VAL_I64, VAL_I32, VAL_I32],
            vec![VAL_I64],
            b,
        ));
    }

    // -- reset ( Region -- ) ------------------------------------------------
    // Native mirror: handle/live guards → 23; off = 0.
    {
        let mut b = Vec::new();
        // params: 0 = handle, 1 = span_start, 2 = span_len; no extra locals
        uleb(&mut b, 0); // zero local groups
        emit_handle_guards(0, None, 1, 2, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(0, OFF_OFF, &mut b);
        b.push(OP_END);
        out.push((REGION_RESET, vec![VAL_I64, VAL_I32, VAL_I32], vec![], b));
    }

    // -- destroy ( Region -- ) ------------------------------------------------
    // Native mirror: handle/live guards → 23; the slot zeroes out (base,
    // size, off, alive) — the size == 0 marking is what makes the slot free
    // for the next create's scan.
    {
        let mut b = Vec::new();
        // params: 0 = handle, 1 = span_start, 2 = span_len; no extra locals
        uleb(&mut b, 0);
        emit_handle_guards(0, None, 1, 2, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(0, OFF_BASE, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(0, OFF_SIZE, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(0, OFF_OFF, &mut b);
        i64_const(&mut b, 0);
        store_slot_field(0, OFF_ALIVE, &mut b);
        b.push(OP_END);
        out.push((REGION_DESTROY, vec![VAL_I64, VAL_I32, VAL_I32], vec![], b));
    }

    out
}


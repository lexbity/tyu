//! The wasm32-unknown-unknown codegen backend (PLAN-WASM-2 B1, §6.7): the
//! production IR lowered to a binary WebAssembly module by a hand-rolled
//! zero-dependency encoder (R-C/DEC-35 — the validator/runtime that check
//! the output are CI tools, never crate deps).
//!
//! Mapping (§6.7, with the as-built notes this file records):
//! - **Uniform slots**: every data-stack slot is a wasm `i64` — the plan's
//!   rationale is uniform 64-bit slots so `ConstI64` wraps identically to
//!   every hosted golden; the §6.7 table's i32 bools would force widening
//!   at every branch and compare for zero semantic gain. Pointer VALUES
//!   travel in i64 slots; memory ops `i32.wrap_i64` the address (the §6.7
//!   "the backend pays extend/truncate at pointer ops" clause —
//!   `pointer_bits: 32`).
//! - **The data stack is locals, not the wasm operand stack**: each slot
//!   lives in a local `$sN` under a compile-time height cursor, because
//!   branches cannot carry an operand stack in wasm and the IR's blocks are
//!   an arbitrary (verified) CFG. Every word lowers to ONE
//!   `loop $dispatch / block $cK* / br_table` ladder driven by an i32
//!   `$cur` block-number local — total over any CFG, while-loop back edges
//!   included (the ladder's byte layout is pinned by `emits_a_valid_module`).
//! - **Calls**: the driver streams words one at a time, so call targets are
//!   resolved by NAME PATCHING — every `call` operand is a fixed-width
//!   padded LEB placeholder recorded as `(offset, key)`, patched in the
//!   postlude once the full key→index map exists (imports, words in arrival
//!   order, used region helpers). Padded LEB is format-legal and keeps the
//!   patch byte-stable (and the `tyu.spans/1` offsets stable).
//! - **Operand order**: a Tyu binary op `( a b -- r )` keeps `a` in the
//!   deeper slot; wasm binary ops compute `i1 <op> i2` with `i2` on top, so
//!   `a` must reach the operand stack FIRST. The `Sub`/`Cmp` arms encode
//!   this order explicitly (the corpus's arithmetic/cmp fixtures pin it).
//! - **Scoped slices** (`ScopedEnter{Slice}`): the native model writes a
//!   16-byte {ptr, len} descriptor into the word's frame; the wasm analogue
//!   is a fixed per-site descriptor slot in the §6.6 descriptor area — no
//!   arena bump, no cursor save/restore (nothing was bumped).
//!   `RegionRef/Mut` enters are plain dups (native parity).
//! - **testio.***: `write-byte` → the `tyu.write_byte` import (the host
//!   shim's output channel); `exit` → the `tyu.exit` import (raises
//!   host-side with the code). `platform.mem.region-*` lower to the
//!   module's own region-heap helpers (§6.6 layout, hosted-runtime
//!   semantics), passing the CALL SITE's span for DEC-40 diagnostics.
//! - **Entry**: `main` is wrapped and exported as `tyu_main` returning i32
//!   (the exit code, `testio.exit` parity); `memory` is exported (B2's
//!   nested instantiation shares it); there is NO start section — the
//!   module does nothing at instantiation (B2 instantiates nested modules;
//!   side effects there would run the program at link time).
//! - **Rejected at compile** (FR-44): TaskSpawn/Interrupt* remain unsupported
//!   on this target. MMIO uses a packed aperture-relative pointer in i64
//!   slots and explicit `tyu.periph_read`/`tyu.periph_write` imports; the
//!   front end has already checked those places against the platform
//!   descriptor before the backend receives the IR.
//! - **Cast/Bitcast**: the native codegen's semantics, mirrored — a cast
//!   between the same primitive is a no-op retag; differing widths are
//!   canonicalized source-width-first then target-width (mask + signed
//!   round-trip), exactly `codegen-x86_64::emit_cast`. `Bitcast` emits
//!   nothing (native parity: the checker's subtype guards arrive as
//!   `TrapIfFalse` ops, which lower to `tyu.trap` calls with spans — the
//!   SubtypeFail parity lives there, matching how native gets it).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use codegen_core::{AsmMode, CodegenBackend, CodegenError};
use frontend::parse::{DeclKind, ModuleAst, Output};
use ir as lir;

mod encoder;
mod regions;
mod spans;

use encoder::*;
use regions::helper_key;

/// The `tyu` import namespace the host installs (§6.7 + DEC-40): the
/// diagnostic-channel byte writer and the span-carrying trap raiser are
/// always present; `exit` joins them iff the module calls `testio.exit`.
/// Signatures: `write_byte(i64)`, `trap(i64 code, i32 span_start,
/// i32 span_len)`, `exit(i64)`.
pub(crate) const IMPORT_WRITE_BYTE: u32 = 0;
pub(crate) const IMPORT_TRAP: u32 = 1;
pub(crate) const IMPORT_EXIT: u32 = 2;
pub(crate) const IMPORT_PERIPH_READ: u32 = 3;
pub(crate) const IMPORT_PERIPH_WRITE: u32 = 4;
pub(crate) const IMPORT_BARRIER: u32 = 5;

/// The region-ABI builtin word names (the sysroot's `platform.mem.*`
/// intrinsics) — the Call arms lower them to the module's own helpers.
pub(crate) const REGION_CREATE: &[u8] = b"platform.mem.region-create";
pub(crate) const REGION_ALLOC: &[u8] = b"platform.mem.region-alloc";
pub(crate) const REGION_RESET: &[u8] = b"platform.mem.region-reset";
pub(crate) const REGION_DESTROY: &[u8] = b"platform.mem.region-destroy";

/// Fixed-width (5-byte, padded) LEB for patchable call operands.
const CALL_LEB_WIDTH: usize = 5;

/// The data-stack slot cap the interpreter bounds `High::Top` words with
/// (wasm-repl `fuel.rs::STACK_CAP`, §6.6/NFR-4): an unbounded word's slot
/// pool is sized for the platform cap, not for a guessed peak.
const STACK_CAP: usize = 4096;

/// The patch key of a call: an import, a module word (arrival order), or a
/// used region helper.
#[derive(Clone, Debug, PartialEq)]
enum Callee {
    Import(u32),
    Exit,
    PeripheralRead,
    PeripheralWrite,
    Barrier { kind: u32, after: bool },
    Word(Vec<u8>),
    Helper(u32),
}

/// Shared, short-lived emission state passed to the operation-family handlers.
/// Each handler borrows only the fields it uses, keeping its contract explicit.
struct OpContext<'a> {
    w: &'a lir::Word,
    op: &'a lir::Op,
    out: &'a mut Vec<u8>,
    h: &'a mut usize,
    cur: usize,
    in_len: usize,
    slots: usize,
    mmio_scratch: Option<u32>,
    k: usize,
    blocks: usize,
    body_calls: &'a mut Vec<(usize, Callee)>,
    body_traps: &'a mut Vec<(usize, lir::Span)>,
}

struct Func {
    name: Vec<u8>,
    type_index: u32,
    body: Vec<u8>,
    /// (offset-in-body, callee) of every 5-byte placeholder call operand.
    calls: Vec<(usize, Callee)>,
}

/// One hardware-trap site: a word-body `Load`/`Store` whose byte offset (in
/// its body) is resolved to a module-absolute code offset in the postlude
/// (DEC-40's `tyu.spans/1` records).
struct TrapSite {
    func: usize,
    body_offset: usize,
    span: lir::Span,
}

pub struct WasmBackend<'a> {
    module: &'a ModuleAst,
    src: &'a [u8],
    out: &'a mut dyn Output,
    /// The `-g` trap-with-location flag. Spans travel unconditionally on
    /// this target (every out-of-line check carries its op span; the
    /// `tyu.spans/1` table covers hardware traps — DEC-40/FR-43), so the
    /// flag has no wasm-side effect; it is part of the shared driver
    /// surface this backend must accept.
    #[allow(dead_code)]
    debug_trap_loc: bool,
    #[allow(dead_code)]
    mode: AsmMode,

    /// Type-section entries (deduped), in first-use order.
    types: Vec<(Vec<u8>, Vec<u8>)>,
    /// Defined functions, in arrival (module-word) order.
    funcs: Vec<Func>,
    /// Does any streamed word call `testio.exit`?
    uses_exit: bool,
    uses_mmio: bool,
    uses_barrier: bool,
    entry_name: &'static [u8],
    /// Which region helpers any word referenced (emitted in the postlude,
    /// in the fixed HELPER_* order — dead helpers never ship).
    used_helpers: [bool; 4],
    /// ConstStr interning: (span.start, address) — deduped by span start;
    /// bytes stored with the interpreter's {u64 len}{bytes} layout.
    strings: Vec<(u32, i64)>,
    string_bytes: Vec<u8>,
    /// Resource places interned in first-use order (AddrOf{Runtime}).
    resources: Vec<Vec<u8>>,
    /// Scoped-slice descriptor sites, in module order (Slice class only).
    scoped_sites: usize,
    /// Hardware-trap sites for the span side table.
    trap_sites: Vec<TrapSite>,
}

impl<'a> WasmBackend<'a> {
    pub fn new(
        module: &'a ModuleAst,
        src: &'a [u8],
        out: &'a mut dyn Output,
        debug_trap_loc: bool,
        mode: AsmMode,
    ) -> Self {
        WasmBackend {
            module,
            src,
            out,
            debug_trap_loc,
            mode,
            types: Vec::new(),
            funcs: Vec::new(),
            uses_exit: false,
            uses_mmio: false,
            uses_barrier: false,
            entry_name: b"main",
            used_helpers: [false; 4],
            strings: Vec::new(),
            string_bytes: Vec::new(),
            resources: Vec::new(),
            scoped_sites: 0,
            trap_sites: Vec::new(),
        }
    }

    fn type_index(&mut self, params: &[u8], results: &[u8]) -> u32 {
        if let Some(i) = self
            .types
            .iter()
            .position(|(p, r)| p.as_slice() == params && r.as_slice() == results)
        {
            return i as u32;
        }
        self.types.push((params.to_vec(), results.to_vec()));
        (self.types.len() - 1) as u32
    }

    /// Select the IR word wrapped by the exported `tyu_main` entry point.
    /// Production compiler invocations keep `main`; the browser session
    /// uses a generated entry word for a trailing interactive phrase.
    pub fn set_entry_word(&mut self, name: &'static [u8]) {
        self.entry_name = name;
    }

    /// The word's stack-slot local count: the verified peak (the IR's
    /// `StackBound.high` is growth-above-entry, so the absolute peak is
    /// `in_len + extra`; unbounded words size for the platform cap) plus
    /// one headroom slot — the `Swap` arm exchanges through the slot just
    /// above the live height.
    fn slot_locals(w: &lir::Word) -> usize {
        let peak = match w.bound.high {
            lir::High::Top => STACK_CAP,
            lir::High::Slots(extra) => w.sig.in_len as usize + extra as usize,
        };
        peak.max(w.sig.in_len as usize) + 1
    }

    /// The wasm local index of an IR `LocalGet/Set{slot}`: IR locals are a
    /// namespace SEPARATE from the data stack. Slots below `in_len` ARE the
    /// wasm params (locals 0..in_len — the prologue's param move is then a
    /// self-assign, and the initial data stack IS the declared inputs);
    /// slots at/above live in their own i64 locals declared after the
    /// plumbing (slot pool, dispatch cursor). Wasm assigns group locals
    /// AFTER the params, so every index is in_len-shifted.
    fn declared_local(slot: u16, in_len: usize, slots: usize) -> u32 {
        if (slot as usize) < in_len {
            slot as u32
        } else {
            (in_len + slots + 1 + (slot as usize - in_len)) as u32
        }
    }

    fn declared_local_with_scratch(
        slot: u16,
        in_len: usize,
        slots: usize,
        has_scratch: bool,
    ) -> u32 {
        if has_scratch && (slot as usize) >= in_len {
            (in_len + slots + 2 + (slot as usize - in_len)) as u32
        } else {
            Self::declared_local(slot, in_len, slots)
        }
    }

    fn mmio_width(w: &lir::Word, ty: lir::TypeId) -> Result<u8, CodegenError> {
        let (bits, _) = lir::storage_bits_signed(w, ty, 64)
            .ok_or(CodegenError::UnknownTypeProperties { type_id: ty })?;
        let width = match bits {
            8 => 1,
            16 => 2,
            32 => 4,
            64 => 8,
            _ => return Err(CodegenError::UnsupportedConstruct),
        };
        Ok(width)
    }

    fn emit_mmio_addr_parts(ptr: u32, out: &mut Vec<u8>) {
        // MMIO pointers are encoded as (aperture << 32) | offset. Pointer
        // arithmetic changes only the low 32-bit aperture-relative offset.
        out.push(OP_LOCAL_GET);
        uleb(out, ptr as u64);
        out.push(OP_I64_CONST);
        sleb(out, 32);
        out.push(OP_I64_SHR_U);
        out.push(OP_I32_WRAP_I64);
        out.push(OP_LOCAL_GET);
        uleb(out, ptr as u64);
        out.push(OP_I32_WRAP_I64);
    }

    fn emit_periph_read(ptr: u32, width: u8, out: &mut Vec<u8>, calls: &mut Vec<(usize, Callee)>) {
        Self::emit_mmio_addr_parts(ptr, out);
        out.push(OP_I32_CONST);
        sleb(out, i64::from(width));
        out.push(OP_CALL);
        let off = out.len();
        out.extend_from_slice(&[0; CALL_LEB_WIDTH]);
        calls.push((off, Callee::PeripheralRead));
    }

    fn emit_periph_write_prefix(ptr: u32, width: u8, out: &mut Vec<u8>) {
        Self::emit_mmio_addr_parts(ptr, out);
        out.push(OP_I32_CONST);
        sleb(out, i64::from(width));
    }

    /// Check every source-level linear-memory access before issuing the Wasm
    /// load/store. JS engines expose compiled-code PCs for hardware traps,
    /// not stable Wasm byte offsets, so DEC-40's custom-section table cannot
    /// be the primary path for a student OOB access. This guard routes that
    /// case through the typed trap import with the originating IR span. The
    /// side table remains as a fail-closed fallback for unexpected engine
    /// traps in generated code.
    fn emit_memory_guard(
        scratch: u32,
        span: lir::Span,
        out: &mut Vec<u8>,
        calls: &mut Vec<(usize, Callee)>,
    ) {
        // Require a zero-extended wasm32 pointer, keep the reserved null page
        // inaccessible, and ensure the full i64 access fits in current memory.
        out.push(OP_LOCAL_GET);
        uleb(out, u64::from(scratch));
        out.push(OP_I64_CONST);
        sleb(out, 32);
        out.push(OP_I64_SHR_U);
        out.push(OP_I64_EQZ);

        out.push(OP_LOCAL_GET);
        uleb(out, u64::from(scratch));
        out.push(OP_I32_WRAP_I64);
        out.push(OP_I32_CONST);
        sleb(out, 4096);
        out.push(OP_I32_GE_U);
        out.push(OP_I32_AND);

        out.push(OP_LOCAL_GET);
        uleb(out, u64::from(scratch));
        out.push(OP_I32_WRAP_I64);
        out.push(OP_MEMORY_SIZE);
        out.push(0); // memory index 0
        out.push(OP_I32_CONST);
        sleb(out, 16);
        out.push(OP_I32_SHL);
        out.push(OP_I32_CONST);
        sleb(out, 8);
        out.push(OP_I32_SUB);
        out.push(OP_I32_LE_U);
        out.push(OP_I32_AND);

        out.push(OP_IF);
        out.push(0x40); // empty block type
        out.push(OP_ELSE);
        out.push(OP_I64_CONST);
        sleb(out, 22); // frozen Bounds trap code
        out.push(OP_I32_CONST);
        sleb(out, span.start as i64);
        out.push(OP_I32_CONST);
        sleb(out, (span.end - span.start) as i64);
        out.push(OP_CALL);
        let off = out.len();
        out.extend_from_slice(&[0; CALL_LEB_WIDTH]);
        calls.push((off, Callee::Import(IMPORT_TRAP)));
        out.push(OP_UNREACHABLE);
        out.push(OP_END);
    }

    fn emit_barrier(
        kind: lir::BarrierKind,
        after: bool,
        out: &mut Vec<u8>,
        calls: &mut Vec<(usize, Callee)>,
    ) {
        let value = match kind {
            lir::BarrierKind::None => return,
            lir::BarrierKind::Before => 1,
            lir::BarrierKind::After => 2,
            lir::BarrierKind::Both => 3,
        };
        out.push(OP_I32_CONST);
        sleb(out, value);
        out.push(OP_I32_CONST);
        sleb(out, i64::from(after));
        out.push(OP_CALL);
        let off = out.len();
        out.extend_from_slice(&[0; CALL_LEB_WIDTH]);
        calls.push((
            off,
            Callee::Barrier {
                kind: value as u32,
                after,
            },
        ));
    }

    fn block_index(w: &lir::Word, id: lir::BlockId) -> Option<usize> {
        w.blocks.iter().position(|b| b.id == id)
    }

    fn intern_string(&mut self, span: lir::Span) -> Result<i64, CodegenError> {
        if let Some((_, a)) = self.strings.iter().find(|(s, _)| *s == span.start as u32) {
            return Ok(*a);
        }
        let bytes = self
            .src
            .get(span.start..span.end)
            .ok_or(CodegenError::MalformedIr { detail: 1 })?;
        // The interpreter's intern layout: {u64 len}{bytes} — 8 bytes over
        // the content.
        let total = 8 + bytes.len() as u64;
        if total > regions::STRINGS_CAP.saturating_sub(self.string_bytes.len() as u64) {
            return Err(CodegenError::StringLiteralCapacityExceeded);
        }
        let addr = regions::STRINGS_BASE + self.string_bytes.len() as i64;
        self.string_bytes
            .extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.string_bytes.extend_from_slice(bytes);
        self.strings.push((span.start as u32, addr));
        Ok(addr)
    }

    /// The interned address of a resource place (native parity: the hosted
    /// runtime emits a distinct stable bss symbol per declared resource; the
    /// wasm analogue is a distinct zero-init slot in the §6.6 resource
    /// area). Undeclared places are the hosted runtime's loud
    /// `UnsupportedAddrOf`, never an invented address.
    fn intern_resource(&mut self, place: &[u8]) -> Result<i64, CodegenError> {
        if !find_resource_decl(self.module, self.src, place) {
            return Err(CodegenError::UnsupportedAddrOf);
        }
        if let Some(i) = self.resources.iter().position(|p| p.as_slice() == place) {
            return Ok(regions::RESOURCES_BASE + i as i64 * regions::RESOURCE_STRIDE);
        }
        if self.resources.len() >= regions::RESOURCES_CAP {
            return Err(CodegenError::ResourceCapacityExceeded);
        }
        self.resources.push(place.to_vec());
        Ok(regions::RESOURCES_BASE + (self.resources.len() - 1) as i64 * regions::RESOURCE_STRIDE)
    }

    /// The next scoped-slice descriptor address (one 16-byte {ptr, len}
    /// slot per `ScopedEnter{Slice}` site, in module order — the native
    /// frame descriptor model; overflow is the hosted runtime's
    /// `ScopedAllocationOverflow`).
    fn next_scoped_desc(&mut self) -> Result<i64, CodegenError> {
        if self.scoped_sites >= regions::SCOPED_SITES_CAP {
            return Err(CodegenError::ScopedAllocationOverflow);
        }
        let addr = regions::SCOPED_BASE + self.scoped_sites as i64 * regions::SCOPED_STRIDE;
        self.scoped_sites += 1;
        Ok(addr)
    }

    /// Emit one IR op, preserving barrier placement while delegating each
    /// operation family to a bounded, reviewable handler.
    #[allow(clippy::too_many_arguments)]
    fn emit_op(
        &mut self,
        w: &lir::Word,
        op: &lir::Op,
        out: &mut Vec<u8>,
        h: &mut usize,
        cur: usize,
        in_len: usize,
        slots: usize,
        mmio_scratch: Option<u32>,
        k: usize,
        blocks: usize,
        body_calls: &mut Vec<(usize, Callee)>,
        body_traps: &mut Vec<(usize, lir::Span)>,
    ) -> Result<(), CodegenError> {
        let barrier = match op.kind {
            lir::OpKind::MmioVolLoad { barrier, .. }
            | lir::OpKind::MmioVolStore { barrier, .. }
            | lir::OpKind::MmioVolLoadField { barrier, .. }
            | lir::OpKind::MmioVolStoreField { barrier, .. } => Some(barrier),
            _ => None,
        };
        if let Some(kind) = barrier {
            Self::emit_barrier(kind, false, out, body_calls);
        }
        let handled = {
            let mut ctx = OpContext {
                w,
                op,
                out,
                h,
                cur,
                in_len,
                slots,
                mmio_scratch,
                k,
                blocks,
                body_calls,
                body_traps,
            };
            self.emit_op_stack(&mut ctx)?
                || self.emit_op_call(&mut ctx)?
                || self.emit_op_memory(&mut ctx)?
                || self.emit_op_mmio(&mut ctx)?
                || self.emit_op_control(&mut ctx)?
                || self.emit_op_pointer(&mut ctx)?
        };
        if !handled {
            return Err(CodegenError::UnsupportedOp {
                op_name: b"operation dispatch",
            });
        }
        if let Some(kind) = barrier {
            Self::emit_barrier(kind, true, out, body_calls);
        }
        Ok(())
    }

    fn emit_op_stack(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        let in_len = ctx.in_len;
        let slots = ctx.slots;
        let mmio_scratch = ctx.mmio_scratch;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        macro_rules! pop {
            () => {{
                *h -= 1;
                let hh = *h;
                out.push(OP_LOCAL_GET);
                uleb(out, hh as u64);
            }};
        }
        macro_rules! binary {
            ($opcode:expr) => {{
                *h -= 2;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push($opcode);
                push!();
            }};
        }
        macro_rules! boolify {
            () => {{
                out.push(OP_I64_EQZ);
                out.push(OP_I32_EQZ);
                out.push(OP_I64_EXTEND_I32_U);
            }};
        }
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                out.push(OP_I64_CONST);
                sleb(out, v);
                push!();
            }
            lir::OpKind::ConstBool(v) => {
                out.push(OP_I64_CONST);
                sleb(out, if v { 1 } else { 0 });
                push!();
            }
            lir::OpKind::ConstStr(span) => {
                let addr = self.intern_string(span)?;
                out.push(OP_I64_CONST);
                sleb(out, addr);
                push!();
            }
            lir::OpKind::Drop { .. } => {
                pop!();
                out.push(OP_DROP);
            }
            lir::OpKind::Dup { .. } => {
                let top = *h - 1;
                out.push(OP_LOCAL_GET);
                uleb(out, top as u64);
                out.push(OP_LOCAL_SET);
                uleb(out, *h as u64);
                *h += 1;
            }
            lir::OpKind::Swap { .. } => {
                // Exchange s(h-1) and s(h-2) through s(h) as scratch (the
                // pool's +1 headroom slot).
                let a = *h - 1;
                let b = *h - 2;
                let scratch = *h;
                out.push(OP_LOCAL_GET);
                uleb(out, a as u64);
                out.push(OP_LOCAL_SET);
                uleb(out, scratch as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, b as u64);
                out.push(OP_LOCAL_SET);
                uleb(out, a as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, scratch as u64);
                out.push(OP_LOCAL_SET);
                uleb(out, b as u64);
            }
            lir::OpKind::AddI64 => {
                #[cfg(feature = "wasm-selftest")]
                {
                    // G54's miscompile selftest (AC-B1): a deliberately
                    // wrong opcode arm under the feature switch — the
                    // parity leg MUST go red, proving the native goldens
                    // are actually consulted. Mirrors the engine-side
                    // `wasm-selftest` corruption (G52/G53 pattern).
                    binary!(OP_I64_SUB);
                }
                #[cfg(not(feature = "wasm-selftest"))]
                binary!(OP_I64_ADD);
            }
            lir::OpKind::SubI64 => binary!(OP_I64_SUB),
            lir::OpKind::MulI64 => binary!(OP_I64_MUL),
            lir::OpKind::Cmp { kind, .. } => {
                *h -= 2;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(match kind {
                    lir::CmpKind::Eq => OP_I64_EQ,
                    lir::CmpKind::Ne => OP_I64_NE,
                    lir::CmpKind::Lt => OP_I64_LT_S,
                    lir::CmpKind::Gt => OP_I64_GT_S,
                    lir::CmpKind::Le => OP_I64_LE_S,
                    lir::CmpKind::Ge => OP_I64_GE_S,
                });
                // Comparisons already produce canonical i32 booleans;
                // widen directly into the backend's uniform i64 slots.
                out.push(OP_I64_EXTEND_I32_U);
                push!();
            }
            lir::OpKind::AndBool => {
                // Native parity: `(a & b) != 0` canonicalized to 0/1.
                *h -= 2;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(OP_I64_AND);
                boolify!();
                push!();
            }
            lir::OpKind::OrBool => {
                // Native parity: `(a | b) != 0` canonicalized to 0/1.
                *h -= 2;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(OP_I64_OR);
                boolify!();
                push!();
            }
            lir::OpKind::NotBool => {
                // Native parity: `(a == 0)` — i64.eqz's verdict IS the
                // canonical bool.
                pop!();
                out.push(OP_I64_EQZ);
                out.push(OP_I64_EXTEND_I32_U);
                push!();
            }
            lir::OpKind::LocalSet { slot, .. } => {
                // IR LocalSet: pop from data stack, store in IR declared local.
                // The value must move from the data stack slot to the IR local.
                pop!(); // Get value from data stack onto wasm operand stack
                out.push(OP_LOCAL_SET);
                uleb(
                    out,
                    Self::declared_local_with_scratch(slot, in_len, slots, mmio_scratch.is_some())
                        as u64,
                );
            }
            lir::OpKind::LocalGet { slot, .. } => {
                // IR LocalGet: get from IR declared local, push to data stack.
                out.push(OP_LOCAL_GET);
                uleb(
                    out,
                    Self::declared_local_with_scratch(slot, in_len, slots, mmio_scratch.is_some())
                        as u64,
                );
                push!(); // Store value from wasm operand stack into data stack
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn emit_op_call(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        let body_calls = &mut *ctx.body_calls;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        macro_rules! span_args {
            () => {{
                out.push(OP_I32_CONST);
                sleb(out, op.span.start as i64);
                out.push(OP_I32_CONST);
                sleb(out, (op.span.end - op.span.start) as i64);
            }};
        }
        macro_rules! pcall {
            ($callee:expr) => {{
                out.push(OP_CALL);
                let off = out.len();
                out.extend_from_slice(&[0u8; CALL_LEB_WIDTH]);
                body_calls.push((off, $callee));
            }};
        }
        match op.kind {
            lir::OpKind::Call { name, sig, .. } => {
                let n = name.as_bytes();
                let argc = sig.in_len as usize;
                let out_len = sig.out_len as usize;
                // args: s(h-argc) .. s(h-1) in order (the callee's first
                // parameter is the DEEPEST slot).
                for i in 0..argc {
                    out.push(OP_LOCAL_GET);
                    uleb(out, (*h - argc + i) as u64);
                }
                *h -= argc;
                let callee = if n == b"testio.write-byte" || n == b"testio.emit-output" {
                    Callee::Import(IMPORT_WRITE_BYTE)
                } else if n == b"testio.exit" {
                    Callee::Exit
                } else if n == REGION_CREATE {
                    self.used_helpers[regions::HELPER_CREATE as usize] = true;
                    span_args!();
                    Callee::Helper(regions::HELPER_CREATE)
                } else if n == REGION_ALLOC {
                    self.used_helpers[regions::HELPER_ALLOC as usize] = true;
                    span_args!();
                    Callee::Helper(regions::HELPER_ALLOC)
                } else if n == REGION_RESET {
                    self.used_helpers[regions::HELPER_RESET as usize] = true;
                    span_args!();
                    Callee::Helper(regions::HELPER_RESET)
                } else if n == REGION_DESTROY {
                    self.used_helpers[regions::HELPER_DESTROY as usize] = true;
                    span_args!();
                    Callee::Helper(regions::HELPER_DESTROY)
                } else {
                    Callee::Word(n.to_vec())
                };
                pcall!(callee);
                for _ in 0..out_len {
                    push!();
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn emit_op_memory(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        let mmio_scratch = ctx.mmio_scratch;
        let body_calls = &mut *ctx.body_calls;
        let body_traps = &mut *ctx.body_traps;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        macro_rules! pop {
            () => {{
                *h -= 1;
                let hh = *h;
                out.push(OP_LOCAL_GET);
                uleb(out, hh as u64);
            }};
        }
        match op.kind {
            lir::OpKind::Load { ty: _ } => {
                pop!(); // addr (i64 slot)
                let scratch = mmio_scratch.expect("every word has the memory scratch local");
                out.push(OP_LOCAL_SET);
                uleb(out, u64::from(scratch));
                Self::emit_memory_guard(scratch, op.span, out, body_calls);
                out.push(OP_LOCAL_GET);
                uleb(out, u64::from(scratch));
                out.push(OP_I32_WRAP_I64); // memory addressing is i32 (§6.7)
                out.push(OP_I64_LOAD);
                memarg(out, 0);
                body_traps.push((out.len() - 2, op.span)); // the load opcode
                push!();
            }
            lir::OpKind::Store { ty: _ } => {
                // stack: [addr, value] → wasm store consumes [addr, value].
                *h -= 2;
                let scratch = mmio_scratch.expect("every word has the memory scratch local");
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_SET);
                uleb(out, u64::from(scratch));
                Self::emit_memory_guard(scratch, op.span, out, body_calls);
                out.push(OP_LOCAL_GET);
                uleb(out, u64::from(scratch));
                out.push(OP_I32_WRAP_I64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(OP_I64_STORE);
                memarg(out, 0);
                body_traps.push((out.len() - 2, op.span)); // the store opcode
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn emit_op_mmio(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let w = ctx.w;
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        let mmio_scratch = ctx.mmio_scratch;
        let body_calls = &mut *ctx.body_calls;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        match op.kind {
            lir::OpKind::MmioVolLoad { ty, .. } => {
                let ptr = (*h - 1) as u32;
                let width = Self::mmio_width(w, ty)?;
                *h -= 1;
                Self::emit_periph_read(ptr, width, out, body_calls);
                let bits = i64::from(width) * 8;
                if width < 8 {
                    out.push(OP_I64_CONST);
                    sleb(out, (1i64 << bits) - 1);
                    out.push(OP_I64_AND);
                }
                push!();
            }
            lir::OpKind::MmioVolStore { ty, write_kind, .. } => {
                let ptr = (*h - 2) as u32;
                let val = (*h - 1) as u32;
                let width = Self::mmio_width(w, ty)?;
                let scratch = mmio_scratch.ok_or(CodegenError::UnsupportedConstruct)?;
                if write_kind != lir::WriteKind::Plain {
                    Self::emit_periph_read(ptr, width, out, body_calls);
                    out.push(OP_LOCAL_SET);
                    uleb(out, u64::from(scratch));
                }
                Self::emit_periph_write_prefix(ptr, width, out);
                if write_kind == lir::WriteKind::Plain {
                    out.push(OP_LOCAL_GET);
                    uleb(out, u64::from(val));
                } else {
                    out.push(OP_LOCAL_GET);
                    uleb(out, u64::from(scratch));
                    out.push(OP_LOCAL_GET);
                    uleb(out, u64::from(val));
                    match write_kind {
                        lir::WriteKind::W1c => {
                            out.push(OP_I64_CONST);
                            sleb(out, -1);
                            out.push(OP_I64_XOR);
                            out.push(OP_I64_AND);
                        }
                        lir::WriteKind::W1s => out.push(OP_I64_OR),
                        lir::WriteKind::Xor => out.push(OP_I64_XOR),
                        lir::WriteKind::Plain => unreachable!(),
                    }
                }
                if width < 8 {
                    out.push(OP_I64_CONST);
                    sleb(out, (1i64 << (u32::from(width) * 8)) - 1);
                    out.push(OP_I64_AND);
                }
                out.push(OP_CALL);
                let off = out.len();
                out.extend_from_slice(&[0; CALL_LEB_WIDTH]);
                body_calls.push((off, Callee::PeripheralWrite));
                *h -= 2;
            }
            lir::OpKind::MmioVolLoadField {
                reg_ty,
                field_ty,
                mask,
                shift,
                ..
            } => {
                let ptr = (*h - 1) as u32;
                let width = Self::mmio_width(w, reg_ty)?;
                let field_width = Self::mmio_width(w, field_ty)?;
                *h -= 1;
                Self::emit_periph_read(ptr, width, out, body_calls);
                out.push(OP_I64_CONST);
                sleb(out, mask as i64);
                out.push(OP_I64_AND);
                if shift != 0 {
                    out.push(OP_I64_CONST);
                    sleb(out, i64::from(shift));
                    out.push(OP_I64_SHR_U);
                }
                let bits = i64::from(field_width) * 8;
                if field_width < 8 {
                    out.push(OP_I64_CONST);
                    sleb(out, (1i64 << bits) - 1);
                    out.push(OP_I64_AND);
                }
                push!();
            }
            lir::OpKind::MmioVolStoreField {
                reg_ty,
                mask,
                shift,
                write_kind,
                ..
            } => {
                let ptr = (*h - 2) as u32;
                let field = (*h - 1) as u32;
                let width = Self::mmio_width(w, reg_ty)?;
                let scratch = mmio_scratch.ok_or(CodegenError::UnsupportedConstruct)?;
                Self::emit_periph_read(ptr, width, out, body_calls);
                out.push(OP_LOCAL_SET);
                uleb(out, u64::from(scratch));
                Self::emit_periph_write_prefix(ptr, width, out);
                if write_kind != lir::WriteKind::Plain {
                    out.push(OP_LOCAL_GET);
                    uleb(out, u64::from(scratch));
                }
                out.push(OP_LOCAL_GET);
                uleb(out, u64::from(scratch));
                out.push(OP_I64_CONST);
                sleb(out, !(mask as i64));
                out.push(OP_I64_AND);
                out.push(OP_LOCAL_GET);
                uleb(out, u64::from(field));
                if shift != 0 {
                    out.push(OP_I64_CONST);
                    sleb(out, i64::from(shift));
                    out.push(OP_I64_SHL);
                }
                out.push(OP_I64_CONST);
                sleb(out, mask as i64);
                out.push(OP_I64_AND);
                out.push(OP_I64_OR);
                if write_kind != lir::WriteKind::Plain {
                    match write_kind {
                        lir::WriteKind::W1c => {
                            out.push(OP_I64_CONST);
                            sleb(out, -1);
                            out.push(OP_I64_XOR);
                            out.push(OP_I64_AND);
                        }
                        lir::WriteKind::W1s => out.push(OP_I64_OR),
                        lir::WriteKind::Xor => out.push(OP_I64_XOR),
                        lir::WriteKind::Plain => unreachable!(),
                    }
                }
                if width < 8 {
                    out.push(OP_I64_CONST);
                    sleb(out, (1i64 << (u32::from(width) * 8)) - 1);
                    out.push(OP_I64_AND);
                }
                out.push(OP_CALL);
                let off = out.len();
                out.extend_from_slice(&[0; CALL_LEB_WIDTH]);
                body_calls.push((off, Callee::PeripheralWrite));
                *h -= 2;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn emit_op_control(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let w = ctx.w;
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        let cur = ctx.cur;
        let k = ctx.k;
        let blocks = ctx.blocks;
        let body_calls = &mut *ctx.body_calls;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        macro_rules! pop {
            () => {{
                *h -= 1;
                let hh = *h;
                out.push(OP_LOCAL_GET);
                uleb(out, hh as u64);
            }};
        }
        macro_rules! pcall {
            ($callee:expr) => {{
                out.push(OP_CALL);
                let off = out.len();
                out.extend_from_slice(&[0u8; CALL_LEB_WIDTH]);
                body_calls.push((off, $callee));
            }};
        }
        match op.kind {
            lir::OpKind::TrapIfFalse { code } => {
                pop!(); // the guard's bool (canonical 0/1 i64)
                out.push(OP_I64_EQZ); // i32: 1 when the guard FAILED
                out.push(OP_IF);
                out.push(0x40);
                // tyu.trap(code, span_start, span_len) — DEC-40: the span
                // travels with the trap, never reconstructed host-side.
                out.push(OP_I64_CONST);
                sleb(out, lir::trap_code_u32(code) as i64);
                out.push(OP_I32_CONST);
                sleb(out, op.span.start as i64);
                out.push(OP_I32_CONST);
                sleb(out, (op.span.end - op.span.start) as i64);
                pcall!(Callee::Import(IMPORT_TRAP));
                out.push(OP_UNREACHABLE);
                out.push(OP_END);
            }
            lir::OpKind::Cast { from, to } => {
                Self::emit_cast(w, from, to, out, *h);
            }
            // Native parity: Bitcast is a pure retag — the slot value is
            // unchanged, no check (§Q7's reading, pinned by the goldens).
            lir::OpKind::Bitcast { .. } => {}
            lir::OpKind::Br { target } => {
                let t =
                    Self::block_index(w, target).ok_or(CodegenError::MalformedIr { detail: 2 })?;
                out.push(OP_I32_CONST);
                sleb(out, t as i64);
                out.push(OP_LOCAL_SET);
                uleb(out, cur as u64);
                // Re-dispatch: the $dispatch loop sits `blocks - k` labels
                // above this case's code (k case blocks remain open).
                out.push(OP_BR);
                uleb(out, (blocks - k) as u64);
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                let t = Self::block_index(w, then_tgt)
                    .ok_or(CodegenError::MalformedIr { detail: 2 })?;
                let e = Self::block_index(w, else_tgt)
                    .ok_or(CodegenError::MalformedIr { detail: 2 })?;
                pop!(); // the condition (canonical 0/1 i64)
                out.push(OP_I32_WRAP_I64); // 0/1 → i32 for select
                out.push(OP_LOCAL_SET);
                uleb(out, cur as u64); // hold condition until it is topmost
                out.push(OP_I32_CONST);
                sleb(out, t as i64);
                out.push(OP_I32_CONST);
                sleb(out, e as i64);
                out.push(OP_LOCAL_GET);
                uleb(out, cur as u64);
                out.push(OP_SELECT); // stack order: then, else, condition
                out.push(OP_LOCAL_SET);
                uleb(out, cur as u64);
                // BOTH arms re-dispatch: the IR has no implicit fallthrough
                // (taken or not, the next dispatch decides).
                out.push(OP_BR);
                uleb(out, (blocks - k) as u64);
            }
            lir::OpKind::Ret => {
                // The function's results: the TOP out_len slots, pushed in
                // order (bottom of the window first).
                let out_len = w.sig.out_len as usize;
                for i in 0..out_len {
                    out.push(OP_LOCAL_GET);
                    uleb(out, (*h - out_len + i) as u64);
                }
                *h -= out_len;
                out.push(OP_RETURN);
            }
            lir::OpKind::ScopedEnter { ty, len } => match type_class(w, ty) {
                lir::TypeClass::Slice => {
                    // Native model: a 16-byte {ptr, len} frame descriptor;
                    // the wasm analogue is this site's fixed descriptor
                    // slot (§6.6 scoped area). Stack: [ptr] → [desc].
                    let desc = self.next_scoped_desc()?;
                    pop!(); // ptr (value stays on the wasm stack)
                            // desc[0] = ptr
                    out.push(OP_I32_CONST);
                    sleb(out, desc);
                    out.push(OP_I64_STORE);
                    memarg(out, 0);
                    // desc[8] = len
                    out.push(OP_I64_CONST);
                    sleb(out, len as i64);
                    out.push(OP_I32_CONST);
                    sleb(out, desc + 8);
                    out.push(OP_I64_STORE);
                    memarg(out, 0);
                    out.push(OP_I64_CONST);
                    sleb(out, desc);
                    push!();
                }
                // Native parity: a region-ref scoped enter is a plain dup.
                lir::TypeClass::RegionRef | lir::TypeClass::RegionRefMut => {
                    let top = *h - 1;
                    out.push(OP_LOCAL_GET);
                    uleb(out, top as u64);
                    out.push(OP_LOCAL_SET);
                    uleb(out, *h as u64);
                    *h += 1;
                }
                // D-13: an unmatched class is a loud error, never a silent
                // no-op (the G-5 bug class the native slice retires).
                _ => {
                    return Err(CodegenError::UnsupportedOp {
                        op_name: b"ScopedEnter",
                    })
                }
            },
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn emit_op_pointer(&mut self, ctx: &mut OpContext<'_>) -> Result<bool, CodegenError> {
        let op = ctx.op;
        let out = &mut *ctx.out;
        let h = &mut *ctx.h;
        macro_rules! push {
            () => {{
                let hh = *h;
                out.push(OP_LOCAL_SET);
                uleb(out, hh as u64);
                *h += 1;
            }};
        }
        macro_rules! pop {
            () => {{
                *h -= 1;
                let hh = *h;
                out.push(OP_LOCAL_GET);
                uleb(out, hh as u64);
            }};
        }
        match op.kind {
            lir::OpKind::AddrOf {
                place,
                mutable: _,
                base: lir::AddrOfBase::Runtime,
            } => {
                // Native parity (BUG-004): resources are addressable — emit
                // the place's stable interned address (the intern validates
                // the declaration; undeclared → UnsupportedAddrOf).
                let addr = self.intern_resource(place.as_bytes())?;
                out.push(OP_I64_CONST);
                sleb(out, addr);
                push!();
            }
            lir::OpKind::AddrOf {
                base: lir::AddrOfBase::Mmio { aperture, offset },
                ..
            }
            | lir::OpKind::MmioPlace {
                aperture, offset, ..
            } => {
                let encoded = (i64::from(aperture) << 32) | i64::from(offset);
                out.push(OP_I64_CONST);
                sleb(out, encoded);
                push!();
            }
            lir::OpKind::PtrAddConst { offset, .. } => {
                pop!();
                out.push(OP_I64_CONST);
                sleb(out, offset as i64);
                out.push(OP_I64_ADD);
                push!();
            }
            lir::OpKind::PtrAddIndex { scale, .. } => {
                // stack: [base, idx] (idx on top) → base + idx*scale.
                *h -= 1;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(OP_I64_CONST);
                sleb(out, scale as i64);
                out.push(OP_I64_MUL);
                out.push(OP_I64_ADD);
            }
            lir::OpKind::TaskSpawn { .. }
            | lir::OpKind::InterruptEnable
            | lir::OpKind::InterruptDisable => {
                return Err(CodegenError::UnsupportedConstruct);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Cast semantics mirrored from the native `emit_cast` (I-12): the same
    /// primitive = no-op; differing widths are canonicalized source-width
    /// first (mask + signed round-trip) then target-width. Transforms are
    /// in-place on the top slot.
    fn emit_cast(w: &lir::Word, from: lir::TypeId, to: lir::TypeId, out: &mut Vec<u8>, h: usize) {
        let (Some((from_bits, from_signed)), Some((to_bits, to_signed))) =
            (prim_bits_signed(w, from), prim_bits_signed(w, to))
        else {
            return; // non-primitive on either side: the native no-op
        };
        if from_bits == to_bits && from_signed == to_signed {
            return;
        }
        let top = h - 1;
        if from_bits < 64 {
            mask_top(out, top, mask_for_bits(from_bits));
            if from_signed {
                sign_extend(out, top, 64 - i64::from(from_bits));
            }
        }
        if to_bits < 64 {
            mask_top(out, top, mask_for_bits(to_bits));
            if to_signed {
                sign_extend(out, top, 64 - i64::from(to_bits));
            }
        }
    }
}

/// `top = top & mask` (in-place slot transform).
fn mask_top(out: &mut Vec<u8>, top: usize, mask: u64) {
    out.push(OP_LOCAL_GET);
    uleb(out, top as u64);
    out.push(OP_I64_CONST);
    sleb(out, mask as i64);
    out.push(OP_I64_AND);
    out.push(OP_LOCAL_SET);
    uleb(out, top as u64);
}

/// In-place sign-extension of a slot: `v = (v << sh) >>s sh`.
fn sign_extend(out: &mut Vec<u8>, top: usize, sh: i64) {
    out.push(OP_LOCAL_GET);
    uleb(out, top as u64);
    out.push(OP_I64_CONST);
    sleb(out, sh);
    out.push(OP_I64_SHL);
    out.push(OP_I64_CONST);
    sleb(out, sh);
    out.push(OP_I64_SHR_S);
    out.push(OP_LOCAL_SET);
    uleb(out, top as u64);
}

fn mask_for_bits(bits: u16) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The storage primitive of a type (subtypes resolve to their base) as
/// (bits, signed) — the exact source of the native cast's widths.
fn prim_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    let storage = lir::resolve_storage_type(w, ty)?;
    let name = w.types.get(storage.0 as usize).map(|a| a.as_bytes())?;
    lir::Prim::from_type_name(name).map(|p| p.bits_signed(64))
}

/// The compiler-computed class tag of a type (decision D-13 — backends
/// dispatch on the tag, never on type-name bytes).
fn type_class(w: &lir::Word, ty: lir::TypeId) -> lir::TypeClass {
    w.type_classes
        .get(ty.0 as usize)
        .copied()
        .unwrap_or(lir::TypeClass::Other)
}

/// Whether `name` names a declared `resource` (the native BUG-004 gate:
/// only declared resources are addressable; everything else is
/// UnsupportedAddrOf).
fn find_resource_decl(m: &ModuleAst, src: &[u8], name: &[u8]) -> bool {
    m.decls.iter().any(|d| {
        d.kind == DeclKind::Resource
            && src
                .get(d.name.start..d.name.end)
                .map(|b| b == name)
                .unwrap_or(false)
    })
}

fn emit_module_sections(
    backend: &mut WasmBackend<'_>,
    import_count: u32,
    funcs: Vec<Func>,
) -> Result<(), CodegenError> {
    // ---- assemble the module (section order is normative) ----
    let main_idx = funcs
        .iter()
        .position(|f| f.name == backend.entry_name)
        .ok_or(CodegenError::MissingEntryPoint {
            name: backend.entry_name,
        })?;
    let (main_in, main_out) = {
        let t = funcs[main_idx].type_index as usize;
        let (p, r) = backend.types[t].clone();
        (p.len(), r.len())
    };
    // The wrapper: () → i32 — the exit code (main's TOP result; the
    // driver's entry gate enforces `main ( -- i64 )`, the drop chain
    // keeps the wrapper total regardless). Zeros feed main's inputs.
    let wrapper_type = backend.type_index(&[], &[VAL_I32]);
    let mut wrap_body = Vec::new();
    uleb(&mut wrap_body, 0); // no local groups
    for _ in 0..main_in {
        wrap_body.push(OP_I64_CONST);
        sleb(&mut wrap_body, 0);
    }
    wrap_body.push(OP_CALL);
    let mut enc = Vec::new();
    padded_uleb(
        &mut enc,
        u64::try_from(main_idx).unwrap_or(0) + u64::from(import_count),
        CALL_LEB_WIDTH,
    );
    wrap_body.extend_from_slice(&enc);
    if main_out == 0 {
        wrap_body.push(OP_I32_CONST);
        sleb(&mut wrap_body, 0);
    } else {
        wrap_body.extend(core::iter::repeat_n(OP_DROP, main_out - 1)); // keep only the TOP result
        wrap_body.push(OP_I32_WRAP_I64);
    }
    wrap_body.push(OP_END);

    // Pre-register import types BEFORE emitting the type section so
    // that import type indices are valid when the import section emits.
    let sig_1_0 = backend.type_index(&[VAL_I64], &[]);
    let sig_3_0 = backend.type_index(&[VAL_I64, VAL_I32, VAL_I32], &[]);
    let (sig_periph_read, sig_periph_write) = if backend.uses_mmio {
        (
            Some(backend.type_index(&[VAL_I32, VAL_I32, VAL_I32], &[VAL_I64])),
            Some(backend.type_index(&[VAL_I32, VAL_I32, VAL_I32, VAL_I64], &[])),
        )
    } else {
        (None, None)
    };
    let sig_barrier = backend
        .uses_barrier
        .then(|| backend.type_index(&[VAL_I32, VAL_I32], &[]));

    let mut bin = Vec::new();
    bin.extend_from_slice(b"\0asm");
    bin.extend_from_slice(&1u32.to_le_bytes());

    // 1. Type.
    section(&mut bin, SEC_TYPE, |b| {
        uleb(b, backend.types.len() as u64);
        for (p, r) in &backend.types {
            functype(b, p, r);
        }
    });

    // 2. Imports: testio, trap, optional exit, and optional MMIO bridge.
    section(&mut bin, SEC_IMPORT, |b| {
        let n = 2
            + usize::from(backend.uses_exit)
            + 2 * usize::from(backend.uses_mmio)
            + usize::from(backend.uses_barrier);
        uleb(b, n as u64);
        for (ns, nm, t) in [
            (&b"tyu"[..], &b"write_byte"[..], sig_1_0),
            (&b"tyu"[..], &b"trap"[..], sig_3_0),
        ] {
            name(b, ns);
            name(b, nm);
            b.push(0x00);
            uleb(b, t as u64);
        }
        if backend.uses_exit {
            name(b, b"tyu");
            name(b, b"exit");
            b.push(0x00);
            uleb(b, sig_1_0 as u64);
        }
        if backend.uses_mmio {
            for (nm, ty) in [
                (
                    &b"periph_read"[..],
                    sig_periph_read.expect("MMIO import type"),
                ),
                (
                    &b"periph_write"[..],
                    sig_periph_write.expect("MMIO import type"),
                ),
            ] {
                name(b, b"tyu");
                name(b, nm);
                b.push(0x00);
                uleb(b, ty as u64);
            }
        }
        if backend.uses_barrier {
            name(b, b"tyu");
            name(b, b"barrier");
            b.push(0x00);
            uleb(b, sig_barrier.expect("barrier import type") as u64);
        }
    });

    // 3. Function (defined funcs + the wrapper).
    section(&mut bin, SEC_FUNCTION, |b| {
        uleb(b, funcs.len() as u64 + 1);
        for f in &funcs {
            uleb(b, f.type_index as u64);
        }
        uleb(b, u64::from(wrapper_type));
    });

    // 4. Memory (§6.6: initial 16, max 256).
    section(&mut bin, SEC_MEMORY, |b| {
        uleb(b, 1);
        uleb(b, 1); // limits flag: min AND max
        uleb(b, regions::MEMORY_MIN_PAGES);
        uleb(b, regions::MEMORY_MAX_PAGES);
    });

    // 5. Global: the mutable region-arena cursor = ARENA_BASE (§6.6).
    section(&mut bin, SEC_GLOBAL, |b| {
        uleb(b, 1);
        b.push(VAL_I64);
        b.push(0x01); // mutable
        b.push(OP_I64_CONST);
        sleb(b, regions::ARENA_BASE);
        b.push(OP_END);
    });

    // 6. Export: tyu_main (the wrapper, appended last) + memory.
    let wrapper_idx = u64::from(import_count) + funcs.len() as u64;
    section(&mut bin, SEC_EXPORT, |b| {
        uleb(b, 2);
        name(b, b"tyu_main");
        b.push(0x00);
        uleb(b, wrapper_idx);
        name(b, b"memory");
        b.push(0x02);
        uleb(b, 0);
    });

    // 7. Code: the word bodies + the used helpers + the wrapper. Built
    // by hand (not `section`) so each body's MODULE-ABSOLUTE offset is
    // known — the `tyu.spans/1` records are absolute code offsets.
    let mut code_payload = Vec::new();
    uleb(&mut code_payload, funcs.len() as u64 + 1);
    let mut func_offsets = Vec::with_capacity(funcs.len() + 1);
    for f in &funcs {
        uleb(&mut code_payload, f.body.len() as u64);
        func_offsets.push(code_payload.len());
        code_payload.extend_from_slice(&f.body);
    }
    uleb(&mut code_payload, wrap_body.len() as u64);
    func_offsets.push(code_payload.len());
    code_payload.extend_from_slice(&wrap_body);

    bin.push(SEC_CODE);
    uleb(&mut bin, code_payload.len() as u64);
    let code_payload_base = bin.len();
    bin.extend_from_slice(&code_payload);

    // 8. Data: the interned strings (one active segment), at their
    // fixed §6.6 addresses.
    if !backend.string_bytes.is_empty() {
        let bytes = core::mem::take(&mut backend.string_bytes);
        section(&mut bin, SEC_DATA, |b| {
            uleb(b, 1);
            uleb(b, 0); // active, memory 0
            b.push(OP_I32_CONST);
            sleb(b, regions::STRINGS_BASE);
            b.push(OP_END);
            uleb(b, bytes.len() as u64);
            b.extend_from_slice(&bytes);
        });
    }

    // 9. The tyu.spans/1 side table (DEC-40): version-first, records
    // for every word-body load/store (the hardware-trap class), with
    // body offsets resolved to module-absolute code offsets. Always
    // emitted (a version header minimum) so a consumer can
    // version-check even a trap-free module.
    let records: Vec<spans::SpanRecord> = backend
        .trap_sites
        .iter()
        .map(|site| spans::SpanRecord {
            code_offset: (code_payload_base + func_offsets[site.func] + site.body_offset) as u32,
            span_start: site.span.start as u32,
            span_len: (site.span.end - site.span.start) as u32,
        })
        .collect();
    spans::section(&records, &mut bin);

    backend.out.write(&bin);
    Ok(())
}

impl CodegenBackend for WasmBackend<'_> {
    fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        // Types and imports register on demand during streaming — the
        // single-threaded word order makes the section layout deterministic.
        Ok(())
    }

    fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        // The per-word reject + capability scan (FR-3/FR-44).
        for blk in w.blocks.iter() {
            for op in blk.ops.iter() {
                match op.kind {
                    lir::OpKind::Call { name, .. } if name.as_bytes() == b"testio.exit" => {
                        self.uses_exit = true;
                    }
                    lir::OpKind::TaskSpawn { .. }
                    | lir::OpKind::InterruptEnable
                    | lir::OpKind::InterruptDisable => {
                        return Err(CodegenError::UnsupportedConstruct);
                    }
                    lir::OpKind::AddrOf {
                        base: lir::AddrOfBase::Mmio { .. },
                        ..
                    }
                    | lir::OpKind::MmioPlace { .. }
                    | lir::OpKind::MmioVolLoad { .. }
                    | lir::OpKind::MmioVolStore { .. }
                    | lir::OpKind::MmioVolLoadField { .. }
                    | lir::OpKind::MmioVolStoreField { .. } => self.uses_mmio = true,
                    _ => {}
                }
                let barrier = match op.kind {
                    lir::OpKind::MmioVolLoad { barrier, .. }
                    | lir::OpKind::MmioVolStore { barrier, .. }
                    | lir::OpKind::MmioVolLoadField { barrier, .. }
                    | lir::OpKind::MmioVolStoreField { barrier, .. } => barrier,
                    _ => lir::BarrierKind::None,
                };
                self.uses_barrier |= barrier != lir::BarrierKind::None;
            }
        }

        let name = w.name.as_bytes().to_vec();
        let in_len = w.sig.in_len as usize;
        let params = vec![VAL_I64; in_len];
        let results = vec![VAL_I64; w.sig.out_len as usize];
        let type_index = self.type_index(&params, &results);
        let slots = Self::slot_locals(w);
        let func_index = self.funcs.len();

        // Declared locals beyond the params (IR locals are a separate
        // namespace from the data stack — each lives in its own wasm local).
        let mut max_declared = in_len;
        for blk in w.blocks.iter() {
            for op in blk.ops.iter() {
                if let lir::OpKind::LocalGet { slot, .. } | lir::OpKind::LocalSet { slot, .. } =
                    op.kind
                {
                    max_declared = max_declared.max(slot as usize + 1);
                }
            }
        }
        let declared_overflow = max_declared.saturating_sub(in_len);
        // Local GROUP declaration (the leading uleb is the GROUP count, not
        // the local count): the slot pool (i64), the dispatch cursor (i32),
        // a memory/MMIO scratch (i64), then declared-overflow locals (i64).
        // Wasm assigns group locals
        // AFTER the params — the pool aliases params 0..in_len (the initial
        // data stack IS the declared inputs), so the cursor's index is
        // in_len-shifted.
        let cur = in_len + slots; // the i32 dispatch cursor's local index
        let mmio_scratch = Some((cur + 1) as u32);
        let mut hdr = Vec::new();
        let mut groups = 0usize;
        let mut decls = Vec::new();
        if slots > 0 {
            locals_group(&mut decls, slots, VAL_I64);
            groups += 1;
        }
        locals_group(&mut decls, 1, VAL_I32);
        groups += 1;
        locals_group(&mut decls, 1, VAL_I64);
        groups += 1;
        if declared_overflow > 0 {
            locals_group(&mut decls, declared_overflow, VAL_I64);
            groups += 1;
        }
        uleb(&mut hdr, groups as u64);
        hdr.extend_from_slice(&decls);
        let mut body = hdr;

        // For single-block words (the common case), emit straight-line code.
        // Multi-block words use the dispatch ladder (DEC-41 fallback: single-block
        // is the fast path; Br/BrIf are rejected at the REPL boundary for v1 anyway,
        // so this covers the B1 corpus).
        let blocks = w.blocks.len();
        let entry = Self::block_index(w, w.entry).unwrap_or(0);

        // Prologue: initialize $cur to the entry block (always 0 for now since
        // we only handle the entry block).
        body.push(OP_I32_CONST);
        sleb(&mut body, entry as i64);
        body.push(OP_LOCAL_SET);
        uleb(&mut body, cur as u64);

        let mut body_calls = Vec::new();
        let mut body_traps = Vec::new();

        if blocks == 1 {
            // Single-block: straight-line, no dispatch needed.
            // Just emit the entry block's ops.
            let blk = w.blocks.get(entry).expect("entry block in range");
            let mut h = blk.entry_stack.len();
            for op in blk.ops.iter() {
                self.emit_op(
                    w,
                    op,
                    &mut body,
                    &mut h,
                    cur,
                    in_len,
                    slots,
                    mmio_scratch,
                    entry,
                    blocks,
                    &mut body_calls,
                    &mut body_traps,
                )?;
            }
        } else {
            // Multi-block: the full dispatch ladder.
            // Structure: loop $dispatch → [block $cN .. block $c0] → block $hdr →
            // br_table → case 0 code → end $c0 → case 1 code → ... → end $dispatch.
            body.push(OP_LOOP);
            body.push(0x40); // $dispatch
            for _ in (1..blocks).rev() {
                body.push(OP_BLOCK);
                body.push(0x40);
            }
            body.push(OP_BLOCK);
            body.push(0x40); // $c0
            body.push(OP_BLOCK);
            body.push(0x40); // $hdr
            body.push(OP_LOCAL_GET);
            uleb(&mut body, cur as u64);
            body.push(OP_BR_TABLE);
            uleb(&mut body, blocks as u64);
            for k in 0..blocks {
                // At the br_table, depth 0 exits $hdr and falls through to
                // case 0; depth 1 exits $c0 and falls through to case 1.
                uleb(&mut body, k as u64);
            }
            uleb(&mut body, 0); // invalid cursor defaults to case 0
            body.push(OP_END); // $hdr

            for k in 0..blocks {
                if k > 0 {
                    body.push(OP_END); // close $c(k-1)
                }
                let blk = w.blocks.get(k).expect("block index in range");
                let mut h = blk.entry_stack.len();
                for op in blk.ops.iter() {
                    self.emit_op(
                        w,
                        op,
                        &mut body,
                        &mut h,
                        cur,
                        in_len,
                        slots,
                        mmio_scratch,
                        k,
                        blocks,
                        &mut body_calls,
                        &mut body_traps,
                    )?;
                }
            }
            body.push(OP_END); // end $c0
            body.push(OP_BR);
            uleb(&mut body, 0); // re-dispatch at the enclosing loop
            body.push(OP_END); // end $dispatch loop
                               // Every IR block terminates in Br/BrIf/Ret. If the dispatch
                               // ladder nevertheless falls through, the module is malformed
                               // or the cursor is corrupt; keep the function result type
                               // sound and fail closed.
            body.push(OP_UNREACHABLE);
        }

        body.push(OP_END); // function frame

        for (off, span) in body_traps {
            self.trap_sites.push(TrapSite {
                func: func_index,
                body_offset: off,
                span,
            });
        }

        self.funcs.push(Func {
            name,
            type_index,
            body,
            calls: body_calls,
        });
        Ok(())
    }

    fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        // ---- the key→index map + patching ----
        let import_count: u32 = 2
            + u32::from(self.uses_exit)
            + 2 * u32::from(self.uses_mmio)
            + u32::from(self.uses_barrier);
        let mut keys: Vec<(Vec<u8>, u32)> = vec![
            (b"$import:write_byte".to_vec(), IMPORT_WRITE_BYTE),
            (b"$import:trap".to_vec(), IMPORT_TRAP),
        ];
        if self.uses_exit {
            keys.push((b"$import:exit".to_vec(), IMPORT_EXIT));
        }
        if self.uses_mmio {
            let first = 2 + u32::from(self.uses_exit);
            keys.push((b"$import:periph_read".to_vec(), first));
            keys.push((b"$import:periph_write".to_vec(), first + 1));
        }
        if self.uses_barrier {
            let idx = 2 + u32::from(self.uses_exit) + 2 * u32::from(self.uses_mmio);
            keys.push((b"$import:barrier".to_vec(), idx));
        }
        for (i, f) in self.funcs.iter().enumerate() {
            keys.push((f.name.clone(), import_count + i as u32));
        }
        // Used helpers join the function list (fixed HELPER_* order) and
        // the key map; dead helpers never ship.
        let helper_base = import_count + self.funcs.len() as u32;
        let mut funcs = core::mem::take(&mut self.funcs);
        let used: Vec<u32> = [
            regions::HELPER_CREATE,
            regions::HELPER_ALLOC,
            regions::HELPER_RESET,
            regions::HELPER_DESTROY,
        ]
        .iter()
        .copied()
        .filter(|h| self.used_helpers[*h as usize])
        .collect();
        let helpers = regions::emit_region_helpers();
        for (pos, h) in used.iter().enumerate() {
            let (hname, hparams, hresults, hbody) = &helpers[*h as usize];
            debug_assert_eq!(*hname, regions::HELPER_NAME[*h as usize]);
            keys.push((helper_key(*h), helper_base + pos as u32));
            funcs.push(Func {
                name: hname.to_vec(),
                type_index: self.type_index(hparams, hresults),
                body: hbody.clone(),
                calls: Vec::new(),
            });
        }

        // Patch every recorded call operand (fixed-width padded LEB — the
        // overwrite never shifts bytes, so `tyu.spans/1` offsets stay put).
        // The region helpers reference the trap import at index 1 in their
        // own bodies (pre-patched: imports are fixed at emit time).
        for f in &mut funcs {
            for (off, callee) in &f.calls {
                let idx = match callee {
                    Callee::Import(i) => *i,
                    Callee::Exit => IMPORT_EXIT,
                    Callee::PeripheralRead => IMPORT_PERIPH_READ + u32::from(self.uses_exit) - 1,
                    Callee::PeripheralWrite => IMPORT_PERIPH_WRITE + u32::from(self.uses_exit) - 1,
                    Callee::Barrier { kind, after } => {
                        let _ = (kind, after); // Arguments are encoded at the call site.
                        IMPORT_BARRIER + u32::from(self.uses_exit) - 1
                    }
                    Callee::Word(n) => *keys.iter().find(|(k, _)| k == n).map(|(_, i)| i).ok_or(
                        CodegenError::UnsupportedOp {
                            op_name: b"call (unresolved call target on wasm32)",
                        },
                    )?,
                    Callee::Helper(h) => *keys
                        .iter()
                        .find(|(k, _)| k == &helper_key(*h))
                        .map(|(_, i)| i)
                        .expect("used helpers are always registered"),
                };
                let mut enc = Vec::new();
                padded_uleb(&mut enc, u64::from(idx), CALL_LEB_WIDTH);
                f.body[*off..*off + CALL_LEB_WIDTH].copy_from_slice(&enc);
            }
        }

        emit_module_sections(self, import_count, funcs)
    }

    fn emit_extern_word(&mut self, _name: &[u8]) -> Result<(), CodegenError> {
        // wasm: imports are the module's own §6.7 surface — no extrn
        // declarations (the trait default is the no-op; explicit here as
        // the documented choice).
        Ok(())
    }
}

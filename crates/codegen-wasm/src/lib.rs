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
//! - **Rejected at compile** (FR-3/FR-44): TaskSpawn/Interrupt* and the
//!   MMIO family → `CodegenError::UnsupportedConstruct` (E9156-class).
//!   MMIO is B2 scope: the aperture table exists only with a platform
//!   descriptor.
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
    Word(Vec<u8>),
    Helper(u32),
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

    /// Emit one IR op into `out` under the height cursor `h` (the data
    /// stack is locals `$s0..`; `h` is the LIVE height at this point).
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
        k: usize,
        blocks: usize,
        body_calls: &mut Vec<(usize, Callee)>,
        body_traps: &mut Vec<(usize, lir::Span)>,
    ) -> Result<(), CodegenError> {
        // push/pop move values between the wasm operand stack and the slot
        // locals $s0.. (h = the live height).
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
        // A Tyu binary op `( a b -- r )`: `a` sits in the deeper slot and
        // must reach the wasm operand stack FIRST (wasm computes `i1 <op>
        // i2` with i2 on top — the Sub/Cmp order is load-bearing).
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
        /// A patchable call: `OP_CALL` + a fixed-width operand + a record.
        macro_rules! pcall {
            ($callee:expr) => {{
                out.push(OP_CALL);
                let off = out.len();
                out.extend_from_slice(&[0u8; CALL_LEB_WIDTH]);
                body_calls.push((off, $callee));
            }};
        }
        /// Canonicalize the i32 verdict on the operand stack to a 0/1 i64.
        macro_rules! boolify {
            () => {{
                out.push(OP_I64_EQZ);
                out.push(OP_I32_EQZ);
                out.push(OP_I64_EXTEND_I32_U);
            }};
        }
        /// The trailing span arguments of a region-helper call (DEC-40: the
        /// CALL SITE's op span travels with the trap).
        macro_rules! span_args {
            () => {{
                out.push(OP_I32_CONST);
                sleb(out, op.span.start as i64);
                out.push(OP_I32_CONST);
                sleb(out, (op.span.end - op.span.start) as i64);
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
                boolify!();
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
                pop!();
                out.push(OP_LOCAL_SET);
                uleb(out, Self::declared_local(slot, in_len, slots) as u64);
            }
            lir::OpKind::LocalGet { slot, .. } => {
                out.push(OP_LOCAL_GET);
                uleb(out, Self::declared_local(slot, in_len, slots) as u64);
                push!();
            }
            lir::OpKind::Call { name, sig, .. } => {
                let n = name.as_bytes();
                let argc = sig.in_len as usize;
                let out_len = sig.out_len as usize;
                // args: s(h-argc) .. s(h-1) in order (the callee's first
                // parameter is the DEEPEST slot).
                for i in (0..argc).rev() {
                    out.push(OP_LOCAL_GET);
                    uleb(out, (*h - argc + i) as u64);
                }
                *h -= argc;
                let callee = if n == b"testio.write-byte" || n == b"testio.emit-output" {
                    Callee::Import(IMPORT_WRITE_BYTE)
                } else if n == b"testio.exit" {
                    Callee::Import(IMPORT_EXIT)
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
            lir::OpKind::Load { ty: _ } => {
                pop!(); // addr (i64 slot)
                out.push(OP_I32_WRAP_I64); // memory addressing is i32 (§6.7)
                out.push(OP_I64_LOAD);
                memarg(out, 0);
                body_traps.push((out.len() - 2, op.span)); // the load opcode
                push!();
            }
            lir::OpKind::Store { ty: _ } => {
                // stack: [addr, value] → wasm store consumes [addr, value].
                *h -= 2;
                out.push(OP_LOCAL_GET);
                uleb(out, *h as u64);
                out.push(OP_I32_WRAP_I64);
                out.push(OP_LOCAL_GET);
                uleb(out, (*h + 1) as u64);
                out.push(OP_I64_STORE);
                memarg(out, 0);
                body_traps.push((out.len() - 2, op.span)); // the store opcode
            }
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
                let t = Self::block_index(w, target)
                    .ok_or(CodegenError::MalformedIr { detail: 2 })?;
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
                out.push(OP_I32_CONST);
                sleb(out, t as i64);
                out.push(OP_I32_CONST);
                sleb(out, e as i64);
                out.push(OP_SELECT); // i32 selected target
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
            | lir::OpKind::InterruptDisable
            | lir::OpKind::AddrOf {
                base: lir::AddrOfBase::Mmio { .. },
                ..
            }
            | lir::OpKind::MmioPlace { .. }
            | lir::OpKind::MmioVolLoad { .. }
            | lir::OpKind::MmioVolStore { .. }
            | lir::OpKind::MmioVolLoadField { .. }
            | lir::OpKind::MmioVolStoreField { .. } => {
                // Rejected in the per-word scan (FR-3/FR-44); the arm
                // exists for match totality over the IR surface.
                return Err(CodegenError::UnsupportedConstruct);
            }
        }
        Ok(())
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
            && src.get(d.name.start..d.name.end).map(|b| b == name).unwrap_or(false)
    })
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
                    | lir::OpKind::InterruptDisable
                    | lir::OpKind::AddrOf {
                        base: lir::AddrOfBase::Mmio { .. },
                        ..
                    }
                    | lir::OpKind::MmioPlace { .. }
                    | lir::OpKind::MmioVolLoad { .. }
                    | lir::OpKind::MmioVolStore { .. }
                    | lir::OpKind::MmioVolLoadField { .. }
                    | lir::OpKind::MmioVolStoreField { .. } => {
                        return Err(CodegenError::UnsupportedConstruct);
                    }
                    _ => {}
                }
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
        // then the declared-overflow locals (i64). Wasm assigns group locals
        // AFTER the params — the pool aliases params 0..in_len (the initial
        // data stack IS the declared inputs), so the cursor's index is
        // in_len-shifted.
        let cur = in_len + slots; // the i32 dispatch cursor's local index
        let mut hdr = Vec::new();
        let mut groups = 0usize;
        let mut decls = Vec::new();
        if slots > 0 {
            locals_group(&mut decls, slots, VAL_I64);
            groups += 1;
        }
        locals_group(&mut decls, 1, VAL_I32);
        groups += 1;
        if declared_overflow > 0 {
            locals_group(&mut decls, declared_overflow, VAL_I64);
            groups += 1;
        }
        uleb(&mut hdr, groups as u64);
        hdr.extend_from_slice(&decls);
        let mut body = hdr;

        // Prologue: $cur = the entry block.
        body.push(OP_I32_CONST);
        sleb(&mut body, Self::block_index(w, w.entry).unwrap_or(0) as i64);
        body.push(OP_LOCAL_SET);
        uleb(&mut body, cur as u64);

        // The dispatch ladder (the corrected canonical shape):
        //
        //   loop $dispatch            ← Br/BrIf re-dispatch (back edges too)
        //     block $c(n-1) … block $c1, block $c0   (outermost first)
        //       block $hdr
        //         local.get $cur
        //         br_table n entries (value k → depth k+1 = $cK), default 1
        //       end $hdr              ← case 0's code follows, inside $c0
        //       … case 0 code …
        //     end $c0                 ← case 1's code follows, inside $c1
        //     … case k code …         (br $dispatch = `blocks - k` labels out)
        //   end $c(n-1)
        //   unreachable               (statically-unreachable fallthrough —
        //                              keeps (→1) function frames valid)
        //   end $dispatch
        //   end (function)
        //
        // ENDs: hdr + (n-1) in-loop + last case + dispatch + function =
        // n+3 = OPENS. Every IR block terminates (the verifier enforces
        // it): Br/BrIf emitted their re-dispatch, Ret its return.
        let blocks = w.blocks.len();
        body.push(OP_LOOP);
        body.push(0x40); // $dispatch
        for _ in (1..blocks).rev() {
            body.push(OP_BLOCK);
            body.push(0x40); // $cK — c(n-1) … c1, then c0 pushed last
        }
        body.push(OP_BLOCK);
        body.push(0x40); // $c0
        body.push(OP_BLOCK);
        body.push(0x40); // $hdr
        body.push(OP_LOCAL_GET);
        uleb(&mut body, cur as u64);
        body.push(OP_BR_TABLE);
        uleb(&mut body, blocks as u64); // table entry count
        for k in 0..blocks {
            uleb(&mut body, (k + 1) as u64); // value k → $cK (k+1 deep)
        }
        uleb(&mut body, 1); // default: $c0 (cur is engine-set and valid)
        body.push(OP_END); // $hdr

        let mut body_calls = Vec::new();
        let mut body_traps = Vec::new();
        for k in 0..blocks {
            if k > 0 {
                body.push(OP_END); // close $c(k-1) — case k's code follows
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
                    k,
                    blocks,
                    &mut body_calls,
                    &mut body_traps,
                )?;
            }
        }
        body.push(OP_END); // end $c(n-1) — the last case block
        body.push(OP_UNREACHABLE);
        body.push(OP_END); // $dispatch
        body.push(OP_END); // the function frame

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
        let import_count: u32 = if self.uses_exit { 3 } else { 2 };
        let mut keys: Vec<(Vec<u8>, u32)> = vec![
            (b"$import:write_byte".to_vec(), IMPORT_WRITE_BYTE),
            (b"$import:trap".to_vec(), IMPORT_TRAP),
        ];
        if self.uses_exit {
            keys.push((b"$import:exit".to_vec(), IMPORT_EXIT));
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
                    Callee::Word(n) => *keys
                        .iter()
                        .find(|(k, _)| k == n)
                        .map(|(_, i)| i)
                        .ok_or(CodegenError::UnsupportedOp {
                            op_name: b"call (unresolved call target on wasm32)",
                        })?,
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

        // ---- assemble the module (section order is normative) ----
        let main_idx = funcs
            .iter()
            .position(|f| f.name == b"main")
            .ok_or(CodegenError::MissingEntryPoint { name: b"main" })?;
        let (main_in, main_out) = {
            let t = funcs[main_idx].type_index as usize;
            let (p, r) = self.types[t].clone();
            (p.len(), r.len())
        };
        // The wrapper: () → i32 — the exit code (main's TOP result; the
        // driver's entry gate enforces `main ( -- i64 )`, the drop chain
        // keeps the wrapper total regardless). Zeros feed main's inputs.
        let wrapper_type = self.type_index(&[], &[VAL_I32]);
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
        for _ in 0..main_out.saturating_sub(1) {
            wrap_body.push(OP_DROP); // keep only the TOP result
        }
        wrap_body.push(OP_I32_WRAP_I64);
        wrap_body.push(OP_END);

        let mut bin = Vec::new();
        bin.extend_from_slice(b"\0asm");
        bin.extend_from_slice(&1u32.to_le_bytes());

        // 1. Type.
        section(&mut bin, SEC_TYPE, |b| {
            uleb(b, self.types.len() as u64);
            for (p, r) in &self.types {
                functype(b, p, r);
            }
        });

        // 2. Import: tyu.write_byte, tyu.trap, [tyu.exit].
        let sig_1_0 = self.type_index(&[VAL_I64], &[]);
        let sig_3_0 = self.type_index(&[VAL_I64, VAL_I32, VAL_I32], &[]);
        section(&mut bin, SEC_IMPORT, |b| {
            let n = if self.uses_exit { 3 } else { 2 };
            uleb(b, n);
            for (ns, nm, t) in [
                (&b"tyu"[..], &b"write_byte"[..], sig_1_0),
                (&b"tyu"[..], &b"trap"[..], sig_3_0),
            ] {
                name(b, ns);
                name(b, nm);
                b.push(0x00);
                uleb(b, t as u64);
            }
            if self.uses_exit {
                name(b, b"tyu");
                name(b, b"exit");
                b.push(0x00);
                uleb(b, sig_1_0 as u64);
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
        if !self.string_bytes.is_empty() {
            let bytes = core::mem::take(&mut self.string_bytes);
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
        let records: Vec<spans::SpanRecord> = self
            .trap_sites
            .iter()
            .map(|site| spans::SpanRecord {
                code_offset: (code_payload_base + func_offsets[site.func] + site.body_offset)
                    as u32,
                span_start: site.span.start as u32,
                span_len: (site.span.end - site.span.start) as u32,
            })
            .collect();
        spans::section(&records, &mut bin);

        self.out.write(&bin);
        Ok(())
    }

    fn emit_extern_word(&mut self, _name: &[u8]) -> Result<(), CodegenError> {
        // wasm: imports are the module's own §6.7 surface — no extrn
        // declarations (the trait default is the no-op; explicit here as
        // the documented choice).
        Ok(())
    }
}

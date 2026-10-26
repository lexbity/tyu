//! Interpreter-provided builtins (PLAN-WASM-1 §6.3): the words the native
//! runtime provides as assembly are provided here as engines with identical
//! observable behavior on the `testio.*` surface (I-4) and the region ABI.
//!
//! Resolved by `compile.rs` when a `Call` target is not a module word; never
//! approximate — each builtin mirrors the hosted runtime byte-for-byte
//! (`runtime/linux-x86_64-hosted.asm`, `codegen-x86_64/src/region.rs`).

use ir::TrapCode;

use crate::engine::ExecState;
use crate::heap::HeapFault;
use crate::journal::Effect;
use crate::value::{Slot, ValKind};

/// What a builtin signals instead of a normal return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinSignal {
    Exit(i64),
    Trap(TrapCode),
}

/// The builtin word table (name → builtin).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Builtin {
    TestioWriteByte,
    TestioWriteStr,
    TestioExit,
    /// `platform.io.log` — the hosted platform's line-print import. The
    /// hosted/metal runtimes link an assembly implementation; the browser
    /// session's implementation streams the string's bytes to the same
    /// diagnostic channel `testio.write-str` uses (W7: the book's
    /// `platform/linux` labs print in the REPL).
    IoLog,
    RegionCreate,
    RegionAlloc,
    RegionReset,
    RegionDestroy,
}

impl Builtin {
    /// Resolve a *canonical* call name (the sysroot's word names) to a
    /// builtin. Nothing else exists on the interpreter's surface.
    pub fn from_name(name: &[u8]) -> Option<Builtin> {
        match name {
            b"testio.write-byte" => Some(Builtin::TestioWriteByte),
            b"testio.write-str" => Some(Builtin::TestioWriteStr),
            b"testio.exit" => Some(Builtin::TestioExit),
            b"platform.io.log" => Some(Builtin::IoLog),
            b"platform.mem.region-create" => Some(Builtin::RegionCreate),
            b"platform.mem.region-alloc" => Some(Builtin::RegionAlloc),
            b"platform.mem.region-reset" => Some(Builtin::RegionReset),
            b"platform.mem.region-destroy" => Some(Builtin::RegionDestroy),
            _ => None,
        }
    }

    /// The expected value kinds of the builtin's inputs, in pop order (the
    /// L2c argument contract the engine checks before execution — DEC-23).
    /// The internal pops below remain raw-slot reads: the kind contract is
    /// enforced here, once, at the call boundary.
    pub fn arg_kinds(&self) -> &'static [ValKind] {
        match self {
            Builtin::TestioWriteByte => &[ValKind::Int],
            Builtin::TestioWriteStr => &[ValKind::Ptr],
            Builtin::TestioExit => &[ValKind::Int],
            Builtin::IoLog => &[ValKind::Ptr],
            Builtin::RegionCreate => &[ValKind::Int],
            Builtin::RegionAlloc => &[ValKind::Int, ValKind::Int],
            Builtin::RegionReset => &[ValKind::Int],
            Builtin::RegionDestroy => &[ValKind::Int],
        }
    }

    /// L2c for builtins (DEC-23): the top args' kinds must match
    /// [`Builtin::arg_kinds`]. `Err(None)` = underflow (trap 10);
    /// `Err(Some(want))` = the first kind mismatch (trap 20 with detail).
    /// No values are consumed.
    pub fn check_args(&self, st: &ExecState) -> Result<(), Option<ValKind>> {
        let kinds = self.arg_kinds();
        if st.stack.len() < kinds.len() {
            return Err(None);
        }
        let base = st.stack.len() - kinds.len();
        for (i, want) in kinds.iter().enumerate() {
            if st.stack[base + i].val.kind() != *want {
                return Err(Some(*want));
            }
        }
        Ok(())
    }

    /// Execute the builtin against the live state.
    pub fn execute(&self, st: &mut ExecState) -> Result<(), BuiltinSignal> {
        match self {
            Builtin::TestioWriteByte => {
                // ( i64 -- ) write the low byte to the diagnostic channel.
                let v = pop_i64(st)?;
                st.out.push((v & 0xFF) as u8);
                st.record_output(1);
                Ok(())
            }
            Builtin::TestioWriteStr | Builtin::IoLog => {
                // ( str -- ) pointer to `[u64 len][bytes...]`, written whole.
                let ptr = st
                    .stack
                    .pop()
                    .map(|s| s.val.as_u64())
                    .ok_or(BuiltinSignal::Trap(TrapCode::StackOverflow))?;
                let len = st
                    .heap
                    .load(ptr, 8)
                    .map_err(|_| BuiltinSignal::Trap(TrapCode::AssertFail))?;
                let mut written = 0u32;
                for i in 0..len {
                    let b = st
                        .heap
                        .load(ptr.wrapping_add(8 + i), 1)
                        .map_err(|_| BuiltinSignal::Trap(TrapCode::AssertFail))?
                        as u8;
                    st.out.push(b);
                    written += 1;
                }
                st.record_output(written);
                Ok(())
            }
            Builtin::TestioExit => {
                let code = pop_i64(st)?;
                Err(BuiltinSignal::Exit(code))
            }
            Builtin::RegionCreate => {
                // ( usize -- Region )
                let size = pop_u64(st)?;
                match st.heap.region_create(size) {
                    Ok((handle, old_cursor)) => {
                        st.stack.push(Slot::int(ir::TY_I64, handle as i64));
                        st.journal.push(Effect::RegionCreate { handle, old_cursor });
                        Ok(())
                    }
                    Err(fault) => Err(region_fault(fault)),
                }
            }
            Builtin::RegionAlloc => {
                // ( Region usize -- ptr_mut )
                let size = pop_u64(st)?;
                let handle = pop_u64(st)?;
                match st.heap.region_alloc(handle, size) {
                    Ok((ptr, old_off)) => {
                        st.stack.push(Slot::ptr(ir::TY_PTR_MUT, ptr));
                        st.journal.push(Effect::RegionAlloc {
                            handle,
                            old_off: old_off as u32,
                        });
                        Ok(())
                    }
                    Err(fault) => Err(region_fault(fault)),
                }
            }
            Builtin::RegionReset => {
                // ( Region -- )
                let handle = pop_u64(st)?;
                match st.heap.region_reset(handle) {
                    Ok(old_off) => {
                        st.journal.push(Effect::RegionReset {
                            handle,
                            old_off: old_off as u32,
                        });
                        Ok(())
                    }
                    Err(fault) => Err(region_fault(fault)),
                }
            }
            Builtin::RegionDestroy => {
                // ( Region -- )
                let handle = pop_u64(st)?;
                match st.heap.region_destroy(handle) {
                    Ok(saved) => {
                        st.journal.push(Effect::RegionDestroy { handle, saved });
                        Ok(())
                    }
                    Err(fault) => Err(region_fault(fault)),
                }
            }
        }
    }
}

fn region_fault(fault: HeapFault) -> BuiltinSignal {
    BuiltinSignal::Trap(fault.trap())
}

fn pop_i64(st: &mut ExecState) -> Result<i64, BuiltinSignal> {
    st.stack
        .pop()
        .map(|s| s.val.as_i64())
        .ok_or(BuiltinSignal::Trap(TrapCode::StackOverflow))
}

fn pop_u64(st: &mut ExecState) -> Result<u64, BuiltinSignal> {
    st.stack
        .pop()
        .map(|s| s.val.as_u64())
        .ok_or(BuiltinSignal::Trap(TrapCode::StackOverflow))
}

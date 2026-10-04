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
use crate::value::Slot;

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
            b"platform.mem.region-create" => Some(Builtin::RegionCreate),
            b"platform.mem.region-alloc" => Some(Builtin::RegionAlloc),
            b"platform.mem.region-reset" => Some(Builtin::RegionReset),
            b"platform.mem.region-destroy" => Some(Builtin::RegionDestroy),
            _ => None,
        }
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
            Builtin::TestioWriteStr => {
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

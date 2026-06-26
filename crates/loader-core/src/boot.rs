//! Device-loader scaffolding for firmware-resident `.lmod` loading.
//!
//! This module is intentionally small in Phase 0: it provides the device
//! [`LoaderPlatform`] implementation and arena rollback primitives that later
//! boot glue will use around `load_module`.

use crate::error::E_BAD_CONTAINER;
use crate::platform::{LoaderPlatform, Region};

const LOADHEAP_ALIGN: usize = 16;

/// Stub runtime ABI hash used until the dynamic firmware build wires the
/// target-specific value into the device-loader image.
pub const DEVICE_EXPECTED_ABI_HASH: u64 = 0;

/// A snapshot of the load-heap bump cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArenaMark(usize);

/// Firmware-side loader platform backed by a single bump arena.
#[derive(Debug)]
pub struct DevicePlatform {
    heap_start: usize,
    heap_end: usize,
    cursor: usize,
    expected_abi_hash: u64,
}

impl DevicePlatform {
    /// Build a device platform from the linker-provided `.loadheap` symbols.
    pub fn from_linker_symbols() -> Self {
        extern "C" {
            static __lang_loadheap_start: u8;
            static __lang_loadheap_end: u8;
        }

        let heap_start = core::ptr::addr_of!(__lang_loadheap_start) as usize;
        let heap_end = core::ptr::addr_of!(__lang_loadheap_end) as usize;
        Self::new(heap_start, heap_end, DEVICE_EXPECTED_ABI_HASH)
    }

    /// Build a platform over an explicit arena.
    ///
    /// This constructor is used by tests and remains useful for future target
    /// bring-up code that receives the arena bounds from another bootstrap layer.
    pub const fn new(heap_start: usize, heap_end: usize, expected_abi_hash: u64) -> Self {
        Self {
            heap_start,
            heap_end,
            cursor: heap_start,
            expected_abi_hash,
        }
    }

    /// Save the current bump cursor.
    pub fn mark(&self) -> ArenaMark {
        ArenaMark(self.cursor)
    }

    /// Reset the arena to a previously saved mark.
    pub fn reset(&mut self, mark: ArenaMark) {
        debug_assert!(mark.0 >= self.heap_start);
        debug_assert!(mark.0 <= self.heap_end);
        if mark.0 >= self.heap_start && mark.0 <= self.heap_end {
            self.cursor = mark.0;
        }
    }

    /// Current bump cursor, exposed for focused allocator tests.
    #[cfg(test)]
    fn cursor(&self) -> usize {
        self.cursor
    }

    fn alloc_bump(&mut self, len: usize) -> Result<Region, u32> {
        let start = align_up(self.cursor, LOADHEAP_ALIGN).ok_or(E_BAD_CONTAINER)?;
        let end = start.checked_add(len).ok_or(E_BAD_CONTAINER)?;
        let next = align_up(end, LOADHEAP_ALIGN).ok_or(E_BAD_CONTAINER)?;

        if self.heap_end < self.heap_start || next > self.heap_end {
            return Err(E_BAD_CONTAINER);
        }

        self.cursor = next;
        Ok(unsafe { Region::from_raw_parts(start as *mut u8, len) })
    }
}

impl LoaderPlatform for DevicePlatform {
    fn alloc_exec(&mut self, len: usize) -> Result<Region, u32> {
        self.alloc_bump(len)
    }

    fn alloc_ro(&mut self, len: usize) -> Result<Region, u32> {
        self.alloc_bump(len)
    }

    fn alloc_rw(&mut self, len: usize) -> Result<Region, u32> {
        self.alloc_bump(len)
    }

    fn make_exec(&mut self, _region: &mut Region) -> Result<(), u32> {
        Ok(())
    }

    fn release(&mut self, _region: &mut Region) {}

    fn expected_abi_hash(&self) -> u64 {
        self.expected_abi_hash
    }
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two());
    value.checked_add(align - 1).map(|v| v & !(align - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_allocates_aligned_regions() {
        let mut backing = [0u8; 128];
        let start = backing.as_mut_ptr() as usize;
        let end = start + backing.len();
        let mut platform = DevicePlatform::new(start + 1, end, 0xabc);

        let first = platform.alloc_exec(7).unwrap();
        let second = platform.alloc_ro(11).unwrap();

        assert_eq!((first.as_ptr() as usize) % LOADHEAP_ALIGN, 0);
        assert_eq!((second.as_ptr() as usize) % LOADHEAP_ALIGN, 0);
        assert_eq!(first.len(), 7);
        assert_eq!(second.len(), 11);
        assert_eq!(platform.expected_abi_hash(), 0xabc);
    }

    #[test]
    fn mark_and_reset_rewinds_the_cursor() {
        let mut backing = [0u8; 128];
        let start = backing.as_mut_ptr() as usize;
        let end = start + backing.len();
        let mut platform = DevicePlatform::new(start, end, 0);

        let mark = platform.mark();
        let first = platform.alloc_rw(24).unwrap();
        assert!(platform.cursor() > start);

        platform.reset(mark);
        let second = platform.alloc_rw(24).unwrap();

        assert_eq!(first.as_ptr(), second.as_ptr());
    }

    #[test]
    fn arena_exhaustion_returns_error_without_overrun() {
        let mut backing = [0u8; 32];
        let start = backing.as_mut_ptr() as usize;
        let end = start + backing.len();
        let mut platform = DevicePlatform::new(start, end, 0);

        let mark = platform.mark();
        let err = platform.alloc_exec(usize::MAX).unwrap_err();

        assert_eq!(err, E_BAD_CONTAINER);
        assert_eq!(platform.mark(), mark);
    }

    #[test]
    fn make_exec_and_release_are_defined_noops() {
        let mut backing = [0u8; 64];
        let start = backing.as_mut_ptr() as usize;
        let end = start + backing.len();
        let mut platform = DevicePlatform::new(start, end, 0);
        let mut region = platform.alloc_exec(16).unwrap();
        let cursor = platform.cursor();

        platform.make_exec(&mut region).unwrap();
        platform.release(&mut region);

        assert_eq!(platform.cursor(), cursor);
    }
}

//! Device-loader boot glue for firmware-resident `.lmod` loading.

use crate::error::{LoadError, E_BAD_CONTAINER};
use crate::load::{load_module, LoadedSet};
use crate::platform::{LoaderPlatform, Region};
use crate::symbols::SymMap;
use lmod::validate::Container;

const LOADHEAP_ALIGN: usize = 16;
const MAIN_HASH: u64 = 0x1f5962a2ce9803c8;

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
        Self::new(heap_start, heap_end, device_expected_abi_hash())
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

fn device_expected_abi_hash() -> u64 {
    let (arch_tag, slot_bytes, word_bits) = device_abi_geometry();
    lmod::abi_hash::compute_abi_hash(arch_tag, slot_bytes, word_bits, lmod::modinfo::MODINFO_VER)
}

#[cfg(target_arch = "x86_64")]
fn device_abi_geometry() -> (u8, u8, u8) {
    (lmod::abi_hash::ARCH_TAG_X86_64, 8, 64)
}

#[cfg(target_arch = "arm")]
fn device_abi_geometry() -> (u8, u8, u8) {
    (lmod::abi_hash::ARCH_TAG_ARM, 4, 32)
}

#[cfg(target_arch = "riscv32")]
fn device_abi_geometry() -> (u8, u8, u8) {
    (lmod::abi_hash::ARCH_TAG_RISCV, 4, 32)
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

#[no_mangle]
pub extern "C" fn __lang_load_and_run() -> ! {
    let mut platform = DevicePlatform::from_linker_symbols();
    let mark = platform.mark();
    let mut symmap: SymMap<'_, 256> = SymMap::new();
    let mut loaded_set = LoadedSet::<64>::new();

    if register_firmware_symtab(&mut symmap, &mut platform).is_err() {
        trap(E_BAD_CONTAINER);
    }

    let lmod = match acquire_modpack_module() {
        Some(bytes) => bytes,
        None => trap(E_BAD_CONTAINER),
    };
    let container = match Container::parse(lmod) {
        Ok(container) => container,
        Err(_) => trap(E_BAD_CONTAINER),
    };

    match load_module(&container, &mut platform, &mut symmap, &mut loaded_set) {
        Ok(_) => {
            let Some(main) = symmap.lookup_by_hash(MAIN_HASH) else {
                trap(LoadError::SymbolUnresolved.code());
            };
            let code = unsafe { __lang_call_loaded_main(main.addr) };
            unsafe { __lang_exit_code(code) }
        }
        Err(err) => {
            platform.reset(mark);
            trap(err.code());
        }
    }
}

fn register_firmware_symtab(
    symmap: &mut SymMap<'_, 256>,
    platform: &mut DevicePlatform,
) -> Result<usize, u32> {
    extern "C" {
        static __lang_symtab_start: u8;
        static __lang_symtab_end: u8;
    }

    let start = core::ptr::addr_of!(__lang_symtab_start) as usize;
    let end = core::ptr::addr_of!(__lang_symtab_end) as usize;
    if end < start {
        return Err(E_BAD_CONTAINER);
    }
    let bytes = unsafe { core::slice::from_raw_parts(start as *const u8, end - start) };
    register_firmware_symtab_bytes(symmap, platform, bytes)
}

fn register_firmware_symtab_bytes(
    symmap: &mut SymMap<'_, 256>,
    platform: &mut DevicePlatform,
    bytes: &[u8],
) -> Result<usize, u32> {
    if bytes.len() < 8 {
        return Err(E_BAD_CONTAINER);
    }
    let count = u32::from_le_bytes(bytes[0..4].try_into().map_err(|_| E_BAD_CONTAINER)?) as usize;
    let expected_len = 8usize
        .checked_add(count.checked_mul(16).ok_or(E_BAD_CONTAINER)?)
        .ok_or(E_BAD_CONTAINER)?;
    if bytes.len() < expected_len {
        return Err(E_BAD_CONTAINER);
    }

    for i in 0..count {
        let off = 8 + i * 16;
        let hash = u64::from_le_bytes(
            bytes[off..off + 8]
                .try_into()
                .map_err(|_| E_BAD_CONTAINER)?,
        );
        let addr = u64::from_le_bytes(
            bytes[off + 8..off + 16]
                .try_into()
                .map_err(|_| E_BAD_CONTAINER)?,
        ) as usize;
        let registered_addr = maybe_veneer(platform, hash, addr)?;
        symmap.register_runtime_hash(hash, registered_addr)?;
    }

    Ok(count)
}

// A loaded module lives in `.loadheap`, megabytes away from the firmware's
// runtime words. Its imported CALLs use range-limited instructions (Thumb `bl`
// ±16 MiB across the FLASH/SRAM split; RISC-V `jal` ±1 MiB), which cannot reach
// the runtime. We route each far CALL target through a small veneer allocated in
// the arena near the module. Data symbols are referenced by absolute/PC-relative
// relocations (not CALLs), so they MUST keep their real address.
//
// NOTE: distinguishing code from data here relies on a hand-maintained data-symbol
// set because `.lang.symtab` carries no kind bit (tracked debt). The proper fix is
// to emit a code/data flag from `lang-symtab-gen` and key the veneer on it.

#[cfg(target_arch = "arm")]
fn maybe_veneer(platform: &mut DevicePlatform, hash: u64, addr: usize) -> Result<usize, u32> {
    // Runtime words live in FLASH (< 0x1000_0000); SRAM symbols are reachable.
    if addr >= 0x1000_0000 || is_runtime_data_symbol(hash) {
        return Ok(addr);
    }

    let mut region = platform.alloc_exec(8)?;
    let veneer = unsafe { region.as_mut_slice() };
    let target = (addr | 1) as u32;
    veneer[0..2].copy_from_slice(&0x4b00u16.to_le_bytes()); // ldr r3, [pc, #0]
    veneer[2..4].copy_from_slice(&0x4718u16.to_le_bytes()); // bx r3
    veneer[4..8].copy_from_slice(&target.to_le_bytes());
    platform.make_exec(&mut region)?;
    Ok((region.as_ptr() as usize) | 1)
}

#[cfg(target_arch = "riscv32")]
fn maybe_veneer(platform: &mut DevicePlatform, hash: u64, addr: usize) -> Result<usize, u32> {
    // Data symbols are reached via R_RISCV_32 / PCREL; keep their real address.
    if is_runtime_data_symbol(hash) {
        return Ok(addr);
    }

    // 8-byte veneer: `lui t1, hi20 ; jalr x0, lo12(t1)` — an absolute jump with
    // ±2 GiB reach that leaves `ra` untouched, so the runtime word returns to the
    // module. `t1` is a caller-saved temporary, free to clobber across a call.
    let mut region = platform.alloc_exec(8)?;
    let veneer = unsafe { region.as_mut_slice() };
    let target = addr as u32;
    let lo12 = target & 0xfff;
    // %hi/%lo split: round hi up when lo12 is negative as a signed 12-bit value.
    let hi20 = (if lo12 >= 0x800 {
        target.wrapping_add(0x1000)
    } else {
        target
    } >> 12)
        & 0xf_ffff;
    let lui = (hi20 << 12) | (6 << 7) | 0x37; // lui t1 (x6)
    let jalr = (lo12 << 20) | (6 << 15) | 0x67; // jalr x0, lo12(t1)
    veneer[0..4].copy_from_slice(&lui.to_le_bytes());
    veneer[4..8].copy_from_slice(&jalr.to_le_bytes());
    platform.make_exec(&mut region)?;
    Ok(region.as_ptr() as usize)
}

#[cfg(not(any(target_arch = "arm", target_arch = "riscv32")))]
fn maybe_veneer(_platform: &mut DevicePlatform, _hash: u64, addr: usize) -> Result<usize, u32> {
    Ok(addr)
}

/// Runtime exports that are *data*, not callable code — referenced by absolute or
/// PC-relative relocations rather than CALLs, so they are never veneered.
#[cfg(any(target_arch = "arm", target_arch = "riscv32"))]
fn is_runtime_data_symbol(hash: u64) -> bool {
    matches!(
        hash,
        h if h == lmod::hash::fnv1a_u64(b"__lang_ds_base")
            || h == lmod::hash::fnv1a_u64(b"__lang_ds_limit")
            || h == lmod::hash::fnv1a_u64(b"__lang_ds_high")
            || h == lmod::hash::fnv1a_u64(b"__lang_stack_limit")
            || h == lmod::hash::fnv1a_u64(b"__lang_expected_abi_hash")
            || h == lmod::hash::fnv1a_u64(b"__lang_v_emitted")
            || h == lmod::hash::fnv1a_u64(b"__lang_gpio_state")
            || h == lmod::hash::fnv1a_u64(b"__lang_time_counter")
    )
}

fn acquire_modpack_module() -> Option<&'static [u8]> {
    extern "C" {
        static __lang_modpack_start: u8;
        static __lang_modpack_end: u8;
    }

    let start = core::ptr::addr_of!(__lang_modpack_start) as usize;
    let end = core::ptr::addr_of!(__lang_modpack_end) as usize;
    if end < start || end - start < 4 {
        return None;
    }

    let bytes = unsafe { core::slice::from_raw_parts(start as *const u8, end - start) };
    let len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    let payload_start = 4usize;
    let payload_end = payload_start.checked_add(len)?;
    if payload_end > bytes.len() {
        return None;
    }
    Some(&bytes[payload_start..payload_end])
}

fn trap(code: u32) -> ! {
    unsafe { __lang_loader_trap(code as u64) }
}

extern "C" {
    fn __lang_call_loaded_main(addr: usize) -> i64;
    fn __lang_exit_code(code: i64) -> !;
    fn __lang_loader_trap(code: u64) -> !;
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

//! `LoaderPlatform` trait — abstract memory management for loading modules.
//!
//! Each target implements this trait to provide platform-specific memory
//! allocation, protection, and verification primitives.  The target-independent
//! loader algorithm (§9 of module-format-and-loading.md) calls these methods
//! through this trait.

use core::marker::PhantomData;

pub use lmod::board_table::{BoardTable, BoardAperture};

// ---------------------------------------------------------------------------
// Region — a slab of mapped memory
// ---------------------------------------------------------------------------

/// Writable, non-executable region state.
#[derive(Debug)]
pub enum Rw {}

/// Executable, non-writable region state.
#[derive(Debug)]
pub enum Rx {}

/// A contiguous mapped memory region carrying its protection state in the type.
///
/// Created by [`LoaderPlatform::alloc_exec`], `alloc_ro`, or `alloc_rw` as
/// [`Region<Rw>`].  [`LoaderPlatform::make_exec`] consumes a writable code
/// region and returns [`Region<Rx>`].
#[derive(Debug)]
pub struct Region<S> {
    ptr: *mut u8,
    len: usize,
    _state: PhantomData<S>,
}

impl<S> Region<S> {
    /// Create a region from a raw pointer and length.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid, uniquely-owned allocation of `len` bytes.
    pub unsafe fn from_raw_parts(ptr: *mut u8, len: usize) -> Self {
        Self {
            ptr,
            len,
            _state: PhantomData,
        }
    }

    /// The raw pointer.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// The raw mutable pointer for platform protection/release operations.
    ///
    /// This does not imply the memory is writable.  Loader code should use
    /// [`Region<Rw>::as_mut_slice`] for writes.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// The length of the region in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the region is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the region as an immutable byte slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Region<Rw> {
    /// Return the writable region as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Region<Rx> {
    /// Return a callable code pointer at `off` bytes into an executable region.
    ///
    /// Writable regions do not expose this method, so calling module code before
    /// the W^X transition is a type error.
    pub fn entry(&self, off: usize) -> Option<*const u8> {
        if off >= self.len {
            None
        } else {
            Some(unsafe { self.ptr.add(off) as *const u8 })
        }
    }
}

// ---------------------------------------------------------------------------
// Placement policy (S2 Phase 16 — PIC/XIP vs copy-to-RAM)
// ---------------------------------------------------------------------------

/// How a module's sections are placed in memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlacementPolicy {
    /// Copy everything to RAM: code, rodata, data, bss.  W^X is enforced
    /// by flipping code from RW to RX after relocation.  Used on hosted
    /// and bare-metal x86_64 targets.
    CopyToRam,
    /// Execute In Place (XIP): code runs directly from flash; only data
    /// and bss are copied to RAM.  Code must be position-independent (PIC).
    /// Used on Cortex-M and other microcontrollers with unified flash.
    XipFromFlash,
}

// ---------------------------------------------------------------------------
// Trust level
// ---------------------------------------------------------------------------

/// Trust level for a loaded module (module-format-and-loading.md §6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustLevel {
    /// Baked into firmware; whole-image secure boot.
    Zero,
    /// External/updatable; verify per-module signature.
    One,
    /// Genuinely untrusted; re-derive safety claims.
    Two,
}

impl TrustLevel {
    /// Numeric rank: 0 = Zero, 1 = One, 2 = Two.
    pub fn rank(self) -> u32 {
        match self {
            TrustLevel::Zero => 0,
            TrustLevel::One => 1,
            TrustLevel::Two => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// LoaderPlatform trait
// ---------------------------------------------------------------------------

/// Abstract interface that each target's runtime implements.
pub trait LoaderPlatform {
    /// Allocate a region for executable code.
    ///
    /// The memory must be mapped RW initially so the loader can apply
    /// relocations.  The caller will call `make_exec` to flip it to RX.
    fn alloc_exec(&mut self, len: usize) -> Result<Region<Rw>, u32>;

    /// Allocate a read-only data region.
    fn alloc_ro(&mut self, len: usize) -> Result<Region<Rw>, u32>;

    /// Allocate a read-write data region.
    fn alloc_rw(&mut self, len: usize) -> Result<Region<Rw>, u32>;

    /// Flip an exec region from RW to RX (W^X discipline).
    ///
    /// After this call the region is no longer writable but is executable.
    fn make_exec(&mut self, region: Region<Rw>) -> Result<Region<Rx>, u32>;

    /// Release an allocated writable region (undo `alloc_*`).
    ///
    /// Called during rollback to free memory.  Default is a no-op
    /// (memory leak is acceptable for some embedded use cases, but
    /// hosted platforms should implement this with `munmap`).
    fn release_rw(&mut self, _region: Region<Rw>) {}

    /// Release an executable region.
    fn release_rx(&mut self, _region: Region<Rx>) {}

    /// Verify a signature/MAC over the signed region.
    fn verify_sig(&self, signed: &[u8], sig: &[u8]) -> bool;

    /// The `abi_hash` that the runtime expects.
    fn expected_abi_hash(&self) -> u64;

    /// Remaining data-stack budget in slots (for `stack_bound` check).
    fn ds_remaining_slots(&self) -> u32 {
        u32::MAX
    }

    /// The device's board `platform_hash` (P6, decision D-5): the canonical
    /// hash of the compiled descriptor the firmware was built against.
    /// `None` when the runtime embeds no descriptor (a module claiming a
    /// nonzero platform_hash is rejected E5220; an unplatformed module with
    /// hash 0 loads anywhere).
    fn platform_hash(&self) -> Option<u64> {
        None
    }

    /// The board's MMIO aperture table (P6 binding pass, §5.8). Derived from
    /// the compiled descriptor the firmware embeds; empty when none. The
    /// loader matches a module's aperture-use entries against this table by
    /// `name_hash` (stable identity) and resolves bases/sizes from it.
    fn aperture_table(&self) -> &[BoardAperture] {
        &[]
    }

    /// The trust level this platform operates at.
    fn trust_level(&self) -> TrustLevel;

    /// The placement policy for this target.
    ///
    /// Defaults to `CopyToRam` — the only policy currently implemented
    /// in `load_module`.  XIP support is reserved for future Cortex-M
    /// and flash-target enablement.
    fn placement_policy(&self) -> PlacementPolicy {
        PlacementPolicy::CopyToRam
    }

    /// Unwrap a content-encryption key from a wrapped slot.
    ///
    /// `key_id` selects which KEK to use (fleet key = 0, device-specific keys
    /// have unique IDs).  `wrapped` is the 60-byte wrapped CEK.
    /// On success writes the 32-byte CEK into `out_cek`.
    ///
    #[cfg(feature = "encryption")]
    fn unwrap_cek(&self, key_id: u64, wrapped: &[u8], out_cek: &mut [u8; 32]) -> Result<(), u32>;
}

//! `LoaderPlatform` trait — abstract memory management for loading modules.
//!
//! Each target implements this trait to provide platform-specific memory
//! allocation, protection, and verification primitives.  The target-independent
//! loader algorithm (§9 of module-format-and-loading.md) calls these methods
//! through this trait.

// ---------------------------------------------------------------------------
// Region — a slab of mapped memory
// ---------------------------------------------------------------------------

/// A contiguous mapped memory region.
///
/// Created by [`LoaderPlatform::alloc_exec`], `alloc_ro`, or `alloc_rw`.
/// The `as_mut_ptr`/`as_ptr` accessors provide access to the underlying bytes.
#[derive(Debug)]
pub struct Region {
    ptr: *mut u8,
    len: usize,
}

impl Region {
    /// Create a region from a raw pointer and length.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid, uniquely-owned allocation of `len` bytes.
    pub unsafe fn from_raw_parts(ptr: *mut u8, len: usize) -> Self {
        Self { ptr, len }
    }

    /// Return the region as a mutable byte slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is writable at this point.
    pub unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// Return the region as an immutable byte slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// The raw mutable pointer.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// The raw const pointer.
    pub fn as_ptr(&self) -> *const u8 {
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
    fn alloc_exec(&mut self, len: usize) -> Result<Region, u32>;

    /// Allocate a read-only data region.
    fn alloc_ro(&mut self, len: usize) -> Result<Region, u32>;

    /// Allocate a read-write data region.
    fn alloc_rw(&mut self, len: usize) -> Result<Region, u32>;

    /// Flip an exec region from RW to RX (W^X discipline).
    ///
    /// After this call the region is no longer writable but is executable.
    fn make_exec(&mut self, region: &mut Region) -> Result<(), u32>;

    /// Release an allocated region (undo `alloc_*`).
    ///
    /// Called during rollback to free memory.  Default is a no-op
    /// (memory leak is acceptable for some embedded use cases, but
    /// hosted platforms should implement this with `munmap`).
    fn release(&mut self, _region: &mut Region) {}

    /// Verify a signature/MAC over the signed region.
    ///
    /// Default: reject unless overridden by the platform.
    fn verify_sig(&self, _signed: &[u8], _sig: &[u8]) -> bool {
        false
    }

    /// The `abi_hash` that the runtime expects.
    fn expected_abi_hash(&self) -> u64;

    /// Remaining data-stack budget in slots (for `stack_bound` check).
    fn ds_remaining_slots(&self) -> u32 {
        u32::MAX
    }

    /// The trust level this platform operates at.
    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Zero
    }

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
    /// The default implementation returns `Err` — platforms without
    /// encryption support leave this unimplemented.
    #[cfg(feature = "encryption")]
    fn unwrap_cek(
        &self,
        _key_id: u64,
        _wrapped: &[u8],
        _out_cek: &mut [u8; 32],
    ) -> Result<(), u32> {
        Err(crate::load::E_ENC_NO_KEY)
    }
}

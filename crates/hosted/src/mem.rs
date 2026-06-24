use crate::c::c_int;
use crate::{c, errno::Errno};
use core::ffi::c_void;
use core::ptr::NonNull;

pub fn malloc(size: usize) -> Result<NonNull<c_void>, Errno> {
    let ptr = unsafe { c::malloc(size) };
    NonNull::new(ptr).ok_or(Errno(12))
}

/// # Safety
///
/// `ptr` must have been previously allocated by `malloc` or `realloc`.
pub unsafe fn realloc(ptr: NonNull<c_void>, size: usize) -> Result<NonNull<c_void>, Errno> {
    let ptr = unsafe { c::realloc(ptr.as_ptr(), size) };
    NonNull::new(ptr).ok_or(Errno(12))
}

/// # Safety
///
/// `ptr` must have been previously allocated by `malloc` or `realloc`.
pub unsafe fn free(ptr: NonNull<c_void>) {
    unsafe { c::free(ptr.as_ptr()) };
}

// ---------------------------------------------------------------------------
// Memory mapping (S2 Phase 7 — section placement + W^X)
// ---------------------------------------------------------------------------

/// Protection flags for mmap/mprotect.
pub mod prot {
    use super::c_int;
    pub const NONE: c_int = 0;
    pub const READ: c_int = 1;
    pub const WRITE: c_int = 2;
    pub const EXEC: c_int = 4;
}

/// Mapping flags for mmap.
pub mod map {
    use super::c_int;
    pub const SHARED: c_int = 1;
    pub const PRIVATE: c_int = 2;
    pub const ANONYMOUS: c_int = 0x20;
}

/// mmap an anonymous RW region.  Returns a mutable slice of `len` bytes.
///
/// The memory is initially readable and writable.  Call `mprotect` to
/// change protections (e.g. flip to RX for code).
pub fn mmap_anon_rw(len: usize) -> Result<&'static mut [u8], Errno> {
    let ptr = unsafe {
        c::mmap(
            core::ptr::null_mut(),
            len,
            prot::READ | prot::WRITE,
            map::PRIVATE | map::ANONYMOUS,
            -1,
            0,
        )
    };
    if ptr as isize == -1 {
        return Err(Errno(12)); // ENOMEM
    }
    unsafe { Ok(core::slice::from_raw_parts_mut(ptr as *mut u8, len)) }
}

/// mmap an anonymous page with the given protection flags.
pub fn mmap_anon(len: usize, protection: c_int) -> Result<*mut c_void, Errno> {
    let ptr = unsafe {
        c::mmap(
            core::ptr::null_mut(),
            len,
            protection,
            map::PRIVATE | map::ANONYMOUS,
            -1,
            0,
        )
    };
    if ptr as isize == -1 {
        return Err(Errno(12));
    }
    Ok(ptr)
}

/// Change protection on a memory region.
///
/// # Safety
///
/// `addr` must be a valid mmap allocation; `len` must match the original
/// allocation size.
pub unsafe fn mprotect(addr: *mut c_void, len: usize, protection: c_int) -> Result<(), Errno> {
    let ret = unsafe { c::mprotect(addr, len, protection) };
    if ret != 0 {
        return Err(Errno(12));
    }
    Ok(())
}

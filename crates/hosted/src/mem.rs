use crate::{c, errno::Errno};
use core::{ffi::c_void, ptr::NonNull};

pub fn malloc(size: usize) -> Result<NonNull<c_void>, Errno> {
    let ptr = unsafe { c::malloc(size) };
    NonNull::new(ptr).ok_or(Errno(12))
}

pub unsafe fn realloc(ptr: NonNull<c_void>, size: usize) -> Result<NonNull<c_void>, Errno> {
    let ptr = c::realloc(ptr.as_ptr(), size);
    NonNull::new(ptr).ok_or(Errno(12))
}

pub unsafe fn free(ptr: NonNull<c_void>) {
    c::free(ptr.as_ptr());
}


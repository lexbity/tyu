use crate::{errno::Errno, mem};
use core::{ffi::c_void, ptr::NonNull};

pub struct CStrBuf {
    ptr: NonNull<u8>,
    len: usize,
}

impl CStrBuf {
    pub fn new(bytes: &[u8]) -> Result<Self, Errno> {
        if bytes.contains(&0) {
            return Err(Errno(22));
        }
        let alloc = mem::malloc(bytes.len() + 1)?;
        let ptr = alloc.cast::<u8>();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
            *ptr.as_ptr().add(bytes.len()) = 0;
        }
        Ok(Self {
            ptr,
            len: bytes.len(),
        })
    }

    pub fn as_ptr_i8(&self) -> *const i8 {
        self.ptr.as_ptr() as *const i8
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for CStrBuf {
    fn drop(&mut self) {
        unsafe { mem::free(self.ptr.cast::<c_void>()) };
    }
}

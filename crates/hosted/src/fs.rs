use crate::{c, cstrbuf::CStrBuf, errno::Errno, mem};
use core::ffi::c_void;
use core::ptr::NonNull;

pub struct ByteBuf {
    ptr: NonNull<u8>,
    len: usize,
    cap: usize,
}

impl ByteBuf {
    pub fn new() -> Result<Self, Errno> {
        let alloc = mem::malloc(1)?;
        Ok(Self {
            ptr: alloc.cast::<u8>(),
            len: 0,
            cap: 1,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    fn reserve_exact(&mut self, additional: usize) -> Result<(), Errno> {
        let needed = self.len.checked_add(additional).ok_or(Errno(12))?;
        if needed <= self.cap {
            return Ok(());
        }
        let mut new_cap = self.cap.max(1);
        while new_cap < needed {
            new_cap = new_cap.saturating_mul(2).max(needed);
        }
        let ptr = unsafe { mem::realloc(self.ptr.cast::<c_void>(), new_cap)? }.cast::<u8>();
        self.ptr = ptr;
        self.cap = new_cap;
        Ok(())
    }

    pub fn push_slice(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        self.reserve_exact(bytes.len())?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ptr.as_ptr().add(self.len),
                bytes.len(),
            );
        }
        self.len += bytes.len();
        Ok(())
    }
}

impl Drop for ByteBuf {
    fn drop(&mut self) {
        unsafe { mem::free(self.ptr.cast::<c_void>()) }
    }
}

pub fn read_file(path: &[u8]) -> Result<ByteBuf, Errno> {
    let path_c = CStrBuf::new(path)?;
    let mode = b"rb\0";

    let f = unsafe { c::fopen(path_c.as_ptr_i8(), mode.as_ptr() as *const i8) };
    if f.is_null() {
        return Err(Errno::last());
    }

    let mut out = ByteBuf::new()?;
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { c::fread(buf.as_mut_ptr() as *mut c_void, 1, buf.len(), f) };
        if n > 0 {
            out.push_slice(&buf[..n])?;
        }
        if n < buf.len() {
            let err = unsafe { c::ferror(f) };
            let eof = unsafe { c::feof(f) };
            let rc = unsafe { c::fclose(f) };
            if err != 0 {
                return Err(Errno::last());
            }
            if eof != 0 {
                if rc != 0 {
                    return Err(Errno::last());
                }
                return Ok(out);
            }
            if rc != 0 {
                return Err(Errno::last());
            }
            return Err(Errno(5));
        }
    }
}

pub fn write_file(path: &[u8], bytes: &[u8]) -> Result<(), Errno> {
    let path_c = CStrBuf::new(path)?;
    let mode = b"wb\0";

    let f = unsafe { c::fopen(path_c.as_ptr_i8(), mode.as_ptr() as *const i8) };
    if f.is_null() {
        return Err(Errno::last());
    }

    let mut off = 0usize;
    while off < bytes.len() {
        let n = unsafe {
            c::fwrite(
                bytes[off..].as_ptr() as *const c_void,
                1,
                bytes.len() - off,
                f,
            )
        };
        if n == 0 {
            let err = unsafe { c::ferror(f) };
            let rc = unsafe { c::fclose(f) };
            if err != 0 {
                return Err(Errno::last());
            }
            if rc != 0 {
                return Err(Errno::last());
            }
            return Err(Errno(5));
        }
        off += n;
    }
    let rc = unsafe { c::fclose(f) };
    if rc != 0 {
        return Err(Errno::last());
    }
    Ok(())
}

use crate::c;

/// # Safety
///
/// `ptr` must point to a valid null-terminated C string.
pub unsafe fn len(mut ptr: *const c::c_char) -> usize {
    let mut n = 0usize;
    while unsafe { *ptr != 0 } {
        n += 1;
        ptr = unsafe { ptr.add(1) };
    }
    n
}

/// # Safety
///
/// `ptr` must point to a valid null-terminated C string.
pub unsafe fn as_bytes<'a>(ptr: *const c::c_char) -> &'a [u8] {
    let n = unsafe { len(ptr) };
    unsafe { core::slice::from_raw_parts(ptr as *const u8, n) }
}

/// # Safety
///
/// `ptr` must point to a valid null-terminated C string whose length is at
/// least `bytes.len()`.
pub unsafe fn eq(ptr: *const c::c_char, bytes: &[u8]) -> bool {
    let mut i = 0usize;
    while i < bytes.len() {
        let c = unsafe { *ptr.add(i) };
        if c == 0 {
            return false;
        }
        if c as u8 != bytes[i] {
            return false;
        }
        i += 1;
    }
    unsafe { *ptr.add(bytes.len()) == 0 }
}

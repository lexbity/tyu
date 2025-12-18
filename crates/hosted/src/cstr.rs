use crate::c;

pub unsafe fn len(mut ptr: *const c::c_char) -> usize {
    let mut n = 0usize;
    while *ptr != 0 {
        n += 1;
        ptr = ptr.add(1);
    }
    n
}

pub unsafe fn as_bytes<'a>(ptr: *const c::c_char) -> &'a [u8] {
    let n = len(ptr);
    core::slice::from_raw_parts(ptr as *const u8, n)
}

pub unsafe fn eq(ptr: *const c::c_char, bytes: &[u8]) -> bool {
    let mut i = 0usize;
    while i < bytes.len() {
        let c = *ptr.add(i);
        if c == 0 {
            return false;
        }
        if c as u8 != bytes[i] {
            return false;
        }
        i += 1;
    }
    *ptr.add(bytes.len()) == 0
}

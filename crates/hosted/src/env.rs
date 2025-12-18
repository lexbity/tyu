use crate::{c, cstr};

pub fn get(name: &[u8]) -> Option<*const c::c_char> {
    let mut buf = [0i8; 128];
    if name.len() + 1 > buf.len() {
        return None;
    }
    for (i, &b) in name.iter().enumerate() {
        buf[i] = b as i8;
    }
    buf[name.len()] = 0;
    let ptr = unsafe { c::getenv(buf.as_ptr()) };
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

pub unsafe fn get_str(name: &[u8]) -> Option<&'static [u8]> {
    let ptr = get(name)?;
    let n = cstr::len(ptr);
    Some(core::slice::from_raw_parts(ptr as *const u8, n))
}


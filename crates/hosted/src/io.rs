use crate::{c, errno::Errno};
use core::ffi::c_void;

pub const STDIN_FD: i32 = 0;
pub const STDOUT_FD: i32 = 1;
pub const STDERR_FD: i32 = 2;

pub fn write_all(fd: i32, mut bytes: &[u8]) -> Result<(), Errno> {
    while !bytes.is_empty() {
        let written = unsafe { c::write(fd, bytes.as_ptr() as *const c_void, bytes.len()) };
        if written < 0 {
            return Err(Errno::last());
        }
        let written = written as usize;
        bytes = &bytes[written..];
    }
    Ok(())
}

pub fn stdout(bytes: &[u8]) -> Result<(), Errno> {
    write_all(STDOUT_FD, bytes)
}

pub fn stderr(bytes: &[u8]) -> Result<(), Errno> {
    write_all(STDERR_FD, bytes)
}

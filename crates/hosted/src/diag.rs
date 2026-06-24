use crate::{errno::Errno, io};

pub fn error_simple(code: u32, msg: &[u8]) -> Result<(), Errno> {
    let _ = io::stderr(b"error[E");
    let mut buf = [0u8; 8];
    let mut n = 0usize;
    let mut v = code;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        buf[..n].reverse();
    }
    let _ = io::stderr(&buf[..n]);
    let _ = io::stderr(b"]: ");
    let _ = io::stderr(msg);
    io::stderr(b"\n")
}

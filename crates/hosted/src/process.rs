use crate::{c, cstrbuf::CStrBuf, errno::Errno};
use core::mem::MaybeUninit;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ExitStatus {
    pub code: i32,
}

pub fn run(prog: &[u8], args: &[&[u8]]) -> Result<ExitStatus, Errno> {
    const MAX_ARGS: usize = 32;
    if args.len() > MAX_ARGS {
        return Err(Errno(7));
    }
    let prog_c = CStrBuf::new(prog)?;

    let mut arg_bufs: [MaybeUninit<CStrBuf>; MAX_ARGS] = [const { MaybeUninit::uninit() }; MAX_ARGS];
    for (i, &a) in args.iter().enumerate() {
        arg_bufs[i].write(CStrBuf::new(a)?);
    }

    let mut argv: [*const c::c_char; MAX_ARGS + 2] = [core::ptr::null(); MAX_ARGS + 2];
    argv[0] = prog_c.as_ptr_i8();
    for i in 0..args.len() {
        let p = unsafe { arg_bufs[i].assume_init_ref() }.as_ptr_i8();
        argv[i + 1] = p;
    }
    argv[args.len() + 1] = core::ptr::null();

    let pid = unsafe { c::fork() };
    if pid < 0 {
        for i in 0..args.len() {
            unsafe { arg_bufs[i].assume_init_drop() };
        }
        return Err(Errno::last());
    }

    if pid == 0 {
        unsafe {
            let _ = c::execvp(prog_c.as_ptr_i8(), argv.as_ptr());
            c::_exit(127);
        }
    }

    let mut status: i32 = 0;
    let waited = unsafe { c::waitpid(pid, &mut status as *mut i32, 0) };
    if waited < 0 {
        for i in 0..args.len() {
            unsafe { arg_bufs[i].assume_init_drop() };
        }
        return Err(Errno::last());
    }

    let code = if status & 0x7f != 0 {
        128 + (status & 0x7f)
    } else {
        (status >> 8) & 0xff
    };

    for i in 0..args.len() {
        unsafe { arg_bufs[i].assume_init_drop() };
    }
    Ok(ExitStatus { code })
}

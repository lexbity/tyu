use crate::{c, cstrbuf::CStrBuf, errno::Errno};
use core::mem::MaybeUninit;

const MAX_ARGS: usize = 32;

/// RAII guard that owns a set of initialized `CStrBuf` values and drops them
/// automatically on destruction. Eliminates the manual-drop hazard across
/// multiple early-return paths in `run`.
struct ArgGuard {
    bufs: [MaybeUninit<CStrBuf>; MAX_ARGS],
    len: usize,
}

impl ArgGuard {
    /// Initialize the guard by converting each byte slice into a `CStrBuf`.
    /// If any conversion fails, the guard drops already-initialized slots
    /// and returns the error.
    fn build(args: &[&[u8]]) -> Result<Self, Errno> {
        let mut g = ArgGuard {
            bufs: [const { MaybeUninit::uninit() }; MAX_ARGS],
            len: 0,
        };
        for &a in args {
            // Safety: slots 0..g.len are uninit; we write before incrementing.
            g.bufs[g.len].write(CStrBuf::new(a)?);
            g.len += 1;
        }
        Ok(g)
    }

    /// Build the `argv` array for `execvp`. The returned array has
    /// `len + 2` entries: `[prog_ptr, arg0_ptr, ..., argN_ptr, null]`.
    fn argv_ptrs(&self, prog_ptr: *const c::c_char) -> [*const c::c_char; MAX_ARGS + 2] {
        let mut argv = [core::ptr::null(); MAX_ARGS + 2];
        argv[0] = prog_ptr;
        for i in 0..self.len {
            // Safety: slots 0..self.len were initialized in `build`.
            argv[i + 1] = unsafe { self.bufs[i].assume_init_ref() }.as_ptr_i8();
        }
        argv
    }
}

impl Drop for ArgGuard {
    fn drop(&mut self) {
        for i in 0..self.len {
            // Safety: slots 0..self.len were initialized in `build`.
            unsafe { self.bufs[i].assume_init_drop() };
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ExitStatus {
    pub code: i32,
}

/// Return the index of the first candidate that is executable, with PATH
/// lookup: names containing `/` are tested directly, bare names are probed
/// in every `PATH` directory via `access(X_OK)`. Candidates like
/// `riscv32-elf-as` vs `riscv64-unknown-elf-as` differ per distro, and the
/// assembler must run the one the system actually ships (S9 audit:
/// ubuntu-26.04 has no `riscv32-elf-as`).
pub fn resolve_executable(candidates: &[&[u8]]) -> Option<usize> {
    for (idx, c) in candidates.iter().enumerate() {
        if c.contains(&b'/') {
            if executable_at(c) {
                return Some(idx);
            }
            continue;
        }
        let path_ptr = crate::env::get(b"PATH")?;
        let mut plen = 0usize;
        unsafe {
            while *path_ptr.add(plen) != 0 {
                plen += 1;
            }
        }
        let path = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, plen) };
        let mut start = 0usize;
        for i in 0..=path.len() {
            if i == path.len() || path[i] == b':' {
                let dir = &path[start..i];
                start = i + 1;
                if dir.is_empty() || dir.len() + 1 + c.len() > 509 {
                    continue;
                }
                let mut probe = [0u8; 512];
                probe[..dir.len()].copy_from_slice(dir);
                let mut n = dir.len();
                if probe[n - 1] != b'/' {
                    probe[n] = b'/';
                    n += 1;
                }
                probe[n..n + c.len()].copy_from_slice(c);
                n += c.len();
                if executable_at(&probe[..n]) {
                    return Some(idx);
                }
            }
        }
    }
    None
}

fn executable_at(path: &[u8]) -> bool {
    let mut buf = [0u8; 512];
    if path.len() >= 511 {
        return false;
    }
    buf[..path.len()].copy_from_slice(path);
    (unsafe { c::access(buf[..path.len()].as_ptr() as *const i8, 1) }) == 0 // X_OK
}

pub fn run(prog: &[u8], args: &[&[u8]]) -> Result<ExitStatus, Errno> {
    if args.len() > MAX_ARGS {
        return Err(Errno(7));
    }
    let prog_c = CStrBuf::new(prog)?;
    let guard = ArgGuard::build(args)?;
    let argv = guard.argv_ptrs(prog_c.as_ptr_i8());

    let pid = unsafe { c::fork() };
    if pid < 0 {
        return Err(Errno::last()); // guard + prog_c dropped here
    }

    if pid == 0 {
        // Child: exec or die. Never return to parent.
        unsafe {
            let _ = c::execvp(prog_c.as_ptr_i8(), argv.as_ptr());
            c::_exit(127);
        }
    }

    let mut status: i32 = 0;
    let waited = unsafe { c::waitpid(pid, &mut status as *mut i32, 0) };
    if waited < 0 {
        return Err(Errno::last()); // guard + prog_c dropped here
    }

    let code = if status & 0x7f != 0 {
        128 + (status & 0x7f)
    } else {
        (status >> 8) & 0xff
    };

    Ok(ExitStatus { code })
    // guard + prog_c dropped here
}

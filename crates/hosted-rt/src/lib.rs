#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Hosted runtime support used by `langc`, `lang-assemble`, and the test
//! harnesses.
//!
//! This crate layers the hosted global allocator and `entry!` macro on top
//! of `hosted`, so the `#![no_std]` host binaries (`langc`, `lang-assemble`)
//! get a working runtime. The no_std runtime *lang items* — `#[panic_handler]`
//! and `rust_eh_personality` — are owned by each binary root behind
//! `cfg(not(test))`, delegating to [`panic`]: a `panic_impl` lang item in a
//! dependency collides with std's in the bin-as-test target that
//! `--all-targets` synthesizes (E0152), and dependency builds cannot see the
//! consumer's `cfg(test)`.

use core::alloc::{GlobalAlloc, Layout};
use hosted::{c, io};

struct HostedAllocator;

unsafe impl GlobalAlloc for HostedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { c::malloc(layout.size() as _) as *mut u8 }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { c::free(ptr as *mut core::ffi::c_void) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, _layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { c::realloc(ptr as *mut core::ffi::c_void, new_size as _) as *mut u8 }
    }
}

#[global_allocator]
static ALLOCATOR: HostedAllocator = HostedAllocator;

/// The panic landing: print and exit 101. A plain function on purpose — the
/// `#[panic_handler]` lang item referencing this lives in each no_std binary
/// root (see the crate docs for why it cannot live here).
pub fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = io::stderr(b"error: panic\n");
    unsafe { c::_exit(101) }
}

/// Terminate the process immediately via `_exit` syscall.
///
/// # Safety
///
/// The caller must ensure that the process is in a state where `_exit` is safe
/// to call (e.g., no outstanding locks or resources that require cleanup).
pub unsafe fn exit(code: i32) -> ! {
    unsafe { c::_exit(code) }
}

#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        pub extern "C" fn main(argc: i32, argv: *const *const hosted::c::c_char) -> i32 {
            $main(argc as isize, argv)
        }
    };
}

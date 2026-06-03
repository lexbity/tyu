#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use core::alloc::{GlobalAlloc, Layout};
use hosted::c;

#[cfg(not(test))]
use core::panic::PanicInfo;
#[cfg(not(test))]
use hosted::io;

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

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

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
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

#![no_std]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use core::ptr::null_mut;

struct DeviceAllocator;

unsafe impl GlobalAlloc for DeviceAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static DEVICE_ALLOCATOR: DeviceAllocator = DeviceAllocator;

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[used]
static KEEP_LOAD_AND_RUN: extern "C" fn() -> ! = loader_core::boot::__lang_load_and_run;

#![no_std]

use hosted::c;
#[cfg(not(test))]
use core::panic::PanicInfo;
#[cfg(not(test))]
use hosted::io;

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = io::stderr(b"error: panic\n");
    unsafe { c::_exit(101) }
}

pub unsafe fn exit(code: i32) -> ! {
    c::_exit(code)
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

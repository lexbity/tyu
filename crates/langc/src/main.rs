#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

hosted_rt::entry!(langc_main);

// The no_std runtime lang items live in the binary root, behind cfg(not(test)):
// under the bin-as-test target `--all-targets` synthesizes, cfg(test) drops
// them and std (linked by the test harness) supplies both — a definition in
// the hosted-rt dependency would collide with std's (E0152), and dependency
// builds cannot see this crate's cfg(test).
#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    hosted_rt::panic(_info)
}

extern "C" fn langc_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    unsafe { langc::run(argc, argv) }
}

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

hosted_rt::entry!(langc_main);

extern "C" fn langc_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    unsafe { langc::run(argc, argv) }
}

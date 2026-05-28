#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

mod assembler;
mod config;
mod driver;

hosted_rt::entry!(assemble_main);

extern "C" fn assemble_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    match unsafe { config::parse_args(argc, argv) } {
        config::ParseResult::Ok(cfg) => driver::run(&cfg),
        config::ParseResult::Help    => 0,
        config::ParseResult::Error => 2,
    }
}

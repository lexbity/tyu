#![no_std]
#![no_main]

use hosted::diag;

mod config;
mod driver;

hosted_rt::entry!(assemble_main);

extern "C" fn assemble_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    let result = unsafe { config::parse_args(argc, argv) };

    match result {
        config::ParseResult::Ok(cfg) => driver::run(&cfg),
        config::ParseResult::Help => 0,
        config::ParseResult::Error(code) => {
            if code == 2001 {
                let _ = diag::error_simple(2001, b"missing input .asm file");
                return 2;
            }
            // For code 2 (usage error), help/error was likely already printed or implicit
            if code == 2 {
                 return 2;
            }
            code
        }
    }
}
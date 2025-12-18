#![no_std]
#![no_main]

use hosted::{args::RawArgs, cstr, diag, io};

hosted_rt::entry!(assemble_main);

extern "C" fn assemble_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    let args = unsafe { RawArgs::new(argc, argv) };

    let mut saw_help = false;
    for (i, a) in args.iter().enumerate() {
        if i == 0 {
            continue;
        }
        unsafe {
            if cstr::eq(a, b"--help") || cstr::eq(a, b"-h") {
                saw_help = true;
            }
        }
    }

    if saw_help || args.len() <= 1 {
        let _ = io::stdout(HELP);
        return if saw_help { 0 } else { 2 };
    }

    let _ = diag::error_simple(2001, b"not implemented: assembler/linker driver");
    2
}

const HELP: &[u8] = b"lang-assemble (tyu_lang) v0.1.0\n\nUSAGE:\n  lang-assemble [options] <file.asm>\n\nOPTIONS:\n  --help, -h          Print help\n+  --out=<path>        Output executable path (stub)\n+  --fasm=<path>       Path to fasm (stub)\n+  --ld=<path>         Path to ld/cc (stub)\n+\n";

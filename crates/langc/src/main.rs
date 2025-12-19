#![no_std]
#![no_main]

use frontend::parse::Parser;
use hosted::{diag, fs};

mod args;
mod codegen;
mod driver;
mod iface;
mod util;

use crate::util::{emit_parse_error, split_dir, Stdout};
use crate::iface::{check_program, iface_error_message};

hosted_rt::entry!(langc_main);

extern "C" fn langc_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    let cfg = match unsafe { args::parse_args(argc, argv) } {
        args::ParseResult::Ok(c) => c,
        args::ParseResult::Help => return 0,
        args::ParseResult::Error(code) => return code,
    };

    let buf = match fs::read_file(cfg.input) {
        Ok(b) => b,
        Err(_) => {
            let _ = diag::error_simple(1003, b"failed to read input file");
            return 2;
        }
    };

    let src = buf.as_slice();
    let module = match Parser::new(src).parse_module_ast() {
        Ok(m) => m,
        Err(e) => {
            emit_parse_error(cfg.input, src, e.code, e.span.start);
            return 2;
        }
    };

    let mut base_dir_buf = [0u8; 512];
    let base_dir = split_dir(cfg.input, &mut base_dir_buf);
    let mut search_dirs: [&[u8]; 10] = [&[]; 10];
    search_dirs[0] = base_dir;
    for j in 0..cfg.include_len {
        search_dirs[j + 1] = cfg.include_dirs[j];
    }
    let mut search_len = 1 + cfg.include_len;

    let sysroot = cfg.sysroot.or_else(|| unsafe { hosted::env::get_str(b"LANG_SYSROOT") });
    if let Some(sr) = sysroot {
        if search_len < search_dirs.len() {
            search_dirs[search_len] = sr;
            search_len += 1;
        }
    }

    if let Err(code) = check_program(&module, src, &search_dirs[..search_len]) {
        let _ = diag::error_simple(code, iface_error_message(code));
        return 2;
    }

    let mut out = Stdout;
    if cfg.emit_ast {
        return match Parser::new(src).parse_module_dump(&mut out) {
            Ok(()) => 0,
            Err(e) => {
                emit_parse_error(cfg.input, src, e.code, e.span.start);
                2
            }
        };
    }

    if cfg.emit_ir {
        return driver::emit_ir_driver(&module, src, &search_dirs[..search_len], cfg.checks, cfg.allow_raw_casts, &mut out);
    }
    if cfg.emit_tc {
        return driver::emit_tc_driver(&module, src, &search_dirs[..search_len], cfg.checks, cfg.allow_raw_casts, &mut out);
    }
    if cfg.emit_obj {
        let out_dir = cfg.out_dir.unwrap_or(b".");
        let target = cfg.target.unwrap_or(b"linux-x86_64-hosted");
        return driver::emit_obj_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            cfg.debug_trap_loc,
            out_dir,
            target,
        );
    }
    driver::emit_asm_driver(
        &module,
        src,
        &search_dirs[..search_len],
        cfg.checks,
        cfg.allow_raw_casts,
        cfg.debug_trap_loc,
        &mut out,
    )
}
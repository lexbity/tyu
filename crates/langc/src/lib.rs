#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Single-module Tyu compiler CLI.
//!
//! The crate exposes the compiler driver and CLI-facing helpers in `args`,
//! `driver`, `iface`, `util`, and `main`.

extern crate alloc;

pub mod args;
mod codegen;
pub mod driver;
pub mod iface;
pub mod util;

use codegen_core::{EmitMode, Target};
use frontend::parse::Parser;
use hosted::{diag, fs};

use crate::iface::{check_program, iface_error_message};
use crate::util::{emit_parse_error, join_path, split_dir, Stdout};

pub unsafe fn run(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
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
            emit_parse_error(cfg.input, src, e.code(), e.span().start);
            return 2;
        }
    };

    let target = cfg.target.unwrap_or(Target::X86_64UnknownLinuxGnu);

    let mut base_dir_buf = [0u8; 512];
    let base_dir = split_dir(cfg.input, &mut base_dir_buf);
    let mut search_dirs: [&[u8]; 11] = [&[]; 11];
    search_dirs[0] = base_dir;
    search_dirs[1..(cfg.include_len + 1)].copy_from_slice(&cfg.include_dirs[..cfg.include_len]);
    let mut search_len = 1 + cfg.include_len;

    let sysroot = cfg
        .sysroot
        .or_else(|| unsafe { hosted::env::get_str(b"LANG_SYSROOT") });
    let mut sysroot_target_buf = [0u8; 512];
    if let Some(sr) = sysroot {
        if search_len < search_dirs.len() {
            search_dirs[search_len] = sr;
            search_len += 1;
        }
        if search_len < search_dirs.len() {
            if let Some(p) = join_path(&mut sysroot_target_buf, sr, target.triple(), b"") {
                search_dirs[search_len] = p;
                search_len += 1;
            }
        }
    }

    if let Err(code) = check_program(&module, src, &search_dirs[..search_len]) {
        let _ = diag::error_simple(code, iface_error_message(code));
        return 2;
    }

    let mut out = Stdout;
    match cfg.emit {
        EmitMode::Ast => match Parser::new(src).parse_module_dump(&mut out) {
            Ok(()) => 0,
            Err(e) => {
                emit_parse_error(cfg.input, src, e.code(), e.span().start);
                2
            }
        },
        EmitMode::Ir => driver::emit_ir_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            target,
            &mut out,
        ),
        EmitMode::StackCheck => driver::emit_tc_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            target,
            &mut out,
        ),
        EmitMode::Asm => driver::emit_asm_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            cfg.debug_trap_loc,
            target,
            cfg.input,
            cfg.features,
            &mut out,
        ),
        EmitMode::Obj => {
            let out_dir = cfg.out_dir.unwrap_or(b".");
            driver::emit_obj_driver(
                &module,
                src,
                &search_dirs[..search_len],
                cfg.checks,
                cfg.allow_raw_casts,
                cfg.debug_trap_loc,
                out_dir,
                target,
                cfg.is_lib,
                cfg.input,
                cfg.features,
            )
        }
    }
}

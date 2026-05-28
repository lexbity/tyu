use hosted::{args::RawArgs, cstr, diag, io};
use semantics::typecheck::ChecksMode;
use codegen_core::{EmitMode, Target};

pub const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h              Print help\n  --emit=ast              Parse and dump AST (inspection)\n  --emit=ir               Typecheck and dump IR (inspection)\n  --emit=tc               Stack-trace typecheck dump (inspection)\n  --emit=asm              Emit assembly text (inspection only, not assemblable standalone)\n  --emit=obj              Emit relocatable object file (production output)\n  --lib                   Compile as a library (no main required, --emit=obj only)\n  -g                      Enable trap-with-location stubs\n  -I <path>               Add include path\n  --checks=off|contracts|all  Checks insertion mode\n  --allow-raw-casts       Enable raw pointer casts\n  --sysroot=<path>        Sysroot root directory\n  --out-dir=<path>        Output directory (--emit=obj)\n  --target=<triple>       Target triple, required for --emit=obj\n                          Supported: x86_64-unknown-linux-gnu\n                                     x86_64-unknown-none\n\n";

/// Validated compiler configuration.
///
/// All fields are well-typed: `emit` is an `EmitMode` enum, `target` is an
/// `Option<Target>` enum. The raw byte strings provided on the command line
/// are parsed and validated by `parse_args` before this struct is constructed.
/// Holding a `Config` means the arguments are structurally valid.
pub struct Config<'a> {
    pub emit: EmitMode,
    pub target: Option<Target>,
    pub debug_trap_loc: bool,
    pub checks: ChecksMode,
    pub allow_raw_casts: bool,
    pub is_lib: bool,
    pub input: &'a [u8],
    pub include_dirs: [&'a [u8]; 8],
    pub include_len: usize,
    pub sysroot: Option<&'a [u8]>,
    pub out_dir: Option<&'a [u8]>,
}

pub enum ParseResult<'a> {
    Ok(Config<'a>),
    Help,
    Error(i32),
}

pub unsafe fn parse_args<'a>(argc: isize, argv: *const *const hosted::c::c_char) -> ParseResult<'a> {
    let args = unsafe { RawArgs::new(argc, argv) };

    let mut saw_help = false;
    let mut emit_ast = false;
    let mut emit_ir = false;
    let mut emit_asm = false;
    let mut emit_obj = false;
    let mut emit_tc = false;
    let mut debug_trap_loc = false;
    let mut checks = ChecksMode::All;
    let mut allow_raw_casts = false;
    let mut is_lib = false;
    let mut input: Option<&[u8]> = None;
    let mut include_dirs: [&[u8]; 8] = [&[]; 8];
    let mut include_len = 0usize;
    let mut sysroot: Option<&[u8]> = None;
    let mut out_dir: Option<&[u8]> = None;
    let mut target: Option<Target> = None;

    let mut i = 1usize;
    while i < args.len() {
        let a = args.get(i).expect("i < args.len() by loop guard");

        if unsafe { cstr::eq(a, b"--help") } || unsafe { cstr::eq(a, b"-h") } {
            saw_help = true;
            i += 1;
            continue;
        }
        let bytes = unsafe { cstr::as_bytes(a) };
        if bytes == b"--emit=ast" {
            emit_ast = true;
            i += 1;
            continue;
        }
        if bytes == b"--emit=ir" {
            emit_ir = true;
            i += 1;
            continue;
        }
        if bytes == b"--emit=tc" {
            emit_tc = true;
            i += 1;
            continue;
        }
        if bytes == b"--emit=asm" {
            emit_asm = true;
            i += 1;
            continue;
        }
        if bytes == b"--emit=obj" {
            emit_obj = true;
            i += 1;
            continue;
        }
        if bytes.starts_with(b"--checks=") {
            checks = match &bytes[b"--checks=".len()..] {
                b"off"       => ChecksMode::Off,
                b"contracts" => ChecksMode::Contracts,
                b"all"       => ChecksMode::All,
                _ => {
                    let _ = diag::error_simple(1006, b"invalid --checks value");
                    return ParseResult::Error(2);
                }
            };
            i += 1;
            continue;
        }
        if bytes == b"--allow-raw-casts" {
            allow_raw_casts = true;
            i += 1;
            continue;
        }
        if bytes == b"--lib" {
            is_lib = true;
            i += 1;
            continue;
        }
        if bytes == b"-g" {
            debug_trap_loc = true;
            i += 1;
            continue;
        }
        if bytes.starts_with(b"--sysroot=") {
            sysroot = Some(&bytes[b"--sysroot=".len()..]);
            i += 1;
            continue;
        }
        if bytes.starts_with(b"--out-dir=") {
            out_dir = Some(&bytes[b"--out-dir=".len()..]);
            i += 1;
            continue;
        }
        if bytes.starts_with(b"--target=") {
            let triple = &bytes[b"--target=".len()..];
            target = match Target::parse(triple) {
                Some(t) => Some(t),
                None => {
                    let _ = diag::error_simple(1019, b"unknown target triple (see --help for supported targets)");
                    return ParseResult::Error(2);
                }
            };
            i += 1;
            continue;
        }
        if bytes == b"-I" {
            if let Some(p) = args.get(i + 1) {
                if include_len < include_dirs.len() {
                    include_dirs[include_len] = unsafe { cstr::as_bytes(p) };
                    include_len += 1;
                }
                i += 2;
                continue;
            } else {
                let _ = diag::error_simple(1004, b"missing argument after -I");
                return ParseResult::Error(2);
            }
        }
        if bytes.starts_with(b"-") {
            i += 1;
            continue;
        }
        if input.is_none() {
            input = Some(bytes);
        }

        i += 1;
    }

    if saw_help || args.len() <= 1 {
        let _ = io::stdout(HELP);
        return if saw_help { ParseResult::Help } else { ParseResult::Error(2) };
    }

    let emit_count = (emit_ast as u8) + (emit_ir as u8) + (emit_asm as u8)
                   + (emit_obj as u8) + (emit_tc as u8);
    if emit_count > 1 {
        let _ = diag::error_simple(1005, b"choose a single --emit=...");
        return ParseResult::Error(2);
    }
    if emit_count == 0 {
        let _ = diag::error_simple(1001, b"use --emit=ast, --emit=ir, --emit=asm, --emit=tc, or --emit=obj");
        return ParseResult::Error(2);
    }

    let emit = if emit_ast      { EmitMode::Ast }
               else if emit_ir  { EmitMode::Ir }
               else if emit_tc  { EmitMode::StackCheck }
               else if emit_obj { EmitMode::Obj }
               else             { EmitMode::Asm };

    // --emit=obj requires an explicit --target triple.
    if emit == EmitMode::Obj && target.is_none() {
        let _ = diag::error_simple(1020, b"--emit=obj requires --target=<triple>");
        return ParseResult::Error(2);
    }

    let Some(input_path) = input else {
        let _ = diag::error_simple(1002, b"missing input file");
        return ParseResult::Error(2);
    };

    ParseResult::Ok(Config {
        emit,
        target,
        debug_trap_loc,
        checks,
        allow_raw_casts,
        is_lib,
        input: input_path,
        include_dirs,
        include_len,
        sysroot,
        out_dir,
    })
}

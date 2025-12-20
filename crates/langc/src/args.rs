use hosted::{args::RawArgs, cstr, diag, io};
use semantics::typecheck::ChecksMode;

pub const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h              Print help\n  --emit=ast              Parse and dump AST (v1 milestone 1)\n  --emit=ir               Typecheck and dump IR (v1 milestone 3)\n  --emit=tc               Legacy stackcheck dump (debug)\n  --emit=asm              Emit FASM ELF64 executable assembly (hosted)\n  --emit=obj              Emit ELF64 relocatable object (v1 milestone 7)\n  -g                      Enable trap-with-location stubs (hosted)\n  -I <path>                Add include path (v1 milestone 2)\n  --checks=off|contracts|all  Checks insertion mode (v1 milestone 4)\n  --allow-raw-casts        Enable raw pointer casts (v1 milestone 3)\n  --sysroot=<path>         Sysroot root (v1 milestone 8)\n  --out-dir=<path>         Output directory (used by --emit=obj)\n  --target=<triple>        Target triple (used by --emit=obj)\n\n";

pub struct Config<'a> {
    pub emit_ast: bool,
    pub emit_ir: bool,
    pub emit_obj: bool,
    pub emit_tc: bool,
    pub debug_trap_loc: bool,
    pub checks: ChecksMode,
    pub allow_raw_casts: bool,
    pub input: &'a [u8],
    pub include_dirs: [&'a [u8]; 8],
    pub include_len: usize,
    pub sysroot: Option<&'a [u8]>,
    pub out_dir: Option<&'a [u8]>,
    pub target: Option<&'a [u8]>,
}

pub enum ParseResult<'a> {
    Ok(Config<'a>),
    Help,
    Error(i32),
}

pub unsafe fn parse_args<'a>(argc: isize, argv: *const *const hosted::c::c_char) -> ParseResult<'a> {
    let args = RawArgs::new(argc, argv);

    let mut saw_help = false;
    let mut emit_ast = false;
    let mut emit_ir = false;
    let mut emit_asm = false;
    let mut emit_obj = false;
    let mut emit_tc = false;
    let mut debug_trap_loc = false;
    let mut checks = ChecksMode::All;
    let mut allow_raw_casts = false;
    let mut input: Option<&[u8]> = None;
    let mut include_dirs: [&[u8]; 8] = [&[]; 8];
    let mut include_len = 0usize;
    let mut sysroot: Option<&[u8]> = None;
    let mut out_dir: Option<&[u8]> = None;
    let mut target: Option<&[u8]> = None;

    let mut i = 1usize;
    while i < args.len() {
        let a = args.get(i).unwrap();
        
        if cstr::eq(a, b"--help") || cstr::eq(a, b"-h") {
            saw_help = true;
            i += 1;
            continue;
        }
        let bytes = cstr::as_bytes(a);
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
                b"off" => ChecksMode::Off,
                b"contracts" => ChecksMode::Contracts,
                b"all" => ChecksMode::All,
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
            target = Some(&bytes[b"--target=".len()..]);
            i += 1;
            continue;
        }
        if bytes == b"-I" {
            if let Some(p) = args.get(i + 1) {
                if include_len < include_dirs.len() {
                    include_dirs[include_len] = cstr::as_bytes(p);
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

    let emit_count = (emit_ast as u8) + (emit_ir as u8) + (emit_asm as u8) + (emit_obj as u8) + (emit_tc as u8);
    if emit_count > 1 {
        let _ = diag::error_simple(1005, b"choose a single --emit=...");
        return ParseResult::Error(2);
    }
    if emit_count == 0 {
        let _ = diag::error_simple(1001, b"use --emit=ast, --emit=ir, --emit=asm, or --emit=obj");
        return ParseResult::Error(2);
    }

    let Some(input_path) = input else {
        let _ = diag::error_simple(1002, b"missing input file");
        return ParseResult::Error(2);
    };

    ParseResult::Ok(Config {
        emit_ast,
        emit_ir,
        emit_obj,
        emit_tc,
        debug_trap_loc,
        checks,
        allow_raw_casts,
        input: input_path,
        include_dirs,
        include_len,
        sysroot,
        out_dir,
        target,
    })
}

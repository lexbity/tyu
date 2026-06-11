use codegen_core::{EmitMode, FeatureSet, Target};
use hosted::{args::RawArgs, cstr, io};
use semantics::typecheck::ChecksMode;

pub const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h                  Print help\n  --emit=ast                  Parse and dump AST (inspection)\n  --emit=ir                   Typecheck and dump IR (inspection)\n  --emit=tc                   Stack-trace typecheck dump (inspection)\n  --emit=asm                  Emit assembly text (inspection only, not assemblable standalone)\n  --emit=obj                  Emit relocatable object file (production output)\n  --lib                       Compile as a library (no main required, --emit=obj only)\n  -g                          Enable trap-with-location stubs\n  -I <path>                   Add include path\n  --checks=off|contracts|all  Checks insertion mode\n  --allow-raw-casts           Enable raw pointer casts\n  --features=<csv>            Image features to enable (default: all) [concurrency, module-loading]\n  --no-default-features       Start from empty feature set\n  --sysroot=<path>            Sysroot root directory\n  --out-dir=<path>            Output directory (--emit=obj)\n  --target=<triple>           Target triple, required for --emit=obj\n                              Supported: x86_64-unknown-linux-gnu\n                                         x86_64-unknown-none\n                                         armv7m-unknown-none\n                                         riscv32-unknown-none\n\n";

/// Validated compiler configuration.
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
    pub features: FeatureSet,
}

pub enum ParseResult<'a> {
    Ok(Config<'a>),
    Help,
    Error(i32),
}

/// Print a diagnostic error to stderr.
fn emit_error(code: u32, msg: &[u8]) {
    let _ = io::stderr(b"error[E");
    let mut buf = [0u8; 12];
    let mut n = 0usize;
    let mut v = code;
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    buf[..n].reverse();
    let _ = io::stderr(&buf[..n]);
    let _ = io::stderr(b"]: ");
    let _ = io::stderr(msg);
    let _ = io::stderr(b"\n");
}

/// Parse arguments from a slice of byte slices.
/// Returns `(result, saw_help_flag)`.
fn parse_args_from_iter<'a>(args: &[&'a [u8]]) -> (ParseResult<'a>, bool) {
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
    let mut features = FeatureSet::all();
    let mut no_default_features = false;

    let mut i = 1usize;
    while i < args.len() {
        let a = args[i];
        if a == b"--help" || a == b"-h" {
            saw_help = true;
            i += 1;
            continue;
        }
        if a == b"--emit=ast" {
            emit_ast = true;
            i += 1;
            continue;
        }
        if a == b"--emit=ir" {
            emit_ir = true;
            i += 1;
            continue;
        }
        if a == b"--emit=tc" {
            emit_tc = true;
            i += 1;
            continue;
        }
        if a == b"--emit=asm" {
            emit_asm = true;
            i += 1;
            continue;
        }
        if a == b"--emit=obj" {
            emit_obj = true;
            i += 1;
            continue;
        }
        if a.starts_with(b"--checks=") {
            checks = match &a[b"--checks=".len()..] {
                b"off" => ChecksMode::Off,
                b"contracts" => ChecksMode::Contracts,
                b"all" => ChecksMode::All,
                _ => {
                    emit_error(1006, b"invalid --checks value");
                    return (ParseResult::Error(2), true);
                }
            };
            i += 1;
            continue;
        }
        if a == b"--allow-raw-casts" {
            allow_raw_casts = true;
            i += 1;
            continue;
        }
        if a == b"--lib" {
            is_lib = true;
            i += 1;
            continue;
        }
        if a == b"-g" {
            debug_trap_loc = true;
            i += 1;
            continue;
        }
        if a == b"--no-default-features" {
            no_default_features = true;
            features = FeatureSet::empty();
            i += 1;
            continue;
        }
        if a.starts_with(b"--features=") {
            let csv = &a[b"--features=".len()..];
            let base = if no_default_features {
                FeatureSet::empty()
            } else {
                FeatureSet::all()
            };
            if csv.is_empty() {
                features = base;
            } else {
                let mut set = base;
                for chunk in csv.split(|&b| b == b',') {
                    let s = core::str::from_utf8(chunk).unwrap_or("");
                    match codegen_core::Feature::parse(s) {
                        Some(f) => set = set.with(f),
                        None => {
                            emit_error(1007, b"unknown --features value");
                            return (ParseResult::Error(2), true);
                        }
                    }
                }
                features = set;
            }
            i += 1;
            continue;
        }
        if a.starts_with(b"--sysroot=") {
            sysroot = Some(&a[b"--sysroot=".len()..]);
            i += 1;
            continue;
        }
        if a.starts_with(b"--out-dir=") {
            out_dir = Some(&a[b"--out-dir=".len()..]);
            i += 1;
            continue;
        }
        if a.starts_with(b"--target=") {
            let triple = &a[b"--target=".len()..];
            target = match Target::parse(triple) {
                Some(t) => Some(t),
                None => {
                    emit_error(
                        1019,
                        b"unknown target triple (see --help for supported targets)",
                    );
                    return (ParseResult::Error(2), true);
                }
            };
            i += 1;
            continue;
        }
        if a == b"-I" {
            if let Some(p) = args.get(i + 1) {
                if include_len < include_dirs.len() {
                    include_dirs[include_len] = p;
                    include_len += 1;
                }
                i += 2;
                continue;
            } else {
                emit_error(1004, b"missing argument after -I");
                return (ParseResult::Error(2), true);
            }
        }
        if a.starts_with(b"-") {
            i += 1;
            continue;
        }
        if input.is_none() {
            input = Some(a);
        }
        i += 1;
    }

    if saw_help || args.len() <= 1 {
        if !saw_help {
            emit_error(1002, b"missing input file");
        }
        return (
            if saw_help {
                ParseResult::Help
            } else {
                ParseResult::Error(2)
            },
            saw_help,
        );
    }

    let emit_count =
        (emit_ast as u8) + (emit_ir as u8) + (emit_asm as u8) + (emit_obj as u8) + (emit_tc as u8);
    if emit_count > 1 {
        emit_error(1005, b"choose a single --emit=...");
        return (ParseResult::Error(2), false);
    }
    if emit_count == 0 {
        emit_error(
            1001,
            b"use --emit=ast, --emit=ir, --emit=asm, --emit=tc, or --emit=obj",
        );
        return (ParseResult::Error(2), false);
    }

    let emit = if emit_ast {
        EmitMode::Ast
    } else if emit_ir {
        EmitMode::Ir
    } else if emit_tc {
        EmitMode::StackCheck
    } else if emit_obj {
        EmitMode::Obj
    } else {
        EmitMode::Asm
    };

    if emit == EmitMode::Obj && target.is_none() {
        emit_error(1020, b"--emit=obj requires --target=<triple>");
        return (ParseResult::Error(2), false);
    }

    let Some(input_path) = input else {
        emit_error(1002, b"missing input file");
        return (ParseResult::Error(2), false);
    };

    (
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
            features,
        }),
        false,
    )
}

pub unsafe fn parse_args<'a>(
    argc: isize,
    argv: *const *const hosted::c::c_char,
) -> ParseResult<'a> {
    let args = unsafe { RawArgs::new(argc, argv) };
    let mut slices: [&[u8]; 256] = [&[]; 256];
    let mut count = 0usize;
    for i in 0..args.len() {
        if count >= slices.len() {
            break;
        }
        let a = args.get(i).expect("i < args.len() by loop guard");
        slices[count] = unsafe { cstr::as_bytes(a) };
        count += 1;
    }
    let (result, saw_help) = parse_args_from_iter(&slices[..count]);
    // Print help if --help was given OR if no actionable args (just the program name).
    if saw_help || count <= 1 {
        let _ = io::stdout(HELP);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(args: &[&[u8]]) -> Config {
        match parse_args_from_iter(args).0 {
            ParseResult::Ok(c) => c,
            ParseResult::Help => panic!("expected Ok, got Help"),
            ParseResult::Error(c) => panic!("expected Ok, got Error({})", c),
        }
    }

    fn err(args: &[&[u8]]) -> i32 {
        match parse_args_from_iter(args).0 {
            ParseResult::Ok(_) => panic!("expected error"),
            ParseResult::Help => panic!("expected error, got Help"),
            ParseResult::Error(c) => c,
        }
    }

    fn is_help(args: &[&[u8]]) -> bool {
        matches!(parse_args_from_iter(args).0, ParseResult::Help)
    }

    #[test]
    fn minimal_input() {
        let cfg = ok(&[b"langc", b"input.mod"]);
        assert_eq!(cfg.emit, EmitMode::Asm);
        assert_eq!(cfg.input, b"input.mod");
    }

    #[test]
    fn help_flag() {
        assert!(is_help(&[b"langc", b"--help"]));
        assert!(is_help(&[b"langc", b"-h"]));
    }

    #[test]
    fn emit_ast() {
        assert_eq!(ok(&[b"langc", b"--emit=ast", b"x.mod"]).emit, EmitMode::Ast);
    }

    #[test]
    fn emit_ir() {
        assert_eq!(ok(&[b"langc", b"--emit=ir", b"x.mod"]).emit, EmitMode::Ir);
    }

    #[test]
    fn emit_tc() {
        assert_eq!(
            ok(&[b"langc", b"--emit=tc", b"x.mod"]).emit,
            EmitMode::StackCheck
        );
    }

    #[test]
    fn emit_asm() {
        assert_eq!(ok(&[b"langc", b"--emit=asm", b"x.mod"]).emit, EmitMode::Asm);
    }

    #[test]
    fn emit_obj() {
        let cfg = ok(&[
            b"langc",
            b"--emit=obj",
            b"--target=x86_64-unknown-linux-gnu",
            b"x.mod",
        ]);
        assert_eq!(cfg.emit, EmitMode::Obj);
        assert_eq!(cfg.target, Some(Target::X86_64UnknownLinuxGnu));
    }

    #[test]
    fn emit_obj_requires_target() {
        err(&[b"langc", b"--emit=obj", b"x.mod"]);
    }

    #[test]
    fn multiple_emit_is_error() {
        err(&[b"langc", b"--emit=ast", b"--emit=ir", b"x.mod"]);
    }

    #[test]
    fn no_emit_is_error() {
        err(&[b"langc", b"x.mod"]);
    }

    #[test]
    fn include_path() {
        let cfg = ok(&[b"langc", b"-I", b"/some/path", b"--emit=ast", b"x.mod"]);
        assert_eq!(cfg.include_len, 1);
        assert_eq!(cfg.include_dirs[0], b"/some/path");
    }

    #[test]
    fn missing_I_arg() {
        err(&[b"langc", b"-I", b"--emit=ast", b"x.mod"]);
    }

    #[test]
    fn checks_modes() {
        let c = ok(&[b"langc", b"--checks=off", b"--emit=ast", b"x.mod"]);
        assert_eq!(c.checks, ChecksMode::Off);
        let c = ok(&[b"langc", b"--checks=contracts", b"--emit=ast", b"x.mod"]);
        assert_eq!(c.checks, ChecksMode::Contracts);
        let c = ok(&[b"langc", b"--checks=all", b"--emit=ast", b"x.mod"]);
        assert_eq!(c.checks, ChecksMode::All);
    }

    #[test]
    fn invalid_checks_value() {
        err(&[b"langc", b"--checks=bogus", b"--emit=ast", b"x.mod"]);
    }

    #[test]
    fn lib_flag() {
        assert!(ok(&[b"langc", b"--lib", b"--emit=ast", b"x.mod"]).is_lib);
    }

    #[test]
    fn debug_trap_flag() {
        assert!(ok(&[b"langc", b"-g", b"--emit=ast", b"x.mod"]).debug_trap_loc);
    }

    #[test]
    fn sysroot() {
        let cfg = ok(&[b"langc", b"--sysroot=/x", b"--emit=ast", b"x.mod"]);
        assert_eq!(cfg.sysroot, Some(b"/x"));
    }

    #[test]
    fn out_dir() {
        let cfg = ok(&[
            b"langc",
            b"--out-dir=/tmp",
            b"--emit=obj",
            b"--target=x86_64-unknown-linux-gnu",
            b"x.mod",
        ]);
        assert_eq!(cfg.out_dir, Some(b"/tmp"));
    }

    #[test]
    fn unknown_target() {
        err(&[b"langc", b"--emit=obj", b"--target=unknown-cpu", b"x.mod"]);
    }

    #[test]
    fn no_input_file() {
        err(&[b"langc", b"--emit=ast"]);
    }

    #[test]
    fn allow_raw_casts() {
        assert!(ok(&[b"langc", b"--allow-raw-casts", b"--emit=ast", b"x.mod"]).allow_raw_casts);
    }

    #[test]
    fn unknown_flag_ignored() {
        ok(&[b"langc", b"--bogus-flag", b"--emit=ast", b"x.mod"]);
    }

    #[test]
    fn features_default_all() {
        let cfg = ok(&[b"langc", b"--emit=ast", b"x.mod"]);
        assert!(cfg.features.contains(codegen_core::Feature::Concurrency));
        assert!(cfg.features.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn features_parse_csv() {
        let cfg = ok(&[
            b"langc",
            b"--features=concurrency",
            b"--emit=ast",
            b"x.mod",
        ]);
        assert!(cfg.features.contains(codegen_core::Feature::Concurrency));
        assert!(!cfg.features.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn features_multiple_csv() {
        let cfg = ok(&[
            b"langc",
            b"--features=concurrency,module-loading",
            b"--emit=ast",
            b"x.mod",
        ]);
        assert!(cfg.features.contains(codegen_core::Feature::Concurrency));
        assert!(cfg.features.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn features_no_default() {
        let cfg = ok(&[
            b"langc",
            b"--no-default-features",
            b"--features=concurrency",
            b"--emit=ast",
            b"x.mod",
        ]);
        assert!(cfg.features.contains(codegen_core::Feature::Concurrency));
        assert!(!cfg.features.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn features_no_default_empty() {
        let cfg = ok(&[
            b"langc",
            b"--no-default-features",
            b"--emit=ast",
            b"x.mod",
        ]);
        assert!(!cfg.features.contains(codegen_core::Feature::Concurrency));
        assert!(!cfg.features.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn features_unknown_value_errs() {
        let code = err(&[
            b"langc",
            b"--features=bogus",
            b"--emit=ast",
            b"x.mod",
        ]);
        assert_eq!(code, 2);
    }
}

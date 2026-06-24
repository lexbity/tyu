use codegen_core::Target;
use hosted::{args::RawArgs, cstr, diag, io};

pub const HELP: &[u8] = b"lang-assemble (tyu_lang) v0.1.0\n\nUSAGE:\n  lang-assemble [options] <file.asm>\n\nOPTIONS:\n  --help, -h                  Print help\n  --out=<path>                Output file path (default: a.out)\n  --target=<triple>           Target triple (default: x86_64-unknown-linux-gnu)\n                              Supported: x86_64-unknown-linux-gnu\n  --assembler=<path>          Override assembler binary path\n                              (default: determined by target)\n\n";

pub struct Config<'a> {
    pub input: &'a [u8],
    pub out_path: &'a [u8],
    /// Target determines which assembler backend to use.
    pub target: Target,
    /// Optional override for the assembler binary path.
    /// When `None`, the default for `target.spec().assembler` is used.
    pub assembler_path: Option<&'a [u8]>,
}

pub enum ParseResult<'a> {
    Ok(Config<'a>),
    Help,
    Error,
}

pub unsafe fn parse_args<'a>(
    argc: isize,
    argv: *const *const hosted::c::c_char,
) -> ParseResult<'a> {
    let args = unsafe { RawArgs::new(argc, argv) };

    let mut saw_help = false;
    let mut out_path: Option<&[u8]> = None;
    let mut target: Option<Target> = None;
    let mut assembler_path: Option<&[u8]> = None;
    let mut input: Option<&[u8]> = None;

    for (i, a) in args.iter().enumerate() {
        if i == 0 {
            continue;
        }

        if unsafe { cstr::eq(a, b"--help") } || unsafe { cstr::eq(a, b"-h") } {
            saw_help = true;
            continue;
        }
        let bytes = unsafe { cstr::as_bytes(a) };
        if bytes.starts_with(b"--out=") {
            out_path = Some(&bytes[b"--out=".len()..]);
            continue;
        }
        if bytes.starts_with(b"--target=") {
            let triple = &bytes[b"--target=".len()..];
            target = match Target::parse(triple) {
                Some(t) => Some(t),
                None => {
                    let _ = diag::error_simple(
                        2004,
                        b"unknown target triple (see --help for supported targets)",
                    );
                    return ParseResult::Error;
                }
            };
            continue;
        }
        if bytes.starts_with(b"--assembler=") {
            assembler_path = Some(&bytes[b"--assembler=".len()..]);
            continue;
        }
        if bytes.starts_with(b"-") {
            // Ignore unknown flags.
            continue;
        }
        if input.is_none() {
            input = Some(bytes);
        }
    }

    if saw_help || args.len() <= 1 {
        let _ = io::stdout(HELP);
        return if saw_help {
            ParseResult::Help
        } else {
            ParseResult::Error
        };
    }

    let Some(input) = input else {
        let _ = diag::error_simple(2001, b"missing input file");
        return ParseResult::Error;
    };

    ParseResult::Ok(Config {
        input,
        out_path: out_path.unwrap_or(b"a.out"),
        target: target.unwrap_or(Target::X86_64UnknownLinuxGnu),
        assembler_path,
    })
}

use hosted::{args::RawArgs, cstr, io};

pub const HELP: &[u8] = b"lang-assemble (tyu_lang) v0.1.0\n\nUSAGE:\n  lang-assemble [options] <file.asm>\n\nOPTIONS:\n  --help, -h          Print help\n  --out=<path>        Output executable path\n  --fasm=<path>       Path to fasm (default: fasm)\n\n";

pub struct Config<'a> {
    pub input: &'a [u8],
    pub out_path: &'a [u8],
    pub fasm_path: &'a [u8],
}

pub enum ParseResult<'a> {
    Ok(Config<'a>),
    Help,
    Error(i32),
}

pub unsafe fn parse_args<'a>(argc: isize, argv: *const *const hosted::c::c_char) -> ParseResult<'a> {
    let args = RawArgs::new(argc, argv);

    let mut saw_help = false;
    let mut out_path: Option<&[u8]> = None;
    let mut fasm_path: Option<&[u8]> = None;
    let mut input: Option<&[u8]> = None;

    for (i, a) in args.iter().enumerate() {
        if i == 0 {
            continue;
        }
        
        if cstr::eq(a, b"--help") || cstr::eq(a, b"-h") {
            saw_help = true;
            continue;
        }
        let bytes = cstr::as_bytes(a);
        if bytes.starts_with(b"--out=") {
            out_path = Some(&bytes[b"--out=".len()..]);
            continue;
        }
        if bytes.starts_with(b"--fasm=") {
            fasm_path = Some(&bytes[b"--fasm=".len()..]);
            continue;
        }
        if bytes.starts_with(b"-") {
            // Ignore unknown flags for now to keep CLI stable.
            continue;
        }
        if input.is_none() {
            input = Some(bytes);
        }
    }

    if saw_help || args.len() <= 1 {
        let _ = io::stdout(HELP);
        return if saw_help { ParseResult::Help } else { ParseResult::Error(2) };
    }

    let Some(input) = input else {
        return ParseResult::Error(2001); // 2001 was the error code for missing input
    };

    ParseResult::Ok(Config {
        input,
        out_path: out_path.unwrap_or(b"a.out"),
        fasm_path: fasm_path.unwrap_or(b"fasm"),
    })
}

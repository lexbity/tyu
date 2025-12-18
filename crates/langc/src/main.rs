#![no_std]
#![no_main]

use frontend::{
    lex::Lexer,
    parse::{DeclAst, DeclKind, ModuleAst, Output, Parser},
    span::Span,
    token::TokenKind,
};
use hosted::{args::RawArgs, cstr, diag, fs, io};
use semantics::types::{TypeAtom, WordEntry, WordSig};
use semantics::typecheck::{ChecksMode, SubtypeInfo};

hosted_rt::entry!(langc_main);

extern "C" fn langc_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    let args = unsafe { RawArgs::new(argc, argv) };

    let mut saw_help = false;
    let mut emit_ast = false;
    let mut emit_ir = false;
    let mut checks = ChecksMode::All;
    let mut input: Option<&[u8]> = None;
    let mut include_dirs: [&[u8]; 8] = [&[]; 8];
    let mut include_len = 0usize;

    let mut i = 1usize;
    while i < args.len() {
        let a = args.get(i).unwrap();
        unsafe {
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
            if bytes.starts_with(b"--checks=") {
                checks = match &bytes[b"--checks=".len()..] {
                    b"off" => ChecksMode::Off,
                    b"contracts" => ChecksMode::Contracts,
                    b"all" => ChecksMode::All,
                    _ => {
                        let _ = diag::error_simple(1006, b"invalid --checks value");
                        return 2;
                    }
                };
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
                    return 2;
                }
            }
            if bytes.starts_with(b"-") {
                i += 1;
                continue;
            }
            if input.is_none() {
                input = Some(bytes);
            }
        }
        i += 1;
    }

    if saw_help || args.len() <= 1 {
        let _ = io::stdout(HELP);
        return if saw_help { 0 } else { 2 };
    }

    if emit_ast && emit_ir {
        let _ = diag::error_simple(1005, b"choose a single --emit=...");
        return 2;
    }
    if !emit_ast && !emit_ir {
        let _ = diag::error_simple(1001, b"not implemented: use --emit=ast or --emit=ir");
        return 2;
    }

    let Some(path) = input else {
        let _ = diag::error_simple(1002, b"missing input file");
        return 2;
    };

    let buf = match fs::read_file(path) {
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
            emit_parse_error(path, src, e.code, e.span.start);
            return 2;
        }
    };

    let mut base_dir_buf = [0u8; 512];
    let base_dir = split_dir(path, &mut base_dir_buf);
    let mut search_dirs: [&[u8]; 9] = [&[]; 9];
    search_dirs[0] = base_dir;
    for j in 0..include_len {
        search_dirs[j + 1] = include_dirs[j];
    }
    let search_len = 1 + include_len;

    if let Err(code) = check_program(&module, src, &search_dirs[..search_len]) {
        let _ = diag::error_simple(code, b"interface/import error");
        return 2;
    }

    let mut out = Stdout;
    if emit_ast {
        return match Parser::new(src).parse_module_dump(&mut out) {
            Ok(()) => 0,
            Err(e) => {
                emit_parse_error(path, src, e.code, e.span.start);
                2
            }
        };
    }

    // --emit=ir
    let mut st_buf: [SubtypeInfo; 64] = [SubtypeInfo {
        name: TypeAtom::new(b"").unwrap(),
        base: TypeAtom::new(b"").unwrap(),
        min: 0,
        max: 0,
    }; 64];
    let mut st_len = 0usize;
    for s in module.subtypes.iter() {
        if st_len >= st_buf.len() {
            break;
        }
        let name = slice_span(src, s.name);
        let base = slice_span(src, s.base);
        let name = match TypeAtom::new(name) {
            Some(n) => n,
            None => continue,
        };
        let base = match TypeAtom::new(base) {
            Some(n) => n,
            None => continue,
        };
        st_buf[st_len] = SubtypeInfo {
            name,
            base,
            min: s.min,
            max: s.max,
        };
        st_len += 1;
    }

    let mut env: [WordEntry; 256] = [WordEntry {
        name: TypeAtom::new(b"").unwrap(),
        sig: WordSig::empty(),
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len);
    if let Err(code) =
        load_import_sigs(&module, src, &search_dirs[..search_len], &mut env, &mut env_len)
    {
        let _ = diag::error_simple(code, b"failed to load import signatures");
        return 2;
    }
    if let Err(code) = load_local_sigs(&module, src, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"invalid local signature");
        return 2;
    }

    struct SemOut<'a>(&'a mut Stdout);
    impl<'a> semantics::typecheck::Output for SemOut<'a> {
        fn write(&mut self, bytes: &[u8]) {
            self.0.write(bytes)
        }
    }
    let mut sem_out = SemOut(&mut out);
    match semantics::typecheck::emit_ir(
        &module,
        src,
        &env[..env_len],
        &st_buf[..st_len],
        checks,
        &mut sem_out,
    ) {
        Ok(()) => 0,
        Err(e) => {
            let _ = diag::error_simple(e.code, b"typecheck error");
            2
        }
    }
}

const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h              Print help\n  --emit=ast              Parse and dump AST (v1 milestone 1)\n  --emit=ir               Typecheck and dump IR (v1 milestone 3)\n  -I <path>                Add include path (v1 milestone 2)\n  --checks=off|contracts|all  Checks insertion mode (v1 milestone 4)\n  --sysroot=<path>         Sysroot root (stub)\n  --out-dir=<path>         Output directory (stub)\n  --target=<triple>        Target triple (stub)\n\n";

struct Stdout;

impl Output for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let _ = io::stdout(bytes);
    }
}

fn emit_parse_error(path: &[u8], src: &[u8], code: u32, offset: usize) {
    let (line, col) = line_col(src, offset);
    let _ = io::stderr(path);
    let _ = io::stderr(b":");
    write_u32_stderr(line);
    let _ = io::stderr(b":");
    write_u32_stderr(col);
    let _ = io::stderr(b": ");
    let _ = diag::error_simple(code, b"parse error");
}

fn write_u32_stderr(mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        buf[..n].reverse();
    }
    let _ = io::stderr(&buf[..n]);
}

fn line_col(src: &[u8], offset: usize) -> (u32, u32) {
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    let mut i = 0usize;
    let end = core::cmp::min(offset, src.len());
    while i < end {
        if src[i] == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += 1;
    }
    (line, col)
}

fn check_program(module: &ModuleAst, src: &[u8], search_dirs: &[&[u8]]) -> Result<(), u32> {
    let root_name = slice_span(src, module.name);
    if let Some(def_src) = try_load_module_file(search_dirs, root_name, b".def") {
        let def_ast = Parser::new(def_src.as_slice())
            .parse_module_ast()
            .map_err(|_| 2300u32)?;
        check_iface(&def_ast, def_src.as_slice(), module, src)?;
    }

    for imp in module.imports.iter() {
        let mname = slice_span(src, imp.module);
        let def_src = try_load_module_file(search_dirs, mname, b".def").ok_or(2201u32)?;
        let def_ast = Parser::new(def_src.as_slice())
            .parse_module_ast()
            .map_err(|_| 2202u32)?;

        for sym in imp.names.iter() {
            let sym_name = slice_span(src, *sym);
            if !is_exported(&def_ast, def_src.as_slice(), sym_name) {
                return Err(2203u32);
            }
        }

        if let Some(mod_src) = try_load_module_file(search_dirs, mname, b".mod") {
            let mod_ast = Parser::new(mod_src.as_slice())
                .parse_module_ast()
                .map_err(|_| 2204u32)?;
            check_iface(&def_ast, def_src.as_slice(), &mod_ast, mod_src.as_slice())?;
        }
    }

    Ok(())
}

fn check_iface(
    def_ast: &ModuleAst,
    def_src: &[u8],
    mod_ast: &ModuleAst,
    mod_src: &[u8],
) -> Result<(), u32> {
    for name in export_iter(def_ast, def_src) {
        if !is_exported(mod_ast, mod_src, name) {
            return Err(2210u32);
        }
    }
    for name in export_iter(mod_ast, mod_src) {
        if !is_exported(def_ast, def_src, name) {
            return Err(2211u32);
        }
    }

    for name in export_iter(def_ast, def_src) {
        let def_decl = find_decl(def_ast, def_src, name).ok_or(2212u32)?;
        let mod_decl = find_decl(mod_ast, mod_src, name).ok_or(2213u32)?;

        if def_decl.kind != mod_decl.kind {
            return Err(2214u32);
        }
        if !attrs_eq(def_src, &def_decl.attrs, mod_src, &mod_decl.attrs) {
            return Err(2215u32);
        }
        if def_decl.kind == DeclKind::Word {
            let def_sig = def_decl.sig.ok_or(2216u32)?;
            let mod_sig = mod_decl.sig.ok_or(2217u32)?;
            if !sig_eq(slice_span(def_src, def_sig), slice_span(mod_src, mod_sig)) {
                return Err(2218u32);
            }
        }
    }

    Ok(())
}

fn sig_eq(a: &[u8], b: &[u8]) -> bool {
    let mut la = Lexer::new(a);
    let mut lb = Lexer::new(b);
    loop {
        let ta = la.next();
        let tb = lb.next();
        if ta.kind != tb.kind {
            return false;
        }
        if ta.kind == TokenKind::Eof {
            return true;
        }
        match ta.kind {
            TokenKind::Ident | TokenKind::Number | TokenKind::String | TokenKind::EffectSet => {
                let sa = &a[ta.span.start..ta.span.end];
                let sb = &b[tb.span.start..tb.span.end];
                if sa != sb {
                    return false;
                }
            }
            _ => {}
        }
    }
}

fn attrs_eq(
    def_src: &[u8],
    def_attrs: &frontend::fixed::FixedVec<Span, 16>,
    mod_src: &[u8],
    mod_attrs: &frontend::fixed::FixedVec<Span, 16>,
) -> bool {
    if def_attrs.len() != mod_attrs.len() {
        return false;
    }
    for i in 0..def_attrs.len() {
        let da = *def_attrs.get(i).unwrap();
        let ma = *mod_attrs.get(i).unwrap();
        if slice_span(def_src, da) != slice_span(mod_src, ma) {
            return false;
        }
    }
    true
}

fn export_iter<'a>(ast: &'a ModuleAst, src: &'a [u8]) -> ExportIter<'a> {
    ExportIter { ast, src, i: 0 }
}

struct ExportIter<'a> {
    ast: &'a ModuleAst,
    src: &'a [u8],
    i: usize,
}

impl<'a> Iterator for ExportIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.ast.has_export_stmt {
            let span = *self.ast.exports.get(self.i)?;
            self.i += 1;
            Some(slice_span(self.src, span))
        } else {
            let decl = self.ast.decls.get(self.i)?;
            self.i += 1;
            Some(slice_span(self.src, decl.name))
        }
    }
}

fn is_exported(ast: &ModuleAst, src: &[u8], name: &[u8]) -> bool {
    if ast.has_export_stmt {
        for s in ast.exports.iter() {
            if slice_span(src, *s) == name {
                return true;
            }
        }
        false
    } else {
        find_decl(ast, src, name).is_some()
    }
}

fn find_decl<'a>(ast: &'a ModuleAst, src: &'a [u8], name: &[u8]) -> Option<&'a DeclAst> {
    for d in ast.decls.iter() {
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
}

fn slice_span<'a>(src: &'a [u8], span: Span) -> &'a [u8] {
    &src[span.start..span.end]
}

fn try_load_module_file(
    search_dirs: &[&[u8]],
    module: &[u8],
    ext: &[u8],
) -> Option<hosted::fs::ByteBuf> {
    let mut path_buf = [0u8; 512];
    for d in search_dirs {
        let path = join_path(&mut path_buf, d, module, ext)?;
        if let Ok(b) = fs::read_file(path) {
            return Some(b);
        }
    }
    None
}

fn join_path<'a>(buf: &'a mut [u8], dir: &[u8], module: &[u8], ext: &[u8]) -> Option<&'a [u8]> {
    let dir = if dir == b"." { b"" } else { dir };
    let sep: &[u8] = if dir.is_empty() || dir.ends_with(b"/") {
        b""
    } else {
        b"/"
    };
    let need = dir.len() + sep.len() + module.len() + ext.len();
    if need > buf.len() {
        return None;
    }
    let mut i = 0usize;
    buf[i..i + dir.len()].copy_from_slice(dir);
    i += dir.len();
    buf[i..i + sep.len()].copy_from_slice(sep);
    i += sep.len();
    buf[i..i + module.len()].copy_from_slice(module);
    i += module.len();
    buf[i..i + ext.len()].copy_from_slice(ext);
    i += ext.len();
    Some(&buf[..i])
}

fn split_dir<'a>(path: &[u8], out: &'a mut [u8]) -> &'a [u8] {
    let mut last = None;
    for (i, &b) in path.iter().enumerate() {
        if b == b'/' {
            last = Some(i);
        }
    }
    let Some(idx) = last else {
        out[0] = b'.';
        return &out[..1];
    };
    if idx == 0 {
        out[0] = b'/';
        return &out[..1];
    }
    if idx > out.len() {
        out[0] = b'.';
        return &out[..1];
    }
    out[..idx].copy_from_slice(&path[..idx]);
    &out[..idx]
}

fn add_builtins(env: &mut [WordEntry; 256], len: &mut usize) {
    let i64t = TypeAtom::new(b"i64").unwrap();
    let boolt = TypeAtom::new(b"bool").unwrap();
    let quot = TypeAtom::new(b"quot").unwrap();
    let empty = TypeAtom::new(b"").unwrap();

    let mut push = |name: &[u8], sig: WordSig| {
        if *len >= env.len() {
            return;
        }
        let name = TypeAtom::new(name).unwrap();
        env[*len] = WordEntry { name, sig };
        *len += 1;
    };

    // Stack ops
    push(
        b"dup",
        WordSig {
            in_len: 1,
            out_len: 2,
            inputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
            outputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"drop",
        WordSig {
            in_len: 1,
            out_len: 0,
            inputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
            outputs: [empty, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"swap",
        WordSig {
            in_len: 2,
            out_len: 2,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
        },
    );

    // Arithmetic/comparisons (MVP: i64 only)
    let bin_i64 = WordSig {
        in_len: 2,
        out_len: 1,
        inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
        outputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
    };
    push(b"+", bin_i64);
    push(b"-", bin_i64);
    push(b"*", bin_i64);

    push(
        b">",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"<",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b">=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"<=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"==",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"!=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"and",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [boolt, boolt, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"or",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [boolt, boolt, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );
    push(
        b"not",
        WordSig {
            in_len: 1,
            out_len: 1,
            inputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
    );

    // Treat quotations as values for now (for call sites we special-case intrinsics).
    push(
        b"call",
        WordSig {
            in_len: 1,
            out_len: 0,
            inputs: [quot, empty, empty, empty, empty, empty, empty, empty],
            outputs: [empty, empty, empty, empty, empty, empty, empty, empty],
        },
    );
}

fn load_import_sigs(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    env: &mut [WordEntry; 256],
    env_len: &mut usize,
) -> Result<(), u32> {
    for imp in module.imports.iter() {
        let mname = slice_span(src, imp.module);
        let def_src = try_load_module_file(search_dirs, mname, b".def").ok_or(2201u32)?;
        let def_ast = Parser::new(def_src.as_slice())
            .parse_module_ast()
            .map_err(|_| 2202u32)?;
        for name in export_iter(&def_ast, def_src.as_slice()) {
            let Some(d) = find_decl(&def_ast, def_src.as_slice(), name) else {
                continue;
            };
            if d.kind != DeclKind::Word {
                continue;
            }
            let Some(sig_span) = d.sig else {
                continue;
            };
            let sig = semantics::typecheck::parse_word_sig(def_src.as_slice(), sig_span)
                .map_err(|_| 2205u32)?;
            if *env_len < env.len() {
                let name_atom = TypeAtom::new(name).ok_or(2206u32)?;
                env[*env_len] = WordEntry { name: name_atom, sig };
                *env_len += 1;
            }
        }
    }
    Ok(())
}

fn load_local_sigs(
    module: &ModuleAst,
    src: &[u8],
    env: &mut [WordEntry; 256],
    env_len: &mut usize,
) -> Result<(), u32> {
    for d in module.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        let name = slice_span(src, d.name);
        let Some(sig_span) = d.sig else {
            continue;
        };
        let sig = semantics::typecheck::parse_word_sig(src, sig_span).map_err(|_| 2219u32)?;
        if *env_len < env.len() {
            let name_atom = TypeAtom::new(name).ok_or(2220u32)?;
            env[*env_len] = WordEntry { name: name_atom, sig };
            *env_len += 1;
        }
    }
    Ok(())
}

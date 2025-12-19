#![no_std]
#![no_main]

use frontend::{
    fixed::FixedVec,
    lex::Lexer,
    parse::{DeclAst, DeclKind, ModuleAst, Output, Parser},
    span::Span,
    token::TokenKind,
};
use hosted::{args::RawArgs, cstr, diag, errno::Errno, fs, io, process};
use ir as lir;
use semantics::types::{TypeAtom, WordEntry, WordSig};
use semantics::typecheck::{ChecksMode, SubtypeInfo};

hosted_rt::entry!(langc_main);

extern "C" fn langc_main(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
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
    let mut input: Option<&[u8]> = None;
    let mut include_dirs: [&[u8]; 8] = [&[]; 8];
    let mut include_len = 0usize;
    let mut sysroot: Option<&[u8]> = None;
    let mut out_dir: Option<&[u8]> = None;
    let mut target: Option<&[u8]> = None;

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
                        return 2;
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

    let emit_count = (emit_ast as u8) + (emit_ir as u8) + (emit_asm as u8) + (emit_obj as u8) + (emit_tc as u8);
    if emit_count > 1 {
        let _ = diag::error_simple(1005, b"choose a single --emit=...");
        return 2;
    }
    if emit_count == 0 {
        let _ = diag::error_simple(1001, b"use --emit=ast, --emit=ir, --emit=asm, or --emit=obj");
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
    let mut search_dirs: [&[u8]; 10] = [&[]; 10];
    search_dirs[0] = base_dir;
    for j in 0..include_len {
        search_dirs[j + 1] = include_dirs[j];
    }
    let mut search_len = 1 + include_len;

    let sysroot = sysroot.or_else(|| unsafe { hosted::env::get_str(b"LANG_SYSROOT") });
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
    if emit_ast {
        return match Parser::new(src).parse_module_dump(&mut out) {
            Ok(()) => 0,
            Err(e) => {
                emit_parse_error(path, src, e.code, e.span.start);
                2
            }
        };
    }

    if emit_ir {
        return emit_ir_driver(&module, src, &search_dirs[..search_len], checks, allow_raw_casts, &mut out);
    }
    if emit_tc {
        return emit_tc_driver(&module, src, &search_dirs[..search_len], checks, allow_raw_casts, &mut out);
    }
    if emit_obj {
        let out_dir = out_dir.unwrap_or(b".");
        let target = target.unwrap_or(b"linux-x86_64-hosted");
        return emit_obj_driver(
            &module,
            src,
            &search_dirs[..search_len],
            checks,
            allow_raw_casts,
            debug_trap_loc,
            out_dir,
            target,
        );
    }
    emit_asm_driver(
        &module,
        src,
        &search_dirs[..search_len],
        checks,
        allow_raw_casts,
        debug_trap_loc,
        &mut out,
    )
}

fn iface_error_message(code: u32) -> &'static [u8] {
    match code {
        // `check_program` import/interface errors (stable codes).
        2201 => b"import interface file (.def) not found",
        2202 => b"failed to parse imported interface (.def)",
        2203 => b"imported symbol not exported by interface",
        2204 => b"failed to parse imported implementation (.mod)",
        2210 => b"implementation missing exported symbol from interface",
        2211 => b"implementation exports symbol not present in interface",
        2212 => b"interface export refers to missing declaration",
        2213 => b"implementation export refers to missing declaration",
        2214 => b"interface/implementation declaration kind mismatch",
        2215 => b"interface/implementation attributes mismatch",
        2216 => b"interface exported word missing signature",
        2217 => b"implementation exported word missing signature",
        2218 => b"interface/implementation word signature mismatch",
        2219 => b"interface/implementation word effect mismatch",
        2300 => b"failed to parse module interface (.def) for current module",
        _ => b"interface/import error",
    }
}

fn emit_ir_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    out: &mut Stdout,
) -> i32 {
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
        may_suspend: false,
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len);
    if let Err(code) = load_import_sigs(module, src, search_dirs, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"failed to load import signatures");
        return 2;
    }
    if let Err(code) = load_local_sigs(module, src, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"invalid local signature");
        return 2;
    }

    struct SemOut<'a>(&'a mut Stdout);
    impl<'a> semantics::typecheck::Output for SemOut<'a> {
        fn write(&mut self, bytes: &[u8]) {
            self.0.write(bytes)
        }
    }
    let mut sem_out = SemOut(out);
    match semantics::typecheck::emit_ir(
        module,
        src,
        &env[..env_len],
        &st_buf[..st_len],
        checks,
        allow_raw_casts,
        &mut sem_out,
    ) {
        Ok(()) => 0,
        Err(e) => {
            let _ = diag::error_simple(e.code, b"typecheck error");
            2
        }
    }
}

const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h              Print help\n  --emit=ast              Parse and dump AST (v1 milestone 1)\n  --emit=ir               Typecheck and dump IR (v1 milestone 3)\n  --emit=tc               Legacy stackcheck dump (debug)\n  --emit=asm              Emit FASM ELF64 executable assembly (hosted)\n  --emit=obj              Emit ELF64 relocatable object (v1 milestone 7)\n  -g                      Enable trap-with-location stubs (hosted)\n  -I <path>                Add include path (v1 milestone 2)\n  --checks=off|contracts|all  Checks insertion mode (v1 milestone 4)\n  --allow-raw-casts        Enable raw pointer casts (v1 milestone 3)\n  --sysroot=<path>         Sysroot root (v1 milestone 8)\n  --out-dir=<path>         Output directory (used by --emit=obj)\n  --target=<triple>        Target triple (used by --emit=obj)\n\n";

struct Stdout;

impl Output for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let _ = io::stdout(bytes);
    }
}

struct MemOut {
    buf: fs::ByteBuf,
    err: Option<Errno>,
}

impl MemOut {
    fn new() -> Result<Self, Errno> {
        Ok(Self {
            buf: fs::ByteBuf::new()?,
            err: None,
        })
    }

    fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }
}

impl Output for MemOut {
    fn write(&mut self, bytes: &[u8]) {
        if self.err.is_some() {
            return;
        }
        if let Err(e) = self.buf.push_slice(bytes) {
            self.err = Some(e);
        }
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

fn emit_asm_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    debug_trap_loc: bool,
    out: &mut Stdout,
) -> i32 {
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
        may_suspend: false,
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len);
    if let Err(code) = load_import_sigs(module, src, search_dirs, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"failed to load import signatures");
        return 2;
    }
    if let Err(code) = load_local_sigs(module, src, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"invalid local signature");
        return 2;
    }

    let mut gen = IrAsmGen::new(module, src, out, debug_trap_loc, AsmMode::Executable);
    if let Err(code) = gen.emit_prelude() {
        let _ = diag::error_simple(code, b"asm emission error");
        return 2;
    }

    match semantics::typecheck::for_each_ir_word(module, src, &env[..env_len], &st_buf[..st_len], checks, allow_raw_casts, |w| gen.emit_word(w))
    {
        Ok(()) => {}
        Err(semantics::typecheck::ForEachIrError::Type(e)) => {
            let _ = diag::error_simple(e.code, b"typecheck error");
            return 2;
        }
        Err(semantics::typecheck::ForEachIrError::Consumer(code)) => {
            let _ = diag::error_simple(code, b"asm emission error");
            return 2;
        }
    }

    if let Err(code) = gen.emit_postlude() {
        let _ = diag::error_simple(code, b"asm emission error");
        return 2;
    }
    0
}

fn emit_tc_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    out: &mut Stdout,
) -> i32 {
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
        may_suspend: false,
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len);
    if let Err(code) = load_import_sigs(module, src, search_dirs, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"failed to load import signatures");
        return 2;
    }
    if let Err(code) = load_local_sigs(module, src, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"invalid local signature");
        return 2;
    }

    struct SemOut<'a>(&'a mut Stdout);
    impl<'a> semantics::typecheck::Output for SemOut<'a> {
        fn write(&mut self, bytes: &[u8]) {
            self.0.write(bytes)
        }
    }
    let mut sem_out = SemOut(out);
    match semantics::typecheck::emit_stackcheck(module, src, &env[..env_len], &st_buf[..st_len], checks, &mut sem_out) {
        Ok(()) => 0,
        Err(e) => {
            let _ = allow_raw_casts; // keep signature stable vs other drivers
            let _ = diag::error_simple(e.code, b"typecheck error");
            2
        }
    }
}

fn emit_obj_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    debug_trap_loc: bool,
    out_dir: &[u8],
    target: &[u8],
) -> i32 {
    if target != b"linux-x86_64-hosted" {
        let _ = diag::error_simple(1012, b"unsupported --target (expected linux-x86_64-hosted)");
        return 2;
    }

    // Hosted runtime currently assumes `main` returns an exit code (i64).
    let main_decl = match find_word_decl(module, src, b"main") {
        Some(d) => d,
        None => {
            let _ = diag::error_simple(7001, b"missing word: main");
            return 2;
        }
    };
    let main_sig = main_decl
        .sig
        .and_then(|s| semantics::typecheck::parse_word_sig(src, s).ok())
        .unwrap_or(WordSig::empty());
    if main_sig.out_len != 1 {
        let _ = diag::error_simple(1018, b"for --emit=obj, main must return exactly one value (exit code)");
        return 2;
    }

    let module_name = slice_span(src, module.name);
    let mut path_buf = [0u8; 512];
    let asm_path = match join_path(&mut path_buf, out_dir, module_name, b".asm") {
        Some(p) => p,
        None => {
            let _ = diag::error_simple(1013, b"--out-dir path too long");
            return 2;
        }
    };
    let mut obj_buf = [0u8; 512];
    let obj_path = match join_path(&mut obj_buf, out_dir, module_name, b".o") {
        Some(p) => p,
        None => {
            let _ = diag::error_simple(1013, b"--out-dir path too long");
            return 2;
        }
    };

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
        may_suspend: false,
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len);
    if let Err(code) = load_import_sigs(module, src, search_dirs, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"failed to load import signatures");
        return 2;
    }
    if let Err(code) = load_local_sigs(module, src, &mut env, &mut env_len) {
        let _ = diag::error_simple(code, b"invalid local signature");
        return 2;
    }

    let mut mem = match MemOut::new() {
        Ok(m) => m,
        Err(_) => {
            let _ = diag::error_simple(1014, b"out of memory");
            return 2;
        }
    };

    let mut gen = IrAsmGen::new(module, src, &mut mem, debug_trap_loc, AsmMode::Object);
    if let Err(code) = gen.emit_prelude() {
        let _ = diag::error_simple(code, b"asm emission error");
        return 2;
    }

    match semantics::typecheck::for_each_ir_word(module, src, &env[..env_len], &st_buf[..st_len], checks, allow_raw_casts, |w| gen.emit_word(w))
    {
        Ok(()) => {}
        Err(semantics::typecheck::ForEachIrError::Type(e)) => {
            let _ = diag::error_simple(e.code, b"typecheck error");
            return 2;
        }
        Err(semantics::typecheck::ForEachIrError::Consumer(code)) => {
            let _ = diag::error_simple(code, b"asm emission error");
            return 2;
        }
    }

    if let Err(code) = gen.emit_postlude() {
        let _ = diag::error_simple(code, b"asm emission error");
        return 2;
    }
    if mem.err.is_some() {
        let _ = diag::error_simple(1014, b"out of memory");
        return 2;
    }

    if fs::write_file(asm_path, mem.as_slice()).is_err() {
        let _ = diag::error_simple(1015, b"failed to write output .asm");
        return 2;
    }

    let status = process::run(b"fasm", &[asm_path, obj_path]).map_err(|_| diag::error_simple(1016, b"failed to run fasm"));
    let status = match status {
        Ok(s) => s,
        Err(_) => return 2,
    };
    if status.code != 0 {
        let _ = diag::error_simple(1017, b"fasm failed");
        return 2;
    }

    0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AsmMode {
    Executable,
    Object,
}

struct IrAsmGen<'a> {
    module: &'a ModuleAst,
    src: &'a [u8],
    out: &'a mut dyn Output,
    mode: AsmMode,
    label_id: u32,
    uses_channels: bool,
    uses_mmio: bool,
    str_len: usize,
    str_spans: [Span; 128],
    str_ids: [u32; 128],
    debug_trap_loc: bool,
    cur_word_id: u32,
}

impl<'a> IrAsmGen<'a> {
    fn new(module: &'a ModuleAst, src: &'a [u8], out: &'a mut dyn Output, debug_trap_loc: bool, mode: AsmMode) -> Self {
        Self {
            module,
            src,
            out,
            mode,
            label_id: 0,
            uses_channels: false,
            uses_mmio: false,
            str_len: 0,
            str_spans: [Span::new(0, 0); 128],
            str_ids: [0u32; 128],
            debug_trap_loc,
            cur_word_id: 0,
        }
    }

    fn fresh_label(&mut self) -> u32 {
        let id = self.label_id;
        self.label_id = self.label_id.wrapping_add(1);
        id
    }

    fn emit_prelude(&mut self) -> Result<(), u32> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"format ELF64 executable\n");
                self.out.write(b"entry __lang_start\n\n");

                self.out.write(b"segment readable executable\n");
                self.out.write(b"__lang_start:\n");
                self.out.write(b"  mov r15, __lang_ds_base\n");
                self.out.write(b"  mov r14, __lang_ds_limit\n");

                let main_decl = find_word_decl(self.module, self.src, b"main").ok_or(7001u32)?;
                let main_sig = main_decl
                    .sig
                    .map(|s| semantics::typecheck::parse_word_sig(self.src, s).ok())
                    .flatten()
                    .unwrap_or(WordSig::empty());

                self.out.write(b"  call ");
                write_label(self.out, b"main");
                self.out.write(b"\n");

                if main_sig.out_len > 0 {
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rdi, [r15]\n");
                    self.out.write(b"  and rdi, 0xff\n");
                } else {
                    self.out.write(b"  xor rdi, rdi\n");
                }
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");

                self.out.write(b"__lang_trap:\n");
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");
                if self.debug_trap_loc {
                    self.out.write(b"__lang_trap_loc:\n");
                    // Signature: (code:u32, file_id:u32, line:u32, word_id:u32) -> !
                    // Hosted stub: ignore location and exit with `code`.
                    self.out.write(b"  mov rax, 60\n");
                    self.out.write(b"  syscall\n\n");
                }
                emit_stack_overflow(self.out);
                Ok(())
            }
            AsmMode::Object => {
                self.out.write(b"format ELF64\n\n");
                self.out.write(b"section '.text' executable\n");
                // Runtime-provided symbols.
                self.out.write(b"extrn __lang_trap\n");
                self.out.write(b"extrn __lang_trap_loc\n");
                self.out.write(b"extrn __stack_overflow\n");
                self.out.write(b"extrn __mmio_mem\n");
                self.out.write(b"extrn __chan_next\n");
                self.out.write(b"extrn __chan_head\n");
                self.out.write(b"extrn __chan_tail\n");
                self.out.write(b"extrn __chan_buf\n");
                self.out.write(b"\n");
                Ok(())
            }
        }
    }

    fn emit_postlude(&mut self) -> Result<(), u32> {
        match self.mode {
            AsmMode::Executable => {
                // Emit string literals (as `str` structs pointing to bytes) into the executable segment.
                for i in 0..self.str_len {
                    let id = self.str_ids[i];
                    let span = self.str_spans[i];
                    let bytes = decode_string_bytes(self.src, span).ok_or(7120u32)?;

                    self.out.write(b"\n__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b":\n");
                    self.out.write(b"  dq __lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes\n");
                    self.out.write(b"  dq ");
                    write_u32(self.out, bytes.len() as u32);
                    self.out.write(b"\n");

                    self.out.write(b"__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes db ");
                    for (j, b) in bytes.iter().enumerate() {
                        if j != 0 {
                            self.out.write(b",");
                        }
                        write_u32(self.out, *b as u32);
                    }
                    if bytes.len() == 0 {
                        self.out.write(b"0");
                    }
                    self.out.write(b"\n");
                }

                self.out.write(b"\nsegment readable writeable\n");
                if self.uses_channels {
                    // Hosted channels (Milestone 16): fixed-size per-channel ring buffers.
                    self.out.write(b"__chan_next dq 0\n");
                    self.out.write(b"__chan_head rq 16\n");
                    self.out.write(b"__chan_tail rq 16\n");
                    self.out.write(b"__chan_buf rq ");
                    write_u32(self.out, 16 * 64);
                    self.out.write(b"\n");
                }
                if self.uses_mmio {
                    // Hosted MMIO (Milestone 6): fixed-size simulated MMIO region.
                    self.out.write(b"__mmio_mem rb 65536\n");
                }
                self.out.write(b"__lang_ds_base rb 65536\n");
                self.out.write(b"__lang_ds_limit:\n");
                Ok(())
            }
            AsmMode::Object => {
                if self.str_len == 0 {
                    return Ok(());
                }
                self.out.write(b"section '.data' writeable\n");
                for i in 0..self.str_len {
                    let id = self.str_ids[i];
                    let span = self.str_spans[i];
                    let bytes = decode_string_bytes(self.src, span).ok_or(7120u32)?;

                    self.out.write(b"\n__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b":\n");
                    self.out.write(b"  dq __lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes\n");
                    self.out.write(b"  dq ");
                    write_u32(self.out, bytes.len() as u32);
                    self.out.write(b"\n");

                    self.out.write(b"__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes db ");
                    for (j, b) in bytes.iter().enumerate() {
                        if j != 0 {
                            self.out.write(b",");
                        }
                        write_u32(self.out, *b as u32);
                    }
                    if bytes.len() == 0 {
                        self.out.write(b"0");
                    }
                    self.out.write(b"\n");
                }
                Ok(())
            }
        }
    }

    fn intern_str(&mut self, span: Span) -> Result<u32, u32> {
        for i in 0..self.str_len {
            if slice_span(self.src, self.str_spans[i]) == slice_span(self.src, span) {
                return Ok(self.str_ids[i]);
            }
        }
        if self.str_len >= self.str_spans.len() {
            return Err(7121);
        }
        let id = self.fresh_label();
        self.str_spans[self.str_len] = span;
        self.str_ids[self.str_len] = id;
        self.str_len += 1;
        Ok(id)
    }

    fn emit_word(&mut self, w: &lir::Word) -> Result<(), u32> {
        self.cur_word_id = fnv1a_u32(w.name.as_bytes());
        self.out.write(b"\n");
        if self.mode == AsmMode::Object {
            self.out.write(b"public ");
            write_label(self.out, w.name.as_bytes());
            self.out.write(b"\n");
        }
        write_label(self.out, w.name.as_bytes());
        self.out.write(b":\n");

        let base = self.fresh_label();

        let slots = max_local_slot_ir(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let frame_bytes = locals_bytes_ir(slots);
        if frame_bytes > 0 {
            self.out.write(b"  sub rsp, ");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }

        // Jump to entry so we don't depend on block emission order.
        self.out.write(b"  jmp .b");
        write_u32(self.out, base);
        self.out.write(b"_");
        write_u32(self.out, w.entry.0 as u32);
        self.out.write(b"\n");

        for b in w.blocks.iter() {
            self.out.write(b".b");
            write_u32(self.out, base);
            self.out.write(b"_");
            write_u32(self.out, b.id.0 as u32);
            self.out.write(b":\n");
            for op in b.ops.iter() {
                self.emit_op(w, op, base)?;
            }
        }

        self.out.write(b".endword_");
        write_u32(self.out, base);
        self.out.write(b":\n");
        if frame_bytes > 0 {
            self.out.write(b"  add rsp, ");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }
        self.out.write(b"  ret\n");
        Ok(())
    }

    fn emit_op(&mut self, w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), u32> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                emit_push_i64(self.out, v);
                Ok(())
            }
            lir::OpKind::ConstBool(v) => {
                emit_push_i64(self.out, if v { 1 } else { 0 });
                Ok(())
            }
            lir::OpKind::ConstStr(span) => {
                let id = self.intern_str(span)?;
                self.out.write(b"  mov rax, __lang_str_");
                write_u32(self.out, id);
                self.out.write(b"\n");
                emit_push_rax(self.out);
                Ok(())
            }

            lir::OpKind::AddrOf { const_addr: Some(addr), .. } => {
                self.uses_mmio = true;
                emit_push_u64(self.out, addr);
                Ok(())
            }
            lir::OpKind::AddrOf { const_addr: None, .. } => Err(7101),
            lir::OpKind::MmioPlace { addr, .. } => {
                self.uses_mmio = true;
                emit_push_u64(self.out, addr);
                Ok(())
            }

            lir::OpKind::ScopedEnter { .. } => Ok(()),

            lir::OpKind::Dup { .. } => {
                emit_dup(self.out);
                Ok(())
            }
            lir::OpKind::Drop { .. } => {
                emit_drop(self.out);
                Ok(())
            }
            lir::OpKind::Swap { .. } => {
                emit_swap(self.out);
                Ok(())
            }

            lir::OpKind::AddI64 => {
                emit_binop(self.out, b"add");
                Ok(())
            }
            lir::OpKind::SubI64 => {
                emit_binop(self.out, b"sub");
                Ok(())
            }
            lir::OpKind::MulI64 => {
                emit_binop(self.out, b"imul");
                Ok(())
            }
            lir::OpKind::Cmp { kind, .. } => {
                let setcc: &[u8] = match kind {
                    lir::CmpKind::Lt => b"setl",
                    lir::CmpKind::Le => b"setle",
                    lir::CmpKind::Gt => b"setg",
                    lir::CmpKind::Ge => b"setge",
                    lir::CmpKind::Eq => b"sete",
                    lir::CmpKind::Ne => b"setne",
                };
                emit_cmp(self.out, setcc);
                Ok(())
            }
            lir::OpKind::AndBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n");
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  and rax, rcx\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  setne al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }
            lir::OpKind::OrBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n");
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  or rax, rcx\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  setne al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }
            lir::OpKind::NotBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  sete al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }

            lir::OpKind::LocalSet { slot, .. } => {
                emit_store_local(self.out, slot as u32);
                Ok(())
            }
            lir::OpKind::LocalGet { slot, .. } => {
                emit_load_local(self.out, slot as u32);
                Ok(())
            }

            lir::OpKind::Cast { from, to } => {
                self.emit_cast(w, from, to);
                Ok(())
            }
            lir::OpKind::Bitcast { .. } => Ok(()),

            lir::OpKind::Call { name, .. } => {
                let n = name.as_bytes();
                if n == b"platform.io.log" {
                    // Stack: `( str -- )` where `str` is `*const { ptr:u64, len:u64 }`.
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rcx, [r15]\n");
                    self.out.write(b"  mov rsi, [rcx]\n");
                    self.out.write(b"  mov rdx, [rcx+8]\n");
                    self.out.write(b"  mov rdi, 2\n");
                    self.out.write(b"  mov rax, 1\n");
                    self.out.write(b"  syscall\n");
                    return Ok(());
                }
                if n == b"platform.channel.make" {
                    // Stack: `( -- Chan(T) )` (hosted: returns small integer handle).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    self.out.write(b"  mov rax, [__chan_next]\n");
                    self.out.write(b"  cmp rax, 16\n");
                    self.out.write(b"  jb .chan_make_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_make_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, rax\n");
                    self.out.write(b"  add rax, 1\n");
                    self.out.write(b"  mov [__chan_next], rax\n");
                    self.out.write(b"  mov rax, rcx\n");
                    emit_push_rax(self.out);
                    return Ok(());
                }
                if n == b"platform.channel.send" {
                    // Stack: `( Chan(T) T -- )` (hosted: enqueue 64-bit payload).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    let space = self.fresh_label();
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rdx, [r15]\n"); // payload
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rax, [r15]\n"); // chan
                    self.out.write(b"  cmp rax, 16\n");
                    self.out.write(b"  jb .chan_send_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_send_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
                    self.out.write(b"  mov r8, [__chan_head + rax*8]\n");
                    self.out.write(b"  sub rcx, r8\n");
                    self.out.write(b"  cmp rcx, 64\n");
                    self.out.write(b"  jb .chan_send_space_");
                    write_u32(self.out, space);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_send_space_");
                    write_u32(self.out, space);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
                    self.out.write(b"  mov r8, rcx\n");
                    self.out.write(b"  and r8, 63\n");
                    self.out.write(b"  mov r9, rax\n");
                    self.out.write(b"  shl r9, 6\n");
                    self.out.write(b"  add r9, r8\n");
                    self.out.write(b"  mov [__chan_buf + r9*8], rdx\n");
                    self.out.write(b"  add rcx, 1\n");
                    self.out.write(b"  mov [__chan_tail + rax*8], rcx\n");
                    return Ok(());
                }
                if n == b"platform.channel.recv" {
                    // Stack: `( Chan(T) -- T )` (hosted: dequeue 64-bit payload).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    let has = self.fresh_label();
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov r11, [r15]\n"); // chan
                    self.out.write(b"  cmp r11, 16\n");
                    self.out.write(b"  jb .chan_recv_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_recv_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_head + r11*8]\n");
                    self.out.write(b"  mov r8, [__chan_tail + r11*8]\n");
                    self.out.write(b"  cmp rcx, r8\n");
                    self.out.write(b"  jne .chan_recv_has_");
                    write_u32(self.out, has);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_recv_has_");
                    write_u32(self.out, has);
                    self.out.write(b":\n");
                    self.out.write(b"  mov r9, rcx\n");
                    self.out.write(b"  and r9, 63\n");
                    self.out.write(b"  mov r10, r11\n");
                    self.out.write(b"  shl r10, 6\n");
                    self.out.write(b"  add r10, r9\n");
                    self.out.write(b"  mov rax, [__chan_buf + r10*8]\n");
                    self.out.write(b"  add rcx, 1\n");
                    self.out.write(b"  mov [__chan_head + r11*8], rcx\n");
                    emit_push_rax(self.out);
                    return Ok(());
                }
                if n == b"platform.time.now_ms" {
                    // Stack: `( -- i64 )`. Uses `clock_gettime(CLOCK_MONOTONIC, &ts)`.
                    self.out.write(b"  sub rsp, 16\n");
                    self.out.write(b"  mov rdi, 1\n");
                    self.out.write(b"  mov rsi, rsp\n");
                    self.out.write(b"  mov rax, 228\n");
                    self.out.write(b"  syscall\n");
                    self.out.write(b"  mov rax, [rsp]\n");
                    self.out.write(b"  imul rax, 1000\n");
                    self.out.write(b"  mov r9, rax\n");
                    self.out.write(b"  mov rax, [rsp+8]\n");
                    self.out.write(b"  xor rdx, rdx\n");
                    self.out.write(b"  mov rcx, 1000000\n");
                    self.out.write(b"  div rcx\n");
                    self.out.write(b"  add rax, r9\n");
                    self.out.write(b"  add rsp, 16\n");
                    emit_push_rax(self.out);
                    return Ok(());
                }
                if n == b"platform.task.yield" {
                    // Hosted baseline: `sched_yield` syscall (Linux x86_64: 24).
                    self.out.write(b"  mov rax, 24\n");
                    self.out.write(b"  syscall\n");
                    return Ok(());
                }
                if n == b"platform.critical.enter" || n == b"platform.critical.exit" {
                    // Single-thread hosted baseline: no-op.
                    return Ok(());
                }
                self.out.write(b"  call ");
                write_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(())
            }

            lir::OpKind::Load { .. } => Err(7103),
            lir::OpKind::Store { .. } => Err(7104),

            lir::OpKind::MmioVolLoad { ty, .. } => {
                self.uses_mmio = true;
                let (bits, signed) = prim_ty_bits_signed(w, ty).ok_or(7105u32)?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                emit_mmio_load(self, width, signed, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolStore { ty, .. } => {
                self.uses_mmio = true;
                let (bits, _signed) = prim_ty_bits_signed(w, ty).ok_or(7106u32)?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                emit_mmio_store(self, width, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolLoadField { reg_ty, field_ty, mask, shift, .. } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty).ok_or(7107u32)?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                let (field_bits, field_signed) = prim_ty_bits_signed(w, field_ty).ok_or(7107u32)?;
                emit_mmio_load_field(self, reg_width, field_bits, field_signed, mask, shift, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolStoreField { reg_ty, mask, shift, .. } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty).ok_or(7108u32)?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                emit_mmio_store_field(self, reg_width, mask, shift, op.span);
                Ok(())
            }

            lir::OpKind::CheckSubtype { .. } => {
                Err(7110)
            }
            lir::OpKind::TrapIfFalse { code } => {
                let ok = self.fresh_label();
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  jne .trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b"\n");
                self.emit_trap_with_loc(lir::trap_code_u32(code), op.span);
                self.out.write(b".trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b":\n");
                let _ = base;
                Ok(())
            }

            lir::OpKind::Br { target } => {
                self.out.write(b"  jmp .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, target.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  je .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, else_tgt.0 as u32);
                self.out.write(b"\n");
                self.out.write(b"  jmp .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, then_tgt.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Ret => {
                self.out.write(b"  jmp .endword_");
                write_u32(self.out, base);
                self.out.write(b"\n");
                Ok(())
            }
        }
    }

    fn emit_trap_with_loc(&mut self, code: u32, span: Span) {
        if self.debug_trap_loc {
            let (line, _col) = line_col(self.src, span.start);
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  mov rsi, 1\n"); // file_id (hosted single file)
            self.out.write(b"  mov rdx, ");
            write_u32(self.out, line);
            self.out.write(b"\n");
            self.out.write(b"  mov rcx, ");
            write_u32(self.out, self.cur_word_id);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap_loc\n");
        } else {
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap\n");
        }
    }
}

fn fnv1a_u32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn prim_bits_signed(ty: &[u8]) -> Option<(u16, bool)> {
    if ty.starts_with(b"Chan(") {
        return Some((64, false));
    }
    let (bits, signed) = match ty {
        b"u8" => (8, false),
        b"u16" => (16, false),
        b"u32" => (32, false),
        b"u64" => (64, false),
        b"usize" => (64, false),
        b"i8" => (8, true),
        b"i16" => (16, true),
        b"i32" => (32, true),
        b"i64" => (64, true),
        b"isize" => (64, true),
        b"bool" => (8, false),
        b"ptr" | b"ptr_mut" | b"str" | b"mmio" => (64, false),
        _ => return None,
    };
    Some((bits, signed))
}

fn prim_ty_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    let b = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    prim_bits_signed(b)
}

fn emit_mmio_bounds_check(gen: &mut IrAsmGen<'_>, width: u32, span: Span) {
    const MMIO_SIZE: u32 = 65536;
    let ok = gen.fresh_label();
    let max = MMIO_SIZE.saturating_sub(width);
    gen.out.write(b"  cmp rax, ");
    write_u32(gen.out, max);
    gen.out.write(b"\n");
    gen.out.write(b"  jbe .mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
    gen.out.write(b".mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
}

fn emit_mmio_load(gen: &mut IrAsmGen<'_>, width: u32, signed: bool, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, width, span);
    match (width, signed) {
        (1, true) => gen.out.write(b"  movsx rax, byte [__mmio_mem + rax]\n"),
        (1, false) => gen.out.write(b"  movzx rax, byte [__mmio_mem + rax]\n"),
        (2, true) => gen.out.write(b"  movsx rax, word [__mmio_mem + rax]\n"),
        (2, false) => gen.out.write(b"  movzx rax, word [__mmio_mem + rax]\n"),
        (4, true) => gen.out.write(b"  movsxd rax, dword [__mmio_mem + rax]\n"),
        (4, false) => gen.out.write(b"  mov eax, dword [__mmio_mem + rax]\n"),
        (8, _) => gen.out.write(b"  mov rax, qword [__mmio_mem + rax]\n"),
        _ => {
            // Shouldn't happen for supported primitive widths; trap if it does.
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }
    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
}

fn emit_mmio_store(gen: &mut IrAsmGen<'_>, width: u32, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n");
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, width, span);
    match width {
        1 => gen.out.write(b"  mov byte [__mmio_mem + rax], cl\n"),
        2 => gen.out.write(b"  mov word [__mmio_mem + rax], cx\n"),
        4 => gen.out.write(b"  mov dword [__mmio_mem + rax], ecx\n"),
        8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rcx\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
        }
    }
}

fn emit_mmio_load_field(
    gen: &mut IrAsmGen<'_>,
    reg_width: u32,
    field_bits: u16,
    field_signed: bool,
    mask: u64,
    shift: u8,
    span: Span,
) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, reg_width, span);

    match reg_width {
        1 => gen.out.write(b"  movzx rcx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rcx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov ecx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rcx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  and rcx, r8\n");
    if shift != 0 {
        gen.out.write(b"  shr rcx, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }

    gen.out.write(b"  mov rax, rcx\n");
    if field_bits < 64 {
        if field_bits <= 32 {
            gen.out.write(b"  and eax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        } else {
            gen.out.write(b"  and rax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        }
        if field_signed {
            let sh = 64u32 - (field_bits as u32);
            gen.out.write(b"  shl rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
            gen.out.write(b"  sar rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
        }
    }

    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
}

fn emit_mmio_store_field(gen: &mut IrAsmGen<'_>, reg_width: u32, mask: u64, shift: u8, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n"); // field value
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n"); // addr
    emit_mmio_bounds_check(gen, reg_width, span);

    match reg_width {
        1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r9, r8\n");
    gen.out.write(b"  not r9\n");
    gen.out.write(b"  and rdx, r9\n");

    gen.out.write(b"  mov r10, rcx\n");
    if shift != 0 {
        gen.out.write(b"  shl r10, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }
    gen.out.write(b"  and r10, r8\n");
    gen.out.write(b"  or rdx, r10\n");

    match reg_width {
        1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
        2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
        4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
        8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
        _ => {}
    }
}

impl<'a> IrAsmGen<'a> {
    fn emit_cast(&mut self, w: &lir::Word, from: lir::TypeId, to: lir::TypeId) {
        let from_ty = w.types.get(from.0 as usize).map(|a| a.as_bytes()).unwrap_or(b"");
        let to_ty = w.types.get(to.0 as usize).map(|a| a.as_bytes()).unwrap_or(b"");
        if from_ty == to_ty {
            return;
        }

        let Some((from_bits, from_signed)) = prim_bits_signed(from_ty) else {
            return;
        };
        let Some((to_bits, to_signed)) = prim_bits_signed(to_ty) else {
            return;
        };

        // Top value lives at [r15-8].
        self.out.write(b"  mov rax, [r15-8]\n");

        // Canonicalize to the source type (truncate + sign/zero extend).
        if from_bits < 64 {
            if from_bits <= 32 {
                self.out.write(b"  and eax, ");
                write_u64_hex(self.out, mask_for_bits(from_bits));
                self.out.write(b"\n");
            } else {
                self.out.write(b"  and rax, ");
                write_u64_hex(self.out, mask_for_bits(from_bits));
                self.out.write(b"\n");
            }
            if from_signed {
                let sh = 64u32 - (from_bits as u32);
                self.out.write(b"  shl rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
                self.out.write(b"  sar rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
            }
        }

        // Convert to target width (truncate + sign/zero extend).
        if to_bits < 64 {
            if to_bits <= 32 {
                self.out.write(b"  and eax, ");
                write_u64_hex(self.out, mask_for_bits(to_bits));
                self.out.write(b"\n");
            } else {
                self.out.write(b"  and rax, ");
                write_u64_hex(self.out, mask_for_bits(to_bits));
                self.out.write(b"\n");
            }
            if to_signed {
                let sh = 64u32 - (to_bits as u32);
                self.out.write(b"  shl rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
                self.out.write(b"  sar rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
            }
        } else if to_signed && !from_signed {
            // 64-bit unsigned -> 64-bit signed: keep bits (two's complement).
        }

        // Special-case casts to bool: normalize to 0/1.
        if to_ty == b"bool" && from_ty != b"bool" {
            self.out.write(b"  cmp rax, 0\n");
            self.out.write(b"  setne al\n");
            self.out.write(b"  movzx rax, al\n");
        }

        self.out.write(b"  mov [r15-8], rax\n");
    }
}

fn mask_for_bits(bits: u16) -> u64 {
    if bits >= 64 {
        !0u64
    } else {
        (1u64 << bits) - 1
    }
}

fn write_u64_hex(out: &mut dyn Output, v: u64) {
    // Minimal hex writer for assembler immediates: 0x....
    out.write(b"0x");
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let shift = (15 - i) * 4;
        let nib = ((v >> shift) & 0xF) as u8;
        buf[i] = match nib {
            0..=9 => b'0' + nib,
            _ => b'a' + (nib - 10),
        };
    }
    // Trim leading zeros.
    let mut start = 0usize;
    while start + 1 < buf.len() && buf[start] == b'0' {
        start += 1;
    }
    out.write(&buf[start..]);
}

fn max_local_slot_ir(w: &lir::Word) -> Option<u16> {
    let mut max = None;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            let s = match op.kind {
                lir::OpKind::LocalSet { slot, .. } => Some(slot),
                lir::OpKind::LocalGet { slot, .. } => Some(slot),
                _ => None,
            };
            if let Some(s) = s {
                max = Some(match max {
                    Some(m) => core::cmp::max(m, s),
                    None => s,
                });
            }
        }
    }
    max
}

fn locals_bytes_ir(slots: u32) -> u32 {
    if slots == 0 {
        return 0;
    }
    let mut bytes = slots * 8;
    if bytes % 16 != 0 {
        bytes += 8;
    }
    bytes
}

fn find_word_decl<'a>(m: &'a ModuleAst, src: &[u8], name: &[u8]) -> Option<&'a DeclAst> {
    for d in m.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
}

fn write_label(out: &mut dyn Output, name: &[u8]) {
    out.write(b"w_");
    for &b in name {
        let hi = b >> 4;
        let lo = b & 0xf;
        out.write(&[hex_digit(hi), hex_digit(lo)]);
    }
}

fn hex_digit(v: u8) -> u8 {
    match v {
        0..=9 => b'0' + v,
        _ => b'a' + (v - 10),
    }
}

fn write_u32(out: &mut dyn Output, mut v: u32) {
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
    out.write(&buf[..n]);
}

fn emit_push_i64(out: &mut dyn Output, v: i64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    emit_i64(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

fn emit_push_u64(out: &mut dyn Output, v: u64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    write_u64_hex(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

fn emit_push_rax(out: &mut dyn Output) {
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_i64(out: &mut dyn Output, mut v: i64) {
    let mut buf = [0u8; 24];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        if v < 0 {
            out.write(b"-");
            v = -v;
        }
        let mut u = v as u64;
        while u > 0 && n < buf.len() {
            buf[n] = b'0' + (u % 10) as u8;
            n += 1;
            u /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

fn decode_string_bytes(src: &[u8], span: Span) -> Option<FixedVec<u8, 256>> {
    if span.end <= span.start + 1 {
        return None;
    }
    let s = &src[span.start..span.end];
    if s.first().copied()? != b'"' {
        return None;
    }
    if s.last().copied()? != b'"' {
        return None;
    }
    let mut out: FixedVec<u8, 256> = FixedVec::new();
    let mut i = 1usize;
    while i + 1 < s.len() {
        let b = s[i];
        if b == b'\\' {
            i += 1;
            if i + 1 >= s.len() {
                return None;
            }
            let e = s[i];
            let v = match e {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'0' => 0,
                b'\\' => b'\\',
                b'"' => b'"',
                _ => e,
            };
            out.push(v).ok()?;
            i += 1;
            continue;
        }
        out.push(b).ok()?;
        i += 1;
    }
    Some(out)
}

fn emit_dup(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_drop(out: &mut dyn Output) {
    out.write(b"  sub r15, 8\n");
}

fn emit_swap(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  mov rcx, [r15-16]\n");
    out.write(b"  mov [r15-8], rcx\n");
    out.write(b"  mov [r15-16], rax\n");
}

fn emit_binop(out: &mut dyn Output, op: &[u8]) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rcx, [r15]\n");
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  ");
    out.write(op);
    out.write(b" rax, rcx\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_cmp(out: &mut dyn Output, setcc: &[u8]) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rcx, [r15]\n");
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  cmp rax, rcx\n");
    out.write(b"  ");
    out.write(setcc);
    out.write(b" al\n");
    out.write(b"  movzx rax, al\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_store_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  mov [rsp+");
    write_u32(out, idx * 8);
    out.write(b"], rax\n");
}

fn emit_load_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov rax, [rsp+");
    write_u32(out, idx * 8);
    out.write(b"]\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_stack_overflow(out: &mut dyn Output) {
    out.write(b"__stack_overflow:\n");
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::StackOverflow));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
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
            if def_decl.effect_suspend != mod_decl.effect_suspend {
                return Err(2219u32);
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

    fn push(env: &mut [WordEntry; 256], len: &mut usize, name: &[u8], sig: WordSig, may_suspend: bool) {
        if *len >= env.len() {
            return;
        }
        let name = TypeAtom::new(name).unwrap();
        env[*len] = WordEntry {
            name,
            sig,
            may_suspend,
        };
        *len += 1;
    }

    // Stack ops
	    push(
	        env,
	        len,
	        b"dup",
	        WordSig {
	            in_len: 1,
	            out_len: 2,
	            inputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
	            outputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
	        },
	        false,
	    );
    push(
        env,
        len,
        b"drop",
        WordSig {
            in_len: 1,
            out_len: 0,
            inputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
            outputs: [empty, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"swap",
        WordSig {
            in_len: 2,
            out_len: 2,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
        },
        false,
    );

    // Arithmetic/comparisons (MVP: i64 only)
    let bin_i64 = WordSig {
        in_len: 2,
        out_len: 1,
        inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
        outputs: [i64t, empty, empty, empty, empty, empty, empty, empty],
    };
    push(env, len, b"+", bin_i64, false);
    push(env, len, b"-", bin_i64, false);
    push(env, len, b"*", bin_i64, false);

    push(
        env,
        len,
        b">",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"<",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b">=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"<=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"==",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"!=",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [i64t, i64t, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"and",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [boolt, boolt, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"or",
        WordSig {
            in_len: 2,
            out_len: 1,
            inputs: [boolt, boolt, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );
    push(
        env,
        len,
        b"not",
        WordSig {
            in_len: 1,
            out_len: 1,
            inputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
            outputs: [boolt, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );

    // Treat quotations as values for now (for call sites we special-case intrinsics).
    push(
        env,
        len,
        b"call",
        WordSig {
            in_len: 1,
            out_len: 0,
            inputs: [quot, empty, empty, empty, empty, empty, empty, empty],
            outputs: [empty, empty, empty, empty, empty, empty, empty, empty],
        },
        false,
    );

    // Suspension points (minimal list)
    push(
        env,
        len,
        b"platform.task.yield",
        WordSig {
            in_len: 0,
            out_len: 0,
            inputs: [empty, empty, empty, empty, empty, empty, empty, empty],
            outputs: [empty, empty, empty, empty, empty, empty, empty, empty],
        },
        true,
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
                env[*env_len] = WordEntry {
                    name: name_atom,
                    sig,
                    may_suspend: d.effect_suspend,
                };
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
        env[*env_len] = WordEntry {
            name: name_atom,
            sig,
            may_suspend: d.effect_suspend,
        };
        *env_len += 1;
    }
    }
    Ok(())
}

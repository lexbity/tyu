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
            if bytes == b"--emit=asm" {
                emit_asm = true;
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

    let emit_count = (emit_ast as u8) + (emit_ir as u8) + (emit_asm as u8);
    if emit_count > 1 {
        let _ = diag::error_simple(1005, b"choose a single --emit=...");
        return 2;
    }
    if emit_count == 0 {
        let _ = diag::error_simple(1001, b"not implemented: use --emit=ast, --emit=ir, or --emit=asm");
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

    if emit_ir {
        return emit_ir_driver(&module, src, &search_dirs[..search_len], checks, &mut out);
    }
    emit_asm_driver(&module, src, &search_dirs[..search_len], checks, &mut out)
}

fn emit_ir_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
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
        &mut sem_out,
    ) {
        Ok(()) => 0,
        Err(e) => {
            let _ = diag::error_simple(e.code, b"typecheck error");
            2
        }
    }
}

const HELP: &[u8] = b"langc (tyu_lang) v0.1.0\n\nUSAGE:\n  langc [options] <file.mod|file.def>\n\nOPTIONS:\n  --help, -h              Print help\n  --emit=ast              Parse and dump AST (v1 milestone 1)\n  --emit=ir               Typecheck and dump IR (v1 milestone 3)\n  --emit=asm              Emit FASM ELF64 assembly (v1 milestone 7)\n  -I <path>                Add include path (v1 milestone 2)\n  --checks=off|contracts|all  Checks insertion mode (v1 milestone 4)\n  --sysroot=<path>         Sysroot root (stub)\n  --out-dir=<path>         Output directory (stub)\n  --target=<triple>        Target triple (stub)\n\n";

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

fn emit_asm_driver(module: &ModuleAst, src: &[u8], search_dirs: &[&[u8]], checks: ChecksMode, out: &mut Stdout) -> i32 {
    // For now, `--emit=asm` is a direct lowering from source after typechecking.
    // Keep the compiler conservative: require typecheck succeeds before emitting asm.

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

    struct NullOut;
    impl semantics::typecheck::Output for NullOut {
        fn write(&mut self, _bytes: &[u8]) {}
    }
    let mut null = NullOut;
    if let Err(e) = semantics::typecheck::emit_ir(module, src, &env[..env_len], &st_buf[..st_len], checks, &mut null) {
        let _ = diag::error_simple(e.code, b"typecheck error");
        return 2;
    }

    let mut gen = AsmGen::new(module, src, out);
    match gen.emit_module() {
        Ok(()) => 0,
        Err(code) => {
            let _ = diag::error_simple(code, b"asm emission error");
            2
        }
    }
}

struct AsmGen<'a> {
    module: &'a ModuleAst,
    src: &'a [u8],
    out: &'a mut Stdout,
    label_id: u32,
}

impl<'a> AsmGen<'a> {
    fn new(module: &'a ModuleAst, src: &'a [u8], out: &'a mut Stdout) -> Self {
        Self {
            module,
            src,
            out,
            label_id: 0,
        }
    }

    fn emit_module(&mut self) -> Result<(), u32> {
        self.out.write(b"format ELF64 executable\n");
        self.out.write(b"entry start\n\n");

        self.out.write(b"segment readable executable\n");
        self.out.write(b"start:\n");
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
            // Pop exit code (i64) and exit with low 8 bits.
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
        emit_stack_overflow(self.out);

        // Emit words.
        for decl in self.module.decls.iter() {
            if decl.kind != DeclKind::Word {
                continue;
            }
            let name = slice_span(self.src, decl.name);
            if decl.body.is_none() {
                continue;
            }
            self.emit_word(name, decl)?;
        }

        self.out.write(b"\nsegment readable writeable\n");
        self.out.write(b"__lang_ds_base rb 65536\n");
        self.out.write(b"__lang_ds_limit:\n");
        Ok(())
    }

    fn emit_word(&mut self, name: &[u8], decl: &DeclAst) -> Result<(), u32> {
        self.out.write(b"\n");
        write_label(self.out, name);
        self.out.write(b":\n");

        let locals = scan_locals(self.src, decl.body.unwrap())?;
        let end_id = self.fresh_label();
        let locals_bytes = locals_stack_bytes(&locals);
        if locals.len > 0 {
            self.out.write(b"  sub rsp, ");
            write_u32(self.out, locals_bytes);
            self.out.write(b"\n");
        }

        self.emit_block(decl.body.unwrap(), &locals, Some(end_id))?;

        self.out.write(b".endword_");
        write_u32(self.out, end_id);
        self.out.write(b":\n");

        if locals.len > 0 {
            self.out.write(b"  add rsp, ");
            write_u32(self.out, locals_bytes);
            self.out.write(b"\n");
        }
        self.out.write(b"  ret\n");
        Ok(())
    }

    fn emit_block(&mut self, span: Span, locals: &Locals, end_label: Option<u32>) -> Result<(), u32> {
        let slice = &self.src[span.start..span.end];
        let mut lex = Lexer::new(slice);

        let mut quote_stack: [Span; 64] = [Span::new(0, 0); 64];
        let mut quote_sp = 0usize;

        loop {
            let tok = lex.next();
            if tok.kind == TokenKind::Eof {
                break;
            }
            match tok.kind {
                TokenKind::Number => {
                    let n = parse_i64_any(&slice[tok.span.start..tok.span.end]).ok_or(7002u32)?;
                    emit_push_i64(self.out, n);
                }
                TokenKind::Ident => {
                    let name = &slice[tok.span.start..tok.span.end];
                    if name == b"true" {
                        emit_push_i64(self.out, 1);
                        continue;
                    }
                    if name == b"false" {
                        emit_push_i64(self.out, 0);
                        continue;
                    }
                    if name == b"dup" {
                        emit_dup(self.out);
                        continue;
                    }
                    if name == b"drop" {
                        emit_drop(self.out);
                        continue;
                    }
                    if name == b"swap" {
                        emit_swap(self.out);
                        continue;
                    }
                    if name == b"+" {
                        emit_binop(self.out, b"add");
                        continue;
                    }
                    if name == b"-" {
                        emit_binop(self.out, b"sub");
                        continue;
                    }
                    if name == b"*" {
                        emit_binop(self.out, b"imul");
                        continue;
                    }
                    if name == b">" {
                        emit_cmp(self.out, b"setg");
                        continue;
                    }
                    if name == b"<" {
                        emit_cmp(self.out, b"setl");
                        continue;
                    }
                    if name == b">=" {
                        emit_cmp(self.out, b"setge");
                        continue;
                    }
                    if name == b"<=" {
                        emit_cmp(self.out, b"setle");
                        continue;
                    }
                    if name == b"==" {
                        emit_cmp(self.out, b"sete");
                        continue;
                    }
                    if name == b"!=" {
                        emit_cmp(self.out, b"setne");
                        continue;
                    }
                    if name == b"return" {
                        if let Some(end_id) = end_label {
                            self.out.write(b"  jmp .endword_");
                            write_u32(self.out, end_id);
                            self.out.write(b"\n");
                        } else {
                            self.out.write(b"  ret\n");
                        }
                        continue;
                    }
                    if name == b"if" {
                        if quote_sp < 2 {
                            return Err(7003);
                        }
                        let else_span = quote_stack[quote_sp - 1];
                        let then_span = quote_stack[quote_sp - 2];
                        quote_sp -= 2;
                        self.emit_if(then_span, else_span, locals, end_label)?;
                        continue;
                    }
                    if name == b"while" {
                        if quote_sp < 2 {
                            return Err(7004);
                        }
                        let body_span = quote_stack[quote_sp - 1];
                        let cond_span = quote_stack[quote_sp - 2];
                        quote_sp -= 2;
                        self.emit_while(cond_span, body_span, locals, end_label)?;
                        continue;
                    }
                    if name == b"loop" {
                        if quote_sp < 1 {
                            return Err(7005);
                        }
                        let body_span = quote_stack[quote_sp - 1];
                        quote_sp -= 1;
                        self.emit_loop(body_span, locals, end_label)?;
                        continue;
                    }
                    if name == b"lock" {
                        if quote_sp < 1 {
                            return Err(7006);
                        }
                        let body_span = quote_stack[quote_sp - 1];
                        quote_sp -= 1;
                        self.emit_block(body_span, locals, end_label)?;
                        continue;
                    }

                    if let Some(idx) = locals.find(name) {
                        emit_load_local(self.out, idx);
                        continue;
                    }

                    self.out.write(b"  call ");
                    write_label(self.out, name);
                    self.out.write(b"\n");
                }
                TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                    let name = &slice[tok.span.start..tok.span.end];
                    match name {
                        b">=" => emit_cmp(self.out, b"setge"),
                        b"<=" => emit_cmp(self.out, b"setle"),
                        b"==" => emit_cmp(self.out, b"sete"),
                        b"!=" => emit_cmp(self.out, b"setne"),
                        _ => return Err(7007),
                    }
                }
                TokenKind::PunctArrowBind => {
                    let name_tok = lex.next();
                    if name_tok.kind != TokenKind::Ident {
                        return Err(7008);
                    }
                    let lname = &slice[name_tok.span.start..name_tok.span.end];
                    let idx = locals.find(lname).ok_or(7009u32)?;
                    emit_store_local(self.out, idx);
                }
                TokenKind::PunctLBracket => {
                    let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)?;
                    if quote_sp >= quote_stack.len() {
                        return Err(7010);
                    }
                    // Store the *inner* span (exclude the surrounding '[' and ']').
                    if q.end <= q.start + 2 {
                        quote_stack[quote_sp] = Span::new(span.start + q.start + 1, span.start + q.start + 1);
                    } else {
                        quote_stack[quote_sp] = Span::new(span.start + q.start + 1, span.start + q.end - 1);
                    }
                    quote_sp += 1;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn fresh_label(&mut self) -> u32 {
        let id = self.label_id;
        self.label_id = self.label_id.wrapping_add(1);
        id
    }

    fn emit_if(&mut self, then_span: Span, else_span: Span, locals: &Locals, end_label: Option<u32>) -> Result<(), u32> {
        let id = self.fresh_label();
        self.out.write(b"  sub r15, 8\n");
        self.out.write(b"  mov rax, [r15]\n");
        self.out.write(b"  cmp rax, 0\n");
        self.out.write(b"  je .else_");
        write_u32(self.out, id);
        self.out.write(b"\n");
        self.emit_block(then_span, locals, end_label)?;
        self.out.write(b"  jmp .endif_");
        write_u32(self.out, id);
        self.out.write(b"\n");
        self.out.write(b".else_");
        write_u32(self.out, id);
        self.out.write(b":\n");
        self.emit_block(else_span, locals, end_label)?;
        self.out.write(b".endif_");
        write_u32(self.out, id);
        self.out.write(b":\n");
        Ok(())
    }

    fn emit_while(&mut self, cond_span: Span, body_span: Span, locals: &Locals, end_label: Option<u32>) -> Result<(), u32> {
        let id = self.fresh_label();
        self.out.write(b".while_");
        write_u32(self.out, id);
        self.out.write(b":\n");
        self.emit_block(cond_span, locals, end_label)?;
        self.out.write(b"  sub r15, 8\n");
        self.out.write(b"  mov rax, [r15]\n");
        self.out.write(b"  cmp rax, 0\n");
        self.out.write(b"  je .endwhile_");
        write_u32(self.out, id);
        self.out.write(b"\n");
        self.emit_block(body_span, locals, end_label)?;
        self.out.write(b"  jmp .while_");
        write_u32(self.out, id);
        self.out.write(b"\n");
        self.out.write(b".endwhile_");
        write_u32(self.out, id);
        self.out.write(b":\n");
        Ok(())
    }

    fn emit_loop(&mut self, body_span: Span, locals: &Locals, end_label: Option<u32>) -> Result<(), u32> {
        let id = self.fresh_label();
        self.out.write(b".loop_");
        write_u32(self.out, id);
        self.out.write(b":\n");
        self.emit_block(body_span, locals, end_label)?;
        self.out.write(b"  jmp .loop_");
        write_u32(self.out, id);
        self.out.write(b"\n");
        Ok(())
    }
}

struct Locals {
    len: usize,
    names: [TypeAtom; 64],
}

impl Locals {
    fn empty() -> Self {
        Self {
            len: 0,
            names: [TypeAtom::new(b"").unwrap(); 64],
        }
    }

    fn find(&self, name: &[u8]) -> Option<u32> {
        let atom = TypeAtom::new(name)?;
        for i in 0..self.len {
            if self.names[i] == atom {
                return Some(i as u32);
            }
        }
        None
    }
}

fn scan_locals(src: &[u8], body: Span) -> Result<Locals, u32> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    let mut out = Locals::empty();
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        match tok.kind {
            TokenKind::PunctLBracket => {
                let _ = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)?;
            }
            TokenKind::PunctArrowBind => {
                let name = lex.next();
                if name.kind != TokenKind::Ident {
                    return Err(7011);
                }
                let bytes = &slice[name.span.start..name.span.end];
                let atom = TypeAtom::new(bytes).ok_or(7012u32)?;
                if out.find(bytes).is_some() {
                    continue;
                }
                if out.len >= out.names.len() {
                    return Err(7013);
                }
                out.names[out.len] = atom;
                out.len += 1;
            }
            _ => {}
        }
    }
    Ok(out)
}

fn locals_stack_bytes(locals: &Locals) -> u32 {
    if locals.len == 0 {
        return 0;
    }
    let mut bytes = (locals.len as u32) * 8;
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

fn write_label(out: &mut Stdout, name: &[u8]) {
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

fn write_u32(out: &mut Stdout, mut v: u32) {
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

fn parse_i64_any(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let mut sign = 1i64;
    if bytes[0] == b'-' {
        sign = -1;
        i = 1;
    }
    let mut base = 10i64;
    if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
        base = 16;
        i += 2;
    }
    let mut v: i64 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'_' {
            i += 1;
            continue;
        }
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as i64,
            b'a'..=b'f' if base == 16 => 10 + (b - b'a') as i64,
            b'A'..=b'F' if base == 16 => 10 + (b - b'A') as i64,
            _ => return None,
        };
        v = v.checked_mul(base)?;
        v = v.checked_add(digit)?;
        i += 1;
    }
    Some(v * sign)
}

fn capture_balanced(lex: &mut Lexer<'_>, slice: &[u8], open: TokenKind, close: TokenKind, open_start: usize) -> Result<Span, u32> {
    let mut depth = 1usize;
    let mut end = open_start + 1;
    while depth > 0 {
        let t = lex.next();
        if t.kind == TokenKind::Eof {
            return Err(7014);
        }
        end = t.span.end;
        if t.kind == open {
            depth += 1;
        } else if t.kind == close {
            depth -= 1;
        } else if t.kind == TokenKind::String {
            let _ = slice;
        }
    }
    Ok(Span::new(open_start, end))
}

fn emit_push_i64(out: &mut Stdout, v: i64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    emit_i64(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

fn emit_i64(out: &mut Stdout, mut v: i64) {
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

fn emit_dup(out: &mut Stdout) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_drop(out: &mut Stdout) {
    out.write(b"  sub r15, 8\n");
}

fn emit_swap(out: &mut Stdout) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  mov rcx, [r15-16]\n");
    out.write(b"  mov [r15-8], rcx\n");
    out.write(b"  mov [r15-16], rax\n");
}

fn emit_binop(out: &mut Stdout, op: &[u8]) {
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

fn emit_cmp(out: &mut Stdout, setcc: &[u8]) {
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

fn emit_store_local(out: &mut Stdout, idx: u32) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  mov [rsp+");
    write_u32(out, idx * 8);
    out.write(b"], rax\n");
}

fn emit_load_local(out: &mut Stdout, idx: u32) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov rax, [rsp+");
    write_u32(out, idx * 8);
    out.write(b"]\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_stack_overflow(out: &mut Stdout) {
    out.write(b"__stack_overflow:\n");
    out.write(b"  mov rdi, 10\n");
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

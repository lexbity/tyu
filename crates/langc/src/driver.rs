use frontend::{
    parse::{DeclKind, ModuleAst, Output, Parser},
};
use hosted::{diag, fs, process};
use semantics::types::{TypeAtom, WordEntry, WordSig};
use semantics::typecheck::{self, ChecksMode, SubtypeInfo};
use crate::codegen::{IrAsmGen, AsmMode};
use crate::util::{
    Stdout, MemOut, slice_span, join_path, try_load_module_file,
};
use crate::iface::{find_decl, export_iter, find_word_decl};

pub fn emit_ir_driver(
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

pub fn emit_asm_driver(
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

pub fn emit_tc_driver(
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

pub fn emit_obj_driver(
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
            let sig = typecheck::parse_word_sig(def_src.as_slice(), sig_span)
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
        let sig = typecheck::parse_word_sig(src, sig_span).map_err(|_| 2219u32)?;
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

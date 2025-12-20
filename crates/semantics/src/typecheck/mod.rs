pub mod error;
pub mod value;
pub mod parse;
pub mod util;
pub mod mmio;
pub mod db;
pub mod stackcheck;
pub mod irgen;

// Re-export public API types and functions
pub use crate::typecheck::error::{TcError, ChecksMode, Output};
pub use crate::typecheck::db::SubtypeInfo;
pub use crate::typecheck::parse::parse_word_sig;

use crate::types::WordEntry;
use crate::typecheck::db::{build_resource_db, build_nominal_db, build_iso_db};
use crate::typecheck::mmio::build_mmio_db;
use crate::typecheck::irgen::{build_ir_word, lir_atom_lossy, IrWordOutput, reset_word_arena};
use crate::typecheck::stackcheck::typecheck_word_body;
use crate::typecheck::util::{slice_span, write_sig};
use frontend::parse::{DeclKind, ModuleAst};
use frontend::fixed::FixedVec;
use ir as lir;

pub fn emit_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src)?;
    let resources = build_resource_db(module, src)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;
    reset_word_arena();

    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");

    struct IrOut<'a, O: Output>(&'a mut O);
    impl<'a, O: Output> lir::Output for IrOut<'a, O> {
        fn write(&mut self, bytes: &[u8]) {
            self.0.write(bytes)
        }
    }
    let mut ir_out = IrOut(out);

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError { code: 3200, span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        if decl.body.is_none() {
            ir_out.0.write(b"word ");
            ir_out.0.write(lir_atom_lossy(slice_span(src, decl.name)).as_bytes());
            ir_out.0.write(b" ");
            // Keep legacy behavior: declarations without bodies don't need IR blocks.
            // Still show the signature so `--emit=ir` is useful on `.def` files.
            {
                // Minimal signature printer for the IR dump.
                ir_out.0.write(b"( ");
                for i in 0..(sig.in_len as usize) {
                    if i != 0 {
                        ir_out.0.write(b" ");
                    }
                    ir_out.0.write(sig.inputs[i].as_bytes());
                }
                ir_out.0.write(b" --");
                if sig.out_len > 0 {
                    ir_out.0.write(b" ");
                }
                for i in 0..(sig.out_len as usize) {
                    if i != 0 {
                        ir_out.0.write(b" ");
                    }
                    ir_out.0.write(sig.outputs[i].as_bytes());
                }
                ir_out.0.write(b" )\n");
            }
            continue;
        }

        let out_words = build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)?;
        lir::verify_word(out_words.word).map_err(|e| TcError { code: e.code, span: e.span })?;
        lir::write_word(&mut ir_out, out_words.word);
        for w in out_words.extra_words.iter() {
            lir::verify_word(*w).map_err(|e| TcError { code: e.code, span: e.span })?;
            lir::write_word(&mut ir_out, *w);
        }
    }
    Ok(())
}

pub fn emit_stackcheck(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src)?;
    let nominals = build_nominal_db(module, src)?;
    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            continue;
        };
        let Some(body_span) = decl.body else {
            continue;
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        out.write(b"word ");
        out.write(slice_span(src, decl.name));
        out.write(b" ");
        write_sig(out, &sig);
        out.write(b"\n");
        typecheck_word_body(out, src, body_span, &sig, env, subtypes, &mmio, &nominals, checks, true)?;
    }
    Ok(())
}

pub fn build_ir_words(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
) -> Result<FixedVec<&'static lir::Word, 256>, TcError> {
    let mmio = build_mmio_db(module, src)?;
    let resources = build_resource_db(module, src)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;
    reset_word_arena();
    let mut out: FixedVec<&'static lir::Word, 256> = FixedVec::new();

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        if decl.body.is_none() {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError { code: 3200, span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        let out_words = build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)?;
        let IrWordOutput { word, extra_words } = out_words;
        lir::verify_word(word).map_err(|e| TcError { code: e.code, span: e.span })?;
        out.push(word).map_err(|_| TcError { code: 3901, span: decl.name })?;
        for w in extra_words.into_iter() {
            lir::verify_word(w).map_err(|e| TcError { code: e.code, span: e.span })?;
            out.push(w).map_err(|_| TcError { code: 3901, span: decl.name })?;
        }
    }
    Ok(out)
}

pub enum ForEachIrError<E> {
    Type(TcError),
    Consumer(E),
}

pub fn for_each_ir_word<E, F>(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    mut f: F,
) -> Result<(), ForEachIrError<E>>
where
    F: FnMut(&lir::Word) -> Result<(), E>,
{
    let mmio = build_mmio_db(module, src).map_err(ForEachIrError::Type)?;
    let resources = build_resource_db(module, src).map_err(ForEachIrError::Type)?;
    let nominals = build_nominal_db(module, src).map_err(ForEachIrError::Type)?;
    let iso = build_iso_db(module, src).map_err(ForEachIrError::Type)?;
    reset_word_arena();

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        if decl.body.is_none() {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(ForEachIrError::Type(TcError { code: 3200, span: decl.name }));
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span }).map_err(ForEachIrError::Type)?;
        let out_words =
            build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)
                .map_err(ForEachIrError::Type)?;
        lir::verify_word(out_words.word)
            .map_err(|e| TcError { code: e.code, span: e.span })
            .map_err(ForEachIrError::Type)?;
        f(out_words.word).map_err(ForEachIrError::Consumer)?;
        for w in out_words.extra_words.iter() {
            lir::verify_word(*w)
                .map_err(|e| TcError { code: e.code, span: e.span })
                .map_err(ForEachIrError::Type)?;
            f(*w).map_err(ForEachIrError::Consumer)?;
        }
    }
    Ok(())
}

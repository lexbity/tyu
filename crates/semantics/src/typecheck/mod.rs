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
pub use crate::typecheck::stackcheck::typecheck_word_body;

use crate::types::WordEntry;
use crate::typecheck::db::{build_resource_db, build_nominal_db, build_iso_db};
use crate::typecheck::mmio::build_mmio_db;
use crate::typecheck::irgen::{build_ir_word, lir_atom};
use crate::typecheck::util::{slice_span, write_sig};
use frontend::parse::{DeclKind, ModuleAst};
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
    let mut arena = irgen::arena::ArenaAllocator::new();

    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError { code: 3200, span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        if decl.body.is_none() {
            out.write(b"word ");
            out.write(lir_atom(slice_span(src, decl.name))?.as_bytes());
            out.write(b" ");
            // Keep legacy behavior: declarations without bodies don't need IR blocks.
            // Still show the signature so `--emit=ir` is useful on `.def` files.
            {
                // Minimal signature printer for the IR dump.
                out.write(b"( ");
                for i in 0..(sig.in_len as usize) {
                    if i != 0 {
                        out.write(b" ");
                    }
                    out.write(sig.inputs[i].as_bytes());
                }
                out.write(b" --");
                if sig.out_len > 0 {
                    out.write(b" ");
                }
                for i in 0..(sig.out_len as usize) {
                    if i != 0 {
                        out.write(b" ");
                    }
                    out.write(sig.outputs[i].as_bytes());
                }
                out.write(b" )\n");
            }
            continue;
        }

        let out_words = build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig, &mut arena)?;
        lir::verify_word(out_words.word).map_err(|e| TcError { code: e.code, span: e.span })?;
        lir::write_word(out, out_words.word);
        for w in out_words.extra_words.iter() {
            lir::verify_word(w).map_err(|e| TcError { code: e.code, span: e.span })?;
            lir::write_word(out, w);
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
    let mut arena = irgen::arena::ArenaAllocator::new();

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
            build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig, &mut arena)
                .map_err(ForEachIrError::Type)?;
        lir::verify_word(out_words.word)
            .map_err(|e| TcError { code: e.code, span: e.span })
            .map_err(ForEachIrError::Type)?;
        f(out_words.word).map_err(ForEachIrError::Consumer)?;
        for w in out_words.extra_words.iter() {
            lir::verify_word(w)
                .map_err(|e| TcError { code: e.code, span: e.span })
                .map_err(ForEachIrError::Type)?;
            f(w).map_err(ForEachIrError::Consumer)?;
        }
    }
    Ok(())
}

pub mod builtins;
pub mod context;
pub mod db;
pub mod error;
pub mod irgen;
pub mod mmio;
pub mod parse;
pub mod place;
pub mod util;
pub mod value;

// Re-export public API types and functions
pub use crate::typecheck::builtins::builtin_words;
pub use crate::typecheck::db::SubtypeInfo;
pub use crate::typecheck::error::{ChecksMode, Output, TcError};
pub use crate::typecheck::parse::parse_word_sig;

use crate::typecheck::db::{
    build_iso_db, build_nominal_db, build_resource_db, compute_resource_sharing, IsoDb, NominalDb,
    ResourceDb,
};
use crate::typecheck::irgen::{build_ir_word, lir_atom, NullObserver};
use crate::typecheck::mmio::{build_mmio_db, MmioDb};
use crate::typecheck::util::{slice_span, write_sig};
use crate::types::WordEntry;
use alloc::vec::Vec;
use frontend::fixed::FixedVec;
use frontend::parse::{DeclKind, ModuleAst};
use frontend::span::Span;
use ir as lir;

/// A `Vec<u8>`-backed `Output` for buffering `--emit=ir` word text.
struct VecOut(Vec<u8>);
impl frontend::parse::Output for VecOut {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

pub fn emit_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src, descriptor)?;
    let mut resources = build_resource_db(module, src)?;
    // Compute resource sharing from ISR roots before compiling any word body.
    compute_resource_sharing(module, src, &mut resources)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;
    let mut arena = irgen::arena::ArenaAllocator::new();
    let summary_env = local_summary_env(
        module,
        src,
        env,
        subtypes,
        &mmio,
        descriptor,
        &resources,
        &nominals,
        &iso,
        checks,
        allow_raw_casts,
    )?;

    // P4: buffer the word text and collect the module's window-use union so
    // the `format_ver`/`module`/`windows` header precedes the words, exactly
    // matching `ir::write_module`.
    let mut words_buf = VecOut(Vec::new());
    let mut windows: FixedVec<lir::WindowUse, 8> = FixedVec::new();

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError::NoSig { span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span)
            .map_err(|_| TcError::TypeParseFailed { span: sig_span })?;
        if decl.body.is_none() {
            words_buf.write(b"word ");
            words_buf.write(lir_atom(slice_span(src, decl.name))?.as_bytes());
            words_buf.write(b" ");
            // Keep legacy behavior: declarations without bodies don't need IR blocks.
            // Still show the signature so `--emit=ir` is useful on `.def` files.
            {
                // Minimal signature printer for the IR dump.
                words_buf.write(b"( ");
                for i in 0..(sig.in_len as usize) {
                    if i != 0 {
                        words_buf.write(b" ");
                    }
                    words_buf.write(sig.inputs[i].as_bytes());
                }
                words_buf.write(b" --");
                if sig.out_len > 0 {
                    words_buf.write(b" ");
                }
                for i in 0..(sig.out_len as usize) {
                    if i != 0 {
                        words_buf.write(b" ");
                    }
                    words_buf.write(sig.outputs[i].as_bytes());
                }
                words_buf.write(b" )\n");
            }
            continue;
        }

        let mut null_obs = NullObserver;
        arena.reset();
        let out_words = build_ir_word(
            decl,
            src,
            &summary_env,
            subtypes,
            &mmio,
            descriptor,
            &resources,
            &nominals,
            &iso,
            checks,
            allow_raw_casts,
            &sig,
            &mut arena,
            &mut null_obs,
        )?;
        lir::verify_word(out_words.word).map_err(|e| TcError::InternalError {
            code: e.code(),
            span: e.span(),
        })?;
        merge_windows(&mut windows, out_words.word);
        lir::write_word(&mut words_buf, out_words.word);
        for w in out_words.extra_words.iter() {
            lir::verify_word(w).map_err(|e| TcError::InternalError {
                code: e.code(),
                span: e.span(),
            })?;
            merge_windows(&mut windows, w);
            lir::write_word(&mut words_buf, w);
        }
    }

    // Header: format_ver, module, windows — then the buffered words.
    out.write(b"format_ver ");
    write_u32(out, lir::FORMAT_VER);
    out.write(b"\n");
    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");
    write_windows(out, &windows);
    out.write(words_buf.0.as_slice());
    Ok(())
}

/// Union `w`'s window-use entries into the module table.
fn merge_windows(module_windows: &mut FixedVec<lir::WindowUse, 8>, w: &lir::Word) {
    for wu in w.windows.iter() {
        if let Some(existing) = module_windows.iter_mut().find(|e| e.id == wu.id) {
            existing.access_mask |= wu.access_mask;
        } else {
            let _ = module_windows.push(*wu);
        }
    }
}

/// Emit the `windows N` section (design doc §5.4).
fn write_windows(out: &mut impl Output, windows: &FixedVec<lir::WindowUse, 8>) {
    out.write(b"windows ");
    write_u32(out, windows.len() as u32);
    out.write(b"\n");
    for wu in windows.iter() {
        out.write(b"window ");
        write_u32(out, wu.id as u32);
        out.write(b" ");
        out.write(wu.name.as_bytes());
        out.write(b" ");
        out.write(match wu.kind {
            lir::WindowKind::Bus => b"bus",
            lir::WindowKind::Emulated => b"emulated",
        });
        out.write(b" ");
        match wu.base {
            Some(base) => {
                out.write(b"0x");
                write_u64_hex(out, base);
            }
            None => out.write(b"link"),
        }
        out.write(b" ");
        out.write(b"0x");
        write_u32_hex(out, wu.size);
        out.write(b"\n");
    }
}

fn write_u32(out: &mut impl Output, mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    buf[..n].reverse();
    out.write(&buf[..n]);
}

fn write_u64_hex(out: &mut impl Output, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        let d = (v & 0xF) as u8;
        buf[n] = match d {
            0..=9 => b'0' + d,
            _ => b'a' + (d - 10),
        };
        n += 1;
        v >>= 4;
    }
    buf[..n].reverse();
    out.write(&buf[..n]);
}

fn write_u32_hex(out: &mut impl Output, mut v: u32) {
    let mut buf = [0u8; 8];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        let d = (v & 0xF) as u8;
        buf[n] = match d {
            0..=9 => b'0' + d,
            _ => b'a' + (d - 10),
        };
        n += 1;
        v >>= 4;
    }
    buf[..n].reverse();
    out.write(&buf[..n]);
}

pub fn emit_stackcheck(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src, descriptor)?;
    let mut resources = build_resource_db(module, src)?;
    compute_resource_sharing(module, src, &mut resources)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;
    let mut arena = irgen::arena::ArenaAllocator::new();
    let allow_raw_casts = false;
    let summary_env = local_summary_env(
        module,
        src,
        env,
        subtypes,
        &mmio,
        descriptor,
        &resources,
        &nominals,
        &iso,
        checks,
        allow_raw_casts,
    )?;

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            continue;
        };
        if decl.body.is_none() {
            continue;
        }
        let sig = parse_word_sig(src, sig_span)
            .map_err(|_| TcError::TypeParseFailed { span: sig_span })?;
        out.write(b"word ");
        out.write(slice_span(src, decl.name));
        out.write(b" ");
        write_sig(out, &sig);
        out.write(b"\n");
        {
            let mut obs = irgen::StackcheckObserver { out: &mut *out };
            arena.reset();
            let _ = build_ir_word(
                decl,
                src,
                &summary_env,
                subtypes,
                &mmio,
                descriptor,
                &resources,
                &nominals,
                &iso,
                checks,
                allow_raw_casts,
                &sig,
                &mut arena,
                &mut obs,
            )
            .map_err(|e| TcError::InternalError {
                code: e.code(),
                span: e.span(),
            })?;
        }
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
    resources: &mut ResourceDb,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    mut f: F,
) -> Result<(), ForEachIrError<E>>
where
    F: FnMut(&lir::Word) -> Result<(), E>,
{
    let mmio = build_mmio_db(module, src, descriptor).map_err(ForEachIrError::Type)?;
    let nominals = build_nominal_db(module, src).map_err(ForEachIrError::Type)?;
    let iso = build_iso_db(module, src).map_err(ForEachIrError::Type)?;
    // Compute resource sharing from ISR roots before compiling any word body.
    compute_resource_sharing(module, src, resources).map_err(ForEachIrError::Type)?;
    let mut arena = irgen::arena::ArenaAllocator::new();
    let summary_env = local_summary_env(
        module,
        src,
        env,
        subtypes,
        &mmio,
        descriptor,
        resources,
        &nominals,
        &iso,
        checks,
        allow_raw_casts,
    )
    .map_err(ForEachIrError::Type)?;

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        if decl.body.is_none() {
            continue;
        }
        arena.reset();
        let Some(sig_span) = decl.sig else {
            return Err(ForEachIrError::Type(TcError::NoSig { span: decl.name }));
        };
        let sig = parse_word_sig(src, sig_span)
            .map_err(|e| TcError::InternalError {
                code: e.code,
                span: e.span,
            })
            .map_err(ForEachIrError::Type)?;
        let mut null_obs = NullObserver;
        let out_words = build_ir_word(
            decl,
            src,
            &summary_env,
            subtypes,
            &mmio,
            descriptor,
            resources,
            &nominals,
            &iso,
            checks,
            allow_raw_casts,
            &sig,
            &mut arena,
            &mut null_obs,
        )
        .map_err(ForEachIrError::Type)?;
        lir::verify_word(out_words.word)
            .map_err(|e| TcError::InternalError {
                code: e.code(),
                span: e.span(),
            })
            .map_err(ForEachIrError::Type)?;
        f(out_words.word).map_err(ForEachIrError::Consumer)?;
        for w in out_words.extra_words.iter() {
            lir::verify_word(w)
                .map_err(|e| TcError::InternalError {
                    code: e.code(),
                    span: e.span(),
                })
                .map_err(ForEachIrError::Type)?;
            f(w).map_err(ForEachIrError::Consumer)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn local_summary_env(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    resources: &ResourceDb,
    nominals: &NominalDb,
    iso: &IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
) -> Result<Vec<WordEntry>, TcError> {
    let mut summary_env: Vec<WordEntry> = env.iter().copied().collect();
    let mut arena = irgen::arena::ArenaAllocator::new();
    let max_passes = module.decls.len().saturating_add(1);

    for _ in 0..max_passes {
        let mut changed = false;
        for decl in module.decls.iter() {
            if decl.kind != DeclKind::Word || decl.body.is_none() {
                continue;
            }
            let Some(sig_span) = decl.sig else {
                return Err(TcError::NoSig { span: decl.name });
            };
            let sig = parse_word_sig(src, sig_span)
                .map_err(|_| TcError::TypeParseFailed { span: sig_span })?;

            arena.reset();
            let mut null_obs = NullObserver;
            let out_words = build_ir_word(
                decl,
                src,
                &summary_env,
                subtypes,
                mmio,
                descriptor,
                resources,
                nominals,
                iso,
                checks,
                allow_raw_casts,
                &sig,
                &mut arena,
                &mut null_obs,
            )?;

            let name = slice_span(src, decl.name);
            if merge_word_summary(&mut summary_env, name, out_words.word) {
                changed = true;
            }
        }

        if !changed {
            return Ok(summary_env);
        }
    }

    Err(TcError::InternalError {
        code: 3908,
        span: Span::UNKNOWN,
    })
}

fn merge_word_summary(env: &mut [WordEntry], name: &[u8], word: &lir::Word) -> bool {
    let Some(entry) = env.iter_mut().find(|entry| entry.name.as_bytes() == name) else {
        return false;
    };

    let performs = entry.performs.union(word.performs);
    let bound = lir::StackBound {
        net: word.bound.net,
        high: max_high(entry.bound.high, word.bound.high),
    };
    let changed = entry.performs != performs || entry.bound != bound;
    if changed {
        entry.performs = performs;
        entry.bound = bound;
    }
    changed
}

fn max_high(a: lir::High, b: lir::High) -> lir::High {
    match (a, b) {
        (lir::High::Top, _) | (_, lir::High::Top) => lir::High::Top,
        (lir::High::Slots(a), lir::High::Slots(b)) => lir::High::Slots(a.max(b)),
    }
}

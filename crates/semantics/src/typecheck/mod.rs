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
use crate::types::{TypeAtom, WordEntry, WordSig};
use alloc::string::ToString;
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

/// Build the production `ir::Module` for a module AST (PLAN-WASM-1 D-1/FR-14).
///
/// This is the IR-*construction* half of the former conflated `emit_ir`:
/// frontend parser output → verified `ir::Module`, in declaration order. The
/// REPL and any other IR-struct consumer call this and never parse
/// `--emit=ir` text back (DEC-2, FR-2).
///
/// Words declared without a body (`.def`-style decls) are represented as
/// synthetic body-less words: empty blocks, raw signature atoms preserved in
/// `types` so [`render_ir`] reproduces the legacy signature line byte-for-byte
/// (the `--emit=ir` convention that keeps `.def` files useful).
pub fn build_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    target_spec: verifier::target::TargetSpec,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
) -> Result<lir::Module, TcError> {
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
        target_spec,
        &resources,
        &nominals,
        &iso,
        checks,
        allow_raw_casts,
    )?;

    let mut words: Vec<lir::Word> = Vec::new();
    let mut apertures: FixedVec<lir::ApertureUse, 8> = FixedVec::new();

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
            // Legacy behavior: declarations without bodies don't need IR
            // blocks, but keep the signature so `--emit=ir` is useful on
            // `.def` files (the synthetic word renders that line — see
            // `render_bodyless_sig`).
            let name = lir_atom(slice_span(src, decl.name))?;
            push_word(&mut words, bodyless_sig_word(name, sig)?, decl.name)?;
            continue;
        }

        let mut null_obs = NullObserver;
        arena.reset();
        let out_words = build_ir_word(
            target_spec,
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
            None,
            None,
            false,
            false, // proven_gate (tests: off)
            None,  // no bind artifact on the inspection surface
            &[],
            &mut null_obs,
        )?;
        lir::verify_word(out_words.word).map_err(|e| TcError::InternalError {
            code: e.code(),
            span: e.span(),
        })?;
        merge_apertures(&mut apertures, out_words.word);
        push_word(&mut words, out_words.word.clone(), decl.name)?;
        for w in out_words.extra_words.iter() {
            lir::verify_word(w).map_err(|e| TcError::InternalError {
                code: e.code(),
                span: e.span(),
            })?;
            merge_apertures(&mut apertures, w);
            push_word(&mut words, (*w).clone(), decl.name)?;
        }
    }

    Ok(lir::Module {
        name: lir_atom(slice_span(src, module.name))?,
        words,
        apertures,
    })
}

/// Push a word into the module's word table, enforcing the IR's `≤ 64 words/
/// module` format cap (surfaced as a diagnostic, never silently truncated —
/// PLAN-WASM-1 §5).
fn push_word(words: &mut Vec<lir::Word>, w: lir::Word, span: Span) -> Result<(), TcError> {
    if words.len() >= 64 {
        return Err(TcError::InternalError {
            code: 3920, // module word-table full (IR format cap, §5)
            span,
        });
    }
    words.push(w);
    Ok(())
}

/// A synthetic body-less `lir::Word` for a `.def`-style declaration. The raw
/// signature type atoms are preserved (in first-appearance order) in `types`
/// so [`render_ir`] can reproduce the legacy `word NAME ( in -- out )` line
/// byte-for-byte, including channel/array atoms that the general IR renderer
/// would reformat (§Q2 — the text is a pin, not an execution input).
fn bodyless_sig_word(name: lir::Atom, wsig: WordSig) -> Result<lir::Word, TcError> {
    fn resolve(
        types: &mut FixedVec<lir::Atom, 64>,
        atom: &TypeAtom,
    ) -> Result<lir::TypeId, TcError> {
        let bytes = atom.as_bytes();
        if let Some(id) = types.iter().position(|t| t.as_bytes() == bytes) {
            return Ok(lir::TypeId(id as u8));
        }
        let id = types.len();
        types
            .push(lir::Atom::new(bytes).ok_or(TcError::AtomTooLong {
                span: Span::UNKNOWN,
            })?)
            .map_err(|_| TcError::InternalError {
                code: 3921, // module signature type table full
                span: Span::UNKNOWN,
            })?;
        Ok(lir::TypeId(id as u8))
    }

    let mut types: FixedVec<lir::Atom, 64> = FixedVec::new();
    let mut inputs = [lir::TY_EMPTY; 8];
    for (i, a) in wsig.inputs.iter().enumerate().take(wsig.in_len as usize) {
        inputs[i] = resolve(&mut types, a)?;
    }
    let mut outputs = [lir::TY_EMPTY; 8];
    for (i, a) in wsig.outputs.iter().enumerate().take(wsig.out_len as usize) {
        outputs[i] = resolve(&mut types, a)?;
    }
    Ok(lir::Word {
        name,
        sig: lir::Sig {
            in_len: wsig.in_len,
            out_len: wsig.out_len,
            inputs,
            outputs,
        },
        performs: lir::EffectSet::empty(),
        requires: lir::CapSet::empty(),
        bound: lir::StackBound::ID,
        entry: lir::BlockId(0),
        types,
        type_sizes: FixedVec::new(),
        type_classes: FixedVec::new(),
        subtype_bases: FixedVec::new(),
        apertures: FixedVec::new(),
        blocks: FixedVec::new(),
    })
}

/// Render a built module as canonical `--emit=ir` text (PLAN-WASM-1 D-1,
/// DEC-13): the header (byte-pinned to `ir::write_module`) followed by the
/// words. Body-less decls render their legacy raw-signature line; bodied
/// words pass through the canonical `ir::write_word` renderer.
pub fn render_ir(out: &mut impl Output, m: &lir::Module) {
    out.write(b"format_ver ");
    write_u32(out, lir::FORMAT_VER);
    out.write(b"\n");
    out.write(b"module ");
    out.write(m.name.as_bytes());
    out.write(b"\n");
    write_apertures(out, &m.apertures);
    for w in m.words.iter() {
        if w.blocks.is_empty() {
            render_bodyless_sig(out, w);
        } else {
            lir::write_word(out, w);
        }
    }
}

/// The legacy `.def` signature line: `word NAME ( in -- out )` with the RAW
/// type atoms (no `Chan`/`Array` reformatting) — the exact bytes the
/// pre-split `emit_ir` wrote for body-less decls.
fn render_bodyless_sig(out: &mut impl Output, w: &lir::Word) {
    out.write(b"word ");
    out.write(w.name.as_bytes());
    out.write(b" ");
    out.write(b"( ");
    for i in 0..(w.sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        raw_type_atom(out, w, w.sig.inputs[i]);
    }
    out.write(b" --");
    if w.sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(w.sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        raw_type_atom(out, w, w.sig.outputs[i]);
    }
    out.write(b" )\n");
}

fn raw_type_atom(out: &mut impl Output, w: &lir::Word, id: lir::TypeId) {
    if let Some(a) = w.types.get(id.0 as usize) {
        out.write(a.as_bytes());
    }
}

pub fn emit_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    target_spec: verifier::target::TargetSpec,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let built = build_ir(
        module,
        src,
        env,
        subtypes,
        checks,
        allow_raw_casts,
        target_spec,
        descriptor,
    )?;
    render_ir(out, &built);
    Ok(())
}

/// Union `w`'s aperture-use entries into the module table.
fn merge_apertures(module_apertures: &mut FixedVec<lir::ApertureUse, 8>, w: &lir::Word) {
    for wu in w.apertures.iter() {
        if let Some(existing) = module_apertures.iter_mut().find(|e| e.id == wu.id) {
            existing.access_mask |= wu.access_mask;
        } else {
            let _ = module_apertures.push(*wu);
        }
    }
}

/// Emit the `apertures N` section (design doc §5.4).
fn write_apertures(out: &mut impl Output, apertures: &FixedVec<lir::ApertureUse, 8>) {
    out.write(b"apertures ");
    write_u32(out, apertures.len() as u32);
    out.write(b"\n");
    for wu in apertures.iter() {
        out.write(b"aperture ");
        write_u32(out, wu.id as u32);
        out.write(b" ");
        out.write(wu.name.as_bytes());
        out.write(b" ");
        out.write(match wu.kind {
            lir::ApertureKind::Bus => b"bus",
            lir::ApertureKind::Emulated => b"emulated",
        });
        out.write(b" bind=");
        out.write(match wu.bind {
            lir::BindKind::None => b"none",
            lir::BindKind::ArmThumbLdrLiteral => b"arm-thumb-ldr-literal",
            lir::BindKind::RiscVHi20Lo12 => b"riscv-hi20-lo12",
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
    target_spec: verifier::target::TargetSpec,
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
        target_spec,
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
                target_spec,
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
                None,
                None,
                false,
                false, // proven_gate (tests: off)
                None,  // no bind artifact on the inspection surface
                &[],
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
    target_spec: verifier::target::TargetSpec,
    resources: &mut ResourceDb,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    mut extraction: Option<&mut verifier::model::ExtractionCtx>,
    verdicts: Option<&verifier::verdict::Verdicts>,
    keep_contract_checks: bool,
    proven_gate: bool,
    bind_obl: Option<&verifier::model::OblSet>,
    refinements: &[verifier::refinements::Refinement],
    mut f: F,
) -> Result<(), ForEachIrError<E>>
where
    F: FnMut(&lir::Word, Option<&verifier::model::ExtractionCtx>) -> Result<(), E>,
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
        target_spec,
        resources,
        &nominals,
        &iso,
        checks,
        allow_raw_casts,
    )
    .map_err(ForEachIrError::Type)?;

    // Module subtype facts feed the artifact's `facts.subtypes` (P2), in
    // declaration order.
    if let Some(ctx) = extraction.as_deref_mut() {
        for st in subtypes {
            ctx.push_subtype_fact(st.name.as_bytes(), st.min, st.max);
        }
    }

    // Slice P6 (Q7): the module's contract-predicate name set — every word
    // name a `needs`/`ensures` clause of this module references. Used to
    // transcribe named predicates into the artifact's `facts.predicates`.
    let mut predicate_names: alloc::vec::Vec<&[u8]> = alloc::vec::Vec::new();
    for d in module.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        for n in crate::typecheck::util::contract_predicate_names(src, d.requires) {
            if !predicate_names.contains(&n) {
                predicate_names.push(n);
            }
        }
        for n in crate::typecheck::util::contract_predicate_names(src, d.ensures) {
            if !predicate_names.contains(&n) {
                predicate_names.push(n);
            }
        }
    }

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
        // Reset per-word occurrence ordinals and set the current word (P2).
        if let Some(ctx) = extraction.as_deref_mut() {
            ctx.begin_word(slice_span(src, decl.name));
        }
        let out_words = build_ir_word(
            target_spec,
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
            extraction.as_deref_mut(),
            verdicts,
            keep_contract_checks,
            proven_gate,
            bind_obl,
            refinements,
            &mut null_obs,
        )
        .map_err(ForEachIrError::Type)?;
        lir::verify_word(out_words.word)
            .map_err(|e| TcError::InternalError {
                code: e.code(),
                span: e.span(),
            })
            .map_err(ForEachIrError::Type)?;
        // Record the declared word's computed facts (bound, performs, diverge)
        // into the artifact (P2) — from the same final IR word codegen sees.
        if let Some(ctx) = extraction.as_deref_mut() {
            // P1.2: the word's canonical op-text (§6.3) + block count + cycle
            // structure (PLAN-VERIFY-3 §Q9). Every statement of the word
            // relativizes against this text (`word_ir_hash`); the cycles are
            // what a loop-bearing word's statement renderer inducts against.
            let mut ops = VecOut(Vec::new());
            lir::write_word_ops(&mut ops, out_words.word);
            let ir_text = core::str::from_utf8(&ops.0).unwrap_or("").to_string();
            let blocks = out_words.word.blocks.len() as u32;
            ctx.push_word_fact_with_ir(
                slice_span(src, decl.name),
                out_words.word.bound,
                out_words.word.performs,
                &ir_text,
                blocks,
            );
            ctx.attach_word_cycles(verifier::model::compute_cycles(out_words.word));
            // Slice P6 (Q7): named contract predicates transcribe their
            // canonical op-text IR into `facts.predicates` (the compiler-
            // computed record callers and external tools match against —
            // E6413 staleness). Only words the module's contract clauses
            // name; the op-text form is `ir::write_word_ops` (§6.3).
            if predicate_names
                .iter()
                .any(|p| *p == slice_span(src, decl.name))
            {
                let mut ops = VecOut(Vec::new());
                lir::write_word_ops(&mut ops, out_words.word);
                let mut lines: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
                for line in ops.0.split(|&b| b == b'\n') {
                    if line.is_empty() {
                        continue;
                    }
                    lines.push(core::str::from_utf8(line).unwrap_or("?").to_string());
                }
                ctx.push_predicate_fact(slice_span(src, decl.name), Some(lines));
            }
        }
        // P4: the codegen consumer reads the current word's mmio elision state
        // (shared borrow) while the word is current.
        f(out_words.word, extraction.as_deref()).map_err(ForEachIrError::Consumer)?;
        for w in out_words.extra_words.iter() {
            lir::verify_word(w)
                .map_err(|e| TcError::InternalError {
                    code: e.code(),
                    span: e.span(),
                })
                .map_err(ForEachIrError::Type)?;
            f(w, extraction.as_deref()).map_err(ForEachIrError::Consumer)?;
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
    target_spec: verifier::target::TargetSpec,
    resources: &ResourceDb,
    nominals: &NominalDb,
    iso: &IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
) -> Result<Vec<WordEntry>, TcError> {
    let mut summary_env: Vec<WordEntry> = env.to_vec();
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
                target_spec,
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
                None,
                // The fixpoint forecasts per-word summaries: no verdicts, so
                // every check is accounted (conservative — the final pass may
                // skip discharged sites, never the reverse).
                None,
                false,
                false, // proven_gate (tests: off)
                None,  // no bind artifact on the forecast pass
                &[],
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

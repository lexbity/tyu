use crate::codegen::{AsmMode, Backend, CodegenBackend};
use crate::iface::{export_iter, find_decl, find_word_decl};
use crate::util::{
    check_word_for_gate, join_path, slice_span, try_load_module_file, MemOut, Stdout,
};
use alloc::vec::Vec;
use codegen_core::compiled_desc::CompiledDescriptor;
use codegen_core::{FeatureSet, MmioApertureSpec, Target};
use frontend::parse::{DeclKind, ModuleAst, Parser};
use hosted::{diag, fs, process};
use ir::CapSet;
use ir::EffectSet;
use ir::High;
use ir::StackBound;
use lmod::abi_hash;
use lmod::modinfo;
use semantics::typecheck::{self, ChecksMode, SubtypeInfo};
use semantics::types::{TypeAtom, WordEntry, WordSig};
use verifier::model::ExtractionCtx;

struct DriverEnv {
    st_buf: [SubtypeInfo; 64],
    st_len: usize,
    env: [WordEntry; 256],
    env_len: usize,
    builtin_env_end: usize,
    import_env_end: usize,
}

/// Human-readable message for a codegen error code.  Most codegen failures
/// share the generic "asm emission error" text, but E8013 (modinfo too large)
/// must surface the specific "too many exports for modinfo" diagnostic (BUG-006).
fn codegen_error_message(code: u32) -> &'static [u8] {
    match code {
        8013 => b"too many exports for modinfo",
        _ => b"asm emission error",
    }
}

/// Count the emulated-aperture MMIO access ops in a word (P4): one runtime
/// bounds check is emitted per access unless the word elides them, so this is
/// the honest `emitted.mmio_bounds` per-word count (FR-15).
fn count_mmio_ops(w: &ir::Word) -> u32 {
    let mut n: u32 = 0;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            match op.kind {
                ir::OpKind::MmioVolLoad { .. }
                | ir::OpKind::MmioVolStore { .. }
                | ir::OpKind::MmioVolLoadField { .. }
                | ir::OpKind::MmioVolStoreField { .. } => n += 1,
                _ => {}
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::codegen_error_message;

    #[test]
    fn e8013_names_too_many_exports() {
        assert_eq!(codegen_error_message(8013), b"too many exports for modinfo");
    }

    #[test]
    fn other_codegen_codes_stay_generic() {
        assert_eq!(codegen_error_message(8001), b"asm emission error");
        assert_eq!(codegen_error_message(0), b"asm emission error");
    }
}

fn init_env(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    target: Target,
) -> Result<DriverEnv, u32> {
    let mut st_buf: [SubtypeInfo; 64] = [SubtypeInfo {
        name: TypeAtom::EMPTY,
        base: TypeAtom::EMPTY,
        min: 0,
        max: 0,
    }; 64];
    let mut st_len = 0usize;
    for s in module.subtypes.iter() {
        if st_len >= st_buf.len() {
            return Err(2020u32); // too many subtypes (max 64)
        }
        let name = slice_span(src, s.name);
        let base = slice_span(src, s.base);
        let name = TypeAtom::new(name).ok_or(2021u32)?; // subtype name too long
        let base = TypeAtom::new(base).ok_or(2022u32)?; // base type name too long
        st_buf[st_len] = SubtypeInfo {
            name,
            base,
            min: s.min,
            max: s.max,
        };
        st_len += 1;
    }

    let mut env: [WordEntry; 256] = [WordEntry {
        name: TypeAtom::EMPTY,
        sig: WordSig::empty(),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }; 256];
    let mut env_len = 0usize;
    add_builtins(&mut env, &mut env_len, target.spec());
    let builtin_env_end = env_len;
    load_import_sigs(module, src, search_dirs, &mut env, &mut env_len)?;
    let import_env_end = env_len;
    load_local_sigs(module, src, &mut env, &mut env_len)?;

    Ok(DriverEnv {
        st_buf,
        st_len,
        env,
        env_len,
        builtin_env_end,
        import_env_end,
    })
}

pub fn emit_ir_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    target: Target,
    descriptor: Option<&CompiledDescriptor>,
    out: &mut Stdout,
) -> i32 {
    let es = match init_env(module, src, search_dirs, target) {
        Ok(e) => e,
        Err(code) => {
            let _ = diag::error_simple(code, b"environment init failed");
            return 2;
        }
    };

    match semantics::typecheck::emit_ir(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        allow_raw_casts,
        descriptor,
        out,
    ) {
        Ok(()) => 0,
        Err(e) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            2
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emit_asm_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    debug_trap_loc: bool,
    target: Target,
    input_path: &[u8],
    feature_set: FeatureSet,
    descriptor: Option<&CompiledDescriptor>,
    mmio_apertures: &[MmioApertureSpec],
    out: &mut Stdout,
) -> i32 {
    let es = match init_env(module, src, search_dirs, target) {
        Ok(e) => e,
        Err(code) => {
            let _ = diag::error_simple(code, b"environment init failed");
            return 2;
        }
    };

    let mut gen_backend = codegen_x86_64::X86_64HostedBackend::new(
        module,
        src,
        out,
        debug_trap_loc,
        AsmMode::Executable,
    );
    if let Err(e) = gen_backend.set_mmio_apertures(mmio_apertures) {
        let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
        return 2;
    }
    let gen: &mut dyn CodegenBackend = &mut gen_backend;
    if let Err(e) = gen.emit_prelude() {
        let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
        return 2;
    }

    let mut resources = match semantics::typecheck::db::build_resource_db(module, src) {
        Ok(r) => r,
        Err(e) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
    };
    let mut gate_hit = false;
    match semantics::typecheck::for_each_ir_word(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        allow_raw_casts,
        &mut resources,
        descriptor,
        None, // --emit=asm: no obligation extraction (P2 scope is obl/obj)
        None,
        |w, _ctx| {
            // Feature gate check — reject gated ops before codegen.
            if check_word_for_gate(w, feature_set, input_path, src) {
                gate_hit = true;
                return Ok(()); // skip codegen for this word, continue processing
            }
            gen.emit_word(w)
        },
    ) {
        Ok(()) => {}
        Err(semantics::typecheck::ForEachIrError::Type(e)) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
        Err(semantics::typecheck::ForEachIrError::Consumer(e)) => {
            let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
            return 2;
        }
    }

    if gate_hit {
        return 2;
    }

    if let Err(e) = gen.emit_postlude() {
        let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
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
    target: Target,
    descriptor: Option<&CompiledDescriptor>,
    out: &mut Stdout,
) -> i32 {
    let es = match init_env(module, src, search_dirs, target) {
        Ok(e) => e,
        Err(code) => {
            let _ = diag::error_simple(code, b"environment init failed");
            return 2;
        }
    };

    match semantics::typecheck::emit_stackcheck(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        descriptor,
        out,
    ) {
        Ok(()) => 0,
        Err(e) => {
            let _ = allow_raw_casts; // keep signature stable vs other drivers
            let _ = diag::error_simple(e.code(), b"typecheck error");
            2
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emit_obj_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    debug_trap_loc: bool,
    out_dir: &[u8],
    target: Target,
    is_lib: bool,
    input_path: &[u8],
    feature_set: FeatureSet,
    descriptor: Option<&CompiledDescriptor>,
    mmio_apertures: &[MmioApertureSpec],
    write_obl: bool,
    verdicts: Option<&verifier::verdict::Verdicts>,
) -> i32 {
    let module_name = slice_span(src, module.name);
    // P2/P4: extract obligations alongside the object and write
    // `<Module>.obl.json`. Extraction runs under `--write-obl` (the default
    // 'tyu build' path) and is IMPLIED by `--checks=undischarged` — the
    // emission decision consults each site's resolved verdict, so every site
    // must be recorded. `None` keeps the default fast path exactly today's
    // path (FR-22).
    let undischarged = checks == ChecksMode::Undischarged;
    let mut extract_ctx = (write_obl || undischarged).then(|| ExtractionCtx::new(module_name));
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

    let es = match init_env(module, src, search_dirs, target) {
        Ok(e) => e,
        Err(code) => {
            let _ = diag::error_simple(code, b"environment init failed");
            return 2;
        }
    };
    let builtin_env_end = es.builtin_env_end;
    let import_env_end = es.import_env_end;

    let mut mem = match MemOut::new() {
        Ok(m) => m,
        Err(_) => {
            let _ = diag::error_simple(1014, b"out of memory");
            return 2;
        }
    };

    // Compute ABI hash before creating the backend (shared across targets).
    let spec = target.spec();
    let abi_hash_val = abi_hash::compute_abi_hash(
        spec.calling_conv.arch_tag(),
        spec.slot_bytes,
        spec.word_bits,
        modinfo::MODINFO_VER,
    );

    // Create target-appropriate backend.
    let platform_hash = descriptor.map(|d| d.platform_hash).unwrap_or(0);
    let mut gen: Backend = match target {
        Target::X86_64UnknownLinuxGnu | Target::X86_64UnknownNone => {
            let mut bk = codegen_x86_64::X86_64HostedBackend::new(
                module,
                src,
                &mut mem,
                debug_trap_loc,
                AsmMode::Object,
            );
            if let Err(e) = bk.set_mmio_apertures(mmio_apertures) {
                let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
                return 2;
            }
            bk.set_expected_abi_hash(abi_hash_val);
            bk.set_platform_hash(platform_hash);
            Backend::X86(bk)
        }
        Target::ArmV7MUnknownNone => {
            let mut bk = codegen_arm::ArmThumbBackend::new(
                module,
                src,
                &mut mem,
                debug_trap_loc,
                AsmMode::Object,
            );
            if let Err(e) = bk.set_mmio_apertures(mmio_apertures) {
                let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
                return 2;
            }
            bk.set_expected_abi_hash(abi_hash_val);
            bk.set_platform_hash(platform_hash);
            Backend::Arm(bk)
        }
        Target::RiscV32UnknownNone => {
            let mut bk = codegen_riscv::RiscVBackend::new(
                module,
                src,
                &mut mem,
                debug_trap_loc,
                AsmMode::Object,
            );
            if let Err(e) = bk.set_mmio_apertures(mmio_apertures) {
                let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
                return 2;
            }
            bk.set_expected_abi_hash(abi_hash_val);
            bk.set_platform_hash(platform_hash);
            Backend::RiscV(bk)
        }
    };

    if let Err(e) = gen.emit_prelude() {
        let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
        return 2;
    }

    // Emit extrn declarations for all imported word symbols so the assembler
    // can resolve cross-module calls at link time.
    for i in builtin_env_end..import_env_end {
        if let Err(e) = gen.emit_extern_word(es.env[i].name.as_bytes()) {
            let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
            return 2;
        }
    }

    let mut resources = match semantics::typecheck::db::build_resource_db(module, src) {
        Ok(r) => r,
        Err(e) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
    };
    let mut gate_hit = false;
    // P4: emitted mmio-bounds checks observed at the codegen boundary (words
    // whose checks stayed: any open access, or a quotation word — the echo's
    // `emitted.mmio_bounds` honesty count, FR-15).
    let mut emitted_mmio: u32 = 0;
    match semantics::typecheck::for_each_ir_word(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        allow_raw_casts,
        &mut resources,
        descriptor,
        extract_ctx.as_mut(),
        verdicts,
        |w, ctx| {
            if check_word_for_gate(w, feature_set, input_path, src) {
                gate_hit = true;
                return Ok(());
            }
            // P4 (Q8): the per-word mmio elision signal. Only armed under
            // `--checks=undischarged`, and only when every mmio-bounds
            // obligation recorded for THIS word was discharged — a word with
            // any open access retains all its checks (FR-13). Quotation words
            // (`_quot_*`) record no obligations (P2), so they always keep
            // their checks — conservative, never an elision without a
            // discharge record.
            let elide_mmio = undischarged
                && !w.name.as_bytes().starts_with(b"_quot_")
                && ctx.map(|c| c.word_mmio_all_discharged()).unwrap_or(false);
            gen.set_mmio_checks_discharged(elide_mmio);
            if !elide_mmio {
                emitted_mmio = emitted_mmio.saturating_add(count_mmio_ops(w));
            }
            gen.emit_word(w)
        },
    ) {
        Ok(()) => {}
        Err(semantics::typecheck::ForEachIrError::Type(e)) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
        Err(semantics::typecheck::ForEachIrError::Consumer(e)) => {
            let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
            return 2;
        }
    }

    // Entry gate: only executables require `main` returning one value. It runs
    // AFTER the typecheck pass so a semantic error (e.g. E5030 ISR budget) is
    // reported as itself instead of masking as "missing word: main"; asm is
    // buffered to `mem` and only written after emit_postlude, so a gate
    // failure here still leaves no partial output.
    if !is_lib {
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
            let _ = diag::error_simple(
                1018,
                b"for --emit=obj, main must return exactly one value (exit code)",
            );
            return 2;
        }
    }

    if gate_hit {
        return 2;
    }

    if let Err(e) = gen.emit_postlude() {
        let _ = diag::error_simple(e.code(), codegen_error_message(e.code()));
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

    let assembler_bin: &[u8] = match target.spec().assembler {
        codegen_core::AssemblerKind::Fasm => b"fasm",
        codegen_core::AssemblerKind::GasArm => b"arm-none-eabi-as",
        codegen_core::AssemblerKind::GasRiscV => b"riscv32-elf-as",
    };
    let mut asm_args: Vec<&[u8]> = Vec::new();
    match target.spec().assembler {
        codegen_core::AssemblerKind::Fasm => {
            asm_args.push(asm_path);
            asm_args.push(obj_path);
        }
        codegen_core::AssemblerKind::GasArm | codegen_core::AssemblerKind::GasRiscV => {
            if matches!(
                target.spec().assembler,
                codegen_core::AssemblerKind::GasRiscV
            ) {
                asm_args.push(b"-march=rv32im");
                asm_args.push(b"-mabi=ilp32");
            }
            asm_args.push(b"-o");
            asm_args.push(obj_path);
            asm_args.push(asm_path);
        }
    }
    let status = process::run(assembler_bin, &asm_args)
        .map_err(|_| diag::error_simple(1016, b"failed to run assembler"));
    let status = match status {
        Ok(s) => s,
        Err(_) => return 2,
    };
    if status.code != 0 {
        let _ = diag::error_simple(1017, b"assembler failed");
        return 2;
    }

    // P2: `--write-obl` — write the obligation artifact only on full success
    // (no partial outputs on failure). The extraction ran in the same
    // lowering pass as codegen above.
    if let Some(ctx) = extract_ctx.as_mut() {
        let mut obl_buf = [0u8; 512];
        let obl_path = match join_path(&mut obl_buf, out_dir, module_name, b".obl.json") {
            Some(p) => p,
            None => {
                let _ = diag::error_simple(1013, b"output path too long");
                return 2;
            }
        };
        match verifier::codec::encode_obl(ctx.set()) {
            Ok(bytes) => {
                if fs::write_file(obl_path, &bytes).is_err() {
                    let _ = diag::error_simple(1015, b"failed to write output .obl.json");
                    return 2;
                }
            }
            Err(_) => {
                let _ = diag::error_simple(6401, b"failed to encode .obl.json artifact");
                return 2;
            }
        }

        // P4: verdict echo `<Module>.verdicts.inTree.json` — the resolved
        // (non-open) verdicts, the module's stale-verdicts count, and the
        // honest `emitted` check accounting (FR-15). The echo is itself a
        // valid `--verdicts` input, which is what makes tyu's `.tyu-verify`
        // cache round-trip (Q11/§7.4); consumers that do not participate keep
        // the default code path (FR-22).
        let resolved = ctx.resolved();
        let mut records: alloc::vec::Vec<verifier::verdict::VerdictRecord> =
            alloc::vec::Vec::new();
        for r in resolved.iter() {
            if r.status.is_open() {
                continue;
            }
            records.push(verifier::verdict::VerdictRecord {
                id: r.id.clone(),
                id_hash: r.id_hash.clone(),
                status: r.status,
                method: r.method.clone(),
                proof_ref: None,
                justification: r.justification.clone(),
            });
        }
        let stale = match verdicts {
            Some(v) => v.stale_count(&ctx.set().obligations),
            None => 0,
        };
        let subtype_emitted = resolved
            .iter()
            .filter(|r| {
                r.kind == verifier::model::Kind::SubtypeRange && r.status.is_open()
            })
            .count() as u32;
        let emitted = verifier::verdict::EmittedChecksData {
            subtype_range: subtype_emitted,
            contract: ctx.contract_emitted(),
            mmio_bounds: emitted_mmio,
        };
        let echo = match verifier::verdict::encode_verdicts(
            "langc",
            "0.1.0",
            &records,
            stale,
            &emitted,
        ) {
            Ok(b) => b,
            Err(_) => {
                let _ = diag::error_simple(6402, b"failed to encode verdicts echo");
                return 2;
            }
        };
        let mut echo_buf = [0u8; 512];
        let echo_path = match join_path(&mut echo_buf, out_dir, module_name, b".verdicts.inTree.json")
        {
            Some(p) => p,
            None => {
                let _ = diag::error_simple(1013, b"output path too long");
                return 2;
            }
        };
        if fs::write_file(echo_path, &echo).is_err() {
            let _ = diag::error_simple(1015, b"failed to write output verdicts echo");
            return 2;
        }
    }

    0
}

/// `--emit=obligations` (static-verification.md slice P2): run the full
/// lowering with obligation extraction and write `<Module>.obl.json` — no
/// codegen. The artifact is complete for every C1/C2/C3 subtype-range site of
/// the module's declared words regardless of `--checks` (FR-1).
#[allow(clippy::too_many_arguments)]
pub fn emit_obl_driver(
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
    checks: ChecksMode,
    allow_raw_casts: bool,
    target: Target,
    descriptor: Option<&CompiledDescriptor>,
    out_dir: &[u8],
) -> i32 {
    let module_name = slice_span(src, module.name);
    let mut obl_buf = [0u8; 512];
    let obl_path = match join_path(&mut obl_buf, out_dir, module_name, b".obl.json") {
        Some(p) => p,
        None => {
            let _ = diag::error_simple(1013, b"output path too long");
            return 2;
        }
    };

    let es = match init_env(module, src, search_dirs, target) {
        Ok(e) => e,
        Err(code) => {
            let _ = diag::error_simple(code, b"environment init failed");
            return 2;
        }
    };

    let mut resources = match semantics::typecheck::db::build_resource_db(module, src) {
        Ok(r) => r,
        Err(e) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
    };

    let mut ctx = ExtractionCtx::new(module_name);
    match semantics::typecheck::for_each_ir_word(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        allow_raw_casts,
        &mut resources,
        descriptor,
        Some(&mut ctx),
        None,
        |_w, _ctx| Ok::<(), ()>(()),
    ) {
        Ok(()) => {}
        Err(semantics::typecheck::ForEachIrError::Type(e)) => {
            let _ = diag::error_simple(e.code(), b"typecheck error");
            return 2;
        }
        // The extraction consumer is a no-op — the Consumer arm is unreachable.
        Err(semantics::typecheck::ForEachIrError::Consumer(())) => {
            let _ = diag::error_simple(6401, b"obligation extraction consumer failed");
            return 2;
        }
    }

    let bytes = match verifier::codec::encode_obl(ctx.set()) {
        Ok(b) => b,
        Err(_) => {
            let _ = diag::error_simple(6401, b"failed to encode .obl.json artifact");
            return 2;
        }
    };
    if fs::write_file(obl_path, &bytes).is_err() {
        let _ = diag::error_simple(1015, b"failed to write output .obl.json");
        return 2;
    }
    0
}

fn add_builtins(env: &mut [WordEntry; 256], len: &mut usize, _spec: &codegen_core::TargetSpec) {
    for w in semantics::typecheck::builtin_words() {
        if *len >= env.len() {
            return;
        }
        env[*len] = *w;
        *len += 1;
    }
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
            let sig =
                typecheck::parse_word_sig(def_src.as_slice(), sig_span).map_err(|_| 2205u32)?;
            if *env_len >= env.len() {
                return Err(2207u32); // too many imported words (max 256 total)
            }
            let name_atom = TypeAtom::new(name).ok_or(2206u32)?;
            let bound = if d.effect_net != 0 || d.effect_high != 0 {
                StackBound {
                    net: d.effect_net,
                    high: if d.effect_high == u32::MAX {
                        High::Top
                    } else {
                        High::Slots(d.effect_high)
                    },
                }
            } else {
                StackBound::ID
            };
            env[*env_len] = WordEntry {
                name: name_atom,
                sig,
                performs: if d.effect_bits != 0 {
                    EffectSet::from_bits(d.effect_bits)
                } else {
                    EffectSet::empty()
                },
                requires: CapSet::empty(),
                bound,
            };
            *env_len += 1;
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
        if *env_len >= env.len() {
            return Err(2223u32); // too many words in module (max 256)
        }
        let name_atom = TypeAtom::new(name).ok_or(2220u32)?;
        let bound = if d.effect_net != 0 || d.effect_high != 0 {
            StackBound {
                net: d.effect_net,
                high: if d.effect_high == u32::MAX {
                    High::Top
                } else {
                    High::Slots(d.effect_high)
                },
            }
        } else {
            StackBound::ID
        };
        env[*env_len] = WordEntry {
            name: name_atom,
            sig,
            performs: if d.effect_bits != 0 {
                EffectSet::from_bits(d.effect_bits)
            } else {
                EffectSet::empty()
            },
            requires: CapSet::empty(),
            bound,
        };
        *env_len += 1;
    }
    Ok(())
}

use crate::codegen::{AsmMode, Backend, CodegenBackend};
use crate::iface::{export_iter, find_decl, find_word_decl};
use crate::util::{
    check_word_for_gate, join_path, slice_span, try_load_module_file, MemOut, Stdout,
};
use alloc::string::ToString;
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
use verifier::model::{ExtractionCtx, VerdictSource};

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
        contract_hash: 0,
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
        feature_set.contains(codegen_core::Feature::ModuleLoading),
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
        feature_set.contains(codegen_core::Feature::ModuleLoading),
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
        // Slice P6 (Q7): fill the contract obligations' transcluded predicate
        // refs from the callee modules' `.obl.json` facts (E6413 on a stale
        // interface). Runs before any artifact is written — a failure leaves
        // no partial outputs.
        if let Err(code) = transclude_contract_predicates(ctx, module, src, search_dirs) {
            let _ = diag::error_simple(code, b"contract predicate interface stale (E6413)");
            return 2;
        }
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
        // P5: provably-failing records (interval ∅ vs the target range, check
        // retained) and open-reason records (FR-18 quality bar) ride the echo
        // so the report's `no-open` diagnostics carry the why.
        let provably_failing: alloc::vec::Vec<verifier::verdict::ProvablyFailingRecord> =
            resolved
                .iter()
                .filter(|r| r.provably_failing)
                .map(|r| verifier::verdict::ProvablyFailingRecord {
                    id: r.id.clone(),
                    note: r
                        .reason
                        .clone()
                        .unwrap_or_else(|| "interval ∩ type range = ∅".to_string()),
                })
                .collect();
        let open_reasons: alloc::vec::Vec<verifier::verdict::OpenReasonRecord> = resolved
            .iter()
            .filter(|r| r.status.is_open() && r.reason.is_some())
            .map(|r| verifier::verdict::OpenReasonRecord {
                id: r.id.clone(),
                reason: r.reason.clone().unwrap_or_default(),
            })
            .collect();
        // The discharge-source split (slice P5): how many closed verdicts
        // came from the in-tree dischargers vs the verdicts file.
        let in_tree_verdicts = resolved
            .iter()
            .filter(|r| !r.status.is_open() && r.source == VerdictSource::InTree)
            .count() as u32;
        let echo = match verifier::verdict::encode_echo(
            "langc",
            "0.1.0",
            &records,
            stale,
            &emitted,
            &provably_failing,
            &open_reasons,
            in_tree_verdicts,
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
        false,
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

    // Slice P6 (Q7): transclude contract-predicate refs before encoding.
    if let Err(code) = transclude_contract_predicates(&mut ctx, module, src, search_dirs) {
        let _ = diag::error_simple(code, b"contract predicate interface stale (E6413)");
        return 2;
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

/// Parse a `[ name, name ]` contract-clause span into its predicate names
/// (slice P6, Q6/Q7). The clause's bracketed content is a comma-separated
/// list of callee-module-local predicate word names — the same capture the
/// lowering compiles inline for the callee, re-read here as names so the
/// `.def` boundary carries them (Q7: `.def` uses names, never bodies). Only
/// clean identifiers count: an inline quotation body (`dup 0 >=`) is not a
/// name and contributes nothing to the named surface.
fn contract_clause_names<'s>(src: &'s [u8], span: Option<frontend::span::Span>) -> alloc::vec::Vec<&'s [u8]> {
    let mut out: alloc::vec::Vec<&'s [u8]> = alloc::vec::Vec::new();
    let Some(span) = span else { return out };
    if span.end <= span.start + 2 {
        return out;
    }
    let inner = &src[span.start + 1..span.end - 1];
    for segment in core::str::from_utf8(inner).unwrap_or("").split(',') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let bytes = trimmed.as_bytes();
        if bytes.len() <= 32 && !bytes.iter().any(|b| b.is_ascii_whitespace()) {
            out.push(bytes);
        }
    }
    out
}

/// The word's contract-surface hash (slice P6): Fnv-1a of its `needs` +
/// `ensures` predicate names; 0 = no contract clause.
fn word_contract_hash(
    src: &[u8],
    needs: Option<frontend::span::Span>,
    ensures: Option<frontend::span::Span>,
) -> u64 {
    let needs_names = contract_clause_names(src, needs);
    let ensures_names = contract_clause_names(src, ensures);
    if needs_names.is_empty() && ensures_names.is_empty() {
        return 0;
    }
    ir::contract::contract_hash(&needs_names, &ensures_names)
}

// ---------------------------------------------------------------------------
// Slice P6 (Q6/Q7): contract-predicate transclusion.
// ---------------------------------------------------------------------------

/// The imported-symbol → module-name table (`<imports>`), used to resolve a
/// contract callee's predicate facts to its module's `.obl.json` via the
/// same include-dir search `.def` uses (Q7 — `try_load_module_file` pattern).
struct ImportTable {
    names: alloc::vec::Vec<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)>,
}

impl ImportTable {
    fn build(module: &ModuleAst, src: &[u8]) -> ImportTable {
        let mut names = alloc::vec::Vec::new();
        for imp in module.imports.iter() {
            let mname = slice_span(src, imp.module).to_vec();
            for s in imp.names.iter() {
                names.push((slice_span(src, *s).to_vec(), mname.clone()));
            }
        }
        ImportTable { names }
    }

    fn module_of(&self, word: &[u8]) -> Option<&[u8]> {
        self.names
            .iter()
            .find(|(w, _)| w.as_slice() == word)
            .map(|(_, m)| m.as_slice())
    }
}

/// Slice P6 (Q6/Q7): fill the `PredicateHolds` formula's transcluded
/// `PredicateRef` on every contract obligation the lowering recorded.
///
/// At record time the ref is a lookup stub (`pred.name` = the *callee word
/// name* for `contract-pre`, empty for `contract-post` — the callee is the
/// record's own `site.word`). This pass resolves, for each stub:
///   1. the callee's `needs`/`ensures` clause names (from the module's own
///      decl, or the imported module's hand-authored `.def` — names only,
///      per Q7: `.def` never carries bodies or bounds);
///   2. the predicate's compiler-computed IR + `ir_hash` (from the callee
///      module's `.obl.json` `facts.predicates` — the artifact, never
///      hand-declared);
///   3. a missing predicate fact is a *stale interface* (E6413): the `.def`
///      declares a contract the callee's artifact does not document.
fn transclude_contract_predicates(
    ctx: &mut ExtractionCtx,
    module: &ModuleAst,
    src: &[u8],
    search_dirs: &[&[u8]],
) -> Result<(), u32> {
    let imports = ImportTable::build(module, src);
    let mut cached: alloc::vec::Vec<(alloc::vec::Vec<u8>, Option<verifier::model::OblSet>)> =
        alloc::vec::Vec::new();
    let out = ctx.set_mut();
    for o in out.obligations.iter_mut() {
        let (kind, stub) = match &mut o.formula {
            verifier::model::Formula::PredicateHolds { pred, .. } => (o.kind, pred),
            _ => continue,
        };
        if !stub.ir_hash.is_empty() {
            continue; // already transcluded
        }
        // The callee word's name: `contract-pre` records carry it in the
        // stub (the called word); `contract-post`'s callee is the record's
        // own site word (the word whose `ensures` this is).
        let callee: alloc::vec::Vec<u8> = if kind == verifier::model::Kind::ContractPre {
            stub.name.clone().into_bytes()
        } else {
            o.site.word.clone().into_bytes()
        };
        let is_needs = kind == verifier::model::Kind::ContractPre;
        let resolved = resolve_predicate_name(module, src, search_dirs, &imports, &callee, is_needs)?;
        let Some((pred_name, callee_module)) = resolved else {
            // Inline (unnamed) predicate clause: nothing to transclude — the
            // record stays an opaque inline reference; its verdict resolves
            // open (the runtime check is retained) unless a verdicts file
            // closes it by id.
            continue;
        };
        // The predicate's compiler-computed fact: the callee module's
        // artifact. Cache per module (a module has few imported deps).
        let obl = load_module_obl(&mut cached, search_dirs, callee_module.as_deref())?;
        let Some(obl) = obl else {
            // Facts unavailable (module compiled without extraction): fail
            // closed to *open* — the runtime check remains (R6 is a hard
            // error only for a *declared-then-missing* predicate).
            stub.name = pred_name;
            stub.module = callee_module.unwrap_or_default();
            continue;
        };
        let fact = obl
            .facts
            .predicates
            .iter()
            .find(|p| p.name.as_bytes() == pred_name.as_bytes());
        let Some(fact) = fact else {
            // E6413: the `.def`/decl declares `pred_name` as a contract
            // predicate, but the callee module's artifact has no such
            // predicate — stale interface (the callee's contract surface
            // drifted without the caller knowing).
            return Err(6413u32);
        };
        stub.name = pred_name;
        stub.module = callee_module.unwrap_or_default();
        stub.ir = fact.ir.clone();
        stub.ir_hash = fact.ir_hash.clone();
        // The discharge relies on the transcluded predicate (T2) — list it
        // in the record's trusted assumptions.
        if !o.assumptions.iter().any(|a| {
            matches!(a, verifier::model::Assumption::ContractPredicate { name, .. }
                if *name == stub.name)
        }) {
            o.assumptions.push(verifier::model::Assumption::ContractPredicate {
                module: stub.module.clone(),
                name: stub.name.clone(),
                ir_hash: stub.ir_hash.clone(),
            });
        }
    }
    Ok(())
}

/// Resolve the predicate NAME a callee's contract clause declares, plus the
/// callee's defining module when imported. `needs=true` reads the `needs`
/// clause, else `ensures`. The clause is a `[ name, … ]` quote span; v1
/// uses the FIRST declared predicate per clause (multi-name clauses are
/// parsed and hashed in full for the ABI surface, but obligations name the
/// first).
fn resolve_predicate_name<'s>(
    module: &'s ModuleAst,
    src: &'s [u8],
    search_dirs: &[&[u8]],
    imports: &ImportTable,
    callee: &[u8],
    is_needs: bool,
) -> Result<Option<(alloc::string::String, Option<alloc::string::String>)>, u32> {
    if let Some(d) = find_decl(module, src, callee) {
        let span = if is_needs { d.requires } else { d.ensures };
        let names = contract_clause_names(src, span);
        let Some(first) = names.first().copied() else {
            // Inline (unnamed) clause — the record stays an opaque inline
            // predicate reference; nothing to transclude.
            return Ok(None);
        };
        return Ok(Some((verifier::model::utf8_lossy(first), None)));
    }
    // Imported: resolve the declaring module from the import table, then
    // read its hand-authored `.def` (names only — Q7, no bodies, no bounds).
    let Some(mname) = imports.module_of(callee) else {
        // Not declared here and not imported — the import machinery would
        // have failed earlier; open, not an error.
        return Err(2203u32);
    };
    let def_src = try_load_module_file(search_dirs, mname, b".def").ok_or(2201u32)?;
    let def_ast = Parser::new(def_src.as_slice())
        .parse_module_ast()
        .map_err(|_| 2202u32)?;
    let d = find_decl(&def_ast, def_src.as_slice(), callee).ok_or(2212u32)?;
    let span = if is_needs { d.requires } else { d.ensures };
    let names = contract_clause_names(def_src.as_slice(), span);
    let Some(first) = names.first().copied() else {
        return Ok(None);
    };
    Ok(Some((
        verifier::model::utf8_lossy(first),
        Some(verifier::model::utf8_lossy(mname)),
    )))
}

/// Load a module's `.obl.json` through the include-dir search, caching per
/// module name inside one pass. `None` = artifacts for that module are
/// unavailable (compiled without extraction) — fail-closed to open.
fn load_module_obl(
    cache: &mut alloc::vec::Vec<(alloc::vec::Vec<u8>, Option<verifier::model::OblSet>)>,
    search_dirs: &[&[u8]],
    module: Option<&str>,
) -> Result<Option<verifier::model::OblSet>, u32> {
    let key = module.unwrap_or_default();
    if let Some((_, hit)) = cache.iter().find(|(k, _)| k.as_slice() == key.as_bytes()) {
        return Ok(hit.clone());
    }
    let mut value: Option<verifier::model::OblSet> = None;
    if let Some(m) = module {
        if let Some(bytes) = try_load_module_file(search_dirs, m.as_bytes(), b".obl.json") {
            match verifier::codec::read_obl(bytes.as_slice()) {
                Ok(set) => value = Some(set),
                Err(e) => return Err(e.code()),
            }
        }
    }
    cache.push((key.as_bytes().to_vec(), value.clone()));
    Ok(value)
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
                // Slice P6 (Q7): the callee's contract surface folds into the
                // entry so call sites can record `contract-pre` obligations
                // and the ABI hash covers contract drift.
                contract_hash: word_contract_hash(def_src.as_slice(), d.requires, d.ensures),
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
            // Slice P6: local words' contract clauses feed call sites the
            // same way imported ones do (`contract-pre` records + ABI v2).
            contract_hash: word_contract_hash(src, d.requires, d.ensures),
        };
        *env_len += 1;
    }
    Ok(())
}

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
        |w| {
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
) -> i32 {
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
    match semantics::typecheck::for_each_ir_word(
        module,
        src,
        &es.env[..es.env_len],
        &es.st_buf[..es.st_len],
        checks,
        allow_raw_casts,
        &mut resources,
        descriptor,
        |w| {
            if check_word_for_gate(w, feature_set, input_path, src) {
                gate_hit = true;
                return Ok(());
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

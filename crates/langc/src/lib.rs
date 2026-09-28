#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
//! Single-module Tyu compiler CLI.
//!
//! The crate exposes the compiler driver and CLI-facing helpers in `args`,
//! `driver`, `iface`, `util`, and `main`.

extern crate alloc;

pub mod args;
mod codegen;
pub mod driver;
pub mod iface;
pub mod util;

use codegen_core::compiled_desc::{
    decode_compiled_desc, validate_compiled_desc, CompiledDescriptor,
};
use codegen_core::{EmitMode, MmioApertureSpec, Target};
use frontend::parse::{DeclKind, ModuleAst, Parser};
use hosted::{diag, fs};

use crate::iface::{check_program, iface_error_message};
use crate::util::{emit_parse_error, join_path, split_dir, Stdout};

/// # Safety
/// `argc`/`argv` must describe a valid C command-line argument array: `argc`
/// pointers at `argv`, each pointing to a NUL-terminated string valid for the
/// program's lifetime.
pub unsafe fn run(argc: isize, argv: *const *const hosted::c::c_char) -> i32 {
    let cfg = match unsafe { args::parse_args(argc, argv) } {
        args::ParseResult::Ok(c) => c,
        args::ParseResult::Help => return 0,
        args::ParseResult::Error(code) => return code,
    };

    let target = cfg.target.unwrap_or(Target::X86_64UnknownLinuxGnu);

    // Effective MMIO apertures: the compiled platform descriptor when
    // `--platform` is present (P3/P4, D-1), otherwise the target's static
    // defaults. Loaded once, before any emit that needs them.
    let mut compiled = CompiledDescriptor::default();
    let descriptor: Option<&CompiledDescriptor> =
        match load_descriptor(cfg.platform_dir, &mut compiled) {
            Ok(Some(())) => Some(&compiled),
            Ok(None) => None,
            Err(code) => return code,
        };
    let mmio_apertures: &[MmioApertureSpec] = match descriptor {
        Some(cd) => cd.apertures(),
        None => target.spec().mmio_apertures,
    };

    // P12.1 (§6.7/§Q3): the bundle's model-semantics identity. tyu forwards
    // the resolved pack's `[model] model_semantics`; a direct langc run
    // without a bundle keeps the §Q15 honest default (`unmodeled`).
    let model_semantics: &[u8] = cfg
        .model_semantics
        .unwrap_or(verifier::model::MODEL_UNMODELED.as_bytes());

    let buf = match fs::read_file(cfg.input) {
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
            emit_parse_error(cfg.input, src, e.code(), e.message(), e.span().start);
            return 2;
        }
    };

    // D-1 / E3640: a module containing any MMIO construct (register-map
    // declaration or instance) requires `--platform`. Raw bases are likewise
    // rejected under a descriptor (E3641) and symbolic bases require one
    // (E3640) — both enforced in the typechecker; this gate fails fast.
    if descriptor.is_none() && module_has_mmio_construct(&module) {
        let _ = diag::error_simple(
            3640,
            b"module uses MMIO but was compiled without --platform=<dir>",
        );
        return 2;
    }

    let mut base_dir_buf = [0u8; 512];
    let base_dir = split_dir(cfg.input, &mut base_dir_buf);
    let mut search_dirs: [&[u8]; 11] = [&[]; 11];
    search_dirs[0] = base_dir;
    search_dirs[1..(cfg.include_len + 1)].copy_from_slice(&cfg.include_dirs[..cfg.include_len]);
    let mut search_len = 1 + cfg.include_len;

    let sysroot = cfg
        .sysroot
        .or_else(|| unsafe { hosted::env::get_str(b"LANG_SYSROOT") });
    let mut sysroot_target_buf = [0u8; 512];
    if let Some(sr) = sysroot {
        if search_len < search_dirs.len() {
            search_dirs[search_len] = sr;
            search_len += 1;
        }
        if search_len < search_dirs.len() {
            if let Some(p) = join_path(&mut sysroot_target_buf, sr, target.triple(), b"") {
                search_dirs[search_len] = p;
                search_len += 1;
            }
        }
    }

    if let Err(code) = check_program(&module, src, &search_dirs[..search_len]) {
        let _ = diag::error_simple(code, iface_error_message(code));
        return 2;
    }

    // P4: `--verdicts=<file>` — load and schema-validate the verdicts file
    // before any codegen. Fail-closed (§7.5): a malformed, wrong-schema, or
    // wrong-semantics file is E6402 (a closed-registry violation E6417) and
    // aborts the compile (never a silent best-effort read — a bad file can
    // only cause *more* checking). The producer-recognition gate (§Q6) runs
    // here: an unrecognized certifier's `proof`/`checked` labels downgrade to
    // `assumed` (with `claimed` preserved) before any site resolution.
    let verdicts: Option<verifier::verdict::Verdicts> = match cfg.verdicts {
        Some(path) => {
            let bytes = match fs::read_file(path) {
                Ok(b) => b,
                Err(_) => {
                    let _ = diag::error_simple(6402, b"cannot read --verdicts file");
                    return 2;
                }
            };
            match verifier::verdict::read_verdicts(bytes.as_slice()) {
                Ok(v) => Some(v.restrict_to_recognized()),
                Err(e) => {
                    let _ = diag::error_simple(e.code(), b"invalid --verdicts file");
                    return 2;
                }
            }
        }
        None => None,
    };

    // PLAN-VERIFY-3 P7.3 (FR-5): `--bind-obl=<file>` — the pass-1 obligation
    // artifact the statement binding recomputes against (the artifact the
    // proofs were certified against; its word IR is verdict-independent,
    // unlike the live lowering's). Fail-closed: a missing or schema-invalid
    // artifact aborts before codegen (a bad binding surface can only cause
    // more checking — every certificate record then fails to bind).
    let bind_obl: Option<verifier::model::OblSet> = match cfg.bind_obl {
        Some(path) => {
            let bytes = match fs::read_file(path) {
                Ok(b) => b,
                Err(_) => {
                    let _ = diag::error_simple(6401, b"cannot read --bind-obl artifact");
                    return 2;
                }
            };
            match verifier::codec::read_obl(bytes.as_slice()) {
                Ok(s) => Some(s),
                Err(e) => {
                    let _ = diag::error_simple(e.code(), b"invalid --bind-obl artifact");
                    return 2;
                }
            }
        }
        None => None,
    };

    // PLAN-VERIFY-3 P13.1: `--refinements=<path>` — the `tyu.refinements/1`
    // context document (the bundle's `[refinements]` device-refinement
    // manifest) relativizing the FR-5 statement-binding recompute. Fail-
    // closed: a missing or malformed document aborts (the refinement context
    // is part of the statement identity — a broken context would silently
    // bind statements to the wrong claim).
    let refinements: alloc::vec::Vec<verifier::refinements::Refinement> = match cfg.refinements {
        Some(path) => {
            let bytes = match fs::read_file(path) {
                Ok(b) => b,
                Err(_) => {
                    let _ = diag::error_simple(6401, b"cannot read --refinements document");
                    return 2;
                }
            };
            match verifier::refinements::parse_manifest(
                core::str::from_utf8(bytes.as_slice()).unwrap_or(""),
            ) {
                Some(r) => r,
                None => {
                    let _ = diag::error_simple(6401, b"invalid --refinements document");
                    return 2;
                }
            }
        }
        None => alloc::vec::Vec::new(),
    };

    let mut out = Stdout;
    match cfg.emit {
        EmitMode::Ast => match Parser::new(src).parse_module_dump(&mut out) {
            Ok(()) => 0,
            Err(e) => {
                emit_parse_error(cfg.input, src, e.code(), e.message(), e.span().start);
                2
            }
        },
        EmitMode::Ir => driver::emit_ir_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            target,
            descriptor,
            &mut out,
        ),
        EmitMode::StackCheck => driver::emit_tc_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            target,
            descriptor,
            &mut out,
        ),
        EmitMode::Asm => driver::emit_asm_driver(
            &module,
            src,
            &search_dirs[..search_len],
            cfg.checks,
            cfg.allow_raw_casts,
            cfg.debug_trap_loc,
            target,
            cfg.input,
            cfg.features,
            descriptor,
            mmio_apertures,
            cfg.verify_policy_proven,
            &mut out,
        ),
        EmitMode::Obj => {
            let out_dir = cfg.out_dir.unwrap_or(b".");
            driver::emit_obj_driver(
                &module,
                src,
                &search_dirs[..search_len],
                cfg.checks,
                cfg.allow_raw_casts,
                cfg.debug_trap_loc,
                out_dir,
                target,
                cfg.is_lib,
                cfg.input,
                cfg.features,
                descriptor,
                mmio_apertures,
                cfg.write_obl,
                verdicts.as_ref(),
                cfg.elide_ds_guards,
                cfg.verify_tool,
                cfg.verify_policy_proven,
                bind_obl.as_ref(),
                model_semantics,
                refinements.as_slice(),
            )
        }
        EmitMode::Obligations => {
            let out_dir = cfg.out_dir.unwrap_or(b".");
            driver::emit_obl_driver(
                &module,
                src,
                &search_dirs[..search_len],
                cfg.checks,
                cfg.allow_raw_casts,
                target,
                descriptor,
                out_dir,
                model_semantics,
            )
        }
    }
}

/// True when the module declares any MMIO construct (register-map or a
/// register-map instance), which requires a platform descriptor (D-1).
fn module_has_mmio_construct(module: &ModuleAst) -> bool {
    if !module.instances.is_empty() {
        return true;
    }
    module.decls.iter().any(|d| d.kind == DeclKind::RegisterMap)
}

/// Load the compiled platform descriptor from `<dir>/platform.desc`.
///
/// `Ok(None)` when no `--platform` was given. Any decode/validation failure is
/// a loud E3647, never a silent fallback.
fn load_descriptor(
    platform_dir: Option<&[u8]>,
    out: &mut CompiledDescriptor,
) -> Result<Option<()>, i32> {
    let Some(dir) = platform_dir else {
        return Ok(None);
    };
    let mut path_buf = [0u8; 512];
    let Some(desc_path) = join_path(&mut path_buf, dir, b"platform.desc", b"") else {
        let _ = diag::error_simple(3647, b"compiled descriptor path too long");
        return Err(2);
    };
    let bytes = match fs::read_file(desc_path) {
        Ok(b) => b,
        Err(_) => {
            let _ = diag::error_simple(
                3647,
                b"cannot read <dir>/platform.desc (run `tyu` to generate the compiled descriptor)",
            );
            return Err(2);
        }
    };
    let cd = match decode_compiled_desc(bytes.as_slice()) {
        Ok(cd) => cd,
        Err(e) => {
            let _ = diag::error_simple(3647, e.as_str().as_bytes());
            return Err(2);
        }
    };
    if let Err(e) = validate_compiled_desc(&cd) {
        let _ = diag::error_simple(3647, e.as_str().as_bytes());
        return Err(2);
    }
    *out = cd;
    Ok(Some(()))
}

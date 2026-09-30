//! The session compile path (PLAN-WASM-1 P2, §7.2/§Q12): source →
//! `frontend::Parser` → `semantics::typecheck::build_ir` → an owned
//! [`Compiled`] artifact (module + dictionary + L3 subtype table). The REPL
//! consumes the struct, never the rendered text (FR-2, DEC-2).
//!
//! `SourceTable` is the §Q12 filesystem seam: imported `.def` signatures are
//! resolved through it, so the crate stays `no_std`/fs-free and the browser
//! (P5) can back it with the sysroot blob. Unresolved dictionary names and
//! REPL-unexecutable constructs (MMIO without a descriptor, tasks) are
//! compile diagnostics here — never mis-execution (FR-3).

use alloc::vec::Vec;
use frontend::parse::{DeclKind, ModuleAst, Parser};
use frontend::span::Span;
use ir::{OpKind, StackBound};
use semantics::typecheck::{self, ChecksMode, SubtypeInfo};
use semantics::types::{TypeAtom, WordEntry};

use crate::builtins::Builtin;
use crate::diag::{code, Diag};

/// The §Q12 source seam: read a module file by its canonical path
/// (e.g. `platform/testio.def`). Host/CI implementations back this with a
/// directory or a memory blob; the engine never touches a filesystem.
pub trait SourceTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]>;
}

/// The hosted-reference `TargetSpec` the REPL defaults to (DEC-8): word 64,
/// pointer/slot 8, SysV64. The wasm target (P7) supplies its own spec.
pub const X86_64_LINUX_SPEC: verifier::target::TargetSpec =
    verifier::target::TargetSpec::new("x86_64-unknown-linux-gnu", 8, 64, 1);

/// A dictionary target: a module word or an interpreter builtin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DictTarget {
    Word(usize),
    Builtin(Builtin),
}

/// An L3 subtype range (the `SubtypeInfo` values, keyed by type name).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubtypeRange {
    pub name: Vec<u8>,
    pub base: Vec<u8>,
    pub min: i64,
    pub max: i64,
}

/// The compiled session artifact.
#[derive(Clone)]
pub struct Compiled {
    pub module: ir::Module,
    pub spec: verifier::target::TargetSpec,
    /// The session source (const strings are sliced from it at build/load).
    pub src: Vec<u8>,
    dict: Vec<(ir::Atom, DictTarget)>,
    subtypes: Vec<SubtypeRange>,
}

impl Compiled {
    pub fn dict_lookup(&self, name: &[u8]) -> Option<DictTarget> {
        self.dict
            .iter()
            .find(|(n, _)| n.as_bytes() == name)
            .map(|(_, t)| *t)
    }

    pub fn subtype_range(&self, name: &[u8]) -> Option<&SubtypeRange> {
        self.subtypes.iter().find(|s| s.name == name)
    }
}

/// Compile a session source buffer. `search_dirs` are the sysroot/include
/// roots the imports resolve through (first hit wins, ordered).
pub fn compile(
    source: &[u8],
    search_dirs: &[&dyn SourceTable],
    spec: verifier::target::TargetSpec,
) -> Result<Compiled, Diag> {
    let src: Vec<u8> = source.to_vec();
    let module = Parser::new(&src)
        .parse_module_ast()
        .map_err(|e| Diag::from_bytes(e.code(), e.span(), e.message()))?;

    // Reject constructs the REPL v1 cannot execute, at submit time (FR-3).
    if module_uses_mmio(&module) {
        return Err(Diag::from_bytes(
            3640, // E3640 parity with `langc` without `--platform`
            Span::UNKNOWN,
            b"module uses MMIO but the REPL was compiled without a platform descriptor",
        ));
    }

    let subtypes = subtype_infos(&src, &module)?;
    let env = build_env(&src, &module, search_dirs)?;

    let built = typecheck::build_ir(
        &module,
        &src,
        &env,
        &subtypes,
        ChecksMode::All,
        false, // allow_raw_casts
        spec,
        None, // descriptor (P4 wires platform packs)
    )
    .map_err(tc_diag)?;

    // FR-3: task/interrupt ops are rejected, never mis-executed (E9156).
    for w in built.words.iter() {
        for b in w.blocks.iter() {
            for op in b.ops.iter() {
                if let OpKind::TaskSpawn { .. }
                | OpKind::InterruptEnable
                | OpKind::InterruptDisable = op.kind
                {
                    return Err(Diag::from_bytes(
                        code::E9156_TASKS,
                        op.span,
                        b"task/interrupt constructs are not supported in the REPL",
                    ));
                }
            }
        }
    }

    // Dictionary: module words first, then interpreter builtins.
    let mut dict: Vec<(ir::Atom, DictTarget)> = Vec::new();
    for (i, w) in built.words.iter().enumerate() {
        dict.push((w.name, DictTarget::Word(i)));
    }
    for name in BUILTIN_NAMES {
        if let Some(b) = Builtin::from_name(name) {
            if let Some(atom) = ir::Atom::new(name) {
                if !dict.iter().any(|(n, _)| *n == atom) {
                    dict.push((atom, DictTarget::Builtin(b)));
                }
            }
        }
    }

    let ranges: Vec<SubtypeRange> = subtypes
        .into_iter()
        .filter(|s| s.min != 0 || s.max != 0 || s.base.as_bytes() != s.name.as_bytes())
        .map(|s| SubtypeRange {
            name: s.name.as_bytes().to_vec(),
            base: s.base.as_bytes().to_vec(),
            min: s.min,
            max: s.max,
        })
        .collect();

    Ok(Compiled {
        module: built,
        spec,
        src,
        dict,
        subtypes: ranges,
    })
}

const BUILTIN_NAMES: &[&[u8]] = &[
    b"testio.write-byte",
    b"testio.write-str",
    b"testio.exit",
    b"platform.mem.region-create",
    b"platform.mem.region-alloc",
    b"platform.mem.region-reset",
    b"platform.mem.region-destroy",
];

fn tc_diag(e: semantics::typecheck::TcError) -> Diag {
    let code = e.code();
    let span = e.span();
    let mut msg = alloc::vec::Vec::new();
    msg.extend_from_slice(b"typecheck error E");
    let mut n = code;
    let mut buf = [0u8; 10];
    let mut k = 0usize;
    if n == 0 {
        buf[0] = b'0';
        k = 1;
    }
    while n > 0 && k < buf.len() {
        buf[k] = b'0' + (n % 10) as u8;
        k += 1;
        n /= 10;
    }
    for &b in buf[..k].iter().rev() {
        msg.push(b);
    }
    Diag::new(code, span, msg)
}

fn module_uses_mmio(module: &ModuleAst) -> bool {
    // The descriptor-less MMIO gate, mirroring `langc`'s fail-fast
    // (`module_has_mmio_construct`): any register-map declaration or symbolic
    // instance requires a descriptor the REPL does not carry.
    !module.instances.is_empty()
        || module
            .decls
            .iter()
            .any(|d| matches!(d.kind, DeclKind::RegisterMap))
}

fn subtype_infos(src: &[u8], module: &ModuleAst) -> Result<Vec<SubtypeInfo>, Diag> {
    let mut out = Vec::new();
    for s in module.subtypes.iter() {
        let name = TypeAtom::new(&src[s.name.start..s.name.end])
            .ok_or_else(|| code_err(2021, "subtype name too long"))?;
        let base = TypeAtom::new(&src[s.base.start..s.base.end])
            .ok_or_else(|| code_err(2022, "subtype base name too long"))?;
        if out.len() >= 64 {
            return Err(code_err(2020, "too many subtypes (max 64)"));
        }
        out.push(SubtypeInfo {
            name,
            base,
            min: s.min,
            max: s.max,
        });
    }
    Ok(out)
}

/// The typecheck environment (mirrors langc's `init_env`): builtins + import
/// sigs (resolved via the `SourceTable` seam) + local sigs.
fn build_env(
    src: &[u8],
    module: &ModuleAst,
    search_dirs: &[&dyn SourceTable],
) -> Result<Vec<WordEntry>, Diag> {
    let mut env: Vec<WordEntry> = Vec::new();

    for w in typecheck::builtin_words() {
        if env.len() >= 256 {
            return Err(code_err(2207, "too many imported words (max 256 total)"));
        }
        env.push(*w);
    }

    // Imported signatures from the resolved `.def` files.
    for imp in module.imports.iter() {
        let mname = &src[imp.module.start..imp.module.end];
        let mut def_src: Option<Vec<u8>> = None;
        for dir in search_dirs {
            let path = join_path_bytes(mname, b".def");
            if let Some(b) = dir.read(&path) {
                def_src = Some(b.to_vec());
                break;
            }
        }
        let def_src = def_src.ok_or_else(|| {
            Diag::from_bytes(2201, imp.module, b"cannot open imported module (.def)")
        })?;
        let def_ast = Parser::new(&def_src)
            .parse_module_ast()
            .map_err(|_| code_err(2202, "imported .def does not parse"))?;

        for name in export_names(&def_ast, &def_src) {
            let Some(decl) = find_decl(&def_ast, &def_src, name) else {
                continue;
            };
            if decl.kind != DeclKind::Word {
                continue;
            }
            let Some(sig_span) = decl.sig else {
                continue;
            };
            let sig = typecheck::parse_word_sig(&def_src, sig_span)
                .map_err(|_| code_err(2205, "imported signature does not parse"))?;
            if env.len() >= 256 {
                return Err(code_err(2207, "too many imported words (max 256 total)"));
            }
            let name_atom =
                TypeAtom::new(name).ok_or_else(|| code_err(2206, "atom name too long"))?;
            env.push(WordEntry {
                name: name_atom,
                sig,
                performs: if decl.effect_bits != 0 {
                    ir::EffectSet::from_bits(decl.effect_bits)
                } else {
                    ir::EffectSet::empty()
                },
                requires: ir::CapSet::empty(),
                bound: decl_bound(decl),
                contract_hash: 0,
            });
        }
    }

    // Local signatures.
    for d in module.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        // langc parity: a sig-less local word is skipped here and rejected
        // by the typechecker (build_ir NoSig) — never a pre-check error.
        let Some(sig_span) = d.sig else {
            continue;
        };
        let sig = typecheck::parse_word_sig(src, sig_span)
            .map_err(|_| code_err(2219, "local signature does not parse"))?;
        if env.len() >= 256 {
            return Err(code_err(2223, "too many words in module (max 256)"));
        }
        let name = &src[d.name.start..d.name.end];
        let name_atom = TypeAtom::new(name).ok_or_else(|| code_err(2220, "atom name too long"))?;
        env.push(WordEntry {
            name: name_atom,
            sig,
            performs: if d.effect_bits != 0 {
                ir::EffectSet::from_bits(d.effect_bits)
            } else {
                ir::EffectSet::empty()
            },
            requires: ir::CapSet::empty(),
            bound: decl_bound(d),
            contract_hash: 0,
        });
    }
    Ok(env)
}

fn decl_bound(d: &frontend::parse::DeclAst) -> StackBound {
    if d.effect_net != 0 || d.effect_high != 0 {
        StackBound {
            net: d.effect_net,
            high: if d.effect_high == u32::MAX {
                ir::High::Top
            } else {
                ir::High::Slots(d.effect_high)
            },
        }
    } else {
        StackBound::ID
    }
}

fn join_path_bytes(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut p = alloc::vec::Vec::with_capacity(a.len() + b.len());
    p.extend_from_slice(a);
    p.extend_from_slice(b);
    p
}

fn slice_span(src: &[u8], span: Span) -> &[u8] {
    &src[span.start..span.end]
}

fn code_err(code: u32, msg: &str) -> Diag {
    Diag::from_bytes(code, Span::UNKNOWN, msg.as_bytes())
}

/// The exported word names of a module (langc `export_iter` mirror).
fn export_names<'a>(ast: &'a ModuleAst, src: &'a [u8]) -> Vec<&'a [u8]> {
    if ast.has_export_stmt {
        ast.exports.iter().map(|s| slice_span(src, *s)).collect()
    } else {
        ast.decls
            .iter()
            .filter(|d| d.kind == DeclKind::Word)
            .map(|d| slice_span(src, d.name))
            .collect()
    }
}

/// langc `find_decl` mirror.
fn find_decl<'a>(
    ast: &'a ModuleAst,
    src: &'a [u8],
    name: &[u8],
) -> Option<&'a frontend::parse::DeclAst> {
    ast.decls.iter().find(|d| slice_span(src, d.name) == name)
}
